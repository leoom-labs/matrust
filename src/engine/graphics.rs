use super::*;
use crate::{GraphicsArray, GraphicsHandle, GraphicsKind};

/// New `Box` state for `box`, `box on|off` (also 0/1 and logical): no
/// option toggles it.
pub(in crate::engine) fn box_state(current: bool, args: &[Value]) -> Result<bool> {
    match args {
        [] => Ok(!current),
        [Value::Text(s)] if s.eq_ignore_ascii_case("on") => Ok(true),
        [Value::Text(s)] if s.eq_ignore_ascii_case("off") => Ok(false),
        [Value::Logical(m)] => m.as_scalar(),
        [Value::Matrix(m)] if m.len() == 1 && matches!(m.as_scalar()?, 0. | 1.) => {
            Ok(m.as_scalar()? == 1.)
        }
        _ => Err("Use box on, box off or box".into()),
    }
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
            if let Some(fig) = self.figure.take() {
                self.closed_figures.insert(id);
                let key = fig.identity.recovery_key();
                self.figure_defaults.remove(&key);
                self.hidden_figures.remove(&key);
                self.ui.delete_classic_figure_children(key);
            }
            if let Some((&next, _)) = self.other_figures.first_key_value() {
                self.current_figure = next;
                self.figure = self.other_figures.remove(&next);
            }
        } else {
            self.figure_file_events.remove(&id);
            if let Some(fig) = self.other_figures.remove(&id) {
                self.closed_figures.insert(id);
                let key = fig.identity.recovery_key();
                self.figure_defaults.remove(&key);
                self.hidden_figures.remove(&key);
                self.ui.delete_classic_figure_children(key);
            }
        }
    }

    /// Ensure a current classic figure exists (like `gcf`) and return its identity.
    pub(in crate::engine) fn ensure_current_classic_figure(
        &mut self,
    ) -> Result<crate::ObjectIdentity> {
        if self.figure.is_none() {
            let mut f = Figure::default();
            self.apply_figure_defaults(&mut f)?;
            self.figure = Some(f);
        }
        Ok(self.figure.as_ref().unwrap().identity.clone())
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
    fn graphics_subplot(&mut self, args: Vec<Value>) -> Result<Value> {
        use super::graphics_handles::{dot_syntax, handle};
        use crate::plot::Position;

        let args = subplot_number_words(args);
        let explicit_position =
            matches!(args.first(), Some(Value::Text(s)) if s.eq_ignore_ascii_case("Position"));
        let (position, options) = if explicit_position {
            let Some(value) = args.get(1) else {
                return Err("Use subplot('Position',[left bottom width height])".into());
            };
            let values = value.clone().matrix(&self.cancel)?;
            if values.len() != 4 || (values.rows() != 1 && values.cols() != 1) {
                return Err("Axes Position must be a four-element vector".into());
            }
            (
                Position::from_normalized(values.data().try_into().unwrap())?,
                &args[2..],
            )
        } else {
            let [r, c, p, options @ ..] = args.as_slice() else {
                return Err("Use subplot(mnp) or subplot(rows,columns,index,...)".into());
            };
            let integer = |value: &Value| -> Result<usize> {
                let value = value.clone().matrix(&self.cancel)?.as_scalar()?;
                if value.fract() != 0. || !(1. ..=16.).contains(&value) {
                    return Err("Subplot rows and columns must be integers in 1..16".into());
                }
                Ok(value as usize)
            };
            let (rows, columns) = (integer(r)?, integer(c)?);
            let values = p.clone().matrix(&self.cancel)?;
            if values.is_empty() || (values.rows() != 1 && values.cols() != 1) || values.len() > 256
            {
                return Err("Subplot position must be a nonempty scalar or vector".into());
            }
            let indices = values
                .data()
                .iter()
                .map(|&v| {
                    if v.fract() != 0. || !(1. ..=(rows * columns) as f64).contains(&v) {
                        Err("Subplot position is outside the grid".to_owned())
                    } else {
                        Ok(v as usize)
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            (Position::from_grid(rows, columns, &indices)?, options)
        };

        let mut replace = false;
        let mut source = None;
        let properties = match options {
            [Value::Text(mode), rest @ ..]
                if !explicit_position
                    && (mode.eq_ignore_ascii_case("replace")
                        || mode.eq_ignore_ascii_case("align")) =>
            {
                replace = mode.eq_ignore_ascii_case("replace");
                rest
            }
            [
                value @ Value::Container(crate::Array::Graphics(_)),
                rest @ ..,
            ] if !explicit_position => {
                let axes = handle(value)?;
                if axes.kind() != GraphicsKind::Axes {
                    return Err("Subplot selection requires an axes handle".into());
                }
                source = Some(self.locate_graphics(axes)?);
                rest
            }
            _ => options,
        };
        if properties.len() > 32 || !properties.len().is_multiple_of(2) {
            return Err("Subplot accepts at most 16 property/value pairs".into());
        }
        let id = source.map_or(self.current_figure, |axes| axes.figure);
        let mut figure = if let Ok(existing) = self.graphics_figure(id) {
            existing.clone()
        } else {
            let mut figure = Figure::default();
            self.apply_figure_defaults(&mut figure)?;
            figure
        };
        if let Some(source) = source {
            figure.move_axes_to_subplot(source.position.unwrap(), position)?;
        } else {
            let old_axes: Vec<_> = figure.all_axes().map(|(_, a)| a.identity.clone()).collect();
            figure.select_subplot(position, replace)?;
            if !old_axes
                .iter()
                .any(|old| old.same_object(&figure.axes.identity))
            {
                self.apply_axes_defaults(&mut figure)?;
            }
        }
        let axes = GraphicsHandle::axes(&figure.identity, &figure.axes.identity);
        let remaining = dot_syntax::set_extended(self, &axes, &mut figure, properties)?;
        let (sync_x, sync_y) = self.graphics_apply_axes_pairs(&mut figure, &remaining)?;
        figure.prepare_legend_layout()?;
        figure.validate()?;
        let handle = GraphicsArray::scalar(axes, &self.cancel)?;
        self.check_cancel()?;
        if id != self.current_figure {
            self.select_figure(id)?;
        }
        let axes_id = figure.axes.identity.clone();
        self.publish_graphics_figure(id, figure);
        if sync_x || sync_y {
            sync_engine_linked_axes(self, &axes_id, sync_x, sync_y);
        }
        Ok(Value::Container(handle.into()))
    }

    pub(super) fn graphics(
        &mut self,
        name: &str,
        args: Vec<Value>,
        out: &mut super::Execution,
    ) -> Result<Value> {
        if matches!(name, "tiledlayout" | "nexttile" | "sgtitle") {
            return self.tiled_graphics(name, args);
        }
        if name == "figure" || name == "close" {
            let mut args = args;
            let mut include_hidden = false;
            if name == "close" && args.len() == 2 {
                // `close all force|hidden`, `close(h,'force')`. Both options
                // close the requested figures without a CloseRequestFcn.
                let option = self.text_arguments(args.split_off(1))?;
                match option.first() {
                    Some(Value::Text(s))
                        if s.eq_ignore_ascii_case("force") || s.eq_ignore_ascii_case("hidden") => {}
                    Some(Value::Text(s)) => {
                        return Err(format!(
                            "close: unknown option '{s}'; use close all force, close all hidden or close(h,'force')"
                        ));
                    }
                    _ => {
                        return Err("close: the second argument must be 'force' or 'hidden'".into());
                    }
                }
                include_hidden = true;
                args = self.text_arguments(args)?;
            }
            if args.len() > 1 {
                return Err("figure/close accept at most one argument".into());
            }
            if name == "close" && matches!(args.first(),Some(Value::Text(s)) if s=="all") {
                if include_hidden {
                    self.dialogs.close_all_headless();
                }
                super::ui_components::close_all(self, out);
                self.closed_figures.extend(self.figure_ids());
                self.figure = None;
                self.other_figures.clear();
                self.figure_file_events.clear();
                self.figure_defaults.clear();
                self.hidden_figures.clear();
                self.current_figure = 1;
                return Ok(Value::Void);
            }
            if name == "close"
                && let [Value::Container(crate::Array::Graphics(handles))] = args.as_slice()
                && let Some(id) = self.dialogs.headless_id(handles.scalar_handle()?)
            {
                self.check_cancel()?;
                self.dialogs.close_headless(id);
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
            return self.graphics_subplot(args);
        }
        let text = |i: usize| match args.get(i) {
            Some(Value::Text(s)) => Ok(s.as_str()),
            _ => Err("Expected a quoted option".to_string()),
        };
        if name == "clf" {
            return self.clf_graphics(&args);
        }
        if name == "cla" {
            return self.cla_graphics(&args);
        }
        if name == "linkaxes" {
            return self.linkaxes_graphics(&args);
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
                let option = match args.len() {
                    0 => String::new(),
                    1 => text(0)?.to_ascii_lowercase(),
                    _ => return Err("Expected zero or one option".into()),
                };
                match option.as_str() {
                    // `grid minor` toggles each minor direction and leaves the
                    // major grid alone (MATLAB doc: "toggles the visibility of
                    // the minor grid lines").
                    "minor" => {
                        for minor in &mut figure.minor_grid {
                            *minor = !*minor;
                        }
                    }
                    // `grid off` removes all grid lines, major and minor.
                    "off" => {
                        (figure.xgrid, figure.ygrid, figure.zgrid) = (false, false, false);
                        figure.minor_grid = [false; 3];
                    }
                    "on" => (figure.xgrid, figure.ygrid, figure.zgrid) = (true, true, true),
                    // Toggle each major grid direction independently.
                    "" => {
                        figure.xgrid = !figure.xgrid;
                        figure.ygrid = !figure.ygrid;
                        figure.zgrid = !figure.zgrid;
                    }
                    _ => return Err("Use grid on, grid off, grid minor or grid".into()),
                }
            }
            "box" => figure.box_outline = box_state(figure.box_outline, &args)?,
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
            "view" => {
                if args.is_empty() {
                    return super::graphics_handles::view_value(figure.view);
                }
                figure.view = super::graphics_handles::parse_view(&args)?;
            }
            "zlim" => match args.as_slice() {
                [] => {
                    return super::plot_time_axes::ruler_values_to_value(
                        &figure.axes.extras.rulers[2].kind,
                        &figure.zbounds(),
                        &self.cancel,
                    );
                }
                [Value::Text(option)] if option == "auto" => figure.zlim = None,
                [Value::Text(option)] if option == "manual" => {
                    figure.zlim = Some(figure.zbounds());
                }
                [Value::Text(_)] => {
                    return Err("Use zlim auto, zlim manual or zlim([zmin zmax])".into());
                }
                [value] => {
                    let vals = super::plot_time_axes::parse_ruler_values(
                        &figure.axes.extras.rulers[2].kind,
                        value,
                        "Limits",
                        &self.cancel,
                    )?;
                    if vals.len() != 2 {
                        return Err("Use zlim([zmin zmax])".into());
                    }
                    let limits = [vals[0], vals[1]];
                    crate::plot::validate_axis_limits(limits)?;
                    figure.zlim = if limits == [f64::NEG_INFINITY, f64::INFINITY] {
                        None
                    } else {
                        Some(limits)
                    };
                }
                _ => return Err("zlim takes one argument".into()),
            },
            "xlim" | "ylim" | "axis" => {
                if args.is_empty() {
                    let (x, y) = figure.bounds();
                    if name == "xlim" {
                        return super::plot_time_axes::ruler_values_to_value(
                            &figure.axes.extras.rulers[0].kind,
                            &x,
                            &self.cancel,
                        );
                    }
                    if name == "ylim" {
                        return super::plot_time_axes::ruler_values_to_value(
                            &figure.axes.extras.rulers[1].kind,
                            &y,
                            &self.cancel,
                        );
                    }
                    return Ok(Value::Matrix(if figure.is_3d() {
                        let z = figure.zbounds();
                        Matrix::from_column_slice(1, 6, &[x[0], x[1], y[0], y[1], z[0], z[1]])?
                    } else {
                        Matrix::from_column_slice(1, 4, &[x[0], x[1], y[0], y[1]])?
                    }));
                }
                if name == "axis" && args.iter().all(|a| matches!(a, Value::Text(_))) {
                    // axis equal / image / ij / xy / off / on / normal / tight /
                    // auto, one or several words (`axis equal tight`).
                    for arg in args {
                        let Value::Text(option) = arg else {
                            unreachable!()
                        };
                        let extras = &mut figure.axes.extras;
                        match option.to_ascii_lowercase().as_str() {
                            "equal" => extras.equal_aspect = true,
                            "image" => {
                                extras.equal_aspect = true;
                                figure.xlim = None;
                                figure.ylim = None;
                            }
                            "square" => extras.square_plot_box = true,
                            "normal" => {
                                extras.equal_aspect = false;
                                extras.square_plot_box = false;
                            }
                            "ij" => extras.y_reversed = true,
                            "xy" => extras.y_reversed = false,
                            "off" => extras.axis_off = true,
                            "on" => extras.axis_off = false,
                            "tight" | "auto" => {
                                figure.xlim = None;
                                figure.ylim = None;
                                figure.zlim = None;
                            }
                            other => {
                                return Err(format!(
                                    "axis {other} is not supported; use auto, tight, equal, image, square, normal, ij, xy, on, off or [xmin xmax ymin ymax]"
                                ));
                            }
                        }
                    }
                    figure.prepare_legend_layout()?;
                    let axes_id = figure.axes.identity.clone();
                    figure.sync_linked_axes(&axes_id, true, true);
                    figure.validate()?;
                    self.figure = Some(figure);
                    sync_engine_linked_axes(self, &axes_id, true, true);
                    return Ok(Value::Void);
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
                    if name == "axis" {
                        figure.zlim = None;
                    }
                } else if name == "xlim" || name == "ylim" {
                    let axis = usize::from(name == "ylim");
                    let vals = super::plot_time_axes::parse_ruler_values(
                        &figure.axes.extras.rulers[axis].kind,
                        &args[0],
                        "Limits",
                        &self.cancel,
                    )?;
                    if vals.len() != 2 {
                        return Err("Use [min max]".into());
                    }
                    let first = [vals[0], vals[1]];
                    crate::plot::validate_axis_limits(first)?;
                    let stored = if first == [f64::NEG_INFINITY, f64::INFINITY] {
                        None
                    } else {
                        Some(first)
                    };
                    if axis == 0 {
                        figure.xlim = stored;
                    } else {
                        figure.ylim = stored;
                    }
                } else {
                    let values = args[0].clone().matrix(&self.cancel)?;
                    if values.len() != 4 && values.len() != 6 {
                        return Err("Use [min max], axis([xmin xmax ymin ymax]) or axis([xmin xmax ymin ymax zmin zmax])".into());
                    }
                    if values.len() == 6 {
                        let third = [values.data()[4], values.data()[5]];
                        crate::plot::validate_axis_limits(third)?;
                        figure.zlim = if third == [f64::NEG_INFINITY, f64::INFINITY] {
                            None
                        } else {
                            Some(third)
                        };
                    }
                    let first = [values.data()[0], values.data()[1]];
                    crate::plot::validate_axis_limits(first)?;
                    let store_first = if first == [f64::NEG_INFINITY, f64::INFINITY] {
                        None
                    } else {
                        Some(first)
                    };
                    let second = [values.data()[2], values.data()[3]];
                    crate::plot::validate_axis_limits(second)?;
                    figure.xlim = store_first;
                    figure.ylim = if second == [f64::NEG_INFINITY, f64::INFINITY] {
                        None
                    } else {
                        Some(second)
                    };
                }
            }
            _ => unreachable!(),
        }
        let axes_id = figure.axes.identity.clone();
        let (sync_x, sync_y) = match name {
            "xlim" => (true, false),
            "ylim" => (false, true),
            "axis" => (true, true),
            _ => (false, false),
        };
        figure.sync_linked_axes(&axes_id, sync_x, sync_y);
        figure.prepare_legend_layout()?;
        figure.validate()?;
        self.figure = Some(figure);
        sync_engine_linked_axes(self, &axes_id, sync_x, sync_y);
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
                _ => return Err("Use tiledlayout(rows,columns) or tiledlayout('flow'/'vertical'/'horizontal'); constructor spacing options are unsupported".into()),
            };
            let figure = if let Some(previous) = &self.figure {
                let fresh = Figure::tiled_layout(grid)?;
                let mut f = previous.clone();
                f.axes = f.fresh_axes();
                f.layout = fresh.layout;
                f
            } else {
                let mut f = Figure::tiled_layout(grid)?;
                self.apply_figure_defaults(&mut f)?;
                f
            };
            let layout_id = figure.layout.as_ref().unwrap().identity.clone();
            let handle = GraphicsArray::scalar(
                GraphicsHandle::tiled_layout(&figure.identity, &layout_id),
                &self.cancel,
            )?;
            self.figure = Some(figure);
            Ok(Value::Container(handle.into()))
        } else if name == "nexttile" {
            let mut rest = args.as_slice();
            let mut target_figure_id = None;
            if let Some(Value::Container(Array::Graphics(handles))) = rest.first() {
                let h = handles.scalar_handle()?;
                if h.kind() != GraphicsKind::TiledLayout {
                    return Err("nexttile target must be a live TiledChartLayout handle".into());
                }
                let located = self.locate_graphics(h)?;
                target_figure_id = Some(located.figure);
                rest = &rest[1..];
            }
            let span = |value: &Value| -> Result<[usize; 2]> {
                integers(value, 16)?
                    .try_into()
                    .map_err(|_| "Tile span must contain [rows columns]".into())
            };
            let (tile, span) = match rest {
                [] => (None, None),
                [v] => {
                    let n = integers(v, 256)?;
                    if n.len() == 1 { (Some(n[0]), None) } else { (None, Some(span(v)?)) }
                }
                [i, s] => (Some(scalar(i, 256)?), Some(span(s)?)),
                _ => return Err("Use nexttile, nexttile(t), nexttile(index), nexttile([rows columns]) or nexttile(index,[rows columns])".into()),
            };
            if let Some(id) = target_figure_id {
                self.select_figure(id)?;
            }
            let mut figure = self.figure.clone().unwrap_or_default();
            if self.figure.is_none() {
                self.apply_figure_defaults(&mut figure)?;
            }
            let old_axes: Vec<_> = figure.all_axes().map(|(_, a)| a.identity.clone()).collect();
            figure.next_tile(tile, span)?;
            if !old_axes
                .iter()
                .any(|id| id.same_object(&figure.axes.identity))
            {
                self.apply_axes_defaults(&mut figure)?;
            }
            let handle = GraphicsArray::scalar(
                GraphicsHandle::axes(&figure.identity, &figure.axes.identity),
                &self.cancel,
            )?;
            self.figure = Some(figure);
            Ok(Value::Container(handle.into()))
        } else {
            self.shared_title_graphics(&args)
        }
    }

    fn clf_graphics(&mut self, args: &[Value]) -> Result<Value> {
        // clf(uifigure) deletes the uifigure's components.
        if let [Value::Container(Array::Graphics(handles))]
        | [Value::Container(Array::Graphics(handles)), Value::Text(_)] = args
            && let Ok(h) = handles.scalar_handle()
            && h.kind() == GraphicsKind::UiComponent
        {
            if let [_, Value::Text(opt)] = args
                && !opt.eq_ignore_ascii_case("reset")
            {
                return Err("Use clf, clf('reset'), clf(fig) or clf(fig,'reset')".into());
            }
            return super::ui_components::clear_figure_components(self, h).map(|()| Value::Void);
        }
        let (target_fig, reset) = match args {
            [] => (None, false),
            [Value::Text(opt)] if opt.eq_ignore_ascii_case("reset") => (None, true),
            [Value::Container(Array::Graphics(handles))] => {
                let h = handles.scalar_handle()?;
                if h.kind() != GraphicsKind::Figure {
                    return Err("clf target must be a live figure handle".into());
                }
                (Some(self.locate_graphics(h)?.figure), false)
            }
            [Value::Container(Array::Graphics(handles)), Value::Text(opt)]
                if opt.eq_ignore_ascii_case("reset") =>
            {
                let h = handles.scalar_handle()?;
                if h.kind() != GraphicsKind::Figure {
                    return Err("clf target must be a live figure handle".into());
                }
                (Some(self.locate_graphics(h)?.figure), true)
            }
            _ => return Err("Use clf, clf('reset'), clf(fig) or clf(fig,'reset')".into()),
        };
        let prev = match target_fig {
            None => self.figure.as_ref(),
            Some(id) if id == self.current_figure => self.figure.as_ref(),
            Some(id) => self.other_figures.get(&id),
        };
        let figure = if !reset {
            if let Some(previous) = prev {
                let mut f = previous.clone();
                f.axes = f.fresh_axes();
                f.layout = None;
                f.textboxes.clear();
                f.annotations.clear();
                f
            } else {
                Figure::default()
            }
        } else {
            let mut f = Figure::default();
            let saved_pos = prev.map(|p| (p.position, p.units));
            if let Some(previous) = prev {
                f.identity = previous.identity.clone();
            }
            self.apply_figure_defaults(&mut f)?;
            if let Some((position, units)) = saved_pos {
                f.position = position;
                f.units = units;
            }
            self.apply_axes_defaults(&mut f)?;
            f
        };
        // clf deletes the figure's uicontrols and ui components too.
        self.ui
            .delete_classic_figure_components(figure.identity.recovery_key());
        match target_fig {
            None => self.figure = Some(figure),
            Some(id) if id == self.current_figure => self.figure = Some(figure),
            Some(id) => {
                self.other_figures.insert(id, figure);
            }
        }
        Ok(Value::Void)
    }

    fn cla_graphics(&mut self, args: &[Value]) -> Result<Value> {
        let (target, reset) = match args {
            [] => (None, false),
            [Value::Text(opt)] if opt.eq_ignore_ascii_case("reset") => (None, true),
            [Value::Container(Array::Graphics(handles))] => {
                let h = handles.scalar_handle()?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("cla target must be a live axes handle".into());
                }
                (Some(self.locate_graphics(h)?), false)
            }
            [Value::Container(Array::Graphics(handles)), Value::Text(opt)]
                if opt.eq_ignore_ascii_case("reset") =>
            {
                let h = handles.scalar_handle()?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("cla target must be a live axes handle".into());
                }
                (Some(self.locate_graphics(h)?), true)
            }
            _ => return Err("Use cla, cla('reset'), cla(ax) or cla(ax,'reset')".into()),
        };
        let clear_axes = |figure: &mut Figure, pos: Option<crate::plot::Position>| {
            let axes = if let Some(p) = pos {
                if figure.layout.as_ref().is_none_or(|l| l.active == p) {
                    &mut figure.axes
                } else if let Some(panel) = figure
                    .layout
                    .as_mut()
                    .and_then(|l| l.others.iter_mut().find(|o| o.position == p))
                {
                    &mut panel.axes
                } else {
                    &mut figure.axes
                }
            } else {
                &mut figure.axes
            };
            let saved_id = axes.identity.clone();
            if reset {
                *axes = crate::plot::Axes::default();
                axes.identity = saved_id;
            } else {
                axes.series.clear();
                axes.extras.texts.clear();
                axes.extras.constant_lines.clear();
                axes.extras.rectangles.clear();
                axes.extras.volumes.clear();
                axes.legend = Default::default();
                axes.legend_state = None;
            }
        };
        if let Some(located) = target {
            let mut figure = self.graphics_figure(located.figure)?.clone();
            clear_axes(&mut figure, located.position);
            self.publish_graphics_figure(located.figure, figure);
        } else if let Some(mut figure) = self.figure.clone() {
            clear_axes(&mut figure, None);
            self.figure = Some(figure);
        }
        Ok(Value::Void)
    }

    fn linkaxes_graphics(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err("Use linkaxes(ax) or linkaxes(ax,'x'|'y'|'xy'|'off')".into());
        }
        let Value::Container(Array::Graphics(handles)) = &args[0] else {
            return Err("linkaxes requires an array of live axes handles".into());
        };
        if handles.is_empty() || handles.len() > 16 || (handles.rows() != 1 && handles.cols() != 1)
        {
            return Err("linkaxes requires a vector of 1..16 axes handles".into());
        }
        let (x, y) = match args.get(1) {
            None => (true, true),
            Some(Value::Text(opt)) => match opt.to_ascii_lowercase().as_str() {
                "x" => (true, false),
                "y" => (false, true),
                "xy" => (true, true),
                "off" => (false, false),
                _ => return Err("linkaxes option must be 'x', 'y', 'xy' or 'off'".into()),
            },
            Some(_) => return Err("linkaxes option must be 'x', 'y', 'xy' or 'off'".into()),
        };
        let mut fig_ids: Vec<u32> = Vec::new();
        let mut axes_ids = Vec::with_capacity(handles.len());
        for i in 0..handles.len() {
            self.check_cancel()?;
            let h = handles.get_linear(i)?;
            if h.kind() != GraphicsKind::Axes {
                return Err("linkaxes requires axes handles".into());
            }
            let located = self.locate_graphics(h)?;
            if !fig_ids.contains(&located.figure) {
                fig_ids.push(located.figure);
            }
            axes_ids.push(h.object.clone());
        }
        if fig_ids.is_empty() {
            return Ok(Value::Void);
        }
        if fig_ids.len() == 1 {
            let fig_id = fig_ids[0];
            let mut figure = self.graphics_figure(fig_id)?.clone();
            figure.set_linked_axes(&axes_ids, x, y);
            figure.validate()?;
            self.publish_graphics_figure(fig_id, figure);
        }
        set_engine_linked_axes(self, &axes_ids, x, y)?;
        Ok(Value::Void)
    }
}

fn prune_engine_linked_axes(engine: &mut Engine) {
    let live: Vec<crate::ObjectIdentity> = engine
        .figures()
        .flat_map(|(_, f)| f.all_axes().map(|(_, a)| a.identity.clone()))
        .collect();
    for link in &mut engine.linked_axes {
        link.axes
            .retain(|id| live.iter().any(|live_id| live_id.same_object(id)));
    }
    engine
        .linked_axes
        .retain(|link| link.axes.len() >= 2 && (link.x || link.y));
}

pub(super) fn set_engine_linked_axes(
    engine: &mut Engine,
    axes_ids: &[crate::ObjectIdentity],
    x: bool,
    y: bool,
) -> Result<()> {
    prune_engine_linked_axes(engine);
    let mut limits: Option<([f64; 2], [f64; 2])> = None;
    for (_, figure) in engine.figures() {
        for (_, axes) in figure
            .all_axes()
            .filter(|(_, a)| axes_ids.iter().any(|id| id.same_object(&a.identity)))
        {
            let (xb, yb) = axes.bounds();
            let (xb, yb) = (axes.xlim.unwrap_or(xb), axes.ylim.unwrap_or(yb));
            limits = Some(match limits {
                None => (xb, yb),
                Some((a, b)) => (
                    [a[0].min(xb[0]), a[1].max(xb[1])],
                    [b[0].min(yb[0]), b[1].max(yb[1])],
                ),
            });
        }
    }
    let (remove_x, remove_y) = if !x && !y { (true, true) } else { (x, y) };
    let mut remaining = Vec::new();
    for link in engine.linked_axes.drain(..) {
        for (dimension, remove) in [(true, remove_x), (false, remove_y)] {
            if (dimension && !link.x) || (!dimension && !link.y) {
                continue;
            }
            let mut ids = link.axes.clone();
            if remove {
                ids.retain(|id| !axes_ids.iter().any(|target| target.same_object(id)));
            }
            if ids.len() >= 2 {
                remaining.push(crate::plot::AxisLink {
                    axes: ids,
                    x: dimension,
                    y: !dimension,
                });
            }
        }
    }
    engine.linked_axes = remaining;
    if (!x && !y) || axes_ids.len() < 2 {
        return Ok(());
    }
    let mut unique = Vec::with_capacity(axes_ids.len());
    for id in axes_ids {
        if !unique
            .iter()
            .any(|u: &crate::ObjectIdentity| u.same_object(id))
        {
            unique.push(id.clone());
        }
    }
    if unique.len() >= 2 {
        engine.linked_axes.push(crate::plot::AxisLink {
            axes: unique.clone(),
            x,
            y,
        });
        if let Some((xb, yb)) = limits {
            let apply_fig = |fig: &mut Figure| -> Result<()> {
                let mut changed = false;
                for a in fig.all_axes_mut() {
                    if unique.iter().any(|id| id.same_object(&a.identity)) {
                        if x {
                            a.xlim = Some(xb);
                            changed = true;
                        }
                        if y {
                            a.ylim = Some(yb);
                            changed = true;
                        }
                    }
                }
                if changed {
                    fig.validate()?;
                }
                Ok(())
            };
            if let Some(fig) = engine.figure.as_mut() {
                apply_fig(fig)?;
            }
            for fig in engine.other_figures.values_mut() {
                apply_fig(fig)?;
            }
        }
    }
    Ok(())
}

pub(in crate::engine) fn sync_engine_linked_axes(
    engine: &mut Engine,
    source_id: &crate::ObjectIdentity,
    sync_x: bool,
    sync_y: bool,
) {
    if !sync_x && !sync_y {
        return;
    }
    prune_engine_linked_axes(engine);
    let mut target_x_ids: Vec<crate::ObjectIdentity> = Vec::new();
    let mut target_y_ids: Vec<crate::ObjectIdentity> = Vec::new();
    for link in &engine.linked_axes {
        if link.axes.iter().any(|id| id.same_object(source_id)) {
            for id in &link.axes {
                if !id.same_object(source_id) {
                    if sync_x && link.x {
                        target_x_ids.push(id.clone());
                    }
                    if sync_y && link.y {
                        target_y_ids.push(id.clone());
                    }
                }
            }
        }
    }
    if target_x_ids.is_empty() && target_y_ids.is_empty() {
        return;
    }
    let Some(source) = engine
        .figures()
        .flat_map(|(_, f)| f.all_axes().map(|(_, a)| a))
        .find(|a| a.identity.same_object(source_id))
    else {
        return;
    };
    let (xb, yb) = source.bounds();
    let new_xlim = if sync_x {
        Some(source.xlim.unwrap_or(xb))
    } else {
        None
    };
    let new_ylim = if sync_y {
        Some(source.ylim.unwrap_or(yb))
    } else {
        None
    };
    let apply_fig = |fig: &mut Figure| {
        for axes in fig.all_axes_mut() {
            if let Some(xl) = new_xlim
                && target_x_ids.iter().any(|id| id.same_object(&axes.identity))
            {
                axes.xlim = Some(xl);
            }
            if let Some(yl) = new_ylim
                && target_y_ids.iter().any(|id| id.same_object(&axes.identity))
            {
                axes.ylim = Some(yl);
            }
        }
    };
    if let Some(fig) = engine.figure.as_mut() {
        apply_fig(fig);
    }
    for fig in engine.other_figures.values_mut() {
        apply_fig(fig);
    }
}

/// A char row that reads as one real number (`'3'`, `'0.5'`), as MATLAB's own
/// functions convert command-syntax words; None for anything else.
pub(in crate::engine) fn number_word(value: &Value) -> Option<f64> {
    match value {
        Value::Text(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// `subplot 211` → (2,1,1); `subplot 2 1 1` → (2,1,1). Other arguments pass
/// through unchanged, so non-numeric text keeps the usual errors.
fn subplot_number_words(args: Vec<Value>) -> Vec<Value> {
    let scalar = |n: f64| Value::Matrix(Matrix::scalar(n));
    match args.as_slice() {
        [value] => {
            let code = match value {
                Value::Matrix(m) => m.as_scalar().ok(),
                _ => number_word(value),
            };
            if let Some(code) = code.filter(|n| n.fract() == 0. && (100. ..=999.).contains(n)) {
                let n = code as u16;
                [n / 100, n / 10 % 10, n % 10]
                    .into_iter()
                    .map(|digit| scalar(f64::from(digit)))
                    .collect()
            } else {
                args
            }
        }
        [first, second, third, ..] => {
            match (number_word(first), number_word(second), number_word(third)) {
                (Some(r), Some(c), Some(i)) => {
                    let mut out = vec![scalar(r), scalar(c), scalar(i)];
                    out.extend_from_slice(&args[3..]);
                    out
                }
                _ => args,
            }
        }
        _ => args,
    }
}
