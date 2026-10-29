use super::{Axes, Figure};
use crate::Result;
use serde::{Deserialize, Serialize};

/// Grid rectangle, with rows numbered from the top. Fractions are normalized
/// so equivalent spans from different grids identify the same axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub top: u8,
    pub bottom: u8,
    pub rows: u8,
    pub left: u8,
    pub right: u8,
    pub columns: u8,
    /// Exact IEEE bits of normalized MATLAB [left, bottom, width, height].
    /// Custom positions describe the plot box; grid positions describe panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<[u64; 4]>,
}
impl PartialOrd for Position {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Position {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let raw = |p: &Self| {
            (
                p.top, p.bottom, p.rows, p.left, p.right, p.columns, p.custom,
            )
        };
        let valid_denominator = |p: &Self| p.rows != 0 && p.columns != 0;
        if !valid_denominator(self) || !valid_denominator(other) {
            return valid_denominator(self)
                .cmp(&valid_denominator(other))
                .then_with(|| raw(self).cmp(&raw(other)));
        }
        if self.custom.is_some() || other.custom.is_some() {
            let a = self.rectangle();
            let b = other.rectangle();
            return [1, 0, 3, 2]
                .into_iter()
                .map(|i| a[i].total_cmp(&b[i]))
                .find(|c| !c.is_eq())
                .unwrap_or_else(|| raw(self).cmp(&raw(other)));
        }
        let fraction = |a: u8, b: u8, c: u8, d: u8| {
            (u16::from(a) * u16::from(d)).cmp(&(u16::from(c) * u16::from(b)))
        };
        fraction(self.top, self.rows, other.top, other.rows)
            .then_with(|| fraction(self.left, self.columns, other.left, other.columns))
            .then_with(|| fraction(self.bottom, self.rows, other.bottom, other.rows))
            .then_with(|| fraction(self.right, self.columns, other.right, other.columns))
            .then_with(|| raw(self).cmp(&raw(other)))
    }
}
impl Default for Position {
    fn default() -> Self {
        Self {
            top: 0,
            bottom: 1,
            rows: 1,
            left: 0,
            right: 1,
            columns: 1,
            custom: None,
        }
    }
}
impl Position {
    pub fn from_normalized(values: [f64; 4]) -> Result<Self> {
        let result = Self {
            custom: Some(values.map(|v| if v == 0. { 0 } else { v.to_bits() })),
            ..Self::default()
        };
        result.validate()?;
        Ok(result)
    }
    pub fn normalized_plot_box(self) -> Option<[f64; 4]> {
        self.custom.map(|v| v.map(f64::from_bits))
    }
    pub fn from_grid(rows: usize, columns: usize, indices: &[usize]) -> Result<Self> {
        if !(1..=16).contains(&rows)
            || !(1..=16).contains(&columns)
            || indices.is_empty()
            || indices.len() > 256
            || indices.iter().any(|&p| p == 0 || p > rows * columns)
        {
            return Err(
                "subplot requires 1..16 rows/columns and valid one-based grid positions".into(),
            );
        }
        let top = indices.iter().map(|p| (p - 1) / columns).min().unwrap();
        let bottom = indices.iter().map(|p| (p - 1) / columns + 1).max().unwrap();
        let left = indices.iter().map(|p| (p - 1) % columns).min().unwrap();
        let right = indices.iter().map(|p| (p - 1) % columns + 1).max().unwrap();
        let mut p = Self {
            top: top as u8,
            bottom: bottom as u8,
            rows: rows as u8,
            left: left as u8,
            right: right as u8,
            columns: columns as u8,
            custom: None,
        };
        p.normalize();
        Ok(p)
    }
    fn normalize(&mut self) {
        fn gcd(mut a: u8, mut b: u8) -> u8 {
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        }
        let r = gcd(gcd(self.top, self.bottom), self.rows);
        let c = gcd(gcd(self.left, self.right), self.columns);
        self.top /= r;
        self.bottom /= r;
        self.rows /= r;
        self.left /= c;
        self.right /= c;
        self.columns /= c;
    }
    pub fn validate(&self) -> Result<()> {
        if let Some([left, bottom, width, height]) = self.normalized_plot_box() {
            let grid = Self {
                custom: None,
                ..*self
            };
            if grid != Self::default()
                || ![left, bottom, width, height].iter().all(|x| x.is_finite())
                || left < 0.
                || bottom < 0.
                || width < 1. / 256.
                || height < 1. / 256.
                || left + width > 1.
                || bottom + height > 1.
                || self
                    .custom
                    .unwrap()
                    .iter()
                    .any(|&v| v == (-0.0_f64).to_bits())
            {
                return Err(
                    "Custom axes position must fit within [0,1], with width/height at least 1/256"
                        .into(),
                );
            }
            return Ok(());
        }
        if self.rows == 0
            || self.rows > 16
            || self.columns == 0
            || self.columns > 16
            || self.top >= self.bottom
            || self.bottom > self.rows
            || self.left >= self.right
            || self.right > self.columns
        {
            return Err("Invalid subplot position".into());
        }
        let mut normalized = *self;
        normalized.normalize();
        if normalized != *self {
            return Err("Subplot position is not normalized".into());
        }
        Ok(())
    }
    /// Left, top, right, bottom in normalized figure coordinates.
    pub fn rectangle(self) -> [f64; 4] {
        if let Some([l, b, w, h]) = self.normalized_plot_box() {
            return [l, 1. - b - h, l + w, 1. - b];
        }
        [
            self.left as f64 / self.columns as f64,
            self.top as f64 / self.rows as f64,
            self.right as f64 / self.columns as f64,
            self.bottom as f64 / self.rows as f64,
        ]
    }
    pub(crate) fn overlaps(self, other: Self) -> bool {
        let [l, t, r, b] = self.rectangle();
        let [ol, ot, or, ob] = other.rectangle();
        l < or && ol < r && t < ob && ot < b
    }
}
#[derive(Clone, Debug)]
pub struct Panel {
    pub position: Position,
    pub axes: Axes,
}
#[derive(Clone, Debug)]
pub struct Layout {
    pub active: Position,
    pub others: Vec<Panel>,
    pub tiled: Option<super::TileGrid>,
    pub title: String,
}
impl Layout {
    pub fn validate(&self) -> Result<()> {
        if self.title.len() > 4096 {
            return Err("Shared figure title exceeds 4096 bytes".into());
        }
        if let Some(grid) = self.tiled {
            grid.validate()?;
            if grid.pending_first && !self.others.is_empty() {
                return Err("Empty tiled layout contains inactive axes".into());
            }
            for position in
                std::iter::once(self.active).chain(self.others.iter().map(|p| p.position))
            {
                grid.cell(position)?;
            }
        }
        if self.others.len() >= 16 {
            return Err("At most 16 subplot axes per figure".into());
        }
        self.active.validate()?;
        for (i, p) in self.others.iter().enumerate() {
            p.position.validate()?;
            if p.position.overlaps(self.active)
                || self.others[..i]
                    .iter()
                    .any(|q| p.position.overlaps(q.position))
            {
                return Err("Subplot axes overlap".into());
            }
        }
        Ok(())
    }
}
impl Figure {
    /// Canonical vector-layout boxes, useful for bounded property previews and
    /// export preflight. Does not inspect samples or mutate requested state.
    pub fn resolved_axes_positions(&self) -> Result<Vec<(Position, [f64; 4])>> {
        let canvas = super::LegendRect::new([0., 0., 1., 1.]);
        Ok(self
            .resolved_axes_geometry()?
            .into_iter()
            .map(|(p, g)| (p, g.data.normalized(canvas)))
            .collect())
    }
    /// Canonical geometry in normalized top-down canvas coordinates. Placement
    /// is metadata-only; `best` remains an inside paint-time decision.
    pub fn resolved_axes_geometry(&self) -> Result<Vec<(Position, super::LegendGeometry)>> {
        let scale = self.all_axes().fold(1_f64, |s, (p, _)| {
            let [l, t, r, b] = p.rectangle();
            s.min(r - l).min(b - t)
        });
        let height = if self.layout.as_ref().is_some_and(|l| !l.title.is_empty()) {
            524.
        } else {
            560.
        };
        let canvas = super::LegendRect::new([0., 0., 960., height]);
        self.all_axes()
            .map(|(p, axes)| {
                let [l, t, r, b] = p.rectangle();
                if p.custom.is_some() {
                    let geometry = axes.export_legend_geometry(
                        960.,
                        height,
                        Some([960. * l, height * t, 960. * r, height * b]),
                        Some(canvas),
                    )?;
                    let convert = |r: super::LegendRect| {
                        super::LegendRect::from_normalized(
                            r.normalized(canvas),
                            super::LegendRect::new([0., 0., 1., 1.]),
                        )
                    };
                    Ok((
                        p,
                        super::LegendGeometry {
                            data: convert(geometry.data),
                            legend: geometry.legend.map(convert),
                            outer: convert(geometry.outer),
                        },
                    ))
                } else {
                    let width = 960. * (r - l) / scale;
                    let h = height * (b - t) / scale;
                    let geometry = axes.export_legend_geometry(width, h, None, None)?;
                    let convert = |rect: super::LegendRect| {
                        let [x, y, w, height] =
                            rect.normalized(super::LegendRect::new([0., 0., width, h]));
                        super::LegendRect::from_normalized(
                            [
                                l + x * (r - l),
                                1. - b + y * (b - t),
                                w * (r - l),
                                height * (b - t),
                            ],
                            super::LegendRect::new([0., 0., 1., 1.]),
                        )
                    };
                    Ok((
                        p,
                        super::LegendGeometry {
                            data: convert(geometry.data),
                            legend: geometry.legend.map(convert),
                            outer: convert(geometry.outer),
                        },
                    ))
                }
            })
            .collect()
    }

    /// Initialize only the active custom axes. Every candidate allocation is
    /// computed from the original requests, so legend addition order cannot
    /// move a neighbor or progressively reduce its available rectangle.
    pub fn prepare_legend_layout(&mut self) -> Result<()> {
        if !self.axes.legend_location.is_outside()
            || self.axes.placement.is_some()
            || self.active_position().custom.is_none()
        {
            return Ok(());
        }
        self.axes.placement = Some(self.initial_axes_placement()?);
        Ok(())
    }
    pub fn initial_axes_placement(&self) -> Result<super::AxesPlacement> {
        if let Some(placement) = self.axes.placement {
            return Ok(placement);
        }
        if self.empty_layout() {
            return Err("Create an axes before changing its placement".into());
        }
        let height = if self.layout.as_ref().is_some_and(|l| !l.title.is_empty()) {
            524.
        } else {
            560.
        };
        let canvas = super::LegendRect::new([0., 0., 960., height]);
        let original = self.normalized_axes_positions();
        let panels: Vec<_> = self.all_axes().collect();
        let requests: Vec<_> = panels
            .iter()
            .map(|(position, axes)| {
                axes.placement.map_or_else(
                    || original.iter().find(|(p, _)| p == position).unwrap().1,
                    |p| p.requested_inner,
                )
            })
            .collect();
        let boxes: Vec<_> = requests
            .iter()
            .map(|&p| super::LegendRect::from_normalized(p, canvas))
            .collect();
        let outer = super::outer_allocations(&boxes, canvas, [70., 60., 40., 70.])?;
        // all_axes() always starts with the active axes.
        let placement = super::AxesPlacement {
            requested_inner: requests[0],
            requested_outer: outer[0].normalized(canvas),
            constraint: super::PositionConstraint::OuterPosition,
            imported_solved: false,
        };
        placement.validate()?;
        Ok(placement)
    }

    /// Explicit metadata edit. No derived box is used as a selection key.
    pub fn set_axes_placement(&mut self, mut placement: super::AxesPlacement) -> Result<()> {
        placement.imported_solved = false;
        placement.validate()?;
        let mut candidate = self.clone();
        let current = candidate.normalized_axes_positions()[0].1;
        if placement.requested_inner != current || candidate.active_position().custom.is_none() {
            candidate.reposition_axes_geometry(placement.requested_inner)?;
        }
        candidate.axes.placement = Some(placement);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Data boxes in [left, bottom, width, height] coordinates, relative to the
    /// canvas below a shared title. Matches the default vector-export geometry.
    /// This inspects bounded axes metadata, never sample values.
    pub fn normalized_axes_positions(&self) -> Vec<(Position, [f64; 4])> {
        let scale = self.all_axes().fold(1_f64, |s, (p, _)| {
            let [l, t, r, b] = p.rectangle();
            s.min(r - l).min(b - t)
        });
        let height = if self.layout.as_ref().is_some_and(|l| !l.title.is_empty()) {
            524.
        } else {
            560.
        };
        self.all_axes()
            .map(|(p, _)| {
                let values = p.normalized_plot_box().unwrap_or_else(|| {
                    let [l, t, r, b] = p.rectangle();
                    [
                        l + 70. * scale / 960.,
                        1. - b + 70. * scale / height,
                        r - l - 110. * scale / 960.,
                        b - t - 130. * scale / height,
                    ]
                });
                (p, values)
            })
            .collect()
    }
    /// Move/resize the active axes without replacing any other axes. Grid
    /// positions become equivalent custom data boxes and detach from tile flow.
    /// All validation precedes mutation; sample buffers remain shared.
    pub fn reposition_axes(&mut self, values: [f64; 4]) -> Result<()> {
        let mut candidate = self.clone();
        candidate.reposition_axes_geometry(values)?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    // Compose paired inner/outer edits without validating a transient half of
    // the transaction. Public callers validate the complete candidate before
    // committing it; sample storage remains shared throughout.
    fn reposition_axes_geometry(&mut self, values: [f64; 4]) -> Result<()> {
        if self.empty_layout() {
            return Err("Create an axes before editing its position".into());
        }
        let active = Position::from_normalized(values)?;
        let positions = self.normalized_axes_positions();
        let mut layout = self.layout.clone().unwrap_or(Layout {
            active,
            others: Vec::new(),
            tiled: None,
            title: String::new(),
        });
        layout.active = active;
        layout.tiled = None;
        for panel in &mut layout.others {
            let (_, values) = positions
                .iter()
                .find(|(p, _)| *p == panel.position)
                .unwrap();
            panel.position = Position::from_normalized(*values)?;
        }
        layout.others.sort_by_key(|p| p.position);
        layout.validate()?;
        self.layout = Some(layout);
        if let Some(placement) = &mut self.axes.placement {
            placement.requested_inner = values;
            placement.constraint = super::PositionConstraint::InnerPosition;
            placement.imported_solved = false;
        }
        Ok(())
    }
    pub fn active_position(&self) -> Position {
        self.layout
            .as_ref()
            .map_or(Position::default(), |l| l.active)
    }
    /// Active axes first, followed by the stored inactive axes; no sample copies.
    pub fn all_axes(&self) -> impl Iterator<Item = (Position, &Axes)> {
        std::iter::once((self.active_position(), &self.axes))
            .take(usize::from(!self.empty_layout()))
            .chain(
                self.layout
                    .iter()
                    .flat_map(|l| l.others.iter())
                    .map(|p| (p.position, &p.axes)),
            )
    }
    pub fn all_axes_mut(&mut self) -> impl Iterator<Item = &mut Axes> {
        let count = usize::from(!self.empty_layout());
        std::iter::once(&mut self.axes).take(count).chain(
            self.layout
                .iter_mut()
                .flat_map(|l| l.others.iter_mut())
                .map(|p| &mut p.axes),
        )
    }
    pub fn select_subplot(&mut self, position: Position, replace: bool) -> Result<()> {
        position.validate()?;
        self.validate()?;
        let empty = self.empty_layout();
        let palette_cycle = self
            .appearance
            .and_then(|a| a.series_palette)
            .map(super::PaletteCycle::new);
        let fresh_axes = || Axes {
            palette_cycle,
            ..Axes::default()
        };
        if let Some(layout) = &mut self.layout {
            if !empty && layout.active == position {
                if replace {
                    self.axes = fresh_axes();
                }
                return Ok(());
            }
            if let Some(index) = layout.others.iter().position(|p| p.position == position) {
                // Existing axes need only an ownership swap. Preserve sample buffers,
                // label allocations, layout metadata and the inactive vector capacity.
                std::mem::swap(&mut self.axes, &mut layout.others[index].axes);
                std::mem::swap(&mut layout.active, &mut layout.others[index].position);
                if replace {
                    self.axes = fresh_axes();
                }
                layout.others.sort_by_key(|p| p.position);
                return Ok(());
            }
            let retained = layout
                .others
                .iter()
                .filter(|p| !p.position.overlaps(position))
                .count()
                + usize::from(!empty && !layout.active.overlaps(position));
            if retained >= 16 {
                return Err("At most 16 subplot axes per figure".into());
            }
        }
        let title = self
            .layout
            .as_ref()
            .map_or_else(String::new, |l| l.title.clone());
        let previous = std::mem::replace(&mut self.axes, fresh_axes());
        let mut panels = if let Some(mut layout) = self.layout.take() {
            if !empty {
                layout.others.push(Panel {
                    position: layout.active,
                    axes: previous,
                });
            }
            layout.others
        } else {
            if position == Position::default() && !replace {
                self.axes = previous;
            }
            Vec::new()
        };
        if !replace && let Some(index) = panels.iter().position(|p| p.position == position) {
            self.axes = panels.remove(index).axes;
        }
        panels.retain(|p| !p.position.overlaps(position));
        panels.sort_by_key(|p| p.position);
        self.layout = Some(Layout {
            active: position,
            others: panels,
            tiled: None,
            title,
        });
        Ok(())
    }
}
