//! Real patch/textbox objects using the ordinary name/value grammar.
//! This child module is composed under graphics_handles, not an export parser.
use super::{
    Engine, GraphicsArray, GraphicsHandle, GraphicsKind, Result, Value, handle, text, vector,
};
use crate::{
    Array, Matrix,
    plot::{
        Annotation, AnnotationHeadStyle, AnnotationKind, IndexedColor, IndexedPatch, LinePattern,
        Series, TextAlign, TextBox,
    },
};

fn pairs(args: &[Value]) -> Result<&[[Value; 2]]> {
    if args.len() > 32 || !args.len().is_multiple_of(2) {
        return Err("Graphics construction accepts at most 16 property/value pairs".into());
    }
    Ok(args.as_chunks::<2>().0)
}
fn rgb(value: &Value) -> Result<[u8; 3]> {
    let values = vector(value, 3)?;
    if values
        .iter()
        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
    {
        return Err("RGB components must be finite values in 0..1".into());
    }
    Ok(std::array::from_fn(|i| (values[i] * 255.).round() as u8))
}
fn on_off(value: &Value) -> Result<bool> {
    match text(value)?.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err("Expected on or off".into()),
    }
}
fn literal_only(value: &Value, expected: &str) -> Result<()> {
    if !text(value)?.eq_ignore_ascii_case(expected) {
        return Err(format!("This property supports only '{expected}'"));
    }
    Ok(())
}
fn edge_color(value: &Value) -> Result<Option<[u8; 3]>> {
    if matches!(value, Value::Text(s) if s.eq_ignore_ascii_case("none")) {
        Ok(None)
    } else {
        Ok(Some(super::super::plot_properties::color(value)?))
    }
}
fn alpha_value(value: &Value, name: &str) -> Result<f64> {
    let a = vector(value, 1)?[0];
    if !a.is_finite() || !(0. ..=1.).contains(&a) {
        return Err(format!("{name} must be a finite scalar in 0..1"));
    }
    Ok(a)
}
fn face_color(value: &Value) -> Result<Option<[u8; 3]>> {
    if matches!(value, Value::Text(s) if s.eq_ignore_ascii_case("none")) {
        Ok(None)
    } else {
        Ok(Some(super::super::plot_properties::color(value)?))
    }
}
fn normalize_patch_matrix(m: &Matrix) -> Result<Matrix> {
    if m.rows() == 1 && matches!(m.cols(), 3..=5) {
        m.reshape(m.cols(), 1)
    } else {
        Ok(m.clone())
    }
}
fn line_pattern(value: &Value) -> Result<crate::plot::LinePattern> {
    use crate::plot::LinePattern;
    match text(value)? {
        "-" => Ok(LinePattern::Solid),
        "--" => Ok(LinePattern::Dashed),
        ":" => Ok(LinePattern::Dotted),
        "-." => Ok(LinePattern::DashDot),
        "none" => Ok(LinePattern::None),
        _ => Err("LineStyle supports -, --, :, -. or none".into()),
    }
}
fn line_width(value: &Value) -> Result<f64> {
    let width = vector(value, 1)?[0];
    crate::plot::validate_line_width(width)?;
    Ok(width)
}
pub(super) fn patch_properties(target: &mut Series, args: &[Value]) -> Result<()> {
    for pair in pairs(args)? {
        match text(&pair[0])?.to_ascii_lowercase().as_str() {
            "facecolor" | "edgecolor" if target.slice_contour_level.is_some() => {
                return Err(
                    "contourslice patches use FaceColor 'none' and EdgeColor 'flat'".into(),
                );
            }
            "facecolor" => {
                if let Some(mesh) = &mut target.indexed_patch {
                    let color = indexed_color(&pair[1])?;
                    // Opacity stays whatever FaceAlpha asked for. `none` and a
                    // mapped color with no CData only hide the fill.
                    let hidden = indexed_fill_hidden(color, mesh.colors.is_some());
                    mesh.face_color = color;
                    mesh.mapped_with = None;
                    target.face_colors = None;
                    target.face_color_none = hidden;
                    if let IndexedColor::Rgb(rgb) = color {
                        target.color = rgb;
                    }
                } else {
                    match face_color(&pair[1])? {
                        Some(c) => {
                            target.color = c;
                            target.face_color_none = false;
                        }
                        None => target.face_color_none = true,
                    }
                }
                target.explicit_color = true;
                if target.indexed_patch.is_some() {
                    target.face_colors = None;
                }
            }
            "edgecolor" => {
                target.face_edge_color = edge_color(&pair[1])?;
                if target.indexed_patch.is_some() {
                    target.face_edge_colors = None;
                }
            }
            "facealpha" => target.face_alpha = Some(alpha_value(&pair[1], "FaceAlpha")?),
            "edgealpha" => target.edge_alpha = Some(alpha_value(&pair[1], "EdgeAlpha")?),
            "linestyle" => target.pattern = line_pattern(&pair[1])?,
            "linewidth" => target.line_width = Some(line_width(&pair[1])?),
            "displayname" => {
                let value = text(&pair[1])?;
                if value.len() > 4096 {
                    return Err("Patch label exceeds 4096 bytes".into());
                }
                target.label = value.into();
            }
            "visible" => target.visible = on_off(&pair[1])?,
            "vertexnormals" if target.indexed_patch.is_some() => {
                let points = target.indexed_patch.as_mut().unwrap();
                let m = patch_matrix(&pair[1], "VertexNormals")?;
                if m.cols() != 3 || m.rows() != points.vertices.rows() {
                    return Err(
                        "Patch VertexNormals must be one three-component normal per vertex".into(),
                    );
                }
                points.normals = Some(m.clone().without_coordinate_proof());
            }
            _ => {
                return Err(
                    "Patch set supports constant FaceColor, RGB/none EdgeColor, FaceAlpha, EdgeAlpha, LineStyle, LineWidth, DisplayName and Visible".into(),
                );
            }
        }
    }
    Ok(())
}
pub(super) fn indexed_patch_property(s: &Series, name: &str) -> Result<Option<Value>> {
    let Some(mesh) = &s.indexed_patch else {
        return Ok(None);
    };
    let value = match name {
        "facecolor" => {
            let mode = match mesh.face_color {
                IndexedColor::Flat => "flat",
                IndexedColor::Interp => "interp",
                IndexedColor::None => "none",
                IndexedColor::Rgb(_) => return Ok(None),
            };
            return Ok(Some(Value::Text(mode.into())));
        }
        "faces" => mesh.faces.clone(),
        "vertices" => mesh.vertices.clone(),
        "facevertexcdata" => mesh.colors.clone().unwrap_or(Matrix::filled(0, 0, 0.)?),
        "vertexnormals" => mesh.normals.clone().unwrap_or(Matrix::filled(0, 3, 0.)?),
        _ => return Ok(None),
    };
    Ok(Some(Value::Matrix(value)))
}

fn patch_matrix<'a>(value: &'a Value, name: &str) -> Result<&'a Matrix> {
    match value {
        Value::Matrix(data) if !data.is_nd() => Ok(data),
        _ => Err(format!("Patch {name} requires a real numeric matrix")),
    }
}

fn indexed_color(value: &Value) -> Result<IndexedColor> {
    Ok(match value {
        Value::Text(s) if s.eq_ignore_ascii_case("flat") => IndexedColor::Flat,
        Value::Text(s) if s.eq_ignore_ascii_case("interp") => IndexedColor::Interp,
        Value::Text(s) if s.eq_ignore_ascii_case("none") => IndexedColor::None,
        _ => IndexedColor::Rgb(super::super::plot_properties::color(value)?),
    })
}

/// Fill is absent for `'none'` and for flat/interp until FaceVertexCData exists.
/// That choice must not write FaceAlpha.
fn indexed_fill_hidden(color: IndexedColor, has_colors: bool) -> bool {
    matches!(color, IndexedColor::None) || (color.mapped() && !has_colors)
}

fn indexed_series(
    engine: &Engine,
    mut mesh: IndexedPatch,
    mut ledger: crate::plot::coordinates::GeometryLedger,
) -> Result<Series> {
    use crate::plot::{MAX_PATCH_FACES, PatchFaces, PlotKind};
    let (f, v, c) = (&mesh.faces, &mesh.vertices, mesh.colors.as_ref());
    if !matches!(f.cols(), 3 | 4) || !matches!(v.cols(), 2 | 3) {
        return Err(
            "Indexed patch requires three or four columns in Faces and two or three in Vertices"
                .into(),
        );
    }
    if f.rows() > MAX_PATCH_FACES || v.rows() > 3 * MAX_PATCH_FACES {
        return Err(format!(
            "Indexed patch exceeds {MAX_PATCH_FACES} faces or {} vertices",
            3 * MAX_PATCH_FACES
        ));
    }
    if let Some(c) = c
        && (c.rows(), c.cols()) != (v.rows(), 1)
    {
        return Err("Patch FaceVertexCData must be a column with one value per vertex".into());
    }
    for (i, x) in v.data().iter().enumerate() {
        if i.is_multiple_of(4096) {
            engine.check_cancel()?;
        }
        if !x.is_finite() {
            return Err("Indexed patch vertices must be finite".into());
        }
    }
    let z_len = if v.cols() == 3 { f.len() } else { 0 };
    ledger.add(mesh.geometry_cost(
        crate::plot::coordinates::GeometryCost::dense_face_allocation(f.len(), f.cols())?,
        z_len,
    )?)?;
    engine.preflight_elements(3 * f.len() + z_len)?;
    if let Some(c) = c {
        for (i, &value) in c.data().iter().enumerate() {
            if i.is_multiple_of(4096) {
                engine.check_cancel()?;
            }
            if value.is_finite() {
                mesh.color_range = Some(
                    mesh.color_range
                        .map_or([value, value], |[lo, hi]| [lo.min(value), hi.max(value)]),
                );
            }
        }
    }
    let (points, z) = mesh.expand(|| engine.check_cancel())?;
    let mut series = Series::new(points, PlotKind::Patch);
    series.patch = Some(PatchFaces::new(f.cols())?);
    series.z = z.into();
    series.face_color_none = indexed_fill_hidden(mesh.face_color, c.is_some());
    series.indexed_patch = Some(mesh);
    Ok(series)
}

/// Re-map only the per-face fills. The shared mesh and the expanded XYZ stream stay in place.
pub(super) fn refresh_indexed_colors(
    engine: &Engine,
    series: &mut Series,
    map: &std::sync::Arc<Vec<[f64; 3]>>,
    limits: [f64; 2],
) -> Result<()> {
    let Some(mesh) = &mut series.indexed_patch else {
        return Ok(());
    };
    let Some(c) = &mesh.colors else { return Ok(()) };
    if !mesh.face_color.mapped()
        || mesh
            .mapped_with
            .as_ref()
            .is_some_and(|(m, l)| *l == limits && (std::sync::Arc::ptr_eq(m, map) || m == map))
    {
        return Ok(());
    }
    let mut fill = Vec::with_capacity(mesh.faces.rows());
    for face in 0..mesh.faces.rows() {
        if face.is_multiple_of(4096) {
            engine.check_cancel()?;
        }
        let mapped = |corner| {
            let vertex = mesh.faces.get(face, corner) as usize - 1;
            crate::plot::colormap::scaled_rgb(map, limits, c.get(vertex, 0))
        };
        fill.push(if mesh.face_color == IndexedColor::Interp {
            let width = mesh.faces.cols() as u16;
            let mut sums = [0u16; 3];
            for corner in 0..mesh.faces.cols() {
                let rgb = mapped(corner);
                for k in 0..3 {
                    sums[k] += u16::from(rgb[k]);
                }
            }
            sums.map(|sum| ((sum + width / 2) / width) as u8)
        } else {
            mapped(0)
        });
    }
    series.face_colors = Some(fill.into());
    mesh.mapped_with = Some((map.clone(), limits));
    Ok(())
}

fn textbox_properties(target: &mut TextBox, args: &[Value], constructor: bool) -> Result<()> {
    let mut literal = false;
    let mut no_edge = false;
    for pair in pairs(args)? {
        match text(&pair[0])?.to_ascii_lowercase().as_str() {
            "string" => {
                let value = match &pair[1] {
                    Value::QuotedString(s) => s.as_str(),
                    other => text(other)?,
                };
                if value.len() > 4096 {
                    return Err("Textbox text exceeds 4096 bytes".into());
                }
                target.text = value.into();
            }
            "position" => target.position = vector(&pair[1], 4)?.try_into().unwrap(),
            "units" => literal_only(&pair[1], "normalized")?,
            "interpreter" => {
                literal_only(&pair[1], "none")?;
                literal = true;
            }
            "edgecolor" => {
                literal_only(&pair[1], "none")?;
                no_edge = true;
            }
            "backgroundcolor" => literal_only(&pair[1], "none")?,
            "fitboxtotext" => literal_only(&pair[1], "off")?,
            "verticalalignment" => literal_only(&pair[1], "top")?,
            "horizontalalignment" => {
                target.horizontal = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                    "left" => TextAlign::Left,
                    "center" => TextAlign::Center,
                    "right" => TextAlign::Right,
                    _ => return Err("Textbox alignment supports left, center or right".into()),
                }
            }
            "fontweight" => {
                target.bold = match text(&pair[1])?.to_ascii_lowercase().as_str() {
                    "normal" => false,
                    "bold" => true,
                    _ => return Err("Textbox FontWeight supports normal or bold".into()),
                }
            }
            "fontsize" => target.font_size = vector(&pair[1], 1)?[0],
            "color" => target.color = rgb(&pair[1])?,
            "visible" => target.visible = on_off(&pair[1])?,
            _ => return Err("Unsupported textbox property; no graph change was published".into()),
        }
    }
    if constructor && (!literal || !no_edge) {
        return Err("Textbox construction requires explicit Interpreter none and EdgeColor none; MATLAB TeX/default outline are unsupported".into());
    }
    target.validate()
}

fn head_style(value: &Value) -> Result<AnnotationHeadStyle> {
    match text(value)?.to_ascii_lowercase().as_str() {
        "vback2" => Ok(AnnotationHeadStyle::VBack2),
        "plain" => Ok(AnnotationHeadStyle::Plain),
        "none" => Ok(AnnotationHeadStyle::None),
        _ => Err("Annotation HeadStyle supports vback2, plain or none".into()),
    }
}

fn annotation_properties(target: &mut Annotation, args: &[Value]) -> Result<()> {
    for pair in pairs(args)? {
        match text(&pair[0])?.to_ascii_lowercase().as_str() {
            "x" => {
                let x: [f64; 2] = vector(&pair[1], 2)?.try_into().unwrap();
                target.set_x_endpoints(x);
            }
            "y" => {
                let y: [f64; 2] = vector(&pair[1], 2)?.try_into().unwrap();
                target.set_y_endpoints(y);
            }
            "position" => target.position = vector(&pair[1], 4)?.try_into().unwrap(),
            "color" => {
                target.color = super::super::plot_properties::color(&pair[1])?;
                target.edge_none = false;
            }
            "edgecolor" => match edge_color(&pair[1])? {
                Some(c) => {
                    target.color = c;
                    target.edge_none = false;
                }
                None => target.edge_none = true,
            },
            "linewidth" => target.line_width = line_width(&pair[1])?,
            "linestyle" => target.pattern = line_pattern(&pair[1])?,
            "headstyle" => target.head_style = head_style(&pair[1])?,
            "headlength" => target.head_length = vector(&pair[1], 1)?[0],
            "headwidth" => target.head_width = vector(&pair[1], 1)?[0],
            "string" => {
                let value = match &pair[1] {
                    Value::QuotedString(s) => s.as_str(),
                    other => text(other)?,
                };
                if value.len() > 4096 {
                    return Err("Annotation text exceeds 4096 bytes".into());
                }
                target.text = value.into();
            }
            "fontsize" => target.font_size = vector(&pair[1], 1)?[0],
            "facecolor" => target.face_color = face_color(&pair[1])?,
            "facealpha" => target.face_alpha = alpha_value(&pair[1], "FaceAlpha")?,
            "units" => literal_only(&pair[1], "normalized")?,
            "visible" => target.visible = on_off(&pair[1])?,
            _ => {
                return Err(
                    "Unsupported annotation property; no graph change was published".into(),
                );
            }
        }
    }
    target.validate()
}

impl Engine {
    pub(super) fn create_patch_object(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (mut target, args) =
            if let Some(value @ Value::Container(crate::Array::Graphics(_))) = args.first() {
                let h = handle(value)?;
                if h.kind() != GraphicsKind::Axes {
                    return Err("Leading patch target must be an axes handle".into());
                }
                (Some(self.locate_graphics(h)?), &args[1..])
            } else {
                (None, args)
            };
        let properties;
        let args = if let Some(Value::Container(Array::Struct(fields))) = args.first() {
            if fields.len() != 1 || fields.field_count() > 16 {
                return Err(
                    "Patch requires a scalar geometry structure with up to 16 properties".into(),
                );
            }
            let mut values = Vec::with_capacity(2 * fields.field_count() + args.len() - 1);
            for (i, name) in fields.field_names().iter().enumerate() {
                values.extend([
                    Value::Text(name.clone()),
                    fields.get(i, 0)?.as_ref().clone().into(),
                ]);
            }
            values.extend_from_slice(&args[1..]);
            properties = values;
            properties.as_slice()
        } else {
            args
        };
        let mut x: Option<Matrix> = None;
        let mut y: Option<Matrix> = None;
        let mut z: Option<Matrix> = None;
        let mut faces = None;
        let mut vertices = None;
        let mut vertex_colors = None;
        let mut color = None;
        let mut edge: Option<Option<[u8; 3]>> = None;
        let mut face_alpha = None;
        let mut edge_alpha = None;
        let mut pattern = None;
        let mut width = None;
        let mut label = None;
        let mut visible = None;
        let pair_args = if args.is_empty() {
            return Err("Patch requires XData/YData or positional arguments".into());
        } else if matches!(args[0], Value::Text(_)) {
            args
        } else {
            let Value::Matrix(xm) = &args[0] else {
                return Err("Patch XData requires a real numeric matrix".into());
            };
            if args.len() < 3 {
                return Err("Patch requires X, Y, and color arguments".into());
            }
            let Value::Matrix(ym) = &args[1] else {
                return Err("Patch YData requires a real numeric matrix".into());
            };
            x = Some(normalize_patch_matrix(xm)?);
            y = Some(normalize_patch_matrix(ym)?);
            let (c_val, rest) = if args.len() >= 4
                && matches!(args[2], Value::Matrix(_))
                && (args.len() - 4).is_multiple_of(2)
            {
                let Value::Matrix(zm) = &args[2] else {
                    unreachable!();
                };
                z = Some(normalize_patch_matrix(zm)?);
                (&args[3], &args[4..])
            } else {
                (&args[2], &args[3..])
            };
            color = Some(c_val);
            edge = Some(Some([0, 0, 0]));
            rest
        };
        for pair in pairs(pair_args)? {
            match text(&pair[0])?.to_ascii_lowercase().as_str() {
                "parent" => {
                    if target.is_some() {
                        return Err(
                            "Specify one patch axes target, not both a leading target and Parent"
                                .into(),
                        );
                    }
                    let h = handle(&pair[1])?;
                    if h.kind() != GraphicsKind::Axes {
                        return Err("Patch Parent must be a live axes".into());
                    }
                    target = Some(self.locate_graphics(h)?);
                }
                "xdata" => {
                    let Value::Matrix(a) = &pair[1] else {
                        return Err("Patch XData requires a real numeric matrix".into());
                    };
                    x = Some(normalize_patch_matrix(a)?);
                }
                "ydata" => {
                    let Value::Matrix(a) = &pair[1] else {
                        return Err("Patch YData requires a real numeric matrix".into());
                    };
                    y = Some(normalize_patch_matrix(a)?);
                }
                "zdata" => {
                    let Value::Matrix(a) = &pair[1] else {
                        return Err("Patch ZData requires a real numeric matrix".into());
                    };
                    z = Some(normalize_patch_matrix(a)?);
                }
                "faces" => faces = Some(patch_matrix(&pair[1], "Faces")?),
                "vertices" => vertices = Some(patch_matrix(&pair[1], "Vertices")?),
                "facevertexcdata" => {
                    vertex_colors = Some(patch_matrix(&pair[1], "FaceVertexCData")?)
                }
                "facecolor" => color = Some(&pair[1]),
                "edgecolor" => edge = Some(edge_color(&pair[1])?),
                "facealpha" => face_alpha = Some(alpha_value(&pair[1], "FaceAlpha")?),
                "edgealpha" => edge_alpha = Some(alpha_value(&pair[1], "EdgeAlpha")?),
                "linestyle" => pattern = Some(line_pattern(&pair[1])?),
                "linewidth" => width = Some(line_width(&pair[1])?),
                "displayname" => {
                    let value = text(&pair[1])?;
                    if value.len() > 4096 {
                        return Err("Patch label exceeds 4096 bytes".into());
                    }
                    label = Some(value.to_owned());
                }
                "visible" => visible = Some(on_off(&pair[1])?),
                _ => return Err("Unsupported patch property".into()),
            }
        }
        let indexed = faces.is_some() || vertices.is_some() || vertex_colors.is_some();
        if indexed && (x.is_some() || y.is_some() || z.is_some()) {
            return Err("Patch accepts either XData/YData or Faces/Vertices".into());
        }
        let id = target.map_or(self.current_figure, |t| t.figure);
        // graphics_figure also resolves uiaxes backing figures.
        let existing = self.graphics_figure(id).ok().cloned();
        let mut figure = match existing {
            Some(f) => f,
            None => {
                let mut f = crate::plot::Figure::default();
                self.apply_figure_defaults(&mut f)?;
                self.apply_axes_defaults(&mut f)?;
                f
            }
        };
        figure.validate()?;
        let previous = figure.axes.identity.clone();
        if let Some(target) = target {
            super::select_live_axes(&mut figure, target.position.unwrap())?;
        }
        let ledger = figure.geometry_ledger()?;
        let explicit_face = color.is_some();
        let explicit_edge = edge.is_some();
        let (mut series, edge, rgb) = if indexed {
            let faces = faces.ok_or("Indexed patch requires Faces")?;
            let vertices = vertices.ok_or("Indexed patch requires Vertices")?;
            let vertex_colors = vertex_colors.filter(|m| !m.is_empty());
            let color = match color {
                Some(value) => indexed_color(value)?,
                None => IndexedColor::Rgb([0; 3]),
            };
            let rgb = match color {
                IndexedColor::Rgb(rgb) => rgb,
                _ => [0; 3],
            };
            let series = indexed_series(
                self,
                IndexedPatch {
                    faces: faces.clone().without_coordinate_proof(),
                    vertices: vertices.clone().without_coordinate_proof(),
                    colors: vertex_colors.cloned().map(Matrix::without_coordinate_proof),
                    normals: None,
                    face_color: color,
                    color_range: None,
                    mapped_with: None,
                },
                ledger,
            )?;
            (series, edge.unwrap_or(Some([0; 3])), rgb)
        } else {
            let edge = edge.ok_or("Patch construction requires explicit RGB/none EdgeColor")?;
            let color =
                face_color(color.ok_or("Patch construction requires explicit RGB FaceColor")?)?;
            let mut x = x.ok_or("Patch requires XData")?;
            let mut y = y.ok_or("Patch requires YData")?;
            if let Some(ref z) = z
                && (z.rows() != x.rows() || z.cols() != x.cols())
            {
                return Err("Patch ZData must match XData and YData shape".into());
            }
            // A five-entry vector may explicitly repeat the first corner of a quad.
            // Drop the endpoint only when every supplied coordinate closes.
            let closed = |m: &Matrix| m.rows() == 5 && m.cols() == 1 && m.data()[0] == m.data()[4];
            if closed(&x) && closed(&y) && z.as_ref().is_none_or(closed) {
                let corners = |m: &Matrix| Matrix::from_column_slice(4, 1, &m.data()[..4]);
                x = corners(&x)?;
                y = corners(&y)?;
                z = z.as_ref().map(corners).transpose()?;
            }
            // The 10,000-face limit applies to low-level 2-D patch objects.
            let mut object_faces = 0usize;
            for (_, axes) in figure.all_axes() {
                for s in &axes.series {
                    if s.kind == crate::plot::PlotKind::Patch
                        && !s.is_3d()
                        && s.face_colors.is_none()
                        && !crate::engine::plots_2d_more::p2_contour_series(self, &s.identity)
                    {
                        object_faces =
                            object_faces.saturating_add(s.geometry_cost()?.general_faces());
                    }
                }
            }
            // Retained/expanded resources are checked before any dense pair copy.
            let mut series = Series::patch_from_matrices_with_ledger(
                &x,
                &y,
                color.unwrap_or([0; 3]),
                &self.cancel,
                ledger,
                object_faces,
            )?;
            if let Some(z) = z {
                if z.data().iter().any(|v| !v.is_finite()) {
                    return Err("Patch ZData must be finite".into());
                }
                series.z = z.data().to_vec().into();
            }
            series.face_color_none = color.is_none();
            (series, edge, color.unwrap_or([0; 3]))
        };
        series.pattern = if edge.is_some() {
            LinePattern::Solid
        } else {
            LinePattern::None
        };
        series.color = rgb;
        series.face_edge_color = edge;
        self.apply_patch_defaults(&mut series, &figure.identity)?;
        if explicit_face {
            series.color = rgb;
        }
        if explicit_edge {
            series.face_edge_color = edge;
        }
        series.explicit_color = true;
        if face_alpha.is_some() {
            series.face_alpha = face_alpha;
        }
        series.edge_alpha = edge_alpha;
        if let Some(p) = pattern {
            series.pattern = p;
        }
        if let Some(w) = width {
            series.line_width = Some(w);
        }
        if let Some(ref label) = label {
            series.label = label.clone();
        }
        if let Some(visible) = visible {
            series.visible = visible;
        }
        let effective_label = series.label.clone();
        let token =
            GraphicsHandle::series(&figure.identity, &figure.axes.identity, &series.identity);
        figure.add(vec![series])?;
        // Low-level patch's empty DisplayName is meaningful; the generic model
        // add helper assigns display defaults for ordinary high-level plots.
        figure.axes.series.last_mut().unwrap().label = effective_label;
        let previous_position = figure
            .all_axes()
            .find(|(_, a)| a.identity.same_object(&previous))
            .map(|(p, _)| p);
        if let Some(position) = previous_position {
            super::select_live_axes(&mut figure, position)?;
        }
        let result = GraphicsArray::scalar(token, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.gobj_publish(id, figure)?;
        Ok(Value::Container(result.into()))
    }

    pub(super) fn create_textbox_object(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (id, args) =
            if let Some(value @ Value::Container(crate::Array::Graphics(_))) = args.first() {
                let h = handle(value)?;
                if h.kind() != GraphicsKind::Figure {
                    return Err("Annotation parent must be a figure handle".into());
                }
                (self.locate_graphics(h)?.figure, &args[1..])
            } else {
                (self.current_figure, args)
            };
        if args.len() < 2 || !text(&args[0])?.eq_ignore_ascii_case("textbox") {
            return Err("Use annotation([figure,] 'textbox', position, properties)".into());
        }
        let mut textbox = TextBox::new(vector(&args[1], 4)?.try_into().unwrap(), String::new())?;
        textbox_properties(&mut textbox, &args[2..], true)?;
        let mut figure = self
            .figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f.clone())
            .unwrap_or_default();
        let token = GraphicsHandle::textbox(&figure.identity, &textbox.identity);
        figure.add_textbox(textbox)?;
        figure.validate()?;
        let result = GraphicsArray::scalar(token, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(Value::Container(result.into()))
    }

    pub(super) fn get_textbox_property(
        &self,
        id: u32,
        index: usize,
        args: &[Value],
    ) -> Result<Value> {
        if args.len() != 1 {
            return Err("get requires one explicit textbox property".into());
        }
        let figure = self.graphics_figure(id)?;
        let target = figure
            .textboxes
            .get(index)
            .ok_or("Textbox no longer exists")?;
        let string = |s: &str| Ok(Value::Text(s.into()));
        match text(&args[0])?.to_ascii_lowercase().as_str() {
            "string" => string(&target.text),
            "parent" => Ok(Value::Container(
                GraphicsArray::scalar(GraphicsHandle::figure(&figure.identity), &self.cancel)?
                    .into(),
            )),
            "position" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                4,
                &target.position,
            )?)),
            "fontsize" => Ok(Value::Matrix(Matrix::scalar(target.font_size))),
            "fontweight" => string(if target.bold { "bold" } else { "normal" }),
            "horizontalalignment" => string(match target.horizontal {
                TextAlign::Left => "left",
                TextAlign::Center => "center",
                TextAlign::Right => "right",
            }),
            "verticalalignment" => string("top"),
            "units" => string("normalized"),
            "interpreter" | "edgecolor" | "backgroundcolor" => string("none"),
            "fitboxtotext" => string("off"),
            "visible" => string(if target.visible { "on" } else { "off" }),
            "color" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                3,
                &target.color.map(|c| f64::from(c) / 255.),
            )?)),
            _ => Err("Unsupported textbox property query".into()),
        }
    }
    pub(super) fn set_textbox_properties(
        &mut self,
        id: u32,
        index: usize,
        args: &[Value],
    ) -> Result<()> {
        if args.is_empty() {
            return Err("set requires textbox property/value pairs".into());
        }
        let mut figure = self.graphics_figure(id)?.clone();
        textbox_properties(
            figure
                .textboxes
                .get_mut(index)
                .ok_or("Textbox no longer exists")?,
            args,
            false,
        )?;
        figure.validate()?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(())
    }

    pub(super) fn create_annotation_object(&mut self, args: &[Value]) -> Result<Value> {
        self.check_cancel()?;
        let (id, rest) =
            if let Some(value @ Value::Container(crate::Array::Graphics(_))) = args.first() {
                let h = handle(value)?;
                if h.kind() != GraphicsKind::Figure {
                    return Err("Annotation parent must be a figure handle".into());
                }
                (self.locate_graphics(h)?.figure, &args[1..])
            } else {
                (self.current_figure, args)
            };
        if rest.is_empty() {
            return Err("Use annotation([figure,] type, coords, properties)".into());
        }
        let kind_name = text(&rest[0])?.to_ascii_lowercase();
        if kind_name == "textbox" {
            return self.create_textbox_object(args);
        }
        let kind = match kind_name.as_str() {
            "line" => AnnotationKind::Line,
            "arrow" => AnnotationKind::Arrow,
            "doublearrow" => AnnotationKind::DoubleArrow,
            "textarrow" => AnnotationKind::TextArrow,
            "rectangle" => AnnotationKind::Rectangle,
            "ellipse" => AnnotationKind::Ellipse,
            _ => {
                return Err(
                    "annotation supports textbox, line, arrow, doublearrow, textarrow, rectangle and ellipse"
                        .into(),
                );
            }
        };
        let (mut ann, prop_args) = if kind.is_box() {
            if rest.len() < 2 {
                return Err("Box annotation requires a [x y w h] position vector".into());
            }
            let pos: [f64; 4] = vector(&rest[1], 4)?.try_into().unwrap();
            (Annotation::new_box(kind, pos)?, &rest[2..])
        } else {
            if rest.len() < 3 {
                return Err("Line annotation requires X and Y 2-element vectors".into());
            }
            let x: [f64; 2] = vector(&rest[1], 2)?.try_into().unwrap();
            let y: [f64; 2] = vector(&rest[2], 2)?.try_into().unwrap();
            (Annotation::new_line(kind, x, y)?, &rest[3..])
        };
        annotation_properties(&mut ann, prop_args)?;
        let mut figure = self
            .figures()
            .find(|(n, _)| *n == id)
            .map(|(_, f)| f.clone())
            .unwrap_or_default();
        let token = GraphicsHandle::annotation(&figure.identity, &ann.identity);
        figure.add_annotation(ann)?;
        figure.validate()?;
        let result = GraphicsArray::scalar(token, &self.cancel)?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(Value::Container(result.into()))
    }

    pub(super) fn get_annotation_property(
        &self,
        id: u32,
        index: usize,
        args: &[Value],
    ) -> Result<Value> {
        if args.len() != 1 {
            return Err("get requires one explicit annotation property".into());
        }
        let figure = self.graphics_figure(id)?;
        let target = figure
            .annotations
            .get(index)
            .ok_or("Annotation no longer exists")?;
        let string = |s: &str| Ok(Value::Text(s.into()));
        match text(&args[0])?.to_ascii_lowercase().as_str() {
            "type" => string(target.kind.type_name()),
            "parent" => Ok(Value::Container(
                GraphicsArray::scalar(GraphicsHandle::figure(&figure.identity), &self.cancel)?
                    .into(),
            )),
            "x" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                2,
                &target.x_endpoints(),
            )?)),
            "y" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                2,
                &target.y_endpoints(),
            )?)),
            "position" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                4,
                &target.position,
            )?)),
            "color" => Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                3,
                &target.color.map(|c| f64::from(c) / 255.),
            )?)),
            "edgecolor" => {
                if target.edge_none {
                    string("none")
                } else {
                    Ok(Value::Matrix(Matrix::from_column_slice(
                        1,
                        3,
                        &target.color.map(|c| f64::from(c) / 255.),
                    )?))
                }
            }
            "linewidth" => Ok(Value::Matrix(Matrix::scalar(target.line_width))),
            "linestyle" => string(match target.pattern {
                LinePattern::Solid => "-",
                LinePattern::Dashed => "--",
                LinePattern::Dotted => ":",
                LinePattern::DashDot => "-.",
                LinePattern::None => "none",
            }),
            "headstyle" => string(target.head_style.name()),
            "headlength" => Ok(Value::Matrix(Matrix::scalar(target.head_length))),
            "headwidth" => Ok(Value::Matrix(Matrix::scalar(target.head_width))),
            "string" => string(&target.text),
            "fontsize" => Ok(Value::Matrix(Matrix::scalar(target.font_size))),
            "facecolor" => match target.face_color {
                Some(c) => Ok(Value::Matrix(Matrix::from_column_slice(
                    1,
                    3,
                    &c.map(|v| f64::from(v) / 255.),
                )?)),
                None => string("none"),
            },
            "facealpha" => Ok(Value::Matrix(Matrix::scalar(target.face_alpha))),
            "units" => string("normalized"),
            "visible" => string(if target.visible { "on" } else { "off" }),
            _ => Err("Unsupported annotation property query".into()),
        }
    }

    pub(super) fn set_annotation_properties(
        &mut self,
        id: u32,
        index: usize,
        args: &[Value],
    ) -> Result<()> {
        if args.is_empty() {
            return Err("set requires annotation property/value pairs".into());
        }
        let mut figure = self.graphics_figure(id)?.clone();
        annotation_properties(
            figure
                .annotations
                .get_mut(index)
                .ok_or("Annotation no longer exists")?,
            args,
        )?;
        figure.validate()?;
        #[cfg(test)]
        super::tests::before_property_publish(&self.cancel);
        self.check_cancel()?;
        self.publish_graphics_figure(id, figure);
        Ok(())
    }
}
