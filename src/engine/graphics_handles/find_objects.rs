//! Bounded property lookup in the graphics and UI hierarchy (`findobj`, `findall`, `allchild`).
//! This fixes the advertised figure-companion path without adding a generic Engine method.
use super::super::ui_components::{HandleVisibility, UiSpecific, is_uiaxes_fig_id};
use super::{
    Array, Engine, GraphicsArray, GraphicsHandle, GraphicsKind, Result, Value, handle,
    handle_or_root, text,
};
use crate::plot::{Axes, Figure, MAX_PLOT_SERIES, MAX_TEXTS, PlotKind, Series};
use std::sync::Arc;

struct FindFilter<'a> {
    kind: Option<String>,
    number: Option<f64>,
    label: Option<&'a str>,
    tag: Option<String>,
    style: Option<String>,
    handle_visibility: Option<HandleVisibility>,
    visible: Option<bool>,
}

pub(super) fn find_objects(engine: &Engine, args: &[Value]) -> Result<Value> {
    engine.check_cancel()?;
    let (scope, properties) = match args.split_first() {
        Some((Value::Text(_), _)) => (None, args),
        Some((scope, rest)) => (Some(scope), rest),
        None => return Err("findobj requires a Type filter: figure, axes, line or volume".into()),
    };
    let (depth, filters) = search_depth(properties, "findobj")?;
    find_objects_inner(engine, scope, filters, depth, false, "findobj")
}

fn search_depth<'a>(properties: &'a [Value], name: &str) -> Result<(f64, &'a [Value])> {
    if !matches!(properties.first(), Some(Value::Text(option)) if option.eq_ignore_ascii_case("-depth"))
    {
        return Ok((f64::INFINITY, properties));
    }
    let [_, Value::Matrix(m), rest @ ..] = properties else {
        return Err(format!(
            "{name} -depth requires a nonnegative integer or Inf"
        ));
    };
    let depth = m.as_scalar()?;
    if !(depth == f64::INFINITY || depth >= 0. && depth.fract() == 0.) {
        return Err(format!(
            "{name} -depth requires a nonnegative integer or Inf"
        ));
    }
    Ok((depth, rest))
}

fn find_objects_inner(
    engine: &Engine,
    scope: Option<&Value>,
    properties: &[Value],
    depth: f64,
    include_hidden: bool,
    fn_name: &str,
) -> Result<Value> {
    if properties.is_empty() || properties.len() > 12 || !properties.len().is_multiple_of(2) {
        return Err(format!(
            "{fn_name} supports at most Type, Number, DisplayName, Tag, Style and HandleVisibility property/value pairs"
        ));
    }
    let mut filter = FindFilter {
        kind: None,
        number: None,
        label: None,
        tag: None,
        style: None,
        handle_visibility: None,
        visible: None,
    };
    for pair in properties.as_chunks::<2>().0 {
        match text(&pair[0])?.to_ascii_lowercase().as_str() {
            "type" if filter.kind.is_none() => {
                let value = text(&pair[1])?.to_ascii_lowercase();
                if !is_supported_type(&value) {
                    return Err(format!(
                        "{fn_name} currently supports Type figure, axes or line, or volume, text, scatter, bar, stair, stem, patch, image, constantline, rectangle or UI component types"
                    ));
                }
                filter.kind = Some(value);
            }
            "number" if filter.number.is_none() => {
                let Value::Matrix(m) = &pair[1] else {
                    return Err(format!("{fn_name} Number must be a real scalar"));
                };
                if m.len() != 1 || !m.data()[0].is_finite() {
                    return Err(format!("{fn_name} Number must be a finite scalar"));
                }
                filter.number = Some(m.data()[0]);
            }
            "displayname" if filter.label.is_none() => {
                filter.label = Some(text(&pair[1])?);
            }
            "tag" if filter.tag.is_none() => {
                filter.tag = Some(text(&pair[1])?.to_owned());
            }
            "style" if filter.style.is_none() => {
                filter.style = Some(text(&pair[1])?.to_ascii_lowercase());
            }
            "visible" if include_hidden && filter.visible.is_none() => {
                filter.visible = Some(visible_filter(&pair[1])?);
            }
            "handlevisibility" if filter.handle_visibility.is_none() => {
                filter.handle_visibility = Some(HandleVisibility::parse(text(&pair[1])?)?);
            }
            _ => {
                return Err(format!(
                    "{fn_name} supports unique Type, Number, DisplayName, Tag, Style and HandleVisibility filters"
                ));
            }
        }
    }

    let effective_hidden = include_hidden || engine.root.show_hidden_handles;
    let in_callback = !engine.ui.callback_stack.is_empty();

    // Direct UI component or uiaxes handle scope.
    if let Some(val) = scope
        && let Value::Container(Array::Graphics(arr)) = val
        && arr.len() == 1
        && let Ok(h) = arr.scalar_handle()
    {
        if h.kind() == GraphicsKind::UiComponent {
            let key = h.object.recovery_key();
            if !engine.ui.nodes.contains_key(&key) {
                return Err("Invalid or deleted object.".into());
            }
            let mut found = Vec::new();
            walk_ui_subtree(
                engine,
                vec![(key, 0, true)],
                depth,
                &filter,
                effective_hidden,
                in_callback,
                &mut found,
            )?;
            return Ok(Value::Container(
                GraphicsArray::from_shared(found.len(), 1, found, &engine.cancel)?.into(),
            ));
        }
        if h.kind() == GraphicsKind::Axes && engine.ui.is_uiaxes(&h.object) {
            let key = h.object.recovery_key();
            let mut found = Vec::new();
            walk_ui_subtree(
                engine,
                vec![(key, 0, true)],
                depth,
                &filter,
                effective_hidden,
                in_callback,
                &mut found,
            )?;
            return Ok(Value::Container(
                GraphicsArray::from_shared(found.len(), 1, found, &engine.cancel)?.into(),
            ));
        }
    }

    // Classic/root scope: require Type unless Tag, Style or HandleVisibility is specified.
    let has_non_classic_filter =
        filter.tag.is_some() || filter.style.is_some() || filter.handle_visibility.is_some();
    if filter.kind.is_none() && !has_non_classic_filter && filter.visible.is_none() {
        return Err(format!(
            "{fn_name} requires a Type filter: figure, axes, line or volume"
        ));
    }

    let mut scope_handle: Option<&GraphicsHandle> = None;
    let mut scope_number: Option<u32> = None;
    let mut scope_figure: Option<u32> = None;
    let mut scope_position = None;
    let scope_level: i32 = match scope {
        None => 0,
        Some(Value::Matrix(m)) if m.len() == 1 && m.data()[0] == 0. => 0,
        Some(Value::Matrix(m))
            if m.len() == 1
                && (1. ..=1_000_000.).contains(&m.data()[0])
                && m.data()[0].fract() == 0. =>
        {
            let id = m.data()[0] as u32;
            engine.graphics_figure(id)?;
            scope_number = Some(id);
            1
        }
        Some(scope_val) => {
            let h = handle(scope_val)?;
            let level = match h.kind() {
                GraphicsKind::Root => 0,
                GraphicsKind::Figure => 1,
                GraphicsKind::Axes => 2,
                GraphicsKind::Series
                | GraphicsKind::Volume
                | GraphicsKind::Text
                | GraphicsKind::Image
                | GraphicsKind::ConstantLine
                | GraphicsKind::Rectangle => 3,
                _ => {
                    return Err(format!(
                        "{fn_name} scope must be root or one live figure, axes, line, volume or UI component"
                    ));
                }
            };
            if level != 0 {
                let location = engine.locate_graphics(h)?;
                if is_uiaxes_fig_id(location.figure) {
                    let mut found = Vec::new();
                    if 0.0 <= depth {
                        let backing_fig = engine.graphics_figure(location.figure)?;
                        append_axes_graphics_children(
                            engine,
                            backing_fig,
                            &backing_fig.axes,
                            &filter,
                            Some(h),
                            &mut found,
                        )?;
                    }
                    return Ok(Value::Container(
                        GraphicsArray::from_shared(found.len(), 1, found, &engine.cancel)?.into(),
                    ));
                }
                scope_figure = Some(location.figure);
                scope_position = location.position;
                scope_handle = Some(h);
            }
            level
        }
    };

    let mut found: Vec<Arc<GraphicsHandle>> = Vec::new();

    // 1. Search classic figures (preserving exact figure-number, axes-position and line order).
    let mut figures = engine.figures().collect::<Vec<_>>();
    if figures.len() > 16 {
        return Err(format!("{fn_name} exceeds 16 figures"));
    }
    figures.sort_by_key(|(id, _)| *id);

    let want_classic_figures = filter.kind.as_deref().is_none_or(|k| k == "figure")
        && filter.tag.is_none()
        && filter.style.is_none()
        && filter.label.is_none();

    if want_classic_figures {
        let fig_dist = 1 - scope_level;
        if fig_dist >= 0 && f64::from(fig_dist) <= depth {
            if filter
                .handle_visibility
                .is_none_or(|hv| hv == HandleVisibility::On)
            {
                for &(id, figure) in &figures {
                    engine.check_cancel()?;
                    let fig_handle = Arc::new(GraphicsHandle::figure(&figure.identity));
                    if scope_number.is_some_and(|n| n != id)
                        || scope_handle.is_some_and(|h| !h.figure.same_object(&fig_handle.figure))
                    {
                        continue;
                    }
                    if filter.number.is_none_or(|n| n == f64::from(id))
                        && filter.visible.is_none_or(|v| {
                            v == (figure.visible && !engine.is_figure_hidden(&figure.identity))
                        })
                    {
                        found.push(fig_handle);
                    }
                }
            }
            if filter.number.is_none()
                && scope_number.is_none()
                && filter
                    .handle_visibility
                    .is_none_or(|hv| hv == HandleVisibility::Off)
            {
                for h in engine.dialogs.hidden_handles() {
                    engine.check_cancel()?;
                    if scope_handle.is_some_and(|scope| !scope.figure.same_object(&h.figure)) {
                        continue;
                    }
                    if (effective_hidden || scope_handle.is_some())
                        && filter.visible.is_none_or(|v| v == dialog_shown(engine, &h))
                    {
                        found.push(Arc::new(h));
                    }
                }
            }
        }
    }

    let want_classic_axes = filter.kind.as_deref().is_none_or(|k| k == "axes")
        && filter.number.is_none()
        && filter.label.is_none()
        && filter.tag.is_none()
        && filter.style.is_none()
        && filter
            .handle_visibility
            .is_none_or(|hv| hv == HandleVisibility::On);
    let axes_dist = 2 - scope_level;
    let visit_classic_axes = want_classic_axes && axes_dist >= 0 && f64::from(axes_dist) <= depth;

    let child_dist = 3 - scope_level;
    let visit_classic_children = child_dist >= 0
        && f64::from(child_dist) <= depth
        && filter.number.is_none()
        && filter.tag.is_none()
        && filter.style.is_none()
        && filter
            .handle_visibility
            .is_none_or(|hv| hv == HandleVisibility::On)
        && filter.kind.as_deref().is_none_or(is_axes_child_type);

    if visit_classic_axes || visit_classic_children {
        for &(id, f) in &figures {
            engine.check_cancel()?;
            if scope_figure.or(scope_number).is_some_and(|n| n != id) {
                continue;
            }
            let mut axes = f.all_axes().collect::<Vec<_>>();
            if axes.len() > 16 {
                return Err(format!("{fn_name} exceeds 16 axes per figure"));
            }
            axes.sort_by_key(|(p, _)| *p);
            for (position, a) in axes {
                engine.check_cancel()?;
                if scope_position.is_some_and(|p| position != p) {
                    continue;
                }
                if visit_classic_axes && filter.visible.is_none_or(|v| v != a.extras.axis_off) {
                    found.push(Arc::new(GraphicsHandle::axes(&f.identity, &a.identity)));
                }
                if visit_classic_children {
                    append_axes_graphics_children(engine, f, a, &filter, scope_handle, &mut found)?;
                }
            }
        }
    }

    // 2. Search UI children attached to classic figures (uicontrol / uiaxes)
    //    and, from root, the UIFigures.
    append_ui_descendants(
        engine,
        scope_level,
        scope_figure.or(scope_number),
        &figures,
        depth,
        &filter,
        effective_hidden,
        in_callback,
        &mut found,
    )?;

    engine.check_cancel()?;
    Ok(Value::Container(
        GraphicsArray::from_shared(found.len(), 1, found, &engine.cancel)?.into(),
    ))
}

/// UI descendants of a classic scope (root = level 0, one classic figure =
/// level 1): the uicontrols and uiaxes of the figures in scope and, from
/// root, the UIFigures. `figures` are the classic figures in number order.
#[allow(clippy::too_many_arguments)]
fn append_ui_descendants(
    engine: &Engine,
    scope_level: i32,
    scope_figure: Option<u32>,
    figures: &[(u32, &Figure)],
    depth: f64,
    filter: &FindFilter<'_>,
    effective_hidden: bool,
    in_callback: bool,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    // Search UI children attached to classic figures (uicontrol / uiaxes).
    if scope_level <= 1 {
        let mut ui_roots: Vec<(usize, usize, bool)> = Vec::new();
        let child_depth = (2 - scope_level) as usize;
        if scope_level == 0 {
            for &(_, f) in figures {
                if let Some(children) = engine
                    .ui
                    .classic_fig_children
                    .get(&f.identity.recovery_key())
                {
                    for &ck in children {
                        ui_roots.push((ck, child_depth, false));
                    }
                }
            }
        } else if let Some(target_id) = scope_figure
            && let Ok(fig) = engine.graphics_figure(target_id)
            && let Some(children) = engine
                .ui
                .classic_fig_children
                .get(&fig.identity.recovery_key())
        {
            for &ck in children {
                ui_roots.push((ck, child_depth, false));
            }
        }
        // 3. When searching from root (0 / groot / omitted), also search UIFigures
        //    whose HandleVisibility allows visibility (or all for findall / ShowHiddenHandles).
        if scope_level == 0 {
            for &fk in &engine.ui.figures {
                ui_roots.push((fk, 1, false));
            }
        }
        if !ui_roots.is_empty() {
            walk_ui_subtree(
                engine,
                ui_roots,
                depth,
                filter,
                effective_hidden,
                in_callback,
                found,
            )?;
        }
    }
    Ok(())
}

fn is_supported_type(value: &str) -> bool {
    matches!(
        value,
        "figure"
            | "axes"
            | "line"
            | "volume"
            | "text"
            | "scatter"
            | "bar"
            | "stair"
            | "stem"
            | "patch"
            | "image"
            | "constantline"
            | "rectangle"
            | "uicontrol"
            | "uibutton"
            | "uistatebutton"
            | "uilabel"
            | "uieditfield"
            | "uinumericeditfield"
            | "uitextarea"
            | "uidropdown"
            | "uilistbox"
            | "uicheckbox"
            | "uibuttongroup"
            | "uiradiobutton"
            | "uislider"
            | "uispinner"
            | "uiswitch"
            | "uitable"
            | "uipanel"
            | "uigridlayout"
            | "uitabgroup"
            | "uitab"
            | "uiprogressdlg"
    )
}

fn is_axes_child_type(value: &str) -> bool {
    matches!(
        value,
        "line"
            | "volume"
            | "text"
            | "scatter"
            | "bar"
            | "stair"
            | "stem"
            | "patch"
            | "image"
            | "constantline"
            | "rectangle"
    )
}

fn series_type_name(kind: PlotKind) -> &'static str {
    match kind {
        PlotKind::Line => "line",
        PlotKind::Scatter => "scatter",
        PlotKind::Bar => "bar",
        PlotKind::Stairs => "stair",
        PlotKind::Stem => "stem",
        PlotKind::Patch => "patch",
    }
}

fn append_axes_graphics_children(
    engine: &Engine,
    f: &Figure,
    a: &Axes,
    filter: &FindFilter<'_>,
    scope_handle: Option<&GraphicsHandle>,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    if filter.number.is_some()
        || filter.tag.is_some()
        || filter.style.is_some()
        || filter
            .handle_visibility
            .is_some_and(|hv| hv != HandleVisibility::On)
    {
        return Ok(());
    }
    let kind = filter.kind.as_deref();

    // Series (line, scatter, bar, stair, stem, patch)
    if kind.is_none_or(|k| matches!(k, "line" | "scatter" | "bar" | "stair" | "stem" | "patch"))
        && scope_handle.is_none_or(|h| {
            !matches!(
                h.kind(),
                GraphicsKind::Volume
                    | GraphicsKind::Text
                    | GraphicsKind::Image
                    | GraphicsKind::ConstantLine
                    | GraphicsKind::Rectangle
            )
        })
    {
        if a.series.len() > MAX_PLOT_SERIES {
            return Err("findobj exceeds series storage budget".into());
        }
        for s in &a.series {
            engine.check_cancel()?;
            let stype = series_type_name(s.kind);
            if filter.visible.is_some_and(|v| v != s.visible)
                || kind.is_some_and(|k| k != stype)
                || filter.label.is_some_and(|l| l != s.label)
                || scope_handle.is_some_and(|h| {
                    h.kind() == GraphicsKind::Series && !h.object.same_object(&s.identity)
                })
            {
                continue;
            }
            found.push(Arc::new(GraphicsHandle::series(
                &f.identity,
                &a.identity,
                &s.identity,
            )));
        }
    }

    if filter.label.is_some() {
        return Ok(());
    }

    // Volumes (stacking order, front to back)
    if kind.is_none_or(|k| k == "volume")
        && scope_handle.is_none_or(|h| {
            !matches!(
                h.kind(),
                GraphicsKind::Series
                    | GraphicsKind::Text
                    | GraphicsKind::Image
                    | GraphicsKind::ConstantLine
                    | GraphicsKind::Rectangle
            )
        })
    {
        if a.extras.volumes.len() > crate::plot::MAX_VOLUMES {
            return Err("findobj exceeds 16 volumes per axes".into());
        }
        for volume in a.extras.volumes.iter().rev() {
            engine.check_cancel()?;
            if filter.visible.is_some_and(|v| v != volume.visible)
                || scope_handle.is_some_and(|h| {
                    h.kind() == GraphicsKind::Volume && !h.object.same_object(&volume.identity)
                })
            {
                continue;
            }
            found.push(Arc::new(GraphicsHandle::volume(&volume.identity)));
        }
    }

    // Text overlays
    if kind.is_none_or(|k| k == "text")
        && scope_handle.is_none_or(|h| {
            !matches!(
                h.kind(),
                GraphicsKind::Series
                    | GraphicsKind::Volume
                    | GraphicsKind::Image
                    | GraphicsKind::ConstantLine
                    | GraphicsKind::Rectangle
            )
        })
    {
        for t in a.extras.texts.iter().rev() {
            engine.check_cancel()?;
            if filter.visible.is_some_and(|v| v != t.visible)
                || scope_handle.is_some_and(|h| {
                    h.kind() == GraphicsKind::Text && !h.object.same_object(&t.identity)
                })
            {
                continue;
            }
            found.push(Arc::new(GraphicsHandle::axes_object(
                &f.identity,
                &a.identity,
                &t.identity,
                GraphicsKind::Text,
            )));
        }
    }

    // Images
    if kind.is_none_or(|k| k == "image")
        && scope_handle.is_none_or(|h| {
            !matches!(
                h.kind(),
                GraphicsKind::Series
                    | GraphicsKind::Volume
                    | GraphicsKind::Text
                    | GraphicsKind::ConstantLine
                    | GraphicsKind::Rectangle
            )
        })
    {
        for im in &a.extras.images {
            engine.check_cancel()?;
            if filter.visible.is_some_and(|v| v != im.visible)
                || scope_handle.is_some_and(|h| {
                    h.kind() == GraphicsKind::Image && !h.object.same_object(&im.identity)
                })
            {
                continue;
            }
            found.push(Arc::new(GraphicsHandle::axes_object(
                &f.identity,
                &a.identity,
                &im.identity,
                GraphicsKind::Image,
            )));
        }
    }

    // ConstantLines
    if kind.is_none_or(|k| k == "constantline")
        && scope_handle.is_none_or(|h| {
            !matches!(
                h.kind(),
                GraphicsKind::Series
                    | GraphicsKind::Volume
                    | GraphicsKind::Text
                    | GraphicsKind::Image
                    | GraphicsKind::Rectangle
            )
        })
    {
        for cl in &a.extras.constant_lines {
            engine.check_cancel()?;
            if filter.visible.is_some_and(|v| v != cl.visible)
                || scope_handle.is_some_and(|h| {
                    h.kind() == GraphicsKind::ConstantLine && !h.object.same_object(&cl.identity)
                })
            {
                continue;
            }
            found.push(Arc::new(GraphicsHandle::axes_object(
                &f.identity,
                &a.identity,
                &cl.identity,
                GraphicsKind::ConstantLine,
            )));
        }
    }

    // Rectangles
    if kind.is_none_or(|k| k == "rectangle")
        && scope_handle.is_none_or(|h| {
            !matches!(
                h.kind(),
                GraphicsKind::Series
                    | GraphicsKind::Volume
                    | GraphicsKind::Text
                    | GraphicsKind::Image
                    | GraphicsKind::ConstantLine
            )
        })
    {
        for rect in &a.extras.rectangles {
            engine.check_cancel()?;
            if filter.visible.is_some_and(|v| v != rect.visible)
                || scope_handle.is_some_and(|h| {
                    h.kind() == GraphicsKind::Rectangle && !h.object.same_object(&rect.identity)
                })
            {
                continue;
            }
            found.push(Arc::new(GraphicsHandle::axes_object(
                &f.identity,
                &a.identity,
                &rect.identity,
                GraphicsKind::Rectangle,
            )));
        }
    }

    Ok(())
}

fn walk_ui_subtree(
    engine: &Engine,
    mut roots: Vec<(usize, usize, bool)>,
    depth: f64,
    filter: &FindFilter<'_>,
    effective_hidden: bool,
    in_callback: bool,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    roots.reverse();
    while let Some((key, d, is_explicit_scope)) = roots.pop() {
        engine.check_cancel()?;
        if (d as f64) > depth {
            continue;
        }
        let Some(node) = engine.ui.get_key(key) else {
            continue;
        };
        if !is_explicit_scope
            && !effective_hidden
            && !node.handle_visibility.is_visible(in_callback)
        {
            continue;
        }
        let node_type = if node.classic.is_some() {
            "uicontrol"
        } else {
            node.kind.type_name()
        };
        let number_ok = filter.number.is_none();
        let label_ok = filter.label.is_none();
        let type_ok = filter.kind.as_deref().is_none_or(|k| k == node_type);
        let tag_ok = filter.tag.as_deref().is_none_or(|t| t == node.tag);
        let style_ok = filter.style.as_deref().is_none_or(|s| {
            node.classic
                .as_ref()
                .is_some_and(|m| m.style.eq_ignore_ascii_case(s))
        });
        let hv_ok = filter
            .handle_visibility
            .is_none_or(|hv| hv == node.handle_visibility);
        let backing_figure = match node.specific {
            UiSpecific::Axes { fig_id, .. } => engine.ui.uiaxes_figures.get(&fig_id),
            _ => None,
        };
        let visible = backing_figure.map_or(node.visible, |fig| !fig.axes.extras.axis_off);
        if number_ok
            && label_ok
            && type_ok
            && tag_ok
            && style_ok
            && hv_ok
            && filter.visible.is_none_or(|v| v == visible)
        {
            found.push(Arc::new(node.handle()));
        }
        if ((d + 1) as f64) <= depth {
            if let Some(backing_fig) = backing_figure {
                append_axes_graphics_children(
                    engine,
                    backing_fig,
                    &backing_fig.axes,
                    filter,
                    None,
                    found,
                )?;
            }
            for &ch in node.children.iter().rev() {
                roots.push((ch, d + 1, false));
            }
        }
    }
    Ok(())
}

/// Live descendants, including hidden dialogs and the axes title/label handles
/// `findobj` does not search. `findobj` depth and visibility stay on their own path.
///
/// Two walkers answer `findall`. Searches only the `findobj` walker knows
/// (finite `-depth`, Tag/Style/HandleVisibility, chart and UI component types, UI
/// component and uiaxes scopes) run there with hidden handles included. Every
/// other search walks the classic figures here; when `findobj` knows its
/// filters too, the UI descendants of the scope follow the classic handles.
/// The scope may be omitted (root), as for `findobj`.
pub(super) fn find_all(engine: &Engine, args: &[Value]) -> Result<Value> {
    engine.check_cancel()?;
    let (scope_value, properties) = match args.split_first() {
        None => return Err("findall requires one scope and property/value pairs".into()),
        Some((Value::Text(_), _)) => (None, args),
        Some((scope_value, properties)) => (Some(scope_value), properties),
    };
    let (depth, properties) = search_depth(properties, "findall")?;
    if depth.is_finite()
        || findobj_only_search(properties)
        || scope_value.is_some_and(|value| findobj_only_scope(engine, value))
    {
        return find_objects_inner(engine, scope_value, properties, depth, true, "findall");
    }
    if properties.is_empty() || properties.len() > 16 || !properties.len().is_multiple_of(2) {
        return Err("findall requires 1..8 property/value pairs".into());
    }
    let query = Query::parse(properties)?;
    let scope = match scope_value {
        Some(value) => scope(engine, value)?,
        None => Scope::Root,
    };
    let mut found = Vec::new();
    match scope {
        Scope::Line(handle) => push_line(engine, &query, &handle, &mut found)?,
        Scope::Text(handle) => push_text_leaf(engine, &query, &handle, &mut found)?,
        Scope::Legend(handle) => push_legend_leaf(engine, &query, &handle, &mut found)?,
        Scope::Dialog(handle) => {
            if query.matches_dialog(engine, &handle) {
                push_found(engine, &mut found, handle)?;
            }
        }
        Scope::Root | Scope::Figure { .. } | Scope::Axes(_) => {
            walk(engine, &query, &scope, &mut found)?;
            push_ui_descendants(engine, &query, &scope, &mut found)?;
        }
    }
    engine.check_cancel()?;
    Ok(Value::Container(
        GraphicsArray::from_shared(found.len(), 1, found, &engine.cancel)?.into(),
    ))
}

/// UI filters and chart types handled by the hierarchy shared with `findobj`.
fn findobj_only_search(properties: &[Value]) -> bool {
    properties.chunks(2).any(|pair| {
        let Some(Value::Text(name)) = pair.first() else {
            return false;
        };
        match name.to_ascii_lowercase().as_str() {
            "tag" | "style" | "handlevisibility" => true,
            "type" => matches!(pair.get(1), Some(Value::Text(kind)) if {
                let kind = kind.to_ascii_lowercase();
                is_supported_type(&kind)
                    && !matches!(kind.as_str(), "figure" | "axes" | "line" | "text")
            }),
            _ => false,
        }
    })
}

/// A scope only the `findobj` walker answers: a UI component, a uiaxes or
/// a handle inside one, or an axes child that is not a line, text or legend.
fn findobj_only_scope(engine: &Engine, value: &Value) -> bool {
    let Ok(h) = handle(value) else {
        return false;
    };
    match h.kind() {
        GraphicsKind::Root => false,
        GraphicsKind::UiComponent
        | GraphicsKind::Volume
        | GraphicsKind::Image
        | GraphicsKind::ConstantLine
        | GraphicsKind::Rectangle => true,
        GraphicsKind::Axes if engine.ui.is_uiaxes(&h.object) => true,
        _ if engine
            .locate_graphics(h)
            .is_ok_and(|located| is_uiaxes_fig_id(located.figure)) =>
        {
            true
        }
        GraphicsKind::Series => figure_of(engine, h)
            .and_then(|figure| axes_of(figure, h))
            .and_then(|(_, axes)| series_of(axes, h))
            .is_ok_and(|series| series.kind != PlotKind::Line),
        _ => false,
    }
}

/// The uicontrols and uiaxes of the classic figures in scope and, from root,
/// the UIFigures, when the query is one `findobj` knows too.
fn push_ui_descendants(
    engine: &Engine,
    query: &Query,
    scope: &Scope,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    let Some(filter) = query.findobj_filter() else {
        return Ok(());
    };
    let (scope_level, scope_figure) = match scope {
        Scope::Root => (0, None),
        Scope::Figure { id: Some(id), .. } => (1, Some(*id)),
        Scope::Figure {
            handle: Some(handle),
            ..
        } => (1, Some(engine.locate_graphics(handle)?.figure)),
        _ => return Ok(()),
    };
    let mut figures = engine.figures().collect::<Vec<_>>();
    figures.sort_by_key(|(id, _)| *id);
    append_ui_descendants(
        engine,
        scope_level,
        scope_figure,
        &figures,
        f64::INFINITY,
        &filter,
        true,
        !engine.ui.callback_stack.is_empty(),
        found,
    )
}

struct Query {
    type_name: Option<String>,
    display_name: Option<String>,
    visible: Option<bool>,
    string: Option<String>,
    number: Option<f64>,
    line_width: Option<f64>,
    font_size: Option<f64>,
}

enum Scope {
    Root,
    Figure {
        id: Option<u32>,
        handle: Option<GraphicsHandle>,
    },
    Axes(GraphicsHandle),
    Line(GraphicsHandle),
    Text(GraphicsHandle),
    Legend(GraphicsHandle),
    Dialog(GraphicsHandle),
}

enum LabelSlot {
    Title,
    X,
    Y,
    Z,
    Subtitle,
    Colorbar,
    ColorbarTitle,
    RightY,
}

impl Query {
    fn parse(properties: &[Value]) -> Result<Self> {
        let mut query = Self {
            type_name: None,
            display_name: None,
            visible: None,
            string: None,
            number: None,
            line_width: None,
            font_size: None,
        };
        for pair in properties.as_chunks::<2>().0 {
            let name = text(&pair[0])?;
            if name.len() > 64 {
                return Err("findall property name is too long".into());
            }
            let name = name.to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "-depth"
                    | "-regexp"
                    | "-property"
                    | "-function"
                    | "-not"
                    | "-and"
                    | "-or"
                    | "-xor"
                    | "-isa"
                    | "-class"
                    | "flat"
            ) {
                return Err("findall does not support search operators".into());
            }
            match name.as_str() {
                "type" if query.type_name.is_none() => {
                    let value = filter_text(&pair[1], "Type")?.to_ascii_lowercase();
                    if !matches!(
                        value.as_str(),
                        "figure" | "axes" | "line" | "text" | "legend"
                    ) {
                        return Err(
                            "findall Type supports figure, axes, line, text or legend, or a findobj chart or UI component type".into()
                        );
                    }
                    query.type_name = Some(value);
                }
                "displayname" if query.display_name.is_none() => {
                    query.display_name = Some(filter_text(&pair[1], "DisplayName")?);
                }
                "visible" if query.visible.is_none() => {
                    query.visible = Some(visible_filter(&pair[1])?);
                }
                "string" if query.string.is_none() => {
                    query.string = Some(filter_text(&pair[1], "String")?);
                }
                "number" if query.number.is_none() => {
                    query.number = Some(finite_scalar(&pair[1], "Number")?);
                }
                "linewidth" if query.line_width.is_none() => {
                    query.line_width = Some(finite_scalar(&pair[1], "LineWidth")?);
                }
                "fontsize" if query.font_size.is_none() => {
                    query.font_size = Some(finite_scalar(&pair[1], "FontSize")?);
                }
                "type" | "displayname" | "visible" | "string" | "number" | "linewidth"
                | "fontsize" => {
                    return Err("findall properties must be unique".into());
                }
                _ => {
                    return Err(
                        "findall supports Type, DisplayName, Visible, String, Number, LineWidth and FontSize, or the findobj filters Tag, Style and HandleVisibility"
                            .into(),
                    );
                }
            }
        }
        Ok(query)
    }

    fn wants(&self, kind: &str) -> bool {
        self.type_name.as_ref().is_none_or(|name| name == kind)
    }

    /// The same query for the `findobj` walker, when it knows every filter:
    /// a Type it searches, with Number, DisplayName or Visible.
    fn findobj_filter(&self) -> Option<FindFilter<'_>> {
        if self.type_name.is_none() && self.visible.is_none()
            || self
                .type_name
                .as_deref()
                .is_some_and(|kind| !matches!(kind, "figure" | "axes" | "line" | "text"))
            || self.string.is_some()
            || self.line_width.is_some()
            || self.font_size.is_some()
        {
            return None;
        }
        Some(FindFilter {
            kind: self.type_name.clone(),
            number: self.number,
            label: self.display_name.as_deref(),
            tag: None,
            style: None,
            handle_visibility: None,
            visible: self.visible,
        })
    }

    fn matches_common(
        &self,
        kind: &str,
        shown: Option<bool>,
        label: Option<&str>,
        width: Option<Option<f64>>,
        size: Option<f64>,
        name: Option<&str>,
    ) -> bool {
        if !self.wants(kind) {
            return false;
        }
        if self.number.is_some() && kind != "figure" {
            return false;
        }
        if self.display_name.is_some() && kind != "line" {
            return false;
        }
        if self.string.is_some() && kind != "text" {
            return false;
        }
        if let Some(expected) = self.visible
            && shown != Some(expected)
        {
            return false;
        }
        if let Some(expected) = &self.display_name
            && name != Some(expected.as_str())
        {
            return false;
        }
        if let Some(expected) = &self.string
            && label != Some(expected.as_str())
        {
            return false;
        }
        if let Some(expected) = self.line_width {
            match width {
                Some(Some(actual)) if actual == expected => {}
                _ => return false,
            }
        }
        if let Some(expected) = self.font_size
            && size != Some(expected)
        {
            return false;
        }
        true
    }

    fn matches_dialog(&self, engine: &Engine, handle: &GraphicsHandle) -> bool {
        self.number.is_none()
            && self.matches_common(
                "figure",
                Some(dialog_shown(engine, handle)),
                None,
                None,
                None,
                None,
            )
    }
}

fn dialog_shown(engine: &Engine, handle: &GraphicsHandle) -> bool {
    let Some(id) = engine.dialogs.headless_id(handle) else {
        return false;
    };
    match engine
        .dialogs
        .headless_property(id, &Value::Text("Visible".into()), &engine.cancel)
    {
        Some(Ok(Value::Text(flag))) => flag.eq_ignore_ascii_case("on"),
        _ => false,
    }
}

fn scope(engine: &Engine, value: &Value) -> Result<Scope> {
    match value {
        Value::Matrix(m) if m.len() == 1 && m.data()[0] == 0. => Ok(Scope::Root),
        Value::Matrix(m)
            if m.len() == 1
                && (1. ..=1_000_000.).contains(&m.data()[0])
                && m.data()[0].fract() == 0. =>
        {
            let id = m.data()[0] as u32;
            engine.graphics_figure(id)?;
            Ok(Scope::Figure {
                id: Some(id),
                handle: None,
            })
        }
        Value::Matrix(_) => {
            Err("findall scope must be root, a figure number, or one live handle".into())
        }
        other => {
            let h = handle(other)?.clone();
            if h.kind() == GraphicsKind::Root {
                return Ok(Scope::Root);
            }
            if h.kind() == GraphicsKind::Figure && engine.dialogs.headless_id(&h).is_some() {
                engine.locate_graphics(&h)?;
                return Ok(Scope::Dialog(h));
            }
            engine.locate_graphics(&h)?;
            match h.kind() {
                GraphicsKind::Figure => Ok(Scope::Figure {
                    id: None,
                    handle: Some(h),
                }),
                GraphicsKind::Axes => Ok(Scope::Axes(h)),
                GraphicsKind::Series => {
                    let figure = figure_of(engine, &h)?;
                    let (_, axes) = axes_of(figure, &h)?;
                    let series = series_of(axes, &h)?;
                    if series.kind != PlotKind::Line {
                        return Err(
                            "findall scope must be root or one live figure, axes, line, text or legend"
                                .into(),
                        );
                    }
                    Ok(Scope::Line(h))
                }
                GraphicsKind::Text => Ok(Scope::Text(h)),
                GraphicsKind::Legend => Ok(Scope::Legend(h)),
                _ => Err(
                    "findall scope must be root or one live figure, axes, line, text or legend"
                        .into(),
                ),
            }
        }
    }
}

fn walk(
    engine: &Engine,
    query: &Query,
    scope: &Scope,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    let mut figures = engine.figures().collect::<Vec<_>>();
    if figures.len() > 16 {
        return Err("findall exceeds 16 figures".into());
    }
    figures.sort_by_key(|(id, _)| *id);
    for (id, figure) in figures {
        engine.check_cancel()?;
        if !figure_in_scope(scope, id, figure) {
            continue;
        }
        if query.wants("figure") && !matches!(scope, Scope::Axes(_)) {
            let shown = figure.visible && !engine.is_figure_hidden(&figure.identity);
            let number = f64::from(id);
            if query.number.is_none_or(|n| n == number)
                && query.matches_common("figure", Some(shown), None, None, None, None)
            {
                push_found(engine, found, GraphicsHandle::figure(&figure.identity))?;
            }
        }
        if query.type_name.as_deref() == Some("figure") {
            continue;
        }
        if !matches!(scope, Scope::Axes(_)) {
            push_layout_texts(engine, query, figure, found)?;
        }
        let mut axes = figure.all_axes().collect::<Vec<_>>();
        if axes.len() > 16 {
            return Err("findall exceeds 16 axes per figure".into());
        }
        axes.sort_by_key(|(position, _)| *position);
        for (_, axes) in axes {
            engine.check_cancel()?;
            if !axes_in_scope(scope, axes) {
                continue;
            }
            push_axes(
                engine,
                query,
                figure,
                axes,
                !matches!(scope, Scope::Axes(_)),
                found,
            )?;
        }
    }
    if matches!(scope, Scope::Root) && query.wants("figure") {
        for handle in engine.dialogs.hidden_handles() {
            engine.check_cancel()?;
            if found.iter().any(|have| {
                have.kind() == GraphicsKind::Figure && have.figure.same_object(&handle.figure)
            }) {
                continue;
            }
            if query.matches_dialog(engine, &handle) {
                push_found(engine, found, handle)?;
            }
        }
    }
    Ok(())
}

fn push_axes(
    engine: &Engine,
    query: &Query,
    figure: &Figure,
    axes: &Axes,
    include_colorbar_children: bool,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    if axes.series.len() > MAX_PLOT_SERIES {
        return Err("findall exceeds series storage budget".into());
    }
    if axes.extras.texts.len() > MAX_TEXTS {
        return Err("findall exceeds text object budget".into());
    }
    let shown = figure.visible && !engine.is_figure_hidden(&figure.identity);
    if query.matches_common(
        "axes",
        Some(!axes.extras.axis_off),
        None,
        Some(Some(axes.extras.line_width.unwrap_or(0.5))),
        Some(axes.extras.font_size.unwrap_or(10.)),
        None,
    ) {
        push_found(
            engine,
            found,
            GraphicsHandle::axes(&figure.identity, &axes.identity),
        )?;
    }
    for slot in [
        LabelSlot::Title,
        LabelSlot::X,
        LabelSlot::Y,
        LabelSlot::Z,
        LabelSlot::Subtitle,
    ] {
        push_label(engine, query, figure, axes, slot, shown, found)?;
    }
    if include_colorbar_children && axes.extras.colorbar.is_some() {
        push_label(
            engine,
            query,
            figure,
            axes,
            LabelSlot::Colorbar,
            shown,
            found,
        )?;
        push_label(
            engine,
            query,
            figure,
            axes,
            LabelSlot::ColorbarTitle,
            shown,
            found,
        )?;
    }
    if axes.extras.yy.is_some() {
        push_label(engine, query, figure, axes, LabelSlot::RightY, shown, found)?;
    }
    for text_object in &axes.extras.texts {
        engine.check_cancel()?;
        if query.matches_common(
            "text",
            Some(text_object.visible),
            Some(text_object.string.as_str()),
            None,
            Some(text_object.font_size),
            None,
        ) {
            push_found(
                engine,
                found,
                GraphicsHandle::axes_object(
                    &figure.identity,
                    &axes.identity,
                    &text_object.identity,
                    GraphicsKind::Text,
                ),
            )?;
        }
    }
    if let Some(state) = &axes.legend_state {
        let size = axes
            .extras
            .legend_style
            .font_size
            .unwrap_or_else(|| axes.extras.font_size.unwrap_or(10.) * 0.9);
        if query.matches_common("legend", Some(axes.legend), None, None, Some(size), None) {
            push_found(
                engine,
                found,
                GraphicsHandle::legend(&figure.identity, &axes.identity, &state.identity),
            )?;
        }
    }
    for series in &axes.series {
        engine.check_cancel()?;
        if series.kind != PlotKind::Line {
            continue;
        }
        if query.matches_common(
            "line",
            Some(series.visible),
            None,
            Some(series.line_width),
            None,
            Some(series.label.as_str()),
        ) {
            push_found(
                engine,
                found,
                GraphicsHandle::series(&figure.identity, &axes.identity, &series.identity),
            )?;
        }
    }
    Ok(())
}

fn push_label(
    engine: &Engine,
    query: &Query,
    figure: &Figure,
    axes: &Axes,
    slot: LabelSlot,
    shown: bool,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    if query.matches_common(
        "text",
        Some(shown),
        Some(label_string(axes, &slot)),
        None,
        Some(label_font(axes, &slot)),
        None,
    ) {
        push_found(
            engine,
            found,
            GraphicsHandle::axes_object(
                &figure.identity,
                &axes.identity,
                label_identity(axes, &slot),
                GraphicsKind::Text,
            ),
        )?;
    }
    Ok(())
}

fn push_layout_texts(
    engine: &Engine,
    query: &Query,
    figure: &Figure,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    let Some(layout) = &figure.layout else {
        return Ok(());
    };
    let entries = [
        (
            layout.title.as_str(),
            &layout.title_identity,
            layout.title_style.font_size.unwrap_or(12.),
        ),
        (
            layout.subtitle.string.as_str(),
            &layout.subtitle_identity,
            layout.subtitle.font_size_points(),
        ),
        (
            layout.xlabel.as_str(),
            &layout.xlabel_identity,
            layout.label_styles[0].font_size.unwrap_or(12.),
        ),
        (
            layout.ylabel.as_str(),
            &layout.ylabel_identity,
            layout.label_styles[1].font_size.unwrap_or(12.),
        ),
    ];
    for (label, identity, size) in entries {
        // Empty layout headings are storage for sgtitle, not extra title handles.
        if label.is_empty() {
            continue;
        }
        if query.matches_common("text", Some(true), Some(label), None, Some(size), None) {
            push_found(
                engine,
                found,
                GraphicsHandle::layout_text(&figure.identity, identity),
            )?;
        }
    }
    Ok(())
}

fn push_line(
    engine: &Engine,
    query: &Query,
    handle: &GraphicsHandle,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    let figure = figure_of(engine, handle)?;
    let (_, axes) = axes_of(figure, handle)?;
    let series = series_of(axes, handle)?;
    if series.kind == PlotKind::Line
        && query.matches_common(
            "line",
            Some(series.visible),
            None,
            Some(series.line_width),
            None,
            Some(series.label.as_str()),
        )
    {
        push_found(engine, found, handle.clone())?;
    }
    Ok(())
}

fn push_legend_leaf(
    engine: &Engine,
    query: &Query,
    handle: &GraphicsHandle,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    let figure = figure_of(engine, handle)?;
    let (_, axes) = axes_of(figure, handle)?;
    let Some(state) = &axes.legend_state else {
        return Ok(());
    };
    if !state.identity.same_object(&handle.object) {
        return Ok(());
    }
    let size = axes
        .extras
        .legend_style
        .font_size
        .unwrap_or_else(|| axes.extras.font_size.unwrap_or(10.) * 0.9);
    if query.matches_common("legend", Some(axes.legend), None, None, Some(size), None) {
        push_found(engine, found, handle.clone())?;
    }
    Ok(())
}

fn push_text_leaf(
    engine: &Engine,
    query: &Query,
    handle: &GraphicsHandle,
    found: &mut Vec<Arc<GraphicsHandle>>,
) -> Result<()> {
    let figure = figure_of(engine, handle)?;
    if handle.axes.is_none() {
        let layout = figure
            .layout
            .as_ref()
            .ok_or("Text object is no longer live")?;
        let entries = [
            (
                layout.title.as_str(),
                &layout.title_identity,
                layout.title_style.font_size.unwrap_or(12.),
            ),
            (
                layout.subtitle.string.as_str(),
                &layout.subtitle_identity,
                layout.subtitle.font_size_points(),
            ),
            (
                layout.xlabel.as_str(),
                &layout.xlabel_identity,
                layout.label_styles[0].font_size.unwrap_or(12.),
            ),
            (
                layout.ylabel.as_str(),
                &layout.ylabel_identity,
                layout.label_styles[1].font_size.unwrap_or(12.),
            ),
        ];
        for (label, identity, size) in entries {
            if !identity.same_object(&handle.object) {
                continue;
            }
            if query.matches_common("text", Some(true), Some(label), None, Some(size), None) {
                push_found(engine, found, handle.clone())?;
            }
            return Ok(());
        }
        return Err("Text object is no longer live".into());
    }
    let (_, axes) = axes_of(figure, handle)?;
    let shown = figure.visible && !engine.is_figure_hidden(&figure.identity);
    for slot in [
        LabelSlot::Title,
        LabelSlot::X,
        LabelSlot::Y,
        LabelSlot::Z,
        LabelSlot::Subtitle,
        LabelSlot::Colorbar,
        LabelSlot::ColorbarTitle,
        LabelSlot::RightY,
    ] {
        if !label_live(axes, &slot) || !label_identity(axes, &slot).same_object(&handle.object) {
            continue;
        }
        if query.matches_common(
            "text",
            Some(shown),
            Some(label_string(axes, &slot)),
            None,
            Some(label_font(axes, &slot)),
            None,
        ) {
            push_found(engine, found, handle.clone())?;
        }
        return Ok(());
    }
    if let Some(text_object) = axes
        .extras
        .texts
        .iter()
        .find(|text_object| text_object.identity.same_object(&handle.object))
    {
        if query.matches_common(
            "text",
            Some(text_object.visible),
            Some(text_object.string.as_str()),
            None,
            Some(text_object.font_size),
            None,
        ) {
            push_found(engine, found, handle.clone())?;
        }
        return Ok(());
    }
    Err("Text object is no longer live".into())
}

fn figure_in_scope(scope: &Scope, id: u32, figure: &Figure) -> bool {
    match scope {
        Scope::Root => true,
        Scope::Figure {
            id: Some(number),
            handle: None,
        } => *number == id,
        Scope::Figure {
            handle: Some(h), ..
        }
        | Scope::Axes(h) => h.figure.same_object(&figure.identity),
        _ => false,
    }
}

fn axes_in_scope(scope: &Scope, axes: &Axes) -> bool {
    match scope {
        Scope::Axes(handle) => handle
            .axes
            .as_ref()
            .is_some_and(|id| id.same_object(&axes.identity)),
        _ => true,
    }
}

fn figure_of<'a>(engine: &'a Engine, handle: &GraphicsHandle) -> Result<&'a Figure> {
    let located = engine.locate_graphics(handle)?;
    engine.graphics_figure(located.figure)
}

fn axes_of<'a>(
    figure: &'a Figure,
    handle: &GraphicsHandle,
) -> Result<(crate::plot::Position, &'a Axes)> {
    figure
        .all_axes()
        .find(|(_, axes)| {
            handle
                .axes
                .as_ref()
                .is_some_and(|id| id.same_object(&axes.identity))
        })
        .ok_or("Graphics handle is stale or does not belong to this engine graph".into())
}

fn series_of<'a>(axes: &'a Axes, handle: &GraphicsHandle) -> Result<&'a Series> {
    axes.series
        .iter()
        .find(|series| series.identity.same_object(&handle.object))
        .ok_or("Graphics handle is stale or does not belong to this engine graph".into())
}

fn label_live(axes: &Axes, slot: &LabelSlot) -> bool {
    match slot {
        LabelSlot::Colorbar | LabelSlot::ColorbarTitle => axes.extras.colorbar.is_some(),
        LabelSlot::RightY => axes.extras.yy.is_some(),
        _ => true,
    }
}

fn label_identity<'a>(axes: &'a Axes, slot: &LabelSlot) -> &'a crate::ObjectIdentity {
    match slot {
        LabelSlot::Title => &axes.extras.title_identity,
        LabelSlot::X => &axes.extras.xlabel_identity,
        LabelSlot::Y => &axes.extras.ylabel_identity,
        LabelSlot::Z => &axes.extras.zlabel_identity,
        LabelSlot::Subtitle => &axes.extras.subtitle_identity,
        LabelSlot::Colorbar => &axes.extras.colorbar.as_ref().unwrap().label_identity,
        LabelSlot::ColorbarTitle => &axes.extras.colorbar.as_ref().unwrap().title_identity,
        LabelSlot::RightY => &axes.extras.yy.as_ref().unwrap().right_label_identity,
    }
}

fn label_string<'a>(axes: &'a Axes, slot: &LabelSlot) -> &'a str {
    match slot {
        LabelSlot::Title => axes.title.as_str(),
        LabelSlot::X => axes.xlabel.as_str(),
        LabelSlot::Y => axes.ylabel.as_str(),
        LabelSlot::Z => axes.zlabel.as_str(),
        LabelSlot::Subtitle => axes.extras.subtitle.string.as_str(),
        LabelSlot::Colorbar => axes
            .extras
            .colorbar
            .as_ref()
            .map(|bar| bar.label.as_str())
            .unwrap_or(""),
        LabelSlot::ColorbarTitle => axes
            .extras
            .colorbar
            .as_ref()
            .map(|bar| bar.title.as_str())
            .unwrap_or(""),
        LabelSlot::RightY => axes
            .extras
            .yy
            .as_ref()
            .map(|yy| yy.right_label.as_str())
            .unwrap_or(""),
    }
}

fn label_font(axes: &Axes, slot: &LabelSlot) -> f64 {
    let base = axes.extras.font_size.unwrap_or(10.);
    match slot {
        LabelSlot::Subtitle => axes.extras.subtitle.font_size_points(),
        LabelSlot::Colorbar | LabelSlot::ColorbarTitle => base,
        LabelSlot::RightY => axes
            .extras
            .yy
            .as_ref()
            .and_then(|yy| yy.right_label_style.font_size)
            .unwrap_or(base * 1.1),
        LabelSlot::Title => axes.extras.title_style.font_size.unwrap_or(base * 1.1),
        LabelSlot::X => axes.extras.label_styles[0].font_size.unwrap_or(base * 1.1),
        LabelSlot::Y => axes.extras.label_styles[1].font_size.unwrap_or(base * 1.1),
        LabelSlot::Z => axes.extras.label_styles[2].font_size.unwrap_or(base * 1.1),
    }
}

fn visible_filter(value: &Value) -> Result<bool> {
    match filter_text(value, "Visible")?.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("findall Visible must be on or off".into()),
    }
}

fn filter_text(value: &Value, what: &str) -> Result<String> {
    let text = text(value).map_err(|_| format!("findall {what} must be character text"))?;
    if text.len() > 4096 {
        return Err(format!("findall {what} exceeds 4096 bytes"));
    }
    Ok(text.to_owned())
}

fn finite_scalar(value: &Value, what: &str) -> Result<f64> {
    let Value::Matrix(m) = value else {
        return Err(format!("findall {what} must be a finite scalar"));
    };
    if m.len() != 1 || !m.data()[0].is_finite() {
        return Err(format!("findall {what} must be a finite scalar"));
    }
    Ok(m.data()[0])
}

fn push_found(
    engine: &Engine,
    found: &mut Vec<Arc<GraphicsHandle>>,
    handle: GraphicsHandle,
) -> Result<()> {
    if found.len() >= crate::script_containers::MAX_CELL_SLOTS {
        return Err("findall exceeds 65536 handles".into());
    }
    engine.check_cancel()?;
    found.push(Arc::new(handle));
    Ok(())
}

pub(super) fn all_children(engine: &mut Engine, args: &[Value]) -> Result<Value> {
    let [target] = args else {
        return Err("allchild requires one scalar graphics handle".into());
    };
    let h = handle_or_root(target)?;
    if h.kind() == GraphicsKind::Root {
        return engine.root_children_handles(true);
    }
    if h.kind() == GraphicsKind::UiComponent {
        let key = h.object.recovery_key();
        let Some(node) = engine.ui.get_key(key) else {
            return Err("Invalid or deleted object.".into());
        };
        let mut handles = Vec::with_capacity(node.children.len());
        for &ck in &node.children {
            if let Some(child) = engine.ui.get_key(ck) {
                handles.push(Arc::new(child.handle()));
            }
        }
        let n = handles.len();
        let cols = if n == 0 { 0 } else { 1 };
        return Ok(Value::Container(
            GraphicsArray::from_shared(n, cols, handles, &engine.cancel)?.into(),
        ));
    }
    let located = engine.locate_graphics(&h)?;
    if matches!(h.kind(), GraphicsKind::Axes | GraphicsKind::Figure) {
        return engine.get_graphics_property(&h, located, &[Value::Text("Children".into())]);
    }
    Ok(Value::Container(
        GraphicsArray::from_shared(0, 1, Vec::new(), &engine.cancel)?.into(),
    ))
}
