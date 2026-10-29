//! Bounded tiled layouts. Planning touches at most 16 axes descriptors, never samples.
use super::{Axes, Figure, Layout, Position};
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arrangement {
    Fixed,
    Flow,
    Vertical,
    Horizontal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileGrid {
    pub arrangement: Arrangement,
    pub rows: usize,
    pub columns: usize,
    pub pending_first: bool,
}
impl TileGrid {
    pub fn new(arrangement: Arrangement, rows: usize, columns: usize) -> Result<Self> {
        let grid = Self {
            arrangement,
            rows,
            columns,
            pending_first: true,
        };
        grid.validate()?;
        Ok(grid)
    }
    pub fn validate(self) -> Result<()> {
        if !(1..=16).contains(&self.rows)
            || !(1..=16).contains(&self.columns)
            || (self.arrangement == Arrangement::Vertical && self.columns != 1)
            || (self.arrangement == Arrangement::Horizontal && self.rows != 1)
        {
            return Err(
                "Tiled layouts require 1..16 rows/columns; stacks have one row or column".into(),
            );
        }
        Ok(())
    }
    /// One-based upper-left tile and [row span, column span].
    pub fn cell(self, p: Position) -> Result<(usize, [usize; 2])> {
        if p.custom.is_some() {
            return Err("Custom-position axes cannot belong to a tiled grid".into());
        }
        self.validate()?;
        p.validate()?;
        let top = p.top as usize * self.rows;
        let bottom = p.bottom as usize * self.rows;
        let left = p.left as usize * self.columns;
        let right = p.right as usize * self.columns;
        if !top.is_multiple_of(p.rows as usize)
            || !bottom.is_multiple_of(p.rows as usize)
            || !left.is_multiple_of(p.columns as usize)
            || !right.is_multiple_of(p.columns as usize)
        {
            return Err("Axes position does not align with the tiled grid".into());
        }
        let row = top / p.rows as usize;
        let col = left / p.columns as usize;
        Ok((
            row * self.columns + col + 1,
            [
                bottom / p.rows as usize - row,
                right / p.columns as usize - col,
            ],
        ))
    }
    fn position(self, tile: usize, span: [usize; 2]) -> Option<Position> {
        if tile == 0 || tile > self.rows * self.columns || span.contains(&0) {
            return None;
        }
        let row = (tile - 1) / self.columns;
        let col = (tile - 1) % self.columns;
        if span[0] > self.rows - row || span[1] > self.columns - col {
            return None;
        }
        Position::from_grid(
            self.rows,
            self.columns,
            &[tile, (row + span[0] - 1) * self.columns + col + span[1]],
        )
        .ok()
    }
    fn first_empty(self, occupied: &[Position], span: [usize; 2]) -> Option<Position> {
        (1..=self.rows * self.columns)
            .filter_map(|i| self.position(i, span))
            .find(|p| occupied.iter().all(|q| !p.overlaps(*q)))
    }
}
impl Figure {
    pub fn tiled_layout(grid: TileGrid) -> Result<Self> {
        grid.validate()?;
        let grid = TileGrid {
            pending_first: true,
            ..grid
        };
        Ok(Self {
            identity: crate::ObjectIdentity::default(),
            textboxes: Vec::new(),
            name: String::new(),
            color: [255; 3],
            appearance: Some(super::FigureAppearance::default()),
            invert_hardcopy: None,
            axes: Axes {
                palette_cycle: Some(super::PaletteCycle::new(
                    super::PaletteVersion::MatRustModern1,
                )),
                ..Axes::default()
            },
            layout: Some(Layout {
                active: grid.position(1, [1, 1]).unwrap(),
                others: Vec::new(),
                tiled: Some(grid),
                title: String::new(),
            }),
        })
    }
    pub fn empty_layout(&self) -> bool {
        self.layout
            .as_ref()
            .and_then(|l| l.tiled)
            .is_some_and(|g| g.pending_first)
    }
    /// Plotting or setting axes properties implicitly creates the first axes.
    pub fn activate_tile(&mut self) {
        if let Some(grid) = self.layout.as_mut().and_then(|l| l.tiled.as_mut()) {
            grid.pending_first = false;
        }
    }
    pub fn set_shared_title(&mut self, title: String) -> Result<()> {
        if title.len() > 4096 {
            return Err("Shared figure title exceeds 4096 bytes".into());
        }
        self.layout
            .get_or_insert_with(|| Layout {
                active: Position::default(),
                others: Vec::new(),
                tiled: None,
                title: String::new(),
            })
            .title = title;
        Ok(())
    }
    /// Select an existing tile or allocate the next unoccupied rectangular region.
    /// The candidate is validated before replacing self; samples remain shared.
    pub fn next_tile(&mut self, tile: Option<usize>, span: Option<[usize; 2]>) -> Result<()> {
        self.validate()?;
        if tile.is_some_and(|n| n == 0 || n > 256)
            || span.is_some_and(|s| s.iter().any(|n| !(1..=16).contains(n)))
        {
            return Err("Tile index must be 1..256 and span dimensions 1..16".into());
        }
        if let Some(grid) = self.layout.as_ref().and_then(|l| l.tiled)
            && let Some(tile) = tile
        {
            let existing = self.all_axes().find_map(|(p, _)| {
                let (index, current_span) = grid.cell(p).ok()?;
                (index == tile && span.is_none_or(|s| s == current_span)).then_some(p)
            });
            if let Some(position) = existing {
                return self.select_subplot(position, false);
            }
        }
        let mut candidate = if self.layout.as_ref().and_then(|l| l.tiled).is_some() {
            self.clone()
        } else {
            Self::tiled_layout(TileGrid::new(Arrangement::Flow, 1, 1)?)?
        };
        candidate.identity = self.identity.clone();
        candidate.name = self.name.clone();
        candidate.color = self.color;
        candidate.appearance = self.appearance;
        candidate.invert_hardcopy = self.invert_hardcopy;
        if candidate.empty_layout() {
            candidate.axes = candidate.fresh_axes();
        }
        let grid = candidate.layout.as_ref().unwrap().tiled.unwrap();
        let span = span.unwrap_or([1, 1]);
        let mut occupied: Vec<_> = candidate.all_axes().map(|(p, _)| p).collect();
        occupied.sort();
        let find = |g: TileGrid, positions: &[Position]| match tile {
            Some(i) => g.position(i, span),
            None => g.first_empty(positions, span),
        };
        let mut selected = find(grid, &occupied).map(|p| (grid, occupied.clone(), p));
        if selected.is_none() && grid.arrangement != Arrangement::Fixed {
            // Flow grows deterministically toward a square. Existing spans
            // pack in reading order; stacks retain their one-dimensional order.
            let mut grids = Vec::new();
            for rows in 1usize..=16 {
                for columns in 1usize..=16 {
                    if rows * columns <= grid.rows * grid.columns {
                        continue;
                    }
                    let g = TileGrid {
                        rows,
                        columns,
                        ..grid
                    };
                    if g.validate().is_ok() {
                        grids.push(g);
                    }
                }
            }
            grids.sort_by_key(|g| {
                (
                    g.rows.max(g.columns),
                    g.rows * g.columns,
                    g.rows.abs_diff(g.columns),
                    g.rows,
                )
            });
            for g in grids {
                let mut positions = Vec::with_capacity(occupied.len());
                for &p in &occupied {
                    let old_span = grid.cell(p)?.1;
                    if let Some(p) = g.first_empty(&positions, old_span) {
                        positions.push(p);
                    } else {
                        break;
                    }
                }
                if positions.len() == occupied.len()
                    && let Some(p) = find(g, &positions)
                {
                    selected = Some((g, positions, p));
                    break;
                }
            }
        }
        let (mut next_grid, positions, position) =
            selected.ok_or("No space for this tile in the layout (maximum 16 rows/columns)")?;
        if next_grid.rows != grid.rows || next_grid.columns != grid.columns {
            let layout = candidate.layout.as_mut().unwrap();
            if !grid.pending_first {
                layout.active = positions[occupied.binary_search(&layout.active).unwrap()];
                for panel in &mut layout.others {
                    panel.position = positions[occupied.binary_search(&panel.position).unwrap()];
                }
                layout.others.sort_by_key(|p| p.position);
            } else {
                layout.active = next_grid.position(1, [1, 1]).unwrap();
            }
            layout.tiled = Some(next_grid);
        }
        candidate.select_subplot(position, false)?;
        next_grid.pending_first = false;
        candidate.layout.as_mut().unwrap().tiled = Some(next_grid);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}
