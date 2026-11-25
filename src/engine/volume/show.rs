//! `volshow`, Volume handle get/set, and `alphamap`.
use super::super::{Engine, Value};
use crate::engine::graphics_handles::Located;
use crate::plot::{
    Axes, Figure, ImageDataClass, MAX_COLORMAP_ROWS, OverlayDataClass, OverlayRenderingStyle,
    OverlayVolume, Position, RenderingStyle, VolumeObject, colormap, default_volume_alphamap,
    view3d,
};
use crate::{
    Array, CategoricalArray, GraphicsArray, GraphicsHandle, GraphicsKind, LogicalMatrix, Matrix,
    NumericClass, ObjectIdentity, Result, TypedArray,
};
use std::sync::Arc;

struct VolumeData {
    dims: [usize; 3],
    samples: Arc<[f64]>,
    class: ImageDataClass,
    exact: Option<TypedArray>,
}

fn extract_volume_data(
    e: &Engine,
    v: &Value,
    who: &str,
    overlay_dims: Option<[usize; 3]>,
) -> Result<VolumeData> {
    let d = v.dims();
    if d.len() != 3 || d.contains(&0) {
        return Err(format!("{who}: V must be a nonempty 3-D array"));
    }
    let dims = [d[0], d[1], d[2]];
    let (class, exact) = match v {
        Value::Matrix(_) => (ImageDataClass::Double, None),
        Value::Logical(_) => (ImageDataClass::Logical, None),
        Value::Container(Array::Typed(t)) => {
            let class = t.class();
            let exact =
                matches!(class, NumericClass::Int64 | NumericClass::Uint64).then(|| t.clone());
            (ImageDataClass::Typed(class), exact)
        }
        Value::Complex(_) => return Err(format!("{who}: complex input is not supported")),
        _ => return Err(format!("{who}: V must be a numeric or logical 3-D array")),
    };
    if let Some(overlay_dims) = overlay_dims
        && dims != overlay_dims
    {
        return Err(format!(
            "Data size [{}] must match attached OverlayData size [{}]",
            fmt_dims(&dims),
            fmt_dims(&overlay_dims)
        ));
    }
    let m = v.clone().matrix(&e.cancel)?.with_dims(&d)?;
    Ok(VolumeData {
        dims,
        samples: Arc::from(m.data()),
        class,
        exact,
    })
}

fn volume_data_to_value(e: &Engine, vol: &VolumeObject) -> Result<Value> {
    if let Some(exact) = &vol.exact_data {
        return Ok(Value::Container(exact.clone().into()));
    }
    let [ny, nx, nz] = vol.dims;
    let dims = [ny, nx, nz];
    let cols = nx * nz;
    match vol.data_class {
        ImageDataClass::Double => Ok(Value::Matrix(
            Matrix::from_column_vec(ny, cols, vol.data.to_vec())?.with_dims(&dims)?,
        )),
        ImageDataClass::Logical => {
            let bits: Vec<u8> = vol.data.iter().map(|&x| u8::from(x != 0.0)).collect();
            Ok(Value::Logical(
                LogicalMatrix::from_column_vec(ny, cols, bits, &e.cancel)?.with_dims(&dims)?,
            ))
        }
        ImageDataClass::Typed(class) => {
            let typed = TypedArray::from_reals(class, ny, cols, vol.data.iter().copied())?;
            Ok(Value::Container(typed.with_dims(&dims)?.into()))
        }
    }
}

fn fmt_dims(dims: &[usize]) -> String {
    dims.iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("x")
}

fn extract_overlay_data(
    e: &Engine,
    v: &Value,
    expected_dims: [usize; 3],
) -> Result<Option<OverlayVolume>> {
    let d = v.dims();
    if d.contains(&0) {
        return Ok(None);
    }
    if d.as_ref() != expected_dims.as_slice() {
        return Err(format!(
            "OverlayData size [{}] must match Data size [{}]",
            fmt_dims(&d),
            fmt_dims(&expected_dims)
        ));
    }
    match v {
        Value::Matrix(_) => {
            let m = v.clone().matrix(&e.cancel)?;
            Ok(Some(OverlayVolume {
                data: Arc::from(m.data()),
                class: OverlayDataClass::Double,
            }))
        }
        Value::Logical(_) => {
            let m = v.clone().matrix(&e.cancel)?;
            Ok(Some(OverlayVolume {
                data: Arc::from(m.data()),
                class: OverlayDataClass::Logical,
            }))
        }
        Value::Container(Array::Typed(t)) => {
            let class = t.class();
            let m = v.clone().matrix(&e.cancel)?;
            Ok(Some(OverlayVolume {
                data: Arc::from(m.data()),
                class: OverlayDataClass::Typed(class),
            }))
        }
        Value::Container(Array::Categorical(c)) => {
            let data: Vec<f64> = c.codes().iter().map(|&code| code as f64).collect();
            Ok(Some(OverlayVolume {
                data: Arc::from(data),
                class: OverlayDataClass::Categorical {
                    categories: c.categories().clone(),
                    ordinal: c.is_ordinal(),
                },
            }))
        }
        Value::Complex(_) => Err("OverlayData: complex input is not supported".into()),
        _ => Err("OverlayData must be empty or a numeric, logical, or categorical array".into()),
    }
}

fn overlay_data_to_value(e: &Engine, vol: &VolumeObject) -> Result<Value> {
    let Some(ov) = &vol.overlay_data else {
        return Ok(Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?));
    };
    let [ny, nx, nz] = vol.dims;
    let dims = [ny, nx, nz];
    let cols = nx * nz;
    match &ov.class {
        OverlayDataClass::Double => Ok(Value::Matrix(
            Matrix::from_column_vec(ny, cols, ov.data.to_vec())?.with_dims(&dims)?,
        )),
        OverlayDataClass::Logical => {
            let bits: Vec<u8> = ov.data.iter().map(|&x| u8::from(x != 0.0)).collect();
            Ok(Value::Logical(
                LogicalMatrix::from_column_vec(ny, cols, bits, &e.cancel)?.with_dims(&dims)?,
            ))
        }
        OverlayDataClass::Typed(class) => {
            let typed = TypedArray::from_reals(*class, ny, cols, ov.data.iter().copied())?;
            Ok(Value::Container(typed.with_dims(&dims)?.into()))
        }
        OverlayDataClass::Categorical {
            categories,
            ordinal,
        } => {
            let codes: Vec<u32> = ov.data.iter().map(|&x| x.round() as u32).collect();
            let cat = CategoricalArray::new(ny, cols, codes, categories.clone(), *ordinal)?
                .with_dims(&dims)?;
            Ok(Value::Container(cat.into()))
        }
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
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            {
                return Err("Colormap values must be finite and in [0, 1]".into());
            }
            Ok(Arc::new(rows))
        }
        _ => Err("Colormap must be a colormap name or an n-by-3 matrix in [0, 1]".into()),
    }
}

fn parse_alphamap_vec(value: &Value, who: &str) -> Result<Vec<f64>> {
    let Value::Matrix(m) = value else {
        return Err(format!("{who}: Alphamap must be a real numeric vector"));
    };
    if !(m.rows() == 1 || m.cols() == 1) || m.is_empty() || m.len() > MAX_COLORMAP_ROWS {
        return Err(format!(
            "{who}: Alphamap must be a vector of 1..{MAX_COLORMAP_ROWS} elements"
        ));
    }
    if !m
        .data()
        .iter()
        .all(|v| v.is_finite() && (0. ..=1.).contains(v))
    {
        return Err(format!(
            "{who}: Alphamap values must be finite and in [0, 1]"
        ));
    }
    Ok(m.data().to_vec())
}

fn parse_limits_pair(value: &Value, who: &str) -> Result<[f64; 2]> {
    let Value::Matrix(m) = value else {
        return Err(format!("{who} must be a 2-element real vector [min max]"));
    };
    if m.len() != 2 {
        return Err(format!("{who} must be a 2-element real vector [min max]"));
    }
    let a = m.data()[0];
    let b = m.data()[1];
    if !a.is_finite() || !b.is_finite() || a > b {
        return Err(format!("{who} must be finite [min max] with min <= max"));
    }
    Ok([a, b])
}

fn parse_isosurface_color(value: &Value) -> Result<[f64; 3]> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => {
            let trimmed = s.trim();
            if let Some(hex) = trimmed.strip_prefix('#') {
                if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    let r = u8::from_str_radix(&hex[0..2], 16).unwrap() as f64 / 255.0;
                    let g = u8::from_str_radix(&hex[2..4], 16).unwrap() as f64 / 255.0;
                    let b = u8::from_str_radix(&hex[4..6], 16).unwrap() as f64 / 255.0;
                    return Ok([r, g, b]);
                }
                if hex.len() == 3 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                    let nib = |i: usize| {
                        let v = u8::from_str_radix(&hex[i..i + 1], 16).unwrap();
                        (v * 17) as f64 / 255.0
                    };
                    return Ok([nib(0), nib(1), nib(2)]);
                }
                return Err("IsosurfaceColor hex string must be '#RGB' or '#RRGGBB'".into());
            }
            match trimmed.to_ascii_lowercase().as_str() {
                "r" | "red" => Ok([1.0, 0.0, 0.0]),
                "g" | "green" => Ok([0.0, 1.0, 0.0]),
                "b" | "blue" => Ok([0.0, 0.0, 1.0]),
                "c" | "cyan" => Ok([0.0, 1.0, 1.0]),
                "m" | "magenta" => Ok([1.0, 0.0, 1.0]),
                "y" | "yellow" => Ok([1.0, 1.0, 0.0]),
                "k" | "black" => Ok([0.0, 0.0, 0.0]),
                "w" | "white" => Ok([1.0, 1.0, 1.0]),
                _ => Err(
                    "IsosurfaceColor must be an RGB triplet in [0, 1], a color name, or '#RRGGBB'"
                        .into(),
                ),
            }
        }
        Value::Matrix(m) => {
            if m.len() != 3 || !(m.rows() == 1 || m.cols() == 1) {
                return Err("IsosurfaceColor must be a 3-element RGB triplet in [0, 1]".into());
            }
            let v = m.data();
            if v.iter().any(|c| !c.is_finite() || !(0.0..=1.0).contains(c)) {
                return Err("IsosurfaceColor RGB values must be finite and in [0, 1]".into());
            }
            Ok([v[0], v[1], v[2]])
        }
        _ => Err(
            "IsosurfaceColor must be an RGB triplet in [0, 1], a color name, or '#RRGGBB'".into(),
        ),
    }
}

fn parse_slice_planes(value: &Value) -> Result<Vec<[f64; 4]>> {
    let Value::Matrix(m) = value else {
        return Err("SlicePlaneValues must be empty or an N-by-4 numeric matrix".into());
    };
    if m.is_empty() {
        return Ok(Vec::new());
    }
    if m.cols() != 4 {
        return Err("SlicePlaneValues must be an N-by-4 numeric matrix".into());
    }
    let mut planes = Vec::with_capacity(m.rows());
    for r in 0..m.rows() {
        let row = [m.get(r, 0), m.get(r, 1), m.get(r, 2), m.get(r, 3)];
        if !row.iter().all(|v| v.is_finite()) {
            return Err("SlicePlaneValues must contain finite numbers".into());
        }
        planes.push(row);
    }
    Ok(planes)
}

fn parse_transformation(value: &Value) -> Result<[f64; 3]> {
    let Value::Matrix(m) = value else {
        return Err(
            "Transformation must be a 3-element scale vector [sx sy sz] or 4-by-4 scaling matrix"
                .into(),
        );
    };
    let scale = if m.len() == 3 && (m.rows() == 1 || m.cols() == 1) {
        [m.data()[0], m.data()[1], m.data()[2]]
    } else if m.rows() == 4 && m.cols() == 4 {
        for r in 0..4 {
            for c in 0..4 {
                if r != c && m.get(r, c) != 0.0 {
                    return Err("Transformation currently supports voxel spacing scale only (diagonal 4x4 or [sx sy sz])".into());
                }
            }
        }
        if m.get(3, 3) != 1.0 {
            return Err("Transformation 4x4 matrix must have 1 in bottom-right entry".into());
        }
        [m.get(0, 0), m.get(1, 1), m.get(2, 2)]
    } else {
        return Err(
            "Transformation must be a 3-element scale vector [sx sy sz] or 4-by-4 scaling matrix"
                .into(),
        );
    };
    if !scale.iter().all(|v| v.is_finite() && *v > 0.0) {
        return Err("Transformation voxel spacing scale must be finite and positive".into());
    }
    Ok(scale)
}

fn parse_visible(value: &Value) -> Result<bool> {
    match value {
        Value::Text(s) | Value::QuotedString(s) => match s.to_ascii_lowercase().as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err("Visible must be 'on' or 'off'".into()),
        },
        Value::Logical(l) if l.len() == 1 => Ok(l.data()[0] != 0),
        _ => Err("Visible must be 'on' or 'off'".into()),
    }
}

fn prop_name(v: &Value, who: &str) -> Result<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Ok(s.to_ascii_lowercase()),
        _ => Err(format!("{who}: property name must be text")),
    }
}

fn apply_volume_prop(e: &Engine, vol: &mut VolumeObject, name: &str, val: &Value) -> Result<()> {
    match name {
        "data" => {
            let overlay_dims = vol.overlay_data.is_some().then_some(vol.dims);
            let data = extract_volume_data(e, val, "Volume", overlay_dims)?;
            vol.dims = data.dims;
            vol.data = data.samples;
            vol.data_class = data.class;
            vol.exact_data = data.exact;
            vol.data_revision = ObjectIdentity::default();
            if !vol.data_limits_manual {
                let (lo, hi) = crate::plot::volume::finite_min_max(&vol.data);
                vol.data_limits = [lo, hi];
            }
        }
        "renderingstyle" => {
            let (Value::Text(s) | Value::QuotedString(s)) = val else {
                return Err("RenderingStyle must be a character vector or string".into());
            };
            vol.rendering_style = RenderingStyle::parse(s)?;
        }
        "colormap" => {
            vol.colormap = parse_colormap(val)?;
        }
        "alphamap" => {
            vol.alphamap = Arc::new(parse_alphamap_vec(val, "Volume")?);
        }
        "datalimits" => {
            vol.data_limits = parse_limits_pair(val, "DataLimits")?;
            vol.data_limits_manual = true;
        }
        "isosurfacevalue" => {
            let Value::Matrix(m) = val else {
                return Err("IsosurfaceValue must be a finite scalar".into());
            };
            let v = m.as_scalar()?;
            if !v.is_finite() {
                return Err("IsosurfaceValue must be finite".into());
            }
            vol.isosurface_value = v;
        }
        "isosurfacecolor" => {
            vol.isosurface_color = parse_isosurface_color(val)?;
        }
        "sliceplanevalues" => {
            vol.slice_plane_values = parse_slice_planes(val)?;
        }
        "transformation" => {
            vol.transformation = parse_transformation(val)?;
        }
        "visible" => {
            vol.visible = parse_visible(val)?;
        }
        "overlaydata" => {
            let ov = extract_overlay_data(e, val, vol.dims)?;
            if !vol.overlay_display_range_manual {
                vol.overlay_display_range = ov.as_ref().map_or([0.0, 1.0], |o| {
                    let (lo, hi) = crate::plot::volume::finite_min_max(&o.data);
                    [lo, hi]
                });
            }
            vol.overlay_data = ov;
        }
        "overlayrenderingstyle" => {
            let (Value::Text(s) | Value::QuotedString(s)) = val else {
                return Err("OverlayRenderingStyle must be a character vector or string".into());
            };
            vol.overlay_rendering_style = OverlayRenderingStyle::parse(s)?;
        }
        "overlaycolormap" => {
            vol.overlay_colormap = parse_colormap(val)?;
        }
        "overlayalphamap" => {
            vol.overlay_alphamap = Arc::new(parse_alphamap_vec(val, "OverlayAlphamap")?);
        }
        "overlaydisplayrange" => {
            vol.overlay_display_range = parse_limits_pair(val, "OverlayDisplayRange")?;
            vol.overlay_display_range_manual = true;
        }
        _ => {
            return Err(format!(
                "Unsupported Volume property '{name}'; supported: Data, RenderingStyle, Colormap, Alphamap, DataLimits, IsosurfaceValue, IsosurfaceColor, SlicePlaneValues, Transformation, Visible, OverlayData, OverlayRenderingStyle, OverlayColormap, OverlayAlphamap, OverlayDisplayRange"
            ));
        }
    }
    vol.validate()
}

fn volume_parent(e: &Engine, parent: &Value) -> Result<Located> {
    let error = "Volume Parent must be a scalar live Axes handle";
    let Value::Container(Array::Graphics(handles)) = parent else {
        return Err(error.into());
    };
    let handle = handles.scalar_handle().map_err(|_| error)?;
    if handle.kind() != GraphicsKind::Axes {
        return Err(error.into());
    }
    match e.locate_graphics(handle) {
        Ok(loc) => Ok(loc),
        Err(_) => {
            e.check_cancel()?;
            Err(error.into())
        }
    }
}

fn volume_axes_mut(figure: &mut Figure, position: Position) -> Result<&mut Axes> {
    if figure.active_position() == position {
        return Ok(&mut figure.axes);
    }
    figure
        .layout
        .as_mut()
        .and_then(|layout| layout.others.iter_mut().find(|p| p.position == position))
        .map(|panel| &mut panel.axes)
        .ok_or_else(|| "Volume Parent axes is no longer live".into())
}

pub(in crate::engine) fn volshow_call(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err("volshow: expected volshow(V) or volshow(V, Name, Value, ...)".into());
    }
    let data = extract_volume_data(e, &args[0], "volshow", None)?;
    let rest = &args[1..];
    if !rest.len().is_multiple_of(2) {
        return Err("volshow: name/value options must come in pairs".into());
    }
    let mut vol = VolumeObject::new(data.dims, data.samples, data.class);
    vol.exact_data = data.exact;
    let mut parent_val: Option<&Value> = None;
    for pair in rest.as_chunks::<2>().0 {
        let name = prop_name(&pair[0], "volshow")?;
        if name == "parent" {
            parent_val = Some(&pair[1]);
        } else {
            apply_volume_prop(e, &mut vol, &name, &pair[1])?;
        }
    }

    let (fig_id, mut figure, axes_pos) = if let Some(parent) = parent_val {
        let loc = volume_parent(e, parent)?;
        let fig = e.graphics_figure(loc.figure)?.clone();
        (loc.figure, fig, loc.position)
    } else {
        let held = e.figure.as_ref().is_some_and(|f| f.hold);
        let mut fig: Figure = e.surface_target();
        if !held {
            fig.axes.view = view3d::VIEW_3D;
        }
        (e.current_figure, fig, None)
    };

    let token = GraphicsHandle::volume(&vol.identity);
    let position = axes_pos.unwrap_or_else(|| figure.active_position());
    let target_axes = volume_axes_mut(&mut figure, position)?;
    target_axes.extras.volumes.push(vol);
    figure.validate()?;
    let array = GraphicsArray::scalar(token, &e.cancel)?;
    e.check_cancel()?;
    e.publish_graphics_figure(fig_id, figure);
    Ok(Value::Container(Array::Graphics(array)))
}

pub(in crate::engine) fn volume_get(
    e: &Engine,
    vol: &VolumeObject,
    name: &str,
    parent: Value,
) -> Result<Value> {
    Ok(match name {
        "type" => Value::Text("volume".into()),
        "parent" => parent,
        "data" => volume_data_to_value(e, vol)?,
        "renderingstyle" => Value::Text(vol.rendering_style.as_str().into()),
        "colormap" => {
            let rows = &vol.colormap;
            let mut data = Vec::with_capacity(rows.len() * 3);
            for c in 0..3 {
                data.extend(rows.iter().map(|r| r[c]));
            }
            Value::Matrix(Matrix::from_column_vec(rows.len(), 3, data)?)
        }
        "alphamap" => {
            let a = &vol.alphamap;
            Value::Matrix(Matrix::from_column_vec(a.len(), 1, a.to_vec())?)
        }
        "datalimits" => Value::Matrix(Matrix::from_column_slice(1, 2, &vol.data_limits)?),
        "isosurfacevalue" => Value::Matrix(Matrix::scalar(vol.isosurface_value)),
        "isosurfacecolor" => Value::Matrix(Matrix::from_column_slice(1, 3, &vol.isosurface_color)?),
        "sliceplanevalues" => {
            let n = vol.slice_plane_values.len();
            if n == 0 {
                Value::Matrix(Matrix::from_column_vec(0, 0, Vec::new())?)
            } else {
                let mut data = Vec::with_capacity(n * 4);
                for c in 0..4 {
                    data.extend(vol.slice_plane_values.iter().map(|r| r[c]));
                }
                Value::Matrix(Matrix::from_column_vec(n, 4, data)?)
            }
        }
        "transformation" => Value::Matrix(Matrix::from_column_slice(1, 3, &vol.transformation)?),
        "visible" => Value::Text(if vol.visible { "on" } else { "off" }.into()),
        "overlaydata" => overlay_data_to_value(e, vol)?,
        "overlayrenderingstyle" => Value::Text(vol.overlay_rendering_style.as_str().into()),
        "overlaycolormap" => {
            let rows = &vol.overlay_colormap;
            let mut data = Vec::with_capacity(rows.len() * 3);
            for c in 0..3 {
                data.extend(rows.iter().map(|r| r[c]));
            }
            Value::Matrix(Matrix::from_column_vec(rows.len(), 3, data)?)
        }
        "overlayalphamap" => {
            let a = &vol.overlay_alphamap;
            if a.len() == 1 {
                Value::Matrix(Matrix::scalar(a[0]))
            } else {
                Value::Matrix(Matrix::from_column_vec(a.len(), 1, a.to_vec())?)
            }
        }
        "overlaydisplayrange" => {
            Value::Matrix(Matrix::from_column_slice(1, 2, &vol.overlay_display_range)?)
        }
        _ => {
            return Err(
                "Supported Volume queries: Type, Parent, Data, RenderingStyle, Colormap, Alphamap, DataLimits, IsosurfaceValue, IsosurfaceColor, SlicePlaneValues, Transformation, Visible, OverlayData, OverlayRenderingStyle, OverlayColormap, OverlayAlphamap, OverlayDisplayRange".into(),
            );
        }
    })
}

pub(in crate::engine) fn volume_display_text(vol: &VolumeObject) -> String {
    let [ny, nx, nz] = vol.dims;
    let data_class = match vol.data_class {
        ImageDataClass::Double => "double",
        ImageDataClass::Logical => "logical",
        ImageDataClass::Typed(c) => c.name(),
    };
    let ov_data = match &vol.overlay_data {
        None => "[]".to_string(),
        Some(ov) => {
            let cls = match &ov.class {
                OverlayDataClass::Double => "double",
                OverlayDataClass::Logical => "logical",
                OverlayDataClass::Typed(c) => c.name(),
                OverlayDataClass::Categorical { .. } => "categorical",
            };
            format!("[{ny}x{nx}x{nz} {cls}]")
        }
    };
    let ov_alpha = if vol.overlay_alphamap.len() == 1 {
        format!("{}", vol.overlay_alphamap[0])
    } else {
        format!("[{}x1 double]", vol.overlay_alphamap.len())
    };
    format!(
        "  Volume with properties:\n\n\
         {:>23}: [{ny}x{nx}x{nz} {data_class}]\n\
         {:>23}: '{}'\n\
         {:>23}: [{}x3 double]\n\
         {:>23}: [{}x1 double]\n\
         {:>23}: [{} {}]\n\
         {:>23}: {}\n\
         {:>23}: [{} {} {}]\n\
         {:>23}: [{}x4 double]\n\
         {:>23}: [{} {} {}]\n\
         {:>23}: '{}'\n\
         {:>23}: {}\n\
         {:>23}: '{}'\n\
         {:>23}: [{}x3 double]\n\
         {:>23}: {}\n\
         {:>23}: [{} {}]\n",
        "Data",
        "RenderingStyle",
        vol.rendering_style.as_str(),
        "Colormap",
        vol.colormap.len(),
        "Alphamap",
        vol.alphamap.len(),
        "DataLimits",
        vol.data_limits[0],
        vol.data_limits[1],
        "IsosurfaceValue",
        vol.isosurface_value,
        "IsosurfaceColor",
        vol.isosurface_color[0],
        vol.isosurface_color[1],
        vol.isosurface_color[2],
        "SlicePlaneValues",
        vol.slice_plane_values.len(),
        "Transformation",
        vol.transformation[0],
        vol.transformation[1],
        vol.transformation[2],
        "Visible",
        if vol.visible { "on" } else { "off" },
        "OverlayData",
        ov_data,
        "OverlayRenderingStyle",
        vol.overlay_rendering_style.as_str(),
        "OverlayColormap",
        vol.overlay_colormap.len(),
        "OverlayAlphamap",
        ov_alpha,
        "OverlayDisplayRange",
        vol.overlay_display_range[0],
        vol.overlay_display_range[1],
    )
}

pub(in crate::engine) fn volume_display(e: &Engine, array: &Array) -> Option<String> {
    let Array::Graphics(g) = array else {
        return None;
    };
    if g.len() != 1 {
        return None;
    }
    let h = g.get_linear(0).ok()?;
    if h.kind() != GraphicsKind::Volume {
        return None;
    }
    let loc = e.locate_graphics(h).ok()?;
    let fig = e.graphics_figure(loc.figure).ok()?;
    let pos = loc.position?;
    let axes = if fig.active_position() == pos {
        &fig.axes
    } else {
        &fig.layout
            .as_ref()?
            .others
            .iter()
            .find(|p| p.position == pos)?
            .axes
    };
    let vol = axes
        .extras
        .volumes
        .iter()
        .find(|v| v.identity.same_object(&h.object))?;
    Some(volume_display_text(vol))
}

pub(in crate::engine) fn volume_set(
    e: &mut Engine,
    id: &ObjectIdentity,
    source: Located,
    args: &[Value],
) -> Result<()> {
    let mut from = e.graphics_figure(source.figure)?.clone();
    let source_position = source.position.ok_or("Volume axes is no longer live")?;
    let volumes = &mut volume_axes_mut(&mut from, source_position)?.extras.volumes;
    let index = volumes
        .iter()
        .position(|v| v.identity.same_object(id))
        .ok_or("Volume is no longer live")?;
    let mut target = source;
    for pair in args.as_chunks::<2>().0 {
        let name = prop_name(&pair[0], "set")?;
        if name == "parent" {
            target = volume_parent(e, &pair[1])?;
        } else {
            apply_volume_prop(e, &mut volumes[index], &name, &pair[1])?;
        }
    }
    let mut other = None;
    if target.figure != source.figure || target.position != source.position {
        let mut volume = volumes.remove(index);
        volume.stack_order = ObjectIdentity::default();
        let position = target
            .position
            .ok_or("Volume Parent axes is no longer live")?;
        let to = if target.figure == source.figure {
            &mut from
        } else {
            other.insert(e.graphics_figure(target.figure)?.clone())
        };
        volume_axes_mut(to, position)?.extras.volumes.push(volume);
    }
    from.validate()?;
    if let Some(figure) = &other {
        figure.validate()?;
    }
    e.check_cancel()?;
    e.publish_graphics_figure(source.figure, from);
    if let Some(figure) = other {
        e.publish_graphics_figure(target.figure, figure);
    }
    Ok(())
}

fn alphamap_ramp(option: &str, current: &[f64], delta: Option<f64>) -> Result<Vec<f64>> {
    let m = current.len().max(1);
    match option.to_ascii_lowercase().as_str() {
        "default" => Ok(default_volume_alphamap(64)),
        "rampup" => Ok(default_volume_alphamap(m)),
        "rampdown" => Ok(if m <= 1 {
            vec![1.0; m]
        } else {
            (0..m).map(|i| 1.0 - i as f64 / (m - 1) as f64).collect()
        }),
        "vup" => Ok(if m <= 1 {
            vec![1.0; m]
        } else {
            (0..m)
                .map(|i| {
                    let t = 2.0 * i as f64 / (m - 1) as f64 - 1.0;
                    (1.0 - t.abs()).clamp(0.0, 1.0)
                })
                .collect()
        }),
        "vdown" => Ok(if m <= 1 {
            vec![1.0; m]
        } else {
            (0..m)
                .map(|i| {
                    let t = 2.0 * i as f64 / (m - 1) as f64 - 1.0;
                    t.abs().clamp(0.0, 1.0)
                })
                .collect()
        }),
        "increase" => {
            let d = delta.unwrap_or(0.1);
            Ok(current.iter().map(|&v| (v + d).clamp(0.0, 1.0)).collect())
        }
        "decrease" => {
            let d = delta.unwrap_or(0.1);
            Ok(current.iter().map(|&v| (v - d).clamp(0.0, 1.0)).collect())
        }
        _ => Err(format!(
            "Unknown alphamap option '{option}'; supported: 'rampup', 'rampdown', 'vup', 'vdown', 'default', 'increase', 'decrease'"
        )),
    }
}

pub(in crate::engine) fn alphamap_call(
    e: &mut Engine,
    args: &[Value],
    nargout: usize,
) -> Result<Value> {
    let (fig_id, rest) = if let Some(first) = args.first() {
        match first {
            Value::Container(Array::Graphics(g)) => {
                if g.len() != 1 {
                    return Err("alphamap: figure handle must be scalar".into());
                }
                let h = g.get_linear(0)?;
                if h.kind() != GraphicsKind::Figure {
                    return Err("alphamap: target handle must be a Figure".into());
                }
                let loc = e.locate_graphics(h)?;
                (loc.figure, &args[1..])
            }
            _ => (e.current_figure, args),
        }
    } else {
        (e.current_figure, args)
    };

    let mut figure = if let Some(f) = e
        .figures()
        .find(|(id, _)| *id == fig_id)
        .map(|(_, f)| f.clone())
    {
        f
    } else {
        e.select_figure(fig_id)?;
        e.figures()
            .find(|(id, _)| *id == fig_id)
            .map(|(_, f)| f.clone())
            .ok_or("alphamap: figure does not exist")?
    };

    let current = figure.effective_alphamap();
    match rest {
        [] => Ok(Value::Matrix(Matrix::from_column_vec(
            current.len(),
            1,
            current.to_vec(),
        )?)),
        [Value::Text(opt)] | [Value::QuotedString(opt)] => {
            let next = alphamap_ramp(opt, &current, None)?;
            if nargout > 0 {
                Ok(Value::Matrix(Matrix::from_column_vec(next.len(), 1, next)?))
            } else {
                figure.alphamap = Some(Arc::new(next));
                e.publish_graphics_figure(fig_id, figure);
                Ok(Value::Void)
            }
        }
        [Value::Text(opt) | Value::QuotedString(opt), delta_val] => {
            let lower = opt.to_ascii_lowercase();
            if !matches!(lower.as_str(), "increase" | "decrease") {
                return Err(
                    "alphamap: delta is only supported with 'increase' or 'decrease'".into(),
                );
            }
            let Value::Matrix(m) = delta_val else {
                return Err("alphamap: delta must be a finite scalar".into());
            };
            let delta = m.as_scalar()?;
            if !delta.is_finite() {
                return Err("alphamap: delta must be finite".into());
            }
            let next = alphamap_ramp(opt, &current, Some(delta))?;
            if nargout > 0 {
                Ok(Value::Matrix(Matrix::from_column_vec(next.len(), 1, next)?))
            } else {
                figure.alphamap = Some(Arc::new(next));
                e.publish_graphics_figure(fig_id, figure);
                Ok(Value::Void)
            }
        }
        [map_val] => {
            let next = parse_alphamap_vec(map_val, "alphamap")?;
            if nargout > 0 {
                Ok(Value::Matrix(Matrix::from_column_vec(next.len(), 1, next)?))
            } else {
                figure.alphamap = Some(Arc::new(next));
                e.publish_graphics_figure(fig_id, figure);
                Ok(Value::Void)
            }
        }
        _ => Err(
            "Use alphamap, alphamap(map), alphamap(option[, delta]), or alphamap(fig, ...)".into(),
        ),
    }
}
