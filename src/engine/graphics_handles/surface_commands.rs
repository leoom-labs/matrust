//! Grid surface plots, shading, and the surface object's properties.
//!
//! A surface is ONE Patch series of quads with z per vertex and a per-face
//! fill (and, for EdgeColor 'flat'/'interp', edge) colour list. The engine
//! keeps a rebuildable [`SurfaceRecord`] per surface (X, Y, Z, C and the
//! colour/line style), keyed by the series identity, so `set(h, 'CData', ..)`,
//! `shading`, `colormap`, `clim`/`caxis` and the axes background colour all
//! rebuild the series through [`build_series`].
//!
//! `surf` and `mesh` are high-level: with hold off they replace the axes and
//! switch to view(3). `surface` is the low-level call on the same grid. It
//! inserts into the current axes and leaves the existing series and view
//! alone. FaceLighting is stored and returned; the renderer does not simulate
//! light objects, so the value does not change face colours.
//!
//! Colours (MATLAB documentation, CDataMapping 'scaled'): FaceColor 'flat'
//! takes the CData of the face's first vertex (row i, column j of the face
//! between rows i..i+1 and columns j..j+1) through the axes colormap and CLim
//! ([`crate::plot::colormap::scaled_rgb`], shared with contour). 'interp' is
//! approximated by the mean of the four vertices' mapped colours (no per-vertex
//! gradients). While the axes CLim is automatic it is [min max] of the finite
//! CData of every surface in the axes; a CLim set by the user is respected.
//! A face with a non-finite X, Y, Z or C at any of its four vertices is a hole.
use super::{Engine, Execution, Value, handle};
use crate::plot::{
    Axes, Figure, LinePattern, MAX_PATCH_FACES, PatchFaces, PlotKind, Series, colormap, view3d,
};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix, ObjectIdentity, Result};
use std::sync::Arc;

/// Builtins of this module.
pub(in crate::engine) const SURFACE_NAMES: &[&str] = &[
    "surf",
    "surface",
    "mesh",
    "pcolor",
    "sphere",
    "surfl",
    "trisurf",
    "trimesh",
    "tetramesh",
    "shading",
    "image",
    "imagesc",
    "imshow",
    "slice",
    "contourslice",
    "volshow",
];
/// Z elements one surface may hold.
const MAX_SURFACE_ELEMENTS: usize = 1_000_000;
/// Surface records kept; dead ones are pruned on every surf/mesh.
const MAX_RECORDS: usize = 1024;

pub(in crate::engine) fn surface_names(name: &str) -> bool {
    SURFACE_NAMES.contains(&name)
}

/// FaceColor / EdgeColor of a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Coloring {
    Flat,
    Interp,
    None,
    Rgb([u8; 3]),
    /// The axes background colour (mesh faces, MATLAB `hidden on`).
    Background,
}

/// Stored FaceLighting. None of these values change the drawn colours.
#[derive(Clone, Copy, Debug, PartialEq)]
enum FaceLighting {
    None,
    Flat,
    Gouraud,
    Phong,
}

/// Colormap, CLim and background colour a surface's colours were built with.
type BuiltWith = (Arc<Vec<[f64; 3]>>, [f64; 2], [u8; 3]);

#[derive(Clone)]
struct SurfaceRecord {
    primary: ObjectIdentity,
    mesh: bool,
    pcolor: bool,
    x: Matrix,
    y: Matrix,
    z: Matrix,
    c: Matrix,
    face: Coloring,
    edge: Coloring,
    /// Returned by get. Not applied when the faces are built.
    lighting: FaceLighting,
    face_alpha: f64,
    pattern: LinePattern,
    line_width: f64,
    /// The automatic CLim last stored on the axes by the surfaces (None after
    /// the user set one). The axes CLim counts as automatic while it equals it.
    auto_clim: Option<[f64; 2]>,
    /// Colormap, CLim and background the current colours were built with.
    built: Option<BuiltWith>,
}

/// Surface records of the engine.
#[derive(Clone, Default)]
pub(crate) struct SurfaceState {
    records: Vec<SurfaceRecord>,
}

impl SurfaceState {
    /// Copy only records belonging to the figures in a bulk-set transaction.
    pub(super) fn for_figures(&self, figures: &[&Figure]) -> Self {
        Self {
            records: self
                .records
                .iter()
                .filter(|record| {
                    figures.iter().any(|figure| {
                        figure.all_axes().any(|(_, axes)| {
                            axes.series
                                .iter()
                                .any(|series| series.identity.same_object(&record.primary))
                        })
                    })
                })
                .cloned()
                .collect(),
        }
    }

    /// Replace the records prepared for those figures, preserving all others.
    pub(super) fn publish_from(&mut self, staged: Self) {
        for record in staged.records {
            if let Some(existing) = self
                .records
                .iter_mut()
                .find(|old| old.primary.same_object(&record.primary))
            {
                *existing = record;
            }
        }
    }
}

// ------------------------------------------------------------- arguments

fn text_of(value: &Value) -> Option<&str> {
    match value {
        Value::Text(s) => Some(s),
        _ => None,
    }
}

fn numeric(e: &Engine, value: &Value, who: &str, what: &str) -> Result<Matrix> {
    match value {
        Value::Matrix(m) => Ok(m.clone()),
        Value::Logical(m) => m.to_numeric(&e.cancel),
        _ => Err(format!("{who}: {what} must be a real numeric array")),
    }
}

fn parse_rgb(value: &Value, what: &str) -> Result<[u8; 3]> {
    if let Some(s) = text_of(value) {
        return Ok(match s.to_ascii_lowercase().as_str() {
            "r" | "red" => [255, 0, 0],
            "g" | "green" => [0, 255, 0],
            "b" | "blue" => [0, 0, 255],
            "c" | "cyan" => [0, 255, 255],
            "m" | "magenta" => [255, 0, 255],
            "y" | "yellow" => [255, 255, 0],
            "k" | "black" => [0, 0, 0],
            "w" | "white" => [255, 255, 255],
            _ => {
                return Err(format!(
                    "{what} must be 'flat', 'interp', 'none', a colour name or an RGB triplet"
                ));
            }
        });
    }
    let Value::Matrix(m) = value else {
        return Err(format!("{what} must be a colour name or an RGB triplet"));
    };
    let v = m.data();
    if v.len() != 3 || v.iter().any(|c| !c.is_finite() || !(0. ..=1.).contains(c)) {
        return Err(format!("{what} RGB triplet must hold three values in 0..1"));
    }
    Ok([0, 1, 2].map(|k| (v[k] * 255.).round() as u8))
}

fn parse_coloring(value: &Value, what: &str) -> Result<Coloring> {
    match text_of(value).map(str::to_ascii_lowercase).as_deref() {
        Some("flat") => Ok(Coloring::Flat),
        Some("interp") => Ok(Coloring::Interp),
        Some("none") => Ok(Coloring::None),
        Some("texturemap") => Err(format!("{what} 'texturemap' is not supported yet")),
        _ => parse_rgb(value, what).map(Coloring::Rgb),
    }
}

fn parse_alpha(value: &Value) -> Result<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 && (0. ..=1.).contains(&m.data()[0]) => Ok(m.data()[0]),
        Value::Text(_) => {
            Err("FaceAlpha 'flat' and 'interp' are not supported yet; use a scalar in 0..1".into())
        }
        _ => Err("FaceAlpha must be a scalar in 0..1".into()),
    }
}

fn parse_style(value: &Value) -> Result<LinePattern> {
    Ok(match text_of(value) {
        Some("-") => LinePattern::Solid,
        Some("--") => LinePattern::Dashed,
        Some(":") => LinePattern::Dotted,
        Some("-.") => LinePattern::DashDot,
        Some(s) if s.eq_ignore_ascii_case("none") => LinePattern::None,
        _ => return Err("LineStyle must be '-', '--', ':', '-.' or 'none'".into()),
    })
}

const LIGHTING_VALUES: &str = "FaceLighting must be 'none', 'flat', 'gouraud' or 'phong'";

fn parse_lighting(value: &Value) -> Result<FaceLighting> {
    let Some(s) = text_of(value) else {
        return Err(LIGHTING_VALUES.into());
    };
    match s.to_ascii_lowercase().as_str() {
        "none" => Ok(FaceLighting::None),
        "flat" => Ok(FaceLighting::Flat),
        "gouraud" => Ok(FaceLighting::Gouraud),
        "phong" => Ok(FaceLighting::Phong),
        _ => Err(LIGHTING_VALUES.into()),
    }
}

fn lighting_name(lighting: FaceLighting) -> &'static str {
    match lighting {
        FaceLighting::None => "none",
        FaceLighting::Flat => "flat",
        FaceLighting::Gouraud => "gouraud",
        FaceLighting::Phong => "phong",
    }
}

fn parse_width(value: &Value) -> Result<f64> {
    match value {
        Value::Matrix(m) if m.len() == 1 && m.data()[0].is_finite() && m.data()[0] > 0. => {
            crate::plot::validate_line_width(m.data()[0])?;
            Ok(m.data()[0])
        }
        _ => Err("LineWidth must be a positive scalar".into()),
    }
}

fn vector_like(m: &Matrix) -> bool {
    m.rows() == 1 || m.cols() == 1
}

/// Check the X, Y, Z, C sizes against Z (X: vector of Z's columns or Z's size;
/// Y: vector of Z's rows or Z's size; C: Z's size).
fn check_data(who: &str, x: &Matrix, y: &Matrix, z: &Matrix, c: &Matrix) -> Result<()> {
    let (nr, nc) = (z.rows(), z.cols());
    if z.len() > MAX_SURFACE_ELEMENTS {
        return Err(format!("{who}: Z exceeds {MAX_SURFACE_ELEMENTS} elements"));
    }
    if nr < 2 || nc < 2 {
        return Err(format!("{who}: Z must be a matrix of at least 2-by-2"));
    }
    let same = |m: &Matrix| m.rows() == nr && m.cols() == nc;
    if !(same(x) || (vector_like(x) && x.len() == nc)) {
        return Err(format!(
            "{who}: X must be a vector with one value per column of Z, or a matrix of Z's size"
        ));
    }
    if !(same(y) || (vector_like(y) && y.len() == nr)) {
        return Err(format!(
            "{who}: Y must be a vector with one value per row of Z, or a matrix of Z's size"
        ));
    }
    if !same(c) {
        return Err(format!("{who}: C must be the same size as Z"));
    }
    Ok(())
}

/// Faces of a grid: (rows - 1) (columns - 1).
fn grid_faces(z: &Matrix) -> usize {
    z.rows().saturating_sub(1) * z.cols().saturating_sub(1)
}

fn row_vector(values: Vec<f64>) -> Result<Matrix> {
    let n = values.len();
    Matrix::from_column_vec(1, n, values)
}

struct Parsed {
    rec: SurfaceRecord,
    label: Option<String>,
    visible: bool,
}

pub(in crate::engine) fn sphere_coordinates(
    e: &Engine,
    args: &[Value],
    count: usize,
) -> Result<Vec<Value>> {
    if count > 3 {
        return Err("sphere supports up to three outputs".into());
    }
    let n = match args {
        [] => 20.,
        [n] => numeric(e, n, "sphere", "n")?.as_scalar()?,
        _ => return Err("Use sphere or sphere(n)".into()),
    };
    if !(1. ..=999.).contains(&n) || n.fract() != 0. {
        return Err("sphere: n must be an integer from 1 to 999".into());
    }
    let n = n as usize;
    if count == 0 && n * n > MAX_PATCH_FACES {
        return Err(format!(
            "sphere: a figure holds at most {MAX_PATCH_FACES} patch faces"
        ));
    }
    let count = if count == 0 { 3 } else { count };
    let size = n + 1;
    e.preflight_elements(size * size * count)?;
    let mut latitude = Vec::with_capacity(size);
    latitude.push((0., -1.));
    for i in 1..n {
        let (sin, cos) = (std::f64::consts::PI * (i as f64 / n as f64 - 0.5)).sin_cos();
        latitude.push((cos, sin));
    }
    latitude.push((0., 1.));

    let mut coordinates: Vec<Vec<f64>> = (0..count)
        .map(|_| Vec::with_capacity(size * size))
        .collect();
    for j in 0..=n {
        e.check_cancel()?;
        // Include the same meridian at both sides of the seam.
        let (sin, cos) = (std::f64::consts::TAU * ((j % n) as f64 / n as f64)).sin_cos();
        for (values, scale) in coordinates.iter_mut().take(2).zip([cos, sin]) {
            values.extend(latitude.iter().map(|&(r, _)| r * scale));
        }
        if let Some(z) = coordinates.get_mut(2) {
            z.extend(latitude.iter().map(|&(_, height)| height));
        }
    }
    coordinates
        .into_iter()
        .map(|data| Ok(Value::Matrix(Matrix::from_column_vec(size, size, data)?)))
        .collect()
}

fn grid_coordinate(
    matrix: &Matrix,
    nr: usize,
    nc: usize,
    row: usize,
    col: usize,
    vector_index: usize,
) -> f64 {
    if matrix.rows() == nr && matrix.cols() == nc {
        matrix.get(row, col)
    } else {
        matrix.data()[vector_index]
    }
}

fn lit_colors(e: &Engine, x: &Matrix, y: &Matrix, z: &Matrix) -> Result<Matrix> {
    let (nr, nc) = (z.rows(), z.cols());
    e.preflight_elements(z.len())?;
    let point = |r, c| {
        [
            grid_coordinate(x, nr, nc, r, c, c),
            grid_coordinate(y, nr, nc, r, c, r),
            z.get(r, c),
        ]
    };
    let direction = |from: [f64; 3], to: [f64; 3]| {
        let d: [f64; 3] = std::array::from_fn(|k| to[k] - from[k]);
        let length = d[0].hypot(d[1]).hypot(d[2]);
        d.map(|v| v / length)
    };
    // Unit light direction (1, -1, 2) / sqrt(6).
    let light_scale = 6_f64.sqrt().recip();
    let mut colors = Vec::with_capacity(z.len());
    for c in 0..nc {
        for r in 0..nr {
            if (c * nr + r).is_multiple_of(8192) {
                e.check_cancel()?;
            }
            let u = direction(point(r, c.saturating_sub(1)), point(r, (c + 1).min(nc - 1)));
            let v = direction(point(r.saturating_sub(1), c), point((r + 1).min(nr - 1), c));
            let [nx, ny, nz] = std::array::from_fn(|i| {
                u[(i + 1) % 3] * v[(i + 2) % 3] - u[(i + 2) % 3] * v[(i + 1) % 3]
            });
            let cosine = (nx - ny + 2. * nz) * light_scale / nx.hypot(ny).hypot(nz);
            let diffuse = if cosine.is_finite() {
                cosine.clamp(0., 1.)
            } else {
                0.
            };
            colors.push(0.2 + 0.8 * diffuse);
        }
    }
    Matrix::from_column_vec(nr, nc, colors)
}

fn parse(e: &Engine, args: &[Value], who: &str) -> Result<Parsed> {
    let mesh = who == "mesh";
    let pcolor = who == "pcolor";
    let split = args
        .iter()
        .position(|a| matches!(a, Value::Text(_)))
        .unwrap_or(args.len());
    let (data, rest) = args.split_at(split);
    if matches!(who, "pcolor" | "surfl") && !matches!(data.len(), 1 | 3) {
        let forms = if pcolor {
            "pcolor(C) or pcolor(X,Y,C)"
        } else {
            "surfl(Z) or surfl(X,Y,Z)"
        };
        return Err(format!("Use {forms}"));
    }
    if data.len() > 4 {
        return Err(format!("{who}: too many data arguments"));
    }
    let mats = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            numeric(
                e,
                v,
                who,
                ["X", "Y", "Z", "C"].get(i).copied().unwrap_or("data"),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let (x, y, z, c) = match mats.as_slice() {
        [z] => (None, None, z.clone(), None),
        [z, c] => (None, None, z.clone(), Some(c.clone())),
        [x, y, z] => (Some(x.clone()), Some(y.clone()), z.clone(), None),
        [x, y, z, c] => (Some(x.clone()), Some(y.clone()), z.clone(), Some(c.clone())),
        [] => return Err(format!("{who} requires input data")),
        _ => {
            return Err(format!(
                "Use {who}(Z), {who}(Z,C), {who}(X,Y,Z), {who}(X,Y,Z,C) and Name,Value pairs"
            ));
        }
    };
    if z.len() > MAX_SURFACE_ELEMENTS {
        return Err(format!("{who}: Z exceeds {MAX_SURFACE_ELEMENTS} elements"));
    }
    let x = match x {
        Some(x) => x,
        None => row_vector((1..=z.cols()).map(|v| v as f64).collect())?,
    };
    let y = match y {
        Some(y) => y,
        None => Matrix::from_column_vec(z.rows(), 1, (1..=z.rows()).map(|v| v as f64).collect())?,
    };
    let mut c = c.unwrap_or_else(|| z.clone());
    check_data(who, &x, &y, &z, &c)?;
    let mut z = z;
    if pcolor {
        e.preflight_elements(z.len())?;
        z = Matrix::filled(z.rows(), z.cols(), 0.)?;
    } else if who == "surfl" {
        c = lit_colors(e, &x, &y, &z)?;
    }
    let mut rec = SurfaceRecord {
        primary: ObjectIdentity::default(),
        mesh,
        pcolor,
        x,
        y,
        z,
        c,
        face: if mesh {
            Coloring::Background
        } else {
            Coloring::Flat
        },
        edge: if mesh {
            Coloring::Flat
        } else {
            Coloring::Rgb([0; 3])
        },
        lighting: FaceLighting::Flat,
        face_alpha: 1.,
        pattern: LinePattern::Solid,
        line_width: 0.5,
        auto_clim: None,
        built: None,
    };
    if let Some(fig) = e.figure.as_ref() {
        for (prop, val) in e.apply_surface_defaults(&fig.identity) {
            let _ = apply_property(e, &mut rec, &prop.to_ascii_lowercase(), &val)?;
        }
    } else {
        for (prop, val) in e.apply_surface_defaults(&ObjectIdentity::default()) {
            let _ = apply_property(e, &mut rec, &prop.to_ascii_lowercase(), &val)?;
        }
    }
    if !rest.len().is_multiple_of(2) {
        return Err(format!("{who}: properties must be Name,Value pairs"));
    }
    let mut label = None;
    let mut visible = true;
    for pair in rest.as_chunks::<2>().0 {
        let name = text_of(&pair[0])
            .ok_or_else(|| format!("{who}: property names must be text"))?
            .to_ascii_lowercase();
        match name.as_str() {
            "displayname" => {
                let s = text_of(&pair[1]).ok_or("DisplayName must be text")?;
                if s.len() > 4096 {
                    return Err("DisplayName exceeds 4096 bytes".into());
                }
                label = Some(s.to_owned());
            }
            "visible" => {
                visible = match text_of(&pair[1]).map(str::to_ascii_lowercase).as_deref() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err("Visible must be 'on' or 'off'".into()),
                }
            }
            _ => {
                if !apply_property(e, &mut rec, &name, &pair[1])? {
                    return Err(format!(
                        "{who}: unsupported property '{}'; supported: XData, YData, ZData, CData, FaceColor, EdgeColor, FaceAlpha, FaceLighting, LineStyle, LineWidth, DisplayName, Visible",
                        text_of(&pair[0]).unwrap_or_default()
                    ));
                }
            }
        }
    }
    check_data(who, &rec.x, &rec.y, &rec.z, &rec.c)?;
    Ok(Parsed {
        rec,
        label,
        visible,
    })
}

/// One record property; Ok(false) for a name this module does not own.
fn apply_property(e: &Engine, rec: &mut SurfaceRecord, name: &str, value: &Value) -> Result<bool> {
    match name {
        "xdata" => rec.x = numeric(e, value, "set", "XData")?,
        "ydata" => rec.y = numeric(e, value, "set", "YData")?,
        "zdata" => rec.z = numeric(e, value, "set", "ZData")?,
        "cdata" => rec.c = numeric(e, value, "set", "CData")?,
        "facecolor" => rec.face = parse_coloring(value, "FaceColor")?,
        "edgecolor" => rec.edge = parse_coloring(value, "EdgeColor")?,
        "facelighting" => rec.lighting = parse_lighting(value)?,
        "facealpha" => rec.face_alpha = parse_alpha(value)?,
        "linestyle" => rec.pattern = parse_style(value)?,
        "linewidth" => rec.line_width = parse_width(value)?,
        _ => return Ok(false),
    }
    Ok(true)
}

// ------------------------------------------------------------- building

/// Colours and limits a surface is built with.
struct Mapping<'a> {
    map: &'a [[f64; 3]],
    clim: [f64; 2],
    background: [u8; 3],
}

fn mean_rgb(colors: [[u8; 3]; 4]) -> [u8; 3] {
    std::array::from_fn(|k| {
        ((colors.iter().map(|c| u32::from(c[k])).sum::<u32>() as f64) / 4.).round() as u8
    })
}

/// The Patch series of a surface record: one quad per finite grid face, in
/// column-major face order (face (i, j) between rows i..i+1 and columns
/// j..j+1), vertices (i,j), (i+1,j), (i+1,j+1), (i,j+1).
fn build_series(
    rec: &SurfaceRecord,
    mapping: &Mapping<'_>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Series> {
    let (nr, nc) = (rec.z.rows(), rec.z.cols());
    let faces = grid_faces(&rec.z);
    if faces > MAX_PATCH_FACES {
        return Err(format!(
            "surf: the surface needs {faces} faces; a figure holds at most {MAX_PATCH_FACES} patch faces"
        ));
    }
    let x_at = |i, j| grid_coordinate(&rec.x, nr, nc, i, j, j);
    let y_at = |i, j| grid_coordinate(&rec.y, nr, nc, i, j, i);
    let mapped =
        |i: usize, j: usize| colormap::scaled_rgb(mapping.map, mapping.clim, rec.c.get(i, j));
    let mut points = Vec::new();
    points
        .try_reserve_exact(faces * 4)
        .map_err(|_| "Cannot allocate surface vertices")?;
    let mut z = Vec::with_capacity(faces * 4);
    let needs_flat = matches!(rec.face, Coloring::Flat | Coloring::Interp)
        || matches!(rec.edge, Coloring::Flat | Coloring::Interp);
    let needs_corner_colors =
        !rec.pcolor || matches!(rec.face, Coloring::Interp) || matches!(rec.edge, Coloring::Interp);
    let mut fill = Vec::new();
    let mut edge = Vec::new();
    for j in 0..nc - 1 {
        if j.is_multiple_of(64) && cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        for i in 0..nr - 1 {
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let finite = corners.iter().all(|&(r, c)| {
                x_at(r, c).is_finite()
                    && y_at(r, c).is_finite()
                    && rec.z.get(r, c).is_finite()
                    && (!needs_corner_colors || rec.c.get(r, c).is_finite())
            }) && (needs_corner_colors || !needs_flat || rec.c.get(i, j).is_finite());
            if !finite {
                continue;
            }
            for &(r, c) in &corners {
                points.push([x_at(r, c), y_at(r, c)]);
                z.push(rec.z.get(r, c));
            }
            if needs_flat {
                let flat = mapped(i, j);
                let interp = || mean_rgb(corners.map(|(r, c)| mapped(r, c)));
                match rec.face {
                    Coloring::Flat => fill.push(flat),
                    Coloring::Interp => fill.push(interp()),
                    _ => {}
                }
                match rec.edge {
                    Coloring::Flat => edge.push(flat),
                    Coloring::Interp => edge.push(interp()),
                    _ => {}
                }
            }
        }
    }
    let mut series = Series::new(points, PlotKind::Patch);
    series.patch = Some(PatchFaces::new(4)?);
    series.z = z.into();
    series.explicit_color = true;
    let solid = |coloring: Coloring| match coloring {
        Coloring::Rgb(rgb) => Some(rgb),
        Coloring::Background => Some(mapping.background),
        _ => None,
    };
    series.color = solid(rec.face)
        .or_else(|| fill.first().copied())
        .or_else(|| solid(rec.edge))
        .or_else(|| edge.first().copied())
        .unwrap_or([0; 3]);
    if matches!(rec.face, Coloring::Flat | Coloring::Interp) {
        series.face_colors = Some(fill.into());
    }
    series.face_alpha = match rec.face {
        Coloring::None => Some(0.),
        _ if rec.face_alpha < 1. => Some(rec.face_alpha),
        _ => None,
    };
    match rec.edge {
        Coloring::Flat | Coloring::Interp => {
            series.face_edge_color = Some(edge.first().copied().unwrap_or([0; 3]));
            series.face_edge_colors = Some(edge.into());
        }
        Coloring::None => series.face_edge_color = None,
        other => series.face_edge_color = solid(other),
    }
    series.pattern = if series.face_edge_color.is_some() {
        rec.pattern
    } else {
        LinePattern::None
    };
    series.line_width = Some(rec.line_width);
    Ok(series)
}

/// Lowest and highest finite CData of a record.
fn c_range(rec: &SurfaceRecord) -> Option<[f64; 2]> {
    let mut range: Option<[f64; 2]> = None;
    for &v in rec.c.data().iter().filter(|v| v.is_finite()) {
        range = Some(range.map_or([v, v], |r| [r[0].min(v), r[1].max(v)]));
    }
    range
}

fn position_in(axes: &Axes, id: &ObjectIdentity) -> Option<usize> {
    axes.series.iter().position(|s| s.identity.same_object(id))
}

pub(super) fn has_indexed_colors(figure: &Figure) -> bool {
    figure.all_axes().any(|(_, axes)| {
        axes.extras.indexed_clim.is_some()
            || axes.series.iter().any(|s| {
                s.indexed_patch
                    .as_ref()
                    .is_some_and(|p| p.face_color.mapped())
            })
    })
}

fn color_limits(range: Option<[f64; 2]>) -> [f64; 2] {
    match range {
        Some([a, b]) if a < b => [a, b],
        Some([a, _]) => [a - 1., a + 1.],
        None => [0., 1.],
    }
}

impl Engine {
    /// Rebuild the surfaces of every axes of `figure` whose colormap, CLim or
    /// background changed since their colours were built (or all of them with
    /// `force`). While an axes' CLim is automatic it becomes [min max] of the
    /// CData of its surfaces.
    pub(in crate::engine) fn surface_refresh(
        &mut self,
        figure: &mut Figure,
        force: bool,
    ) -> Result<()> {
        if self.surf.records.is_empty() && !has_indexed_colors(figure) {
            return Ok(());
        }
        let appearance = figure.resolve_appearance(self.graphics_theme_context);
        for axes in figure.all_axes_mut() {
            let members: Vec<usize> = self
                .surf
                .records
                .iter()
                .enumerate()
                .filter(|(_, r)| position_in(axes, &r.primary).is_some())
                .map(|(i, _)| i)
                .collect();
            let patch_range = axes
                .series
                .iter()
                .filter_map(|s| s.indexed_patch.as_ref())
                .filter(|p| p.face_color.mapped())
                .filter_map(|p| p.color_range)
                .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])]);
            axes.extras.indexed_clim = patch_range.map(|r| color_limits(Some(r)));
            if members.is_empty()
                && !axes.series.iter().any(|s| {
                    s.indexed_patch
                        .as_ref()
                        .is_some_and(|p| p.face_color.mapped())
                })
            {
                continue;
            }
            let auto = match axes.extras.clim {
                None => true,
                Some(c) => members
                    .iter()
                    .any(|&i| self.surf.records[i].auto_clim == Some(c)),
            };
            let clim = if auto && !members.is_empty() {
                let mut range = patch_range;
                for &i in &members {
                    if let Some(r) = c_range(&self.surf.records[i]) {
                        range = Some(range.map_or(r, |a| [a[0].min(r[0]), a[1].max(r[1])]));
                    }
                }
                let clim = color_limits(range);
                axes.extras.clim = Some(clim);
                clim
            } else {
                axes.extras.color_limits()
            };
            let map = axes.extras.effective_colormap();
            for series in &mut axes.series {
                super::object_commands::refresh_indexed_colors(self, series, &map, clim)?;
            }
            let background = axes.resolve_colors(appearance).background;
            for &i in &members {
                let rec = &mut self.surf.records[i];
                rec.auto_clim = auto.then_some(clim);
                if !force
                    && rec.built.as_ref().is_some_and(|(m, c, b)| {
                        (Arc::ptr_eq(m, &map) || **m == *map) && *c == clim && *b == background
                    })
                {
                    continue;
                }
                let mapping = Mapping {
                    map: &map,
                    clim,
                    background,
                };
                let built = build_series(rec, &mapping, &self.cancel)?;
                let index = position_in(axes, &rec.primary).expect("member");
                let old = &axes.series[index];
                let mut series = built;
                series.identity = old.identity.clone();
                series.baseline_identity = old.baseline_identity.clone();
                series.label = old.label.clone();
                series.legend_hidden = old.legend_hidden;
                series.visible = old.visible;
                axes.series[index] = series;
                rec.built = Some((map.clone(), clim, background));
            }
        }
        Ok(())
    }

    /// Whether any surface record exists (publication then recolours).
    pub(in crate::engine) fn surface_records(&self) -> bool {
        !self.surf.records.is_empty()
    }

    fn surface_prune(&mut self) {
        let live: Vec<ObjectIdentity> = self
            .figures()
            .flat_map(|(_, f)| f.all_axes().flat_map(|(_, a)| a.series.iter()))
            .map(|s| s.identity.clone())
            .collect();
        self.surf
            .records
            .retain(|r| live.iter().any(|l| l.same_object(&r.primary)));
        if self.surf.records.len() > MAX_RECORDS {
            let excess = self.surf.records.len() - MAX_RECORDS;
            self.surf.records.drain(..excess);
        }
    }

    /// The figure a new surface draws into: the current axes when held, else a
    /// fresh axes in view(3) that keeps identity, placement and layout.
    pub(in crate::engine) fn surface_target(&self) -> Figure {
        if let Some(f) = self.figure.as_ref().filter(|f| f.hold) {
            return f.clone();
        }
        let old = self.figure.as_ref();
        Figure {
            identity: old.map_or_else(ObjectIdentity::default, |f| f.identity.clone()),
            axes: Axes {
                palette_cycle: old.map_or(
                    Some(crate::plot::PaletteCycle::new(
                        crate::plot::PaletteVersion::MatRustModern1,
                    )),
                    |f| f.axes.palette_cycle.map(crate::plot::PaletteCycle::reset),
                ),
                identity: old.map_or_else(ObjectIdentity::default, |f| f.axes.identity.clone()),
                placement: old.and_then(|f| f.axes.placement),
                view: view3d::VIEW_3D,
                ..Axes::default()
            },
            layout: old.and_then(|f| f.layout.clone()),
            textboxes: crate::engine::plots_2d_more::p2_kept_textboxes(self),
            annotations: old.map_or_else(Vec::new, |f| f.annotations.clone()),
            name: old.map_or_else(String::new, |f| f.name.clone()),
            color: old.map_or([255; 3], |f| f.color),
            invert_hardcopy: old.and_then(|f| f.invert_hardcopy),
            alphamap: old.and_then(|f| f.alphamap.clone()),
            slice_viewers: Vec::new(),
            orthoslice_viewers: Vec::new(),
            appearance: old.map_or(Some(crate::plot::FigureAppearance::default()), |f| {
                f.appearance
            }),
            position: old.map_or(crate::plot::DEFAULT_FIGURE_POSITION, |f| f.position),
            units: old.map_or(Default::default(), |f| f.units),
            number_title: old.is_none_or(|f| f.number_title),
            visible: old.is_none_or(|f| f.visible),
            resize: old.is_none_or(|f| f.resize),
            window_state: old.map_or(Default::default(), |f| f.window_state),
        }
    }

    fn surface_draw(&mut self, args: &[Value], who: &str) -> Result<Value> {
        let Parsed {
            mut rec,
            label,
            visible,
        } = parse(self, args, who)?;
        // Low-level surface inserts. High-level surf/mesh/pcolor replace when
        // hold is off and choose the view themselves.
        let low_level = who == "surface";
        let previous = self.figure.clone();
        let inserting = low_level && previous.is_some();
        let mut figure = if let (true, Some(existing)) = (inserting, previous.as_ref()) {
            existing.clone()
        } else if low_level {
            let mut figure = self.surface_target();
            figure.axes.view = view3d::VIEW_2D;
            figure
        } else {
            self.surface_target()
        };
        if !inserting {
            super::plot_carry_over(previous.as_ref(), &mut figure, &mut [])?;
        }
        let axes_defaults = if inserting {
            if self.graphics_axes_defaults_pending(&figure) {
                self.graphics_existing_axes_defaults(&mut figure, None)?
            } else {
                None
            }
        } else {
            self.graphics_plot_axes_defaults(&mut figure)?
        };
        if !low_level && previous.as_ref().is_none_or(|f| !f.hold) {
            figure.axes.view = if who == "pcolor" {
                view3d::VIEW_2D
            } else {
                view3d::VIEW_3D
            };
        }
        // Bound check before anything is built.
        let used = figure.geometry_ledger()?.cost().general_faces();
        let faces = grid_faces(&rec.z);
        if used.saturating_add(faces) > MAX_PATCH_FACES {
            return Err(format!(
                "{who}: the surface needs {faces} faces and the figure already holds {used}; a figure holds at most {MAX_PATCH_FACES} patch faces"
            ));
        }
        let map = figure.axes.extras.effective_colormap();
        let mut series = build_series(
            &rec,
            &Mapping {
                map: &map,
                clim: [0., 1.],
                background: [255; 3],
            },
            &self.cancel,
        )?;
        series.visible = visible;
        let id = series.identity.clone();
        figure.add(vec![series])?;
        if let Some(label) = label
            && let Some(s) = figure.axes.series.last_mut()
        {
            s.label = label;
        }
        rec.primary = id.clone();
        let token = GraphicsHandle::series(&figure.identity, &figure.axes.identity, &id);
        self.surf.records.push(rec);
        self.surface_refresh(&mut figure, false)?;
        figure.prepare_legend_layout()?;
        figure.validate()?;
        self.check_cancel()?;
        self.graphics_finish_axes_defaults(&figure, axes_defaults);
        self.figure = Some(figure);
        self.surface_prune();
        Ok(Value::Container(
            GraphicsArray::scalar(token, &self.cancel)?.into(),
        ))
    }

    /// `shading flat|faceted|interp` on the surfaces of the current axes.
    fn surface_shading(&mut self, args: &[Value]) -> Result<Value> {
        let [Value::Text(mode)] = args else {
            return Err(
                "Use shading flat, shading faceted, shading interp or shading(ax, mode)".into(),
            );
        };
        let mode = mode.to_ascii_lowercase();
        if !matches!(mode.as_str(), "flat" | "faceted" | "interp") {
            return Err("shading accepts 'flat', 'faceted' or 'interp'".into());
        }
        let mut figure = self.figure.clone().unwrap_or_default();
        for rec in &mut self.surf.records {
            if position_in(&figure.axes, &rec.primary).is_none() {
                continue;
            }
            match (mode.as_str(), rec.mesh) {
                ("flat", false) => (rec.face, rec.edge) = (Coloring::Flat, Coloring::None),
                ("faceted", false) => {
                    (rec.face, rec.edge) = (Coloring::Flat, Coloring::Rgb([0; 3]))
                }
                ("interp", false) => (rec.face, rec.edge) = (Coloring::Interp, Coloring::None),
                ("interp", true) => rec.edge = Coloring::Interp,
                (_, true) => rec.edge = Coloring::Flat,
                _ => {}
            }
            rec.built = None;
        }
        self.surface_refresh(&mut figure, false)?;
        figure.validate()?;
        self.check_cancel()?;
        if self.figure.is_some() {
            self.figure = Some(figure);
        }
        Ok(Value::Void)
    }

    /// get(h, Name) for a surface; None leaves the query to the series code.
    pub(in crate::engine) fn surface_get(
        &self,
        h: &GraphicsHandle,
        name: &str,
    ) -> Result<Option<Value>> {
        if h.kind() != GraphicsKind::Series {
            return Ok(None);
        }
        let Some(rec) = self
            .surf
            .records
            .iter()
            .find(|r| r.primary.same_object(&h.object))
        else {
            return Ok(None);
        };
        let text = |s: &str| Value::Text(s.into());
        let rgb = |c: [u8; 3]| -> Result<Value> {
            Ok(Value::Matrix(Matrix::from_column_slice(
                1,
                3,
                &c.map(|v| f64::from(v) / 255.),
            )?))
        };
        let background = || rec.built.as_ref().map_or([255; 3], |b| b.2);
        let coloring = |c: Coloring| -> Result<Value> {
            Ok(match c {
                Coloring::Flat => text("flat"),
                Coloring::Interp => text("interp"),
                Coloring::None => text("none"),
                Coloring::Rgb(c) => rgb(c)?,
                Coloring::Background => rgb(background())?,
            })
        };
        Ok(Some(match name.to_ascii_lowercase().as_str() {
            "type" => text("surface"),
            "xdata" => Value::Matrix(rec.x.clone()),
            "ydata" => Value::Matrix(rec.y.clone()),
            "zdata" => Value::Matrix(rec.z.clone()),
            "cdata" => Value::Matrix(rec.c.clone()),
            "facecolor" => coloring(rec.face)?,
            "edgecolor" => coloring(rec.edge)?,
            "facelighting" => text(lighting_name(rec.lighting)),
            "facealpha" => Value::Matrix(Matrix::scalar(rec.face_alpha)),
            "linestyle" => text(match rec.pattern {
                LinePattern::Solid => "-",
                LinePattern::Dashed => "--",
                LinePattern::Dotted => ":",
                LinePattern::DashDot => "-.",
                LinePattern::None => "none",
            }),
            "linewidth" => Value::Matrix(Matrix::scalar(rec.line_width)),
            "cdatamapping" => text("scaled"),
            _ => return Ok(None),
        }))
    }

    /// set(h, ...) for a surface: applies the surface properties, rebuilds the
    /// series and returns the remaining (generic series) pairs.
    pub(in crate::engine) fn surface_set(
        &mut self,
        h: &GraphicsHandle,
        figure_number: u32,
        args: &[Value],
    ) -> Result<Option<Vec<Value>>> {
        if h.kind() != GraphicsKind::Series {
            return Ok(None);
        }
        let Some(index) = self
            .surf
            .records
            .iter()
            .position(|r| r.primary.same_object(&h.object))
        else {
            return Ok(None);
        };
        let args = self.text_arguments(args.to_vec())?;
        let mut staged = self.surf.records[index].clone();
        let mut rest = Vec::new();
        let mut mine = false;
        for pair in args.as_chunks::<2>().0 {
            let name = text_of(&pair[0])
                .ok_or("Graphics property requires character text")?
                .to_ascii_lowercase();
            if apply_property(self, &mut staged, &name, &pair[1])? {
                mine = true;
            } else {
                rest.extend_from_slice(pair);
            }
        }
        if !mine {
            return Ok(None);
        }
        check_data("set", &staged.x, &staged.y, &staged.z, &staged.c)?;
        let mut figure = self.graphics_figure(figure_number)?.clone();
        let old = self.surf.records[index].clone();
        staged.built = None;
        self.surf.records[index] = staged;
        let result = (|| {
            let used: usize = figure
                .all_axes()
                .flat_map(|(_, a)| a.series.iter())
                .filter(|s| !s.identity.same_object(&h.object))
                .map(Series::patch_face_count)
                .sum();
            let faces = grid_faces(&self.surf.records[index].z);
            if used.saturating_add(faces) > MAX_PATCH_FACES {
                return Err(format!(
                    "set: the surface needs {faces} faces; a figure holds at most {MAX_PATCH_FACES} patch faces"
                ));
            }
            self.surface_refresh(&mut figure, false)?;
            figure.validate()
        })();
        if let Err(error) = result {
            self.surf.records[index] = old;
            return Err(error);
        }
        self.check_cancel()?;
        self.publish_graphics_figure(figure_number, figure);
        Ok(Some(rest))
    }
}

/// Surface and mesh commands. A leading axes handle sets the plot target.
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Value> {
    e.check_cancel()?;
    if count > 1 {
        return Err(format!("{name}: too many output arguments"));
    }
    if name == "sphere" && count == 1 {
        return Ok(sphere_coordinates(e, args, count)?.remove(0));
    }
    let mut args = e.text_arguments(args.to_vec())?;
    let old = (
        e.current_figure,
        e.figure.clone(),
        e.other_figures.clone(),
        e.surf.clone(),
    );
    let mut uiaxes_id: Option<u32> = None;
    let result = (|| {
        if let Some(Value::Container(Array::Graphics(h))) = args.first()
            && h.len() == 1
            && handle(&args[0])?.kind() == GraphicsKind::Axes
        {
            let target = args.remove(0);
            let th = handle(&target)?;
            let located = e.locate_graphics(th)?;
            if super::super::ui_components::is_uiaxes_fig_id(located.figure) {
                let mut selected = e.graphics_figure(located.figure)?.clone();
                super::select_live_axes(&mut selected, located.position.unwrap())?;
                e.figure = Some(selected);
                uiaxes_id = Some(located.figure);
            } else {
                e.call("axes", vec![target], out)?;
            }
        }
        match name {
            "sphere" => {
                let xyz = sphere_coordinates(e, &args, 0)?;
                e.surface_draw(&xyz, "surf")
            }
            "trisurf" | "trimesh" | "tetramesh" => super::indexed_mesh::draw(e, name, &args),
            "slice" => crate::engine::volume::slice::call(e, &args, out),
            "contourslice" => crate::engine::volume::contourslice::call(e, &args),
            "volshow" => crate::engine::volume::show::volshow_call(e, &args),
            "shading" => e.surface_shading(&args),
            "image" | "imagesc" | "imshow" => e.image_draw(name, &args, out),
            _ => e.surface_draw(&args, name),
        }
    })();
    if let Some(uid) = uiaxes_id {
        if result.is_ok()
            && let Some(updated) = e.figure.take()
        {
            e.publish_graphics_figure(uid, updated);
        }
        if result.is_err() {
            e.surf = old.3;
        }
        e.current_figure = old.0;
        e.figure = old.1;
        e.other_figures = old.2;
    } else if result.is_err() {
        (e.current_figure, e.figure, e.other_figures, e.surf) = old;
    }
    result
}
