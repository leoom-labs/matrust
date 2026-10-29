//! Prepared scene painting independent of Workspace selection and window ownership.
use super::Geometry;
use crate::{figure_target::Key, figure_view, worker};
use eframe::egui;
use matrust::{
    Result,
    plot::{Figure, Position, SampleQuery},
};
use std::sync::atomic::{AtomicU64, Ordering};

/// One live view identity. Reordering rectangles does not replace it; closing or
/// reusing a figure number does. The token holds no model or prepared point data.
pub(crate) struct SurfaceIdentity {
    key: Key,
    id: egui::Id,
}
impl SurfaceIdentity {
    // The actual tiled surface is a separate layer; detached keeps its old IDs.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(key: Key) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let generation = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(1)
            })
            .map_err(|_| "Figure view generation limit reached")?
            + 1;
        let id = egui::Id::new(("figure_scene", key.number, generation));
        Ok(Self { key, id })
    }
    pub fn matches(&self, key: &Key) -> bool {
        self.key.same(key)
    }
    pub fn plot_id(&self) -> egui::Id {
        self.id
    }
    /// Retire this view's plot/legend state without touching another view of
    /// the same figure. Pass the retained painted source, including any axes
    /// removed by a later revision. The owner separately drops its cache.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn forget(&self, ctx: &egui::Context, source: &Figure) -> Result<()> {
        if !self.key.identity.same_object(&source.identity) {
            return Err("This figure view belongs to a different document".into());
        }
        figure_view::forget_prepared(ctx, self.id);
        for (position, _) in source.all_axes() {
            figure_view::forget_prepared(ctx, self.id.with(("axes", position)));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Identity<'a> {
    LegacyDetached,
    #[cfg_attr(not(test), allow(dead_code))]
    Explicit(&'a SurfaceIdentity),
    ExplicitFitted(&'a SurfaceIdentity),
}

/// All inputs belong to this live figure, not the globally selected document.
/// `prepared_source` is the immutable source retained with the worker result.
pub(crate) struct Frame<'a> {
    pub key: &'a Key,
    pub revision: u64,
    pub figure: &'a Figure,
    pub prepared_source: &'a Figure,
    pub marker: Option<(Position, [f64; 2])>,
    pub restored_view: Option<worker::HistoryView>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct View {
    pub bounds: [f64; 4],
    pub auto: [bool; 2],
}
pub(crate) enum Interaction {
    Edit(Box<worker::FigureEdit>, bool),
    Pick(Position, SampleQuery),
}
pub(crate) struct Outcome {
    pub key: Key,
    pub revision: u64,
    pub view: Option<View>,
    pub interaction: Option<Interaction>,
}
impl Outcome {
    /// Validate again at final-pass admission; stale paint is never authority.
    pub fn matches(&self, key: &Key, revision: u64) -> bool {
        self.key.same(key) && self.revision == revision
    }
}

/// This function neither prepares geometry nor mutates an Engine/Workspace.
/// The caller owns busy/modal/final-pass admission and the aggregate cache budget.
pub(crate) fn show(
    ui: &mut egui::Ui,
    frame: Frame<'_>,
    geometry: &mut Geometry,
    reset: bool,
    identity: Identity<'_>,
) -> Result<Outcome> {
    if !frame.key.identity.same_object(&frame.figure.identity)
        || !frame
            .prepared_source
            .identity
            .same_object(&frame.figure.identity)
        || !frame.prepared_source.same_snapshot(frame.figure)
        || matches!(identity, Identity::Explicit(view) | Identity::ExplicitFitted(view) if !view.matches(frame.key))
    {
        return Err("Figure changed; preparing its display".into());
    }
    let mut outcome = Outcome {
        key: frame.key.clone(),
        revision: frame.revision,
        view: None,
        interaction: None,
    };
    match geometry {
        Geometry::Ordinary(data) => {
            if frame.figure.layout.is_some() {
                return Err("Figure changed; preparing its display".into());
            }
            let mut displayed = std::borrow::Cow::Borrowed(frame.figure);
            if reset && let Some((bounds, auto)) = frame.restored_view {
                let figure = displayed.to_mut();
                if !auto[0] {
                    figure.xlim = Some([bounds[0], bounds[1]]);
                }
                if !auto[1] {
                    figure.ylim = Some([bounds[2], bounds[3]]);
                }
            }
            let shown = match identity {
                Identity::LegacyDetached => figure_view::show_interactive_prepared(
                    ui,
                    &displayed,
                    data.view(),
                    reset,
                    frame.marker.map(|(_, p)| p),
                ),
                Identity::Explicit(view) => figure_view::show_interactive_prepared_with_id(
                    ui,
                    view.plot_id(),
                    &displayed,
                    data.view(),
                    reset,
                    frame.marker.map(|(_, p)| p),
                ),
                Identity::ExplicitFitted(view) => {
                    figure_view::show_interactive_prepared_fitted_with_id(
                        ui,
                        view.plot_id(),
                        &displayed,
                        data.view(),
                        reset,
                        frame.marker.map(|(_, p)| p),
                    )
                }
            };
            if let Some(visibility) = shown.visibility {
                let mut updated = frame.figure.clone();
                for (series, visible) in updated.series.iter_mut().zip(visibility) {
                    series.visible = visible;
                }
                let mut edit = worker::FigureEdit {
                    id: frame.key.number,
                    original: frame.figure.clone(),
                    updated,
                    view_before: None,
                };
                edit.preserve_manual_view(Some(shown.bounds), shown.auto);
                outcome.interaction = Some(Interaction::Edit(Box::new(edit), true));
            }
            // Preserve existing detached precedence when one frame picks a sample.
            if let Some(query) = shown.pick {
                outcome.interaction =
                    Some(Interaction::Pick(frame.figure.active_position(), query));
            }
            outcome.view = Some(View {
                bounds: shown.bounds,
                auto: shown.auto,
            });
        }
        Geometry::Subplots(canvas) => {
            let updated = match identity {
                Identity::LegacyDetached => {
                    canvas.show_prepared(ui, frame.key.number, frame.figure, reset, frame.marker)?
                }
                Identity::Explicit(view) => canvas.show_prepared_with_id(
                    ui,
                    view.plot_id(),
                    frame.key.number,
                    frame.figure,
                    reset,
                    frame.marker,
                )?,
                Identity::ExplicitFitted(view) => canvas.show_prepared_fitted_with_id(
                    ui,
                    view.plot_id(),
                    frame.key.number,
                    frame.figure,
                    reset,
                    frame.marker,
                )?,
            };
            if let Some((position, query)) = canvas.pick.take() {
                outcome.interaction = Some(Interaction::Pick(position, query));
            } else if let Some(updated) = updated {
                outcome.interaction = Some(Interaction::Edit(
                    Box::new(worker::FigureEdit {
                        id: frame.key.number,
                        original: frame.figure.clone(),
                        updated,
                        view_before: None,
                    }),
                    canvas.undoable_change,
                ));
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "figure_scene_tests.rs"]
mod tests;
