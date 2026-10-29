use super::*;

fn apply_axes_layout_pairs(
    figure: &mut Figure,
    args: &[Value],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use crate::plot::PositionConstraint;
    if args.is_empty() {
        return Ok(());
    }
    if args.len() > 32 || !args.len().is_multiple_of(2) {
        return Err("Axes layout requires at most 16 name/value pairs".into());
    }
    let mut placement = figure.initial_axes_placement()?;
    let mut modern = None;
    let mut legacy = None;
    for pair in args.as_chunks::<2>().0 {
        let Value::Text(name) = &pair[0] else {
            return Err("Expected an axes property name".into());
        };
        let text = || match &pair[1] {
            Value::Text(s) => Ok(s.as_str()),
            _ => Err("Expected a quoted axes property value".to_owned()),
        };
        match name.to_ascii_lowercase().as_str() {
            "position" | "innerposition" | "outerposition" => {
                let values = pair[1].clone().matrix(cancel)?;
                if values.len() != 4 || (values.rows() != 1 && values.cols() != 1) { return Err("Axes rectangle must be a four-element vector".into()); }
                let rectangle = values.data().try_into().unwrap();
                crate::plot::Position::from_normalized(rectangle)?;
                if name.eq_ignore_ascii_case("outerposition") { placement.requested_outer = rectangle; }
                else { placement.requested_inner = rectangle; }
            }
            "positionconstraint" => { modern = Some(PositionConstraint::parse(text()?)?); }
            "activepositionproperty" => {
                legacy = Some(if text()?.eq_ignore_ascii_case("position") { PositionConstraint::InnerPosition }
                    else if text()?.eq_ignore_ascii_case("outerposition") { PositionConstraint::OuterPosition }
                    else { return Err("ActivePositionProperty must be position or outerposition".into()); });
            }
            "units" if text()?.eq_ignore_ascii_case("normalized") => {},
            "units" => return Err("Axes layout supports normalized Units only".into()),
            _ => return Err("Supported axes layout properties: Position, InnerPosition, OuterPosition, PositionConstraint, ActivePositionProperty and normalized Units".into()),
        }
    }
    if modern.is_some() && legacy.is_some() && modern != legacy {
        return Err("Conflicting axes position constraints".into());
    }
    if let Some(constraint) = modern.or(legacy) {
        placement.constraint = constraint;
    }
    figure.set_axes_placement(placement)
}
impl Engine {
    /// Install a validated recovery snapshot atomically, preserving variables.
    pub fn restore_figures(&mut self, current: u32, figures: Vec<(u32, Figure)>) -> Result<()> {
        if figures.len() > 16 {
            return Err("At most 16 recovered figures are allowed".into());
        }
        let mut restored = BTreeMap::new();
        for (id, figure) in figures {
            if !(1..=1_000_000).contains(&id) || restored.contains_key(&id) {
                return Err("Invalid or duplicate recovery figure number".into());
            }
            figure.validate()?;
            restored.insert(id, figure);
        }
        if !restored.is_empty() && !restored.contains_key(&current) {
            return Err("Active recovery figure is missing".into());
        }
        self.current_figure = if restored.is_empty() { 1 } else { current };
        self.figure = restored.remove(&self.current_figure);
        self.other_figures = restored;
        self.closed_figures.clear();
        self.figure_file_events.clear();
        Ok(())
    }

    /// Figure lifecycle notifications from the most recent script, including
    /// successful statements before an error. Draining releases the metadata.
    pub fn take_closed_figures(&mut self) -> std::collections::BTreeSet<u32> {
        std::mem::take(&mut self.closed_figures)
    }
    pub fn figures(&self) -> impl Iterator<Item = (u32, &Figure)> {
        self.other_figures
            .iter()
            .map(|(&id, f)| (id, f))
            .chain(self.figure.as_ref().map(|f| (self.current_figure, f)))
    }
    pub fn figure_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self.other_figures.keys().copied().collect();
        if self.figure.is_some() {
            ids.push(self.current_figure);
        }
        ids.sort_unstable();
        ids
    }
    /// Close exactly an existing document, never a later document reusing its number.
    pub fn close_existing_figure(
        &mut self,
        id: u32,
        identity: &crate::ObjectIdentity,
    ) -> Result<()> {
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Figure close cancelled".into());
        }
        if !self
            .figures()
            .any(|(n, f)| n == id && f.identity.same_object(identity))
        {
            return Err("This figure document is no longer open".into());
        }
        self.close_figure_id(id);
        Ok(())
    }
    fn close_figure_id(&mut self, id: u32) {
        if id == self.current_figure {
            self.figure_file_events.remove(&id);
            if self.figure.is_some() {
                self.closed_figures.insert(id);
            }
            self.figure = None;
            if let Some((&next, _)) = self.other_figures.first_key_value() {
                self.current_figure = next;
                self.figure = self.other_figures.remove(&next);
            }
        } else {
            self.figure_file_events.remove(&id);
            if self.other_figures.remove(&id).is_some() {
                self.closed_figures.insert(id);
            }
        }
    }
    pub fn select_figure(&mut self, id: u32) -> Result<()> {
        if id == 0 || id > 1_000_000 {
            return Err("Figure number must be 1..1000000".into());
        }
        if id == self.current_figure {
            self.figure.get_or_insert_with(Figure::default);
            return Ok(());
        }
        if !self.other_figures.contains_key(&id) && self.figure_ids().len() >= 16 {
            return Err("At most 16 figures can be open; close unused figures".into());
        }
        if let Some(current) = self.figure.take() {
            self.other_figures.insert(self.current_figure, current);
        }
        self.figure = Some(self.other_figures.remove(&id).unwrap_or_default());
        self.current_figure = id;
        Ok(())
    }
    pub(super) fn graphics(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        if matches!(name, "tiledlayout" | "nexttile" | "sgtitle") {
            return self.tiled_graphics(name, args);
        }
        if name == "figure" || name == "close" {
            if args.len() > 1 {
                return Err("figure/close accept at most one argument".into());
            }
            if name == "close" && matches!(args.first(),Some(Value::Text(s)) if s=="all") {
                self.closed_figures.extend(self.figure_ids());
                self.figure = None;
                self.other_figures.clear();
                self.figure_file_events.clear();
                self.current_figure = 1;
                return Ok(Value::Void);
            }
            let id = if let Some(value) = args.first() {
                let n = value.clone().matrix(&self.cancel)?.as_scalar()?;
                if !n.is_finite() || n.fract() != 0. || !(1. ..=1_000_000.).contains(&n) {
                    return Err("Figure number must be an integer in 1..1000000".into());
                }
                n as u32
            } else if name == "figure" {
                let ids = self.figure_ids();
                (1..=17).find(|i| !ids.contains(i)).unwrap()
            } else {
                self.current_figure
            };
            if name == "figure" {
                self.select_figure(id)?;
                return Ok(Value::Matrix(Matrix::scalar(id as f64)));
            }
            self.close_figure_id(id);
            return Ok(Value::Void);
        }
        if name == "subplot" {
            if matches!(args.first(), Some(Value::Text(s)) if s == "Position") {
                if args.len() < 2 || !args.len().is_multiple_of(2) {
                    return Err("Use subplot('Position',[left bottom width height])".into());
                }
                let values = args[1].clone().matrix(&self.cancel)?;
                if values.len() != 4 || (values.rows() != 1 && values.cols() != 1) {
                    return Err("Axes Position must be a four-element vector".into());
                }
                let position =
                    crate::plot::Position::from_normalized(values.data().try_into().unwrap())?;
                let mut figure = self.figure.clone().unwrap_or_default();
                figure.select_subplot(position, false)?;
                apply_axes_layout_pairs(&mut figure, &args[2..], &self.cancel)?;
                figure.prepare_legend_layout()?;
                figure.validate()?;
                self.figure = Some(figure);
                return Ok(Value::Void);
            }
            if args.len() < 3 {
                return Err("Use subplot(rows,columns,index[, 'replace'])".into());
            }
            let integer = |i: usize| -> Result<usize> {
                let value = args[i].clone().matrix(&self.cancel)?.as_scalar()?;
                if !value.is_finite() || value.fract() != 0. || !(1. ..=16.).contains(&value) {
                    return Err("Subplot rows and columns must be integers in 1..16".into());
                }
                Ok(value as usize)
            };
            let rows = integer(0)?;
            let columns = integer(1)?;
            let values = args[2].clone().matrix(&self.cancel)?;
            if values.is_empty() || (values.rows() != 1 && values.cols() != 1) || values.len() > 256
            {
                return Err("Subplot position must be a nonempty scalar or vector".into());
            }
            let mut indices = Vec::with_capacity(values.len());
            for &v in values.data() {
                if !v.is_finite() || v.fract() != 0. || v < 1. || v > (rows * columns) as f64 {
                    return Err("Subplot position is outside the grid".into());
                }
                indices.push(v as usize);
            }
            let (replace, properties) = match args.get(3) {
                None => (false, 3),
                Some(Value::Text(s)) if s == "replace" => (true, 4),
                Some(Value::Text(s)) if s == "align" => (false, 4),
                _ => (false, 3),
            };
            let position = crate::plot::Position::from_grid(rows, columns, &indices)?;
            let mut figure = self.figure.clone().unwrap_or_default();
            figure.select_subplot(position, replace)?;
            apply_axes_layout_pairs(&mut figure, &args[properties..], &self.cancel)?;
            figure.prepare_legend_layout()?;
            figure.validate()?;
            self.figure = Some(figure);
            return Ok(Value::Void);
        }
        let text = |i: usize| match args.get(i) {
            Some(Value::Text(s)) => Ok(s.as_str()),
            _ => Err("Expected a quoted option".to_string()),
        };
        if name == "clf" {
            if !args.is_empty() {
                return Err("clf takes no arguments".into());
            }
            let mut figure = Figure::default();
            if let Some(previous) = &self.figure {
                figure.identity = previous.identity.clone();
                figure.name = previous.name.clone();
                figure.color = previous.color;
                figure.appearance = previous.appearance;
                figure.invert_hardcopy = previous.invert_hardcopy;
                figure.axes = figure.fresh_axes();
            }
            self.figure = Some(figure);
            return Ok(Value::Void);
        }
        let mut figure = self.figure.clone().unwrap_or_default();
        figure.activate_tile();
        match name {
            "xscale" | "yscale" => {
                let (scale, limits) = if name == "xscale" {
                    (&mut figure.axes.xscale, &mut figure.axes.xlim)
                } else {
                    (&mut figure.axes.yscale, &mut figure.axes.ylim)
                };
                if args.is_empty() {
                    return Ok(Value::Text(
                        if *scale == crate::plot::AxisScale::Log {
                            "log"
                        } else {
                            "linear"
                        }
                        .into(),
                    ));
                }
                if args.len() != 1 {
                    return Err("Axis scale takes one option: linear or log".into());
                }
                *scale = match text(0)? {
                    "linear" => crate::plot::AxisScale::Linear,
                    "log" => crate::plot::AxisScale::Log,
                    _ => return Err("Use linear or log".into()),
                };
                if limits.is_some_and(|r| scale.validate_limits(r).is_err()) {
                    *limits = None;
                }
            }
            "grid" => {
                let state = match args.len() {
                    0 => None,
                    1 => Some(match text(0)?.to_ascii_lowercase().as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("Use on or off".into()),
                    }),
                    _ => return Err("Expected zero or one option".into()),
                };
                // Toggle each major grid direction independently.
                figure.xgrid = state.unwrap_or(!figure.xgrid);
                figure.ygrid = state.unwrap_or(!figure.ygrid);
            }
            "hold" => {
                let target = &mut figure.hold;
                *target = match args.len() {
                    0 => !*target,
                    1 => match text(0)? {
                        "on" => true,
                        "off" => false,
                        _ => return Err("Use on or off".into()),
                    },
                    _ => return Err("Expected zero or one option".into()),
                };
            }
            "legend" => {
                // Only trailing Location/value pairs are properties. Earlier strings
                // remain labels, including a label literally named "Location".
                let mut label_count = args.len();
                let mut location = None;
                while label_count >= 2 && text(label_count - 2)?.eq_ignore_ascii_case("Location") {
                    let parsed = crate::plot::LegendLocation::parse(text(label_count - 1)?)?;
                    location.get_or_insert(parsed); // last pair wins, validate all pairs
                    label_count -= 2;
                }
                if let Some(location) = location {
                    if location != figure.legend_location
                        && let Some(placement) = &mut figure.placement
                    {
                        placement.imported_solved = false;
                    }
                    figure.legend_location = location;
                }
                if label_count == 0 {
                    figure.legend = true;
                } else if label_count == 1 && matches!(text(0)?, "on" | "off") {
                    figure.legend = text(0)? == "on";
                } else {
                    if label_count != figure.series.len() {
                        return Err("Provide one legend label per series".into());
                    }
                    for (i, s) in figure.series.iter_mut().enumerate() {
                        let label = text(i)?;
                        if label.len() > 4096 {
                            return Err("Legend label exceeds 4096 bytes".into());
                        }
                        s.label = label.into();
                    }
                    figure.legend = true;
                }
            }
            "xlim" | "ylim" | "axis" => {
                if args.is_empty() {
                    let (x, y) = figure.bounds();
                    return Ok(Value::Matrix(if name == "axis" {
                        Matrix::from_column_slice(1, 4, &[x[0], x[1], y[0], y[1]])?
                    } else {
                        Matrix::from_column_slice(1, 2, if name == "xlim" { &x } else { &y })?
                    }));
                }
                if args.len() != 1 {
                    return Err("Axis controls take one argument".into());
                }
                if let Value::Text(option) = &args[0] {
                    if option != "auto" && !(name == "axis" && option == "tight") {
                        return Err("Use auto, axis tight, or numeric limits".into());
                    }
                    if name != "ylim" {
                        figure.xlim = None;
                    }
                    if name != "xlim" {
                        figure.ylim = None;
                    }
                } else {
                    let values = args[0].clone().matrix(&self.cancel)?;
                    if values.len() != if name == "axis" { 4 } else { 2 } {
                        return Err("Use [min max] or axis([xmin xmax ymin ymax])".into());
                    }
                    let first = [values.data()[0], values.data()[1]];
                    crate::plot::validate_limits(first)?;
                    if name == "axis" {
                        let second = [values.data()[2], values.data()[3]];
                        crate::plot::validate_limits(second)?;
                        figure.xlim = Some(first);
                        figure.ylim = Some(second);
                    } else if name == "xlim" {
                        figure.xlim = Some(first);
                    } else {
                        figure.ylim = Some(first);
                    }
                }
            }
            _ => unreachable!(),
        }
        figure.prepare_legend_layout()?;
        figure.validate()?;
        self.figure = Some(figure);
        Ok(Value::Void)
    }
}

impl Engine {
    fn tiled_graphics(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        use crate::plot::{Arrangement, TileGrid};
        let integers = |value: &Value, max: usize| -> Result<Vec<usize>> {
            let m = value.clone().matrix(&self.cancel)?;
            if m.is_empty() || m.len() > 2 || (m.rows() != 1 && m.cols() != 1) {
                return Err("Expected an integer scalar or two-element span".into());
            }
            m.data()
                .iter()
                .map(|&v| {
                    if !v.is_finite() || v.fract() != 0. || v < 1. || v > max as f64 {
                        Err(format!("Layout integer must be in 1..{max}"))
                    } else {
                        Ok(v as usize)
                    }
                })
                .collect()
        };
        let scalar = |value: &Value, max| -> Result<usize> {
            let v = integers(value, max)?;
            if v.len() != 1 {
                return Err("Expected a scalar layout dimension or tile index".into());
            }
            Ok(v[0])
        };
        if name == "tiledlayout" {
            let grid = match args.as_slice() {
                [] => TileGrid::new(Arrangement::Flow, 1, 1)?,
                [Value::Text(s)] => TileGrid::new(match s.as_str() {
                    "flow" => Arrangement::Flow,
                    "vertical" => Arrangement::Vertical,
                    "horizontal" => Arrangement::Horizontal,
                    _ => return Err("Supported tiled arrangements: flow, vertical, horizontal".into()),
                }, 1, 1)?,
                [r, c] => TileGrid::new(Arrangement::Fixed, scalar(r, 16)?, scalar(c, 16)?)?,
                _ => return Err("Use tiledlayout(rows,columns) or tiledlayout('flow'/'vertical'/'horizontal'); layout handles and spacing options are unsupported".into()),
            };
            let mut figure = Figure::tiled_layout(grid)?;
            if let Some(previous) = &self.figure {
                figure.identity = previous.identity.clone();
                figure.name = previous.name.clone();
                figure.color = previous.color;
                figure.appearance = previous.appearance;
                figure.invert_hardcopy = previous.invert_hardcopy;
                figure.axes = figure.fresh_axes();
            }
            self.figure = Some(figure);
        } else if name == "nexttile" {
            let span = |value: &Value| -> Result<[usize; 2]> {
                integers(value, 16)?
                    .try_into()
                    .map_err(|_| "Tile span must contain [rows columns]".into())
            };
            let (tile, span) = match args.as_slice() {
                [] => (None, None),
                [v] => {
                    let n = integers(v, 256)?;
                    if n.len() == 1 { (Some(n[0]), None) } else { (None, Some(span(v)?)) }
                }
                [i, s] => (Some(scalar(i, 256)?), Some(span(s)?)),
                _ => return Err("Use nexttile, nexttile(index), nexttile([rows columns]) or nexttile(index,[rows columns])".into()),
            };
            if let Some(figure) = &mut self.figure {
                figure.next_tile(tile, span)?;
            } else {
                let mut figure = Figure::default();
                figure.next_tile(tile, span)?;
                self.figure = Some(figure);
            }
        } else {
            return self.shared_title_graphics(&args);
        }
        Ok(Value::Void)
    }
}
