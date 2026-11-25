//! `sliceViewer`, `orthosliceViewer`, `getAxesHandles`, and handle get/set.
use super::super::{Engine, Execution, Value};
use crate::plot::{
    Arrangement, Axes, Figure, ImageDataClass, ImageObject, Layout, MAX_COLORMAP_ROWS,
    OrthosliceViewerObject, Panel, Position, Series, SliceDirection, SliceViewerObject, TileGrid,
    colormap,
};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix, ObjectIdentity, Result};
use std::sync::Arc;

pub(in crate::engine) fn handles(name: &str) -> bool {
    matches!(name, "sliceViewer" | "orthosliceViewer" | "getAxesHandles")
}

pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    _out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "sliceViewer" => {
            if nargout > 1 {
                return Err("Too many output arguments.".into());
            }
            let h = sliceviewer_call(e, args)?;
            Ok(if nargout == 0 { vec![] } else { vec![h] })
        }
        "orthosliceViewer" => {
            if nargout > 1 {
                return Err("Too many output arguments.".into());
            }
            let h = orthosliceviewer_call(e, args)?;
            Ok(if nargout == 0 { vec![] } else { vec![h] })
        }
        "getAxesHandles" => get_axes_handles_call(e, args, nargout),
        _ => Err(format!("Unknown slice viewer command '{name}'")),
    }
}

fn extract_viewer_data(
    e: &Engine,
    v: &Value,
    who: &str,
) -> Result<([usize; 3], Arc<[f64]>, ImageDataClass)> {
    let d = v.dims();
    if d.contains(&0) {
        return Err(format!("{who}: V must be a nonempty 2-D or 3-D array"));
    }
    if d.len() == 4 && d[3] == 3 {
        return Err(format!("{who}: RGB volumes are not supported yet"));
    }
    if d.len() < 2 || d.len() > 3 {
        return Err(format!("{who}: Input volume must be 2-D, 3-D, or 4-D RGB"));
    }
    let data_class = match v {
        Value::Matrix(_) => ImageDataClass::Double,
        Value::Logical(_) => ImageDataClass::Logical,
        Value::Container(Array::Typed(t)) => ImageDataClass::Typed(t.class()),
        Value::Complex(_) => return Err(format!("{who}: complex input is not supported")),
        _ => {
            return Err(format!(
                "{who}: V must be a numeric or logical 2-D or 3-D array"
            ));
        }
    };
    let m = v.clone().matrix(&e.cancel)?.with_dims(&d)?;
    let dims = if d.len() == 2 {
        [d[0], d[1], 1]
    } else {
        [d[0], d[1], d[2]]
    };
    Ok((dims, Arc::from(m.data()), data_class))
}

fn prop_name(val: &Value, who: &str) -> Result<String> {
    match val {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.to_ascii_lowercase()),
        _ => Err(format!("{who}: property name must be text")),
    }
}

fn parse_colormap(value: &Value) -> Result<Arc<Vec<[f64; 3]>>> {
    match value {
        Value::Text(name) | Value::QuotedString(name) => {
            let rows = colormap::named(name, colormap::DEFAULT_ROWS)?.ok_or_else(|| {
                format!(
                    "Unknown colormap '{name}'; supported: {}",
                    colormap::NAMES.join(", ")
                )
            })?;
            Ok(Arc::new(rows))
        }
        Value::Matrix(m) => {
            if m.cols() != 3 || m.rows() == 0 || m.rows() > MAX_COLORMAP_ROWS {
                return Err(format!(
                    "Colormap must be an n-by-3 matrix with 1..{MAX_COLORMAP_ROWS} rows"
                ));
            }
            let rows: Vec<[f64; 3]> = (0..m.rows())
                .map(|r| [m.get(r, 0), m.get(r, 1), m.get(r, 2)])
                .collect();
            if !rows
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            {
                return Err("Colormap values must be finite and in [0, 1]".into());
            }
            Ok(Arc::new(rows))
        }
        _ => Err("Colormap must be a colormap name or an n-by-3 matrix in [0, 1]".into()),
    }
}

fn colormap_to_value(rows: &[[f64; 3]]) -> Result<Value> {
    let mut data = Vec::with_capacity(rows.len() * 3);
    for c in 0..3 {
        data.extend(rows.iter().map(|r| r[c]));
    }
    Ok(Value::Matrix(Matrix::from_column_vec(rows.len(), 3, data)?))
}

fn parse_display_range(value: &Value) -> Result<Option<[f64; 2]>> {
    let Value::Matrix(m) = value else {
        return Err("DisplayRange must be [] or a 2-element real vector [low high]".into());
    };
    if m.is_empty() {
        return Ok(None);
    }
    if m.len() != 2 {
        return Err("DisplayRange must be [] or a 2-element real vector [low high]".into());
    }
    let a = m.data()[0];
    let b = m.data()[1];
    if !a.is_finite() || !b.is_finite() || a >= b {
        return Err("DisplayRange must be finite [low high] with low < high".into());
    }
    Ok(Some([a, b]))
}

fn parse_scale_factors(value: &Value) -> Result<[f64; 3]> {
    let Value::Matrix(m) = value else {
        return Err("ScaleFactors must be a 3-element positive finite vector".into());
    };
    if m.len() != 3 {
        return Err("ScaleFactors must be a 3-element positive finite vector".into());
    }
    let d = m.data();
    if !d.iter().all(|v| v.is_finite() && *v > 0.0) {
        return Err("ScaleFactors must be a 3-element positive finite vector".into());
    }
    Ok([d[0], d[1], d[2]])
}

fn parse_on_off(value: &Value, prop: &str) -> Result<bool> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => match s.to_ascii_lowercase().as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err(format!("{prop} must be 'on' or 'off'")),
        },
        Value::Logical(m) => Ok(m.as_scalar()?),
        _ => Err(format!("{prop} must be 'on' or 'off'")),
    }
}

fn parse_tag(value: &Value) -> Result<String> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.clone()),
        Value::Matrix(m) if m.is_empty() => Ok(String::new()),
        _ => Err("Tag must be text".into()),
    }
}

fn parse_slice_direction(value: &Value) -> Result<SliceDirection> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => match s.to_ascii_lowercase().as_str() {
            "x" => Ok(SliceDirection::X),
            "y" => Ok(SliceDirection::Y),
            "z" => Ok(SliceDirection::Z),
            _ => Err("SliceDirection must be [1 0 0], [0 1 0], [0 0 1], 'X', 'Y', or 'Z'".into()),
        },
        Value::Matrix(m) if m.len() == 3 => {
            let d = m.data();
            if d == [1.0, 0.0, 0.0] {
                Ok(SliceDirection::X)
            } else if d == [0.0, 1.0, 0.0] {
                Ok(SliceDirection::Y)
            } else if d == [0.0, 0.0, 1.0] {
                Ok(SliceDirection::Z)
            } else {
                Err("SliceDirection must be [1 0 0], [0 1 0], [0 0 1], 'X', 'Y', or 'Z'".into())
            }
        }
        _ => Err("SliceDirection must be [1 0 0], [0 1 0], [0 0 1], 'X', 'Y', or 'Z'".into()),
    }
}

fn parse_slice_index(value: &Value, max_n: usize, prop: &str) -> Result<usize> {
    let Value::Matrix(m) = value else {
        return Err(format!("{prop} must be an integer between 1 and {max_n}"));
    };
    let v = m
        .as_scalar()
        .map_err(|_| format!("{prop} must be an integer scalar between 1 and {max_n}"))?;
    if !v.is_finite() || v.fract() != 0.0 || v < 1.0 || v > max_n as f64 {
        return Err(format!("{prop} must be an integer between 1 and {max_n}"));
    }
    Ok(v as usize)
}

fn parse_slice_numbers(value: &Value, dims: [usize; 3]) -> Result<[usize; 3]> {
    let [ny, nx, nz] = dims;
    let Value::Matrix(m) = value else {
        return Err(format!(
            "SliceNumbers must be a 3-element integer vector [x y z] within [1..{nx}, 1..{ny}, 1..{nz}]"
        ));
    };
    if m.len() != 3 {
        return Err(format!(
            "SliceNumbers must be a 3-element integer vector [x y z] within [1..{nx}, 1..{ny}, 1..{nz}]"
        ));
    }
    let d = m.data();
    let limits = [nx, ny, nz];
    let mut out = [1usize; 3];
    for (i, (&v, &lim)) in d.iter().zip(&limits).enumerate() {
        if !v.is_finite() || v.fract() != 0.0 || v < 1.0 || v > lim as f64 {
            return Err(format!(
                "SliceNumbers({}): value must be an integer between 1 and {lim}",
                i + 1
            ));
        }
        out[i] = v as usize;
    }
    Ok(out)
}

fn parent_figure(e: &Engine, parent: &Value, who: &str) -> Result<u32> {
    let Value::Container(Array::Graphics(g)) = parent else {
        return Err(format!("{who}: Parent must be a live Figure handle"));
    };
    let h = g.scalar_handle()?;
    if h.kind() != GraphicsKind::Figure {
        return Err(format!("{who}: Parent must be a Figure handle"));
    }
    let number = e.locate_graphics(h)?.figure;
    e.graphics_figure(number)?;
    Ok(number)
}

fn resolve_target_figure(
    e: &Engine,
    parent_val: Option<&Value>,
    who: &str,
) -> Result<(u32, Figure)> {
    if let Some(parent) = parent_val {
        let number = parent_figure(e, parent, who)?;
        Ok((number, e.graphics_figure(number)?.clone()))
    } else {
        Ok((e.current_figure, e.surface_target()))
    }
}

fn apply_slice_viewer_prop(sv: &mut SliceViewerObject, name: &str, val: &Value) -> Result<()> {
    match name {
        "slicenumber" => {
            sv.slice_number = parse_slice_index(val, sv.max_slice(), "SliceNumber")?;
        }
        "slicedirection" => {
            let dir = parse_slice_direction(val)?;
            if dir != sv.slice_direction {
                sv.slice_direction = dir;
                sv.slice_number = dir.default_slice(sv.dims);
            }
        }
        "colormap" => {
            sv.colormap = parse_colormap(val)?;
        }
        "displayrange" => {
            sv.display_range = parse_display_range(val)?;
        }
        "scalefactors" => {
            sv.scale_factors = parse_scale_factors(val)?;
        }
        "visible" => {
            sv.visible = parse_on_off(val, "Visible")?;
        }
        "tag" => {
            sv.tag = parse_tag(val)?;
        }
        _ => {
            return Err(format!(
                "Unsupported SliceViewer property '{name}'; supported: SliceNumber, SliceDirection, Colormap, DisplayRange, ScaleFactors, Parent, Visible, Tag"
            ));
        }
    }
    sv.validate()
}

fn apply_orthoslice_viewer_prop(
    ov: &mut OrthosliceViewerObject,
    name: &str,
    val: &Value,
) -> Result<()> {
    match name {
        "slicenumbers" => {
            ov.slice_numbers = parse_slice_numbers(val, ov.dims)?;
        }
        "crosshairenabled" => {
            ov.crosshair_enabled = parse_on_off(val, "CrosshairEnabled")?;
        }
        "colormap" => {
            ov.colormap = parse_colormap(val)?;
        }
        "displayrange" => {
            ov.display_range = parse_display_range(val)?;
        }
        "scalefactors" => {
            ov.scale_factors = parse_scale_factors(val)?;
        }
        "visible" => {
            ov.visible = parse_on_off(val, "Visible")?;
        }
        "tag" => {
            ov.tag = parse_tag(val)?;
        }
        _ => {
            return Err(format!(
                "Unsupported OrthosliceViewer property '{name}'; supported: SliceNumbers, CrosshairEnabled, Colormap, DisplayRange, ScaleFactors, Parent, Visible, Tag"
            ));
        }
    }
    ov.validate()
}

pub(in crate::engine) fn sliceviewer_call(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(
            "sliceViewer: expected sliceViewer(V) or sliceViewer(V, Name, Value, ...)".into(),
        );
    }
    let (dims, data, data_class) = extract_viewer_data(e, &args[0], "sliceViewer")?;
    let rest = &args[1..];
    if !rest.len().is_multiple_of(2) {
        return Err("sliceViewer: name/value options must come in pairs".into());
    }
    let mut parent_val: Option<&Value> = None;
    let mut direction_val: Option<&Value> = None;
    let mut other_props: Vec<(String, &Value)> = Vec::new();
    for pair in rest.as_chunks::<2>().0 {
        let name = prop_name(&pair[0], "sliceViewer")?;
        if name == "parent" {
            parent_val = Some(&pair[1]);
        } else if name == "slicedirection" {
            direction_val = Some(&pair[1]);
        } else {
            other_props.push((name, &pair[1]));
        }
    }

    let (fig_id, mut figure) = resolve_target_figure(e, parent_val, "sliceViewer")?;
    figure.layout = None;
    figure.axes = figure.fresh_axes();
    figure.slice_viewers.clear();
    figure.orthoslice_viewers.clear();

    let mut sv = SliceViewerObject::new(dims, data, data_class, figure.axes.identity.clone());
    if let Some(dir_val) = direction_val {
        apply_slice_viewer_prop(&mut sv, "slicedirection", dir_val)?;
    }
    for (name, val) in other_props {
        apply_slice_viewer_prop(&mut sv, &name, val)?;
    }

    sv.sync_axes(&mut figure.axes);
    let token = sv.graphics_handle(&figure.identity);
    figure.slice_viewers.push(sv);
    figure.validate()?;
    e.publish_graphics_figure(fig_id, figure);
    let array = GraphicsArray::scalar(token, &e.cancel)?;
    Ok(Value::Container(Array::Graphics(array)))
}

pub(in crate::engine) fn orthosliceviewer_call(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err(
            "orthosliceViewer: expected orthosliceViewer(V) or orthosliceViewer(V, Name, Value, ...)"
                .into(),
        );
    }
    let (dims, data, data_class) = extract_viewer_data(e, &args[0], "orthosliceViewer")?;
    let rest = &args[1..];
    if !rest.len().is_multiple_of(2) {
        return Err("orthosliceViewer: name/value options must come in pairs".into());
    }
    let mut parent_val: Option<&Value> = None;
    let mut props: Vec<(String, &Value)> = Vec::new();
    for pair in rest.as_chunks::<2>().0 {
        let name = prop_name(&pair[0], "orthosliceViewer")?;
        if name == "parent" {
            parent_val = Some(&pair[1]);
        } else {
            props.push((name, &pair[1]));
        }
    }

    let (fig_id, mut figure) = resolve_target_figure(e, parent_val, "orthosliceViewer")?;
    let mut panes: [Axes; 3] = std::array::from_fn(|_| figure.fresh_axes());
    let mut ov = OrthosliceViewerObject::new(
        dims,
        data,
        data_class,
        panes.each_ref().map(|a| a.identity.clone()),
    );
    for (name, val) in props {
        apply_orthoslice_viewer_prop(&mut ov, &name, val)?;
    }

    for (i, axes) in panes.iter_mut().enumerate() {
        ov.sync_pane(i, axes);
    }
    figure.layout = None;
    install_panes(&mut figure, Vec::from(panes))?;
    figure.slice_viewers.clear();
    figure.orthoslice_viewers.clear();
    let token = ov.graphics_handle(&figure.identity);
    figure.orthoslice_viewers.push(ov);
    figure.validate()?;
    e.publish_graphics_figure(fig_id, figure);
    let array = GraphicsArray::scalar(token, &e.cancel)?;
    Ok(Value::Container(Array::Graphics(array)))
}

fn get_axes_handles_call(e: &Engine, args: &[Value], nargout: usize) -> Result<Vec<Value>> {
    if args.len() != 1 {
        return Err("getAxesHandles: expected getAxesHandles(viewer)".into());
    }
    let Value::Container(Array::Graphics(g)) = &args[0] else {
        return Err(
            "getAxesHandles: input must be a SliceViewer or OrthosliceViewer handle".into(),
        );
    };
    if g.len() != 1 {
        return Err("getAxesHandles: input must be a scalar handle".into());
    }
    let h = g.get_linear(0)?;
    let loc = e.locate_graphics(h)?;
    let fig = e
        .figures()
        .find(|(id, _)| *id == loc.figure)
        .map(|(_, f)| f)
        .ok_or("getAxesHandles: viewer figure is no longer live")?;

    match h.kind() {
        GraphicsKind::SliceViewer => {
            if nargout > 1 {
                return Err("Too many output arguments.".into());
            }
            let sv = fig
                .slice_viewers
                .iter()
                .find(|v| v.identity.same_object(&h.object))
                .ok_or("SliceViewer is no longer live")?;
            let ax = GraphicsHandle::axes(&fig.identity, &sv.axes_identity);
            let val = Value::Container(Array::Graphics(GraphicsArray::scalar(ax, &e.cancel)?));
            Ok(if nargout == 0 { vec![] } else { vec![val] })
        }
        GraphicsKind::OrthosliceViewer => {
            if nargout > 3 {
                return Err("Too many output arguments.".into());
            }
            let ov = fig
                .orthoslice_viewers
                .iter()
                .find(|v| v.identity.same_object(&h.object))
                .ok_or("OrthosliceViewer is no longer live")?;
            let handles: Vec<GraphicsHandle> = ov
                .axes_identities
                .iter()
                .map(|id| GraphicsHandle::axes(&fig.identity, id))
                .collect();
            if nargout == 0 {
                Ok(vec![])
            } else if nargout == 1 {
                let shared: Vec<Arc<GraphicsHandle>> = handles.into_iter().map(Arc::new).collect();
                let arr = GraphicsArray::from_shared(1, 3, shared, &e.cancel)?;
                Ok(vec![Value::Container(Array::Graphics(arr))])
            } else {
                let mut out = Vec::with_capacity(nargout);
                for ax in handles.into_iter().take(nargout) {
                    out.push(Value::Container(Array::Graphics(GraphicsArray::scalar(
                        ax, &e.cancel,
                    )?)));
                }
                Ok(out)
            }
        }
        _ => Err("getAxesHandles: input must be a SliceViewer or OrthosliceViewer handle".into()),
    }
}

pub(in crate::engine) fn slice_viewer_get(
    sv: &SliceViewerObject,
    name: &str,
    parent: Value,
) -> Result<Value> {
    Ok(match name {
        "type" => Value::Text("sliceviewer".into()),
        "parent" => parent,
        "slicenumber" => Value::Matrix(Matrix::scalar(sv.slice_number as f64)),
        "slicedirection" => Value::Matrix(Matrix::from_column_slice(
            1,
            3,
            &sv.slice_direction.as_vec(),
        )?),
        "colormap" => colormap_to_value(&sv.colormap)?,
        "displayrange" => Value::Matrix(Matrix::from_column_slice(
            1,
            2,
            &sv.effective_display_range(),
        )?),
        "scalefactors" => Value::Matrix(Matrix::from_column_slice(1, 3, &sv.scale_factors)?),
        "visible" => Value::Text(if sv.visible { "on" } else { "off" }.into()),
        "tag" => Value::Text(sv.tag.clone()),
        _ => {
            return Err(format!(
                "Unsupported SliceViewer property '{name}'; supported: Type, Parent, SliceNumber, SliceDirection, Colormap, DisplayRange, ScaleFactors, Visible, Tag"
            ));
        }
    })
}

pub(in crate::engine) fn orthoslice_viewer_get(
    ov: &OrthosliceViewerObject,
    name: &str,
    parent: Value,
) -> Result<Value> {
    Ok(match name {
        "type" => Value::Text("orthosliceviewer".into()),
        "parent" => parent,
        "slicenumbers" => Value::Matrix(Matrix::from_column_slice(
            1,
            3,
            &ov.slice_numbers.map(|n| n as f64),
        )?),
        "crosshairenabled" => Value::Text(if ov.crosshair_enabled { "on" } else { "off" }.into()),
        "colormap" => colormap_to_value(&ov.colormap)?,
        "displayrange" => Value::Matrix(Matrix::from_column_slice(
            1,
            2,
            &ov.effective_display_range(),
        )?),
        "scalefactors" => Value::Matrix(Matrix::from_column_slice(1, 3, &ov.scale_factors)?),
        "visible" => Value::Text(if ov.visible { "on" } else { "off" }.into()),
        "tag" => Value::Text(ov.tag.clone()),
        _ => {
            return Err(format!(
                "Unsupported OrthosliceViewer property '{name}'; supported: Type, Parent, SliceNumbers, CrosshairEnabled, Colormap, DisplayRange, ScaleFactors, Visible, Tag"
            ));
        }
    })
}

/// Operate on the compound object as a unit while its source and destination
/// are staged. The stored figure provenance of a viewer alias can predate a move.
enum Viewer {
    Slice(SliceViewerObject),
    Ortho(OrthosliceViewerObject),
}

impl Viewer {
    fn pane_ids(&self) -> &[ObjectIdentity] {
        match self {
            Self::Slice(s) => std::slice::from_ref(&s.axes_identity),
            Self::Ortho(o) => &o.axes_identities,
        }
    }

    fn sync(&self, pane: usize, axes: &mut Axes) {
        match self {
            Self::Slice(s) => s.sync_axes(axes),
            Self::Ortho(o) => o.sync_pane(pane, axes),
        }
    }

    fn store(self, figure: &mut Figure, index: usize) {
        match self {
            Self::Slice(s) => figure.slice_viewers.insert(index, s),
            Self::Ortho(o) => figure.orthoslice_viewers.insert(index, o),
        }
    }

    fn colors(&self) -> (&Arc<Vec<[f64; 3]>>, [f64; 2]) {
        match self {
            Self::Slice(s) => (&s.colormap, s.clim()),
            Self::Ortho(o) => (&o.colormap, o.clim()),
        }
    }
}

pub(in crate::engine) fn retire_axes_viewer(
    figure: &mut Figure,
    id: &ObjectIdentity,
) -> Result<()> {
    figure
        .slice_viewers
        .retain(|s| !s.axes_identity.same_object(id));
    if let Some(i) = figure
        .orthoslice_viewers
        .iter()
        .position(|o| o.axes_identities.iter().any(|a| a.same_object(id)))
    {
        let o = figure.orthoslice_viewers.remove(i);
        for axes in figure.all_axes_mut() {
            if let Some(pane) = o
                .axes_identities
                .iter()
                .position(|a| a.same_object(&axes.identity))
            {
                take_pane_graphics(axes, o.pane_graphics(pane))?;
            }
        }
    }
    Ok(())
}

pub(in crate::engine) fn viewer_set(
    e: &mut Engine,
    handle: &GraphicsHandle,
    source: u32,
    args: &[Value],
) -> Result<()> {
    let mut from = e.graphics_figure(source)?.clone();
    let (index, mut viewer) = match handle.kind() {
        GraphicsKind::SliceViewer => {
            let i = from
                .slice_viewers
                .iter()
                .position(|v| v.identity.same_object(&handle.object))
                .ok_or("SliceViewer is no longer live")?;
            (i, Viewer::Slice(from.slice_viewers.remove(i)))
        }
        GraphicsKind::OrthosliceViewer => {
            let i = from
                .orthoslice_viewers
                .iter()
                .position(|v| v.identity.same_object(&handle.object))
                .ok_or("OrthosliceViewer is no longer live")?;
            (i, Viewer::Ortho(from.orthoslice_viewers.remove(i)))
        }
        _ => return Err("Expected a slice viewer handle".into()),
    };
    let mut target = source;
    let mut refresh = false;
    let mut recolor = false;
    let mut rescale = false;
    for pair in args.as_chunks::<2>().0 {
        let name = prop_name(&pair[0], "set")?;
        if name == "parent" {
            target = parent_figure(e, &pair[1], "set")?;
            continue;
        }
        match &mut viewer {
            Viewer::Slice(s) => apply_slice_viewer_prop(s, &name, &pair[1])?,
            Viewer::Ortho(o) => apply_orthoslice_viewer_prop(o, &name, &pair[1])?,
        }
        match name.as_str() {
            "tag" => {}
            "colormap" => recolor = true,
            "displayrange" => rescale = true,
            _ => refresh = true,
        }
    }

    let (colormap, clim) = viewer.colors();
    for (i, id) in viewer.pane_ids().iter().enumerate() {
        let axes = from
            .all_axes_mut()
            .find(|a| a.identity.same_object(id))
            .ok_or("Viewer axes is no longer live")?;
        if refresh {
            viewer.sync(i, axes);
        }
        if recolor {
            axes.extras.colormap = Some(colormap.clone());
        }
        if rescale {
            axes.extras.clim = Some(clim);
        }
    }

    let mut destination = None;
    if target == source {
        viewer.store(&mut from, index);
    } else {
        let mut to = e.graphics_figure(target)?.clone();
        move_viewer(viewer, &mut from, &mut to)?;
        to.validate()?;
        destination = Some(to);
    }
    from.validate()?;
    e.check_cancel()?;
    e.publish_graphics_figure(source, from);
    if let Some(to) = destination {
        e.publish_graphics_figure(target, to);
    }
    Ok(())
}

/// Detach exactly the image and crosshairs. remove_series also updates a
/// user's legend when it contains a crosshair.
fn take_pane_graphics(
    axes: &mut Axes,
    (image, crosshairs): (&ObjectIdentity, &[ObjectIdentity]),
) -> Result<(Option<ImageObject>, Vec<Series>)> {
    let image = axes
        .extras
        .images
        .iter()
        .position(|i| i.identity.same_object(image))
        .map(|i| axes.extras.images.remove(i));
    let mut lines = Vec::new();
    for id in crosshairs {
        if let Some(i) = axes.series.iter().position(|s| s.identity.same_object(id)) {
            lines.push(axes.remove_series(i)?);
        }
    }
    Ok((image, lines))
}

fn move_viewer(mut viewer: Viewer, from: &mut Figure, to: &mut Figure) -> Result<()> {
    let mut panes = Vec::new();
    for (pane, id) in viewer.pane_ids().iter().enumerate() {
        let mut moved = from.fresh_axes();
        let axes = from
            .all_axes_mut()
            .find(|a| a.identity.same_object(id))
            .ok_or("Viewer axes is no longer live")?;
        let graphics = match &viewer {
            Viewer::Slice(s) => (s.image_id(), &[][..]),
            Viewer::Ortho(o) => o.pane_graphics(pane),
        };
        let (image, lines) = take_pane_graphics(axes, graphics)?;
        moved.xlim = axes.xlim;
        moved.ylim = axes.ylim;
        moved.zlim = axes.zlim;
        moved.view = axes.view;
        moved.extras.colormap = axes.extras.colormap.clone();
        moved.extras.clim = axes.extras.clim;
        moved.extras.y_reversed = axes.extras.y_reversed;
        moved.extras.equal_aspect = axes.extras.equal_aspect;
        moved.extras.axis_off = axes.extras.axis_off;
        // Leave independent children and their axes handles in the old figure.
        if axes.same_metadata(&moved) {
            super::super::graphics_handles::remove_axes(from, id)?;
        }
        moved.extras.images.extend(image);
        moved.series = lines;
        panes.push(moved);
    }
    match &mut viewer {
        Viewer::Slice(s) => s.axes_identity = panes[0].identity.clone(),
        Viewer::Ortho(o) => {
            o.axes_identities = std::array::from_fn(|i| panes[i].identity.clone());
        }
    }
    // Keep occupied destination axes and their children after the viewer panes.
    let implicit_axes = to.layout.is_none() && to.axes.same_metadata(&to.fresh_axes());
    if !implicit_axes {
        let mut existing: Vec<_> = to.all_axes().collect();
        existing.sort_by_key(|(pos, _)| *pos);
        panes.extend(existing.into_iter().map(|(_, axes)| axes.clone()));
    }
    install_panes(to, panes)?;
    viewer.store(to, 0);
    Ok(())
}

/// Full canvas for one pane, or reading-order tiles starting with XY/YZ/XZ.
fn install_panes(to: &mut Figure, panes: Vec<Axes>) -> Result<()> {
    let count = panes.len();
    if count > 16 {
        return Err("At most 16 subplot axes per figure".into());
    }
    let mut panes = panes.into_iter();
    to.axes = panes.next().unwrap();
    if count == 1 && to.layout.is_none() {
        return Ok(());
    }
    let columns = count.min(2);
    let rows = count.div_ceil(columns);
    let pos = |idx| Position::from_grid(rows, columns, &[idx]);
    let layout = to
        .layout
        .get_or_insert_with(|| Layout::plain(Position::default()));
    layout.active = pos(1)?;
    layout.tiled = Some(TileGrid {
        pending_first: false,
        ..TileGrid::new(Arrangement::Fixed, rows, columns)?
    });
    layout.others = panes
        .enumerate()
        .map(|(i, axes)| {
            Ok(Panel {
                axes,
                position: pos(i + 2)?,
            })
        })
        .collect::<Result<_>>()?;
    Ok(())
}
