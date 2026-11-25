//! `delete` dispatches app-owned graphics before the file-delete overload.
use super::{Array, Engine, GraphicsKind, Result, Value};
use crate::ObjectIdentity;
use crate::plot::{Arrangement, Figure, Layout, Position, TileGrid};
use std::collections::BTreeMap;

/// Stage every target before publishing, so invalid/mixed targets and
/// cancellation preserve the graph. Sample buffers remain shared.
pub(in crate::engine) fn graphics_delete_lines(
    e: &mut Engine,
    args: &[Value],
    out: &mut super::super::Execution,
) -> Result<Value> {
    let [Value::Container(Array::Graphics(handles))] = args else {
        return Err("delete graphics requires one typed graphics handle array".into());
    };
    if (0..handles.len()).all(|i| {
        handles
            .get_linear(i)
            .is_ok_and(|h| h.kind() == GraphicsKind::UiComponent)
    }) {
        super::super::ui_components::delete_handles(e, handles, out)?;
        return Ok(Value::Void);
    }
    // delete(fig) closes classic figures without CloseRequestFcn, like
    // close(fig,'force'); uicontrols and uifigures in the same array go too.
    if (0..handles.len()).all(|i| {
        handles
            .get_linear(i)
            .is_ok_and(|h| matches!(h.kind(), GraphicsKind::Figure | GraphicsKind::UiComponent))
    }) {
        let mut figures = Vec::new();
        for i in 0..handles.len() {
            e.check_cancel()?;
            let h = handles.get_linear(i)?;
            if h.kind() == GraphicsKind::Figure {
                let id = e.locate_graphics(h)?.figure;
                if super::super::ui_components::is_uiaxes_fig_id(id) {
                    return Err("delete cannot delete the figure of a uiaxes".into());
                }
                figures.push((id, h.object.clone()));
            }
        }
        // Skips the figure handles.
        super::super::ui_components::delete_handles(e, handles, out)?;
        for (id, identity) in figures {
            e.close_existing_figure(id, &identity)?;
        }
        return Ok(Value::Void);
    }
    if handles.len() > crate::plot::MAX_PLOT_SERIES {
        return Err("delete graphics exceeds plot handle budget".into());
    }
    let mut staged = BTreeMap::new();
    for i in 0..handles.len() {
        e.check_cancel()?;
        let h = handles.get_linear(i)?;
        if !matches!(
            h.kind(),
            GraphicsKind::Series
                | GraphicsKind::Text
                | GraphicsKind::Axes
                | GraphicsKind::ConstantLine
                | GraphicsKind::Rectangle
                | GraphicsKind::Annotation
                | GraphicsKind::TextBox
                | GraphicsKind::Volume
        ) {
            return Err("delete graphics supports series, text, constantline, rectangle, annotation, textbox, axes and volume handles".into());
        }
        if h.kind() == GraphicsKind::Axes && e.ui.is_uiaxes(&h.object) {
            e.ui.delete_subtree(h.object.recovery_key());
            continue;
        }
        let target = e.locate_graphics(h)?;
        let original = e.graphics_figure(target.figure)?;
        let figure = staged
            .entry(target.figure)
            .or_insert_with(|| original.clone());
        if h.kind() == GraphicsKind::Annotation {
            figure
                .annotations
                .retain(|a| !a.identity.same_object(&h.object));
            continue;
        }
        if h.kind() == GraphicsKind::TextBox {
            figure
                .textboxes
                .retain(|t| !t.identity.same_object(&h.object));
            continue;
        }
        if h.kind() == GraphicsKind::Axes {
            super::super::volume::slice_viewer::retire_axes_viewer(figure, &h.object)?;
            remove_axes(figure, &h.object)?;
            continue;
        }
        let axes_id = original
            .all_axes()
            .find_map(|(p, axes)| (Some(p) == target.position).then_some(&axes.identity))
            .ok_or("Graphics axes is no longer live")?;
        let axes = figure
            .all_axes_mut()
            .find(|a| axes_id.same_object(&a.identity))
            .ok_or("Graphics axes is no longer live")?;
        if h.kind() == GraphicsKind::Volume {
            axes.extras
                .volumes
                .retain(|v| !v.identity.same_object(&h.object));
            continue;
        }
        if h.kind() == GraphicsKind::Text {
            axes.extras
                .texts
                .retain(|t| !t.identity.same_object(&h.object));
            continue;
        }
        if h.kind() == GraphicsKind::ConstantLine {
            axes.extras
                .constant_lines
                .retain(|l| !l.identity.same_object(&h.object));
            continue;
        }
        if h.kind() == GraphicsKind::Rectangle {
            axes.extras
                .rectangles
                .retain(|r| !r.identity.same_object(&h.object));
            continue;
        }
        if let Some(index) = axes
            .series
            .iter()
            .position(|s| s.identity.same_object(&h.object))
        {
            axes.remove_series(index)?;
            super::super::plot_properties::refresh_scatter_clim(axes);
        }
    }
    for figure in staged.values() {
        figure.validate()?;
    }
    e.check_cancel()?;
    for (number, figure) in staged {
        e.publish_graphics_figure(number, figure);
    }
    Ok(Value::Void)
}

pub(in crate::engine) fn remove_axes(figure: &mut Figure, id: &ObjectIdentity) -> Result<()> {
    if figure.axes.identity.same_object(id) {
        if let Some(layout) = &mut figure.layout
            && !layout.others.is_empty()
        {
            let panel = layout.others.remove(0);
            figure.axes = panel.axes;
            layout.active = panel.position;
        } else {
            figure.axes = figure.fresh_axes();
            let title = figure
                .layout
                .as_ref()
                .map_or_else(String::new, |l| l.title.clone());
            figure.layout = Some(Layout {
                tiled: Some(TileGrid::new(Arrangement::Fixed, 1, 1)?),
                title,
                ..Layout::plain(Position::default())
            });
        }
    } else if let Some(layout) = &mut figure.layout {
        layout.others.retain(|p| !p.axes.identity.same_object(id));
    }
    figure.prune_linked_axes();
    Ok(())
}
