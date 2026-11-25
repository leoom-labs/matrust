//! 2-D PDE solution plotting (`pdeplot`): colour maps, 3-D surfaces, P1
//! iso-contours, mesh overlays, and nodal vector fields (`FlowData`).
use super::{Engine, Execution, Value, objects};
use crate::{Array, GraphicsArray, GraphicsHandle, Matrix, Result};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// Default number of automatic contour levels when `'Contour','on'` is set
/// without explicit `'Levels'` (MATLAB documents 10).
const DEFAULT_CONTOUR_LEVELS: usize = 10;
/// Upper bound on automatic or explicit contour levels.
const MAX_CONTOUR_LEVELS: usize = 1000;
/// Number of sub-segments used to draw each curved quadratic triangle edge in 2-D.
const CURVED_EDGE_STEPS: usize = 8;

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    if count > 1 {
        return Err(format!("{name}: too many output arguments"));
    }
    let val = pdeplot(e, args, out)?;
    Ok(if count == 0 { vec![] } else { vec![val] })
}

// ---------------------------------------------------------------------------
// Options & mesh extraction

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum XYStyle {
    Interp,
    Flat,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZStyle {
    Continuous,
    Discontinuous,
    Off,
}

#[derive(Clone, Debug)]
enum ContourLevels {
    Count(usize),
    Values(Vec<f64>),
}

struct PdePlotOptions {
    xy_data: Option<Vec<f64>>,
    z_data: Option<Vec<f64>>,
    colorbar: Option<bool>,
    contour: bool,
    levels: ContourLevels,
    mesh: Option<bool>,
    flow_data: Option<(Vec<f64>, Vec<f64>)>,
    colormap: Option<Value>,
    xy_style: XYStyle,
    z_style: ZStyle,
    title: Option<String>,
}

impl Default for PdePlotOptions {
    fn default() -> Self {
        Self {
            xy_data: None,
            z_data: None,
            colorbar: None,
            contour: false,
            levels: ContourLevels::Count(DEFAULT_CONTOUR_LEVELS),
            mesh: None,
            flow_data: None,
            colormap: None,
            xy_style: XYStyle::Interp,
            z_style: ZStyle::Continuous,
            title: None,
        }
    }
}

struct PlotMeshData {
    nodes: Vec<[f64; 2]>,
    tris: Vec<[usize; 3]>,
    mid: Option<Vec<[usize; 3]>>,
}

fn extract_mesh<'a>(e: &Engine, args: &'a [Value]) -> Result<(PlotMeshData, &'a [Value])> {
    if args.is_empty() {
        return Err("pdeplot: expected pdeplot(model | mesh | nodes, elements, ...)".into());
    }
    if let Some(m) = objects::pde_mesh_value(e, &args[0]) {
        return Ok((
            PlotMeshData {
                nodes: m.nodes.clone(),
                tris: m.tris.clone(),
                mid: m.mid.clone(),
            },
            &args[1..],
        ));
    }
    if objects::is_pde_model(e, &args[0]) {
        let id = objects::pde_model_id(e, &args[0], "pdeplot")?;
        let model =
            objects::pde_model_get(e, id).ok_or("pdeplot: invalid or deleted pde.PDEModel")?;
        let mesh_arr = model
            .mesh
            .as_ref()
            .ok_or("pdeplot: model has no mesh; call generateMesh first")?;
        let mesh_val = Value::Container(mesh_arr.as_ref().clone());
        let m = objects::pde_mesh_value(e, &mesh_val)
            .ok_or("pdeplot: model.Mesh is not a valid pde.FEMesh")?;
        return Ok((
            PlotMeshData {
                nodes: m.nodes.clone(),
                tris: m.tris.clone(),
                mid: m.mid.clone(),
            },
            &args[1..],
        ));
    }
    if args.len() < 2 {
        return Err("pdeplot: expected pdeplot(model | mesh | nodes, elements, ...)".into());
    }
    let nodes_mat = numeric_matrix(&args[0], "pdeplot: nodes")?;
    let elems_mat = numeric_matrix(&args[1], "pdeplot: elements")?;
    if nodes_mat.rows() != 2 {
        return Err("pdeplot: nodes must be a 2-by-N numeric matrix".into());
    }
    let n_nodes = nodes_mat.cols();
    if elems_mat.rows() != 3 && elems_mat.rows() != 6 {
        return Err("pdeplot: elements must be a 3-by-M or 6-by-M index matrix".into());
    }
    let mut nodes = Vec::with_capacity(n_nodes);
    for j in 0..n_nodes {
        if j.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        nodes.push([nodes_mat.get(0, j), nodes_mat.get(1, j)]);
    }
    let n_elems = elems_mat.cols();
    let is_quad = elems_mat.rows() == 6;
    let mut tris = Vec::with_capacity(n_elems);
    let mut mids = is_quad.then(|| Vec::with_capacity(n_elems));
    let check_idx = |v: f64| -> Result<usize> {
        if v.fract() != 0.0 || v < 1.0 || v > n_nodes as f64 {
            return Err(format!(
                "pdeplot: element node index must be an integer in 1..={n_nodes}"
            ));
        }
        Ok(v as usize - 1)
    };
    for j in 0..n_elems {
        if j.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let a = check_idx(elems_mat.get(0, j))?;
        let b = check_idx(elems_mat.get(1, j))?;
        let c = check_idx(elems_mat.get(2, j))?;
        tris.push([a, b, c]);
        if let Some(mv) = &mut mids {
            let m0 = check_idx(elems_mat.get(3, j))?;
            let m1 = check_idx(elems_mat.get(4, j))?;
            let m2 = check_idx(elems_mat.get(5, j))?;
            mv.push([m0, m1, m2]);
        }
    }
    Ok((
        PlotMeshData {
            nodes,
            tris,
            mid: mids,
        },
        &args[2..],
    ))
}

fn parse_options(args: &[Value], n_nodes: usize) -> Result<PdePlotOptions> {
    if !args.len().is_multiple_of(2) {
        return Err("pdeplot: options must be name-value pairs".into());
    }
    let mut opts = PdePlotOptions::default();
    for pair in args.as_chunks::<2>().0 {
        let name = text_value(&pair[0])
            .ok_or("pdeplot: parameter names must be character vectors or strings")?;
        if name.eq_ignore_ascii_case("XYData") {
            opts.xy_data = parse_nodal_vector(&pair[1], n_nodes, "XYData")?;
        } else if name.eq_ignore_ascii_case("ZData") {
            opts.z_data = parse_nodal_vector(&pair[1], n_nodes, "ZData")?;
        } else if name.eq_ignore_ascii_case("ColorBar") {
            opts.colorbar = Some(parse_on_off(&pair[1], "ColorBar")?);
        } else if name.eq_ignore_ascii_case("Contour") {
            opts.contour = parse_on_off(&pair[1], "Contour")?;
        } else if name.eq_ignore_ascii_case("Levels") {
            opts.levels = parse_levels(&pair[1])?;
        } else if name.eq_ignore_ascii_case("Mesh") {
            opts.mesh = Some(parse_on_off(&pair[1], "Mesh")?);
        } else if name.eq_ignore_ascii_case("FlowData") {
            opts.flow_data = parse_flow_data(&pair[1], n_nodes)?;
        } else if name.eq_ignore_ascii_case("ColorMap") {
            opts.colormap = Some(pair[1].clone());
        } else if name.eq_ignore_ascii_case("XYStyle") {
            let val = text_value(&pair[1])
                .ok_or("pdeplot: 'XYStyle' must be 'interp', 'flat', or 'off'")?;
            opts.xy_style = if val.eq_ignore_ascii_case("interp") {
                XYStyle::Interp
            } else if val.eq_ignore_ascii_case("flat") {
                XYStyle::Flat
            } else if val.eq_ignore_ascii_case("off") {
                XYStyle::Off
            } else {
                return Err(format!(
                    "pdeplot: invalid 'XYStyle' value '{val}' (expected 'interp', 'flat', or 'off')"
                ));
            };
        } else if name.eq_ignore_ascii_case("ZStyle") {
            let val = text_value(&pair[1])
                .ok_or("pdeplot: 'ZStyle' must be 'continuous', 'discontinuous', or 'off'")?;
            opts.z_style = if val.eq_ignore_ascii_case("continuous") {
                ZStyle::Continuous
            } else if val.eq_ignore_ascii_case("discontinuous") {
                ZStyle::Discontinuous
            } else if val.eq_ignore_ascii_case("off") {
                ZStyle::Off
            } else {
                return Err(format!(
                    "pdeplot: invalid 'ZStyle' value '{val}' (expected 'continuous', 'discontinuous', or 'off')"
                ));
            };
        } else if name.eq_ignore_ascii_case("Title") {
            let val = text_value(&pair[1])
                .ok_or("pdeplot: 'Title' must be a character vector or string")?;
            opts.title = Some(val);
        } else {
            return Err(format!("Unrecognized parameter name '{name}'."));
        }
    }
    Ok(opts)
}

// ---------------------------------------------------------------------------
// P1 triangle split & contour extraction

fn split_to_p1(e: &Engine, data: &PlotMeshData) -> Result<Vec<[usize; 3]>> {
    match &data.mid {
        None => Ok(data.tris.clone()),
        Some(mids) => {
            let n_faces = data
                .tris
                .len()
                .checked_mul(4)
                .ok_or("pdeplot: element count overflow")?;
            if n_faces > crate::plot::MAX_PATCH_FACES {
                return Err(format!(
                    "pdeplot: split mesh exceeds {} triangular faces",
                    crate::plot::MAX_PATCH_FACES
                ));
            }
            let mut out = Vec::with_capacity(n_faces);
            for (i, (tri, m)) in data.tris.iter().zip(mids.iter()).enumerate() {
                if i.is_multiple_of(4096) {
                    e.check_cancel()?;
                }
                let [v0, v1, v2] = *tri;
                let [m0, m1, m2] = *m;
                out.push([v0, m0, m2]);
                out.push([m0, v1, m1]);
                out.push([m2, m1, v2]);
                out.push([m0, m1, m2]);
            }
            Ok(out)
        }
    }
}

fn resolve_levels(spec: &ContourLevels, values: &[f64]) -> Vec<f64> {
    match spec {
        ContourLevels::Values(v) => v.clone(),
        ContourLevels::Count(k) => {
            if *k == 0 || values.is_empty() {
                return Vec::new();
            }
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for &v in values {
                if v < lo {
                    lo = v;
                }
                if v > hi {
                    hi = v;
                }
            }
            if !lo.is_finite() || !hi.is_finite() || hi <= lo {
                return Vec::new();
            }
            let denom = (*k + 1) as f64;
            (1..=*k)
                .map(|i| lo + (i as f64) * (hi - lo) / denom)
                .collect()
        }
    }
}

/// Extract NaN-separated iso-line segments for a single level `level` on P1
/// triangles. Every emitted vertex linearly interpolates `u` to `level` within
/// floating-point roundoff.
fn extract_contour_segments(
    e: &Engine,
    nodes: &[[f64; 2]],
    p1_tris: &[[usize; 3]],
    u: &[f64],
    z: Option<&[f64]>,
    level: f64,
    discontinuous_z: bool,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut zs = Vec::new();
    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();

    let push_seg =
        |xs: &mut Vec<f64>, ys: &mut Vec<f64>, zs: &mut Vec<f64>, a: [f64; 3], b: [f64; 3]| {
            if !xs.is_empty() {
                xs.push(f64::NAN);
                ys.push(f64::NAN);
                if z.is_some() {
                    zs.push(f64::NAN);
                }
            }
            xs.push(a[0]);
            ys.push(a[1]);
            xs.push(b[0]);
            ys.push(b[1]);
            if z.is_some() {
                zs.push(a[2]);
                zs.push(b[2]);
            }
        };

    // `Some` pins every sample of this triangle to one height (ZStyle discontinuous).
    let node_pt = |idx: usize, z_const: Option<f64>| -> [f64; 3] {
        let p = nodes[idx];
        let zz = z_const.unwrap_or_else(|| z.map_or(0.0, |zv| zv[idx]));
        [p[0], p[1], zz]
    };

    let interp_edge = |ia: usize, ib: usize, da: f64, db: f64, z_const: Option<f64>| -> [f64; 3] {
        let t = da / (da - db);
        let pa = node_pt(ia, z_const);
        let pb = node_pt(ib, z_const);
        [
            pa[0] + t * (pb[0] - pa[0]),
            pa[1] + t * (pb[1] - pa[1]),
            pa[2] + t * (pb[2] - pa[2]),
        ]
    };

    for (ti, tri) in p1_tris.iter().enumerate() {
        if ti.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        let z_const = z.and_then(|zv| {
            if !discontinuous_z {
                return None;
            }
            Some((zv[tri[0]] + zv[tri[1]] + zv[tri[2]]) / 3.0)
        });
        let d = [u[tri[0]] - level, u[tri[1]] - level, u[tri[2]] - level];
        let zero_count = d.iter().filter(|&&v| v == 0.0).count();
        match zero_count {
            3 => {
                // Entire triangle sits at the contour level; no unique iso-line.
            }
            2 => {
                // Exactly one edge lies on the contour level; deduplicate shared edges.
                let mut zidx = [0usize; 2];
                let mut k = 0;
                for i in 0..3 {
                    if d[i] == 0.0 {
                        zidx[k] = tri[i];
                        k += 1;
                    }
                }
                let key = (zidx[0].min(zidx[1]), zidx[0].max(zidx[1]));
                if seen_edges.insert(key) {
                    push_seg(
                        &mut xs,
                        &mut ys,
                        &mut zs,
                        node_pt(zidx[0], z_const),
                        node_pt(zidx[1], z_const),
                    );
                }
            }
            1 => {
                // One vertex is on the level; emit a segment only if the opposite edge crosses.
                let zi = if d[0] == 0.0 {
                    0
                } else if d[1] == 0.0 {
                    1
                } else {
                    2
                };
                let i1 = (zi + 1) % 3;
                let i2 = (zi + 2) % 3;
                if (d[i1] < 0.0 && d[i2] > 0.0) || (d[i1] > 0.0 && d[i2] < 0.0) {
                    let q = interp_edge(tri[i1], tri[i2], d[i1], d[i2], z_const);
                    push_seg(&mut xs, &mut ys, &mut zs, node_pt(tri[zi], z_const), q);
                }
            }
            _ => {
                // No vertex is on the level; find the two edges that cross strictly.
                let mut pts = [[0.0; 3]; 2];
                let mut found = 0usize;
                for k in 0..3 {
                    let a = k;
                    let b = (k + 1) % 3;
                    if (d[a] < 0.0 && d[b] > 0.0) || (d[a] > 0.0 && d[b] < 0.0) {
                        if found < 2 {
                            pts[found] = interp_edge(tri[a], tri[b], d[a], d[b], z_const);
                        }
                        found += 1;
                    }
                }
                if found == 2 {
                    push_seg(&mut xs, &mut ys, &mut zs, pts[0], pts[1]);
                }
            }
        }
    }

    Ok((xs, ys, zs))
}

// ---------------------------------------------------------------------------
// Main pdeplot implementation

fn pdeplot(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let (data, opt_args) = extract_mesh(e, args)?;
    let opts = parse_options(opt_args, data.nodes.len())?;

    if opts.contour && opts.xy_data.is_none() && opts.z_data.is_none() {
        return Err("pdeplot: 'Contour' requires 'XYData' or 'ZData'".into());
    }

    let p1_tris = split_to_p1(e, &data)?;
    if p1_tris.len() > crate::plot::MAX_PATCH_FACES {
        return Err(format!(
            "pdeplot: mesh exceeds {} triangular faces",
            crate::plot::MAX_PATCH_FACES
        ));
    }

    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let res = draw_pdeplot(e, &data, &p1_tris, &opts, out);
    if res.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    res
}

fn draw_pdeplot(
    e: &mut Engine,
    data: &PlotMeshData,
    p1_tris: &[[usize; 3]],
    opts: &PdePlotOptions,
    out: &mut Execution,
) -> Result<Value> {
    let ax = e.call("gca", vec![], out)?;
    let next = e.call("get", vec![ax.clone(), Value::Text("NextPlot".into())], out)?;
    let held = text_value(&next).as_deref() == Some("add");

    let has_z = opts.z_data.is_some() && opts.z_style != ZStyle::Off;
    let color_data: Option<&[f64]> =
        opts.xy_data
            .as_deref()
            .or(if has_z { opts.z_data.as_deref() } else { None });
    // XYStyle 'off' is "no shading, only the mesh": edges in 2-D, surface edges with ZData.
    let xy_off = opts.xy_style == XYStyle::Off;
    let draw_patch = color_data.is_some() && !xy_off;
    let draw_filled_patch = draw_patch;
    let default_mesh = opts.xy_data.is_none() && opts.z_data.is_none() && opts.flow_data.is_none();
    let show_mesh = opts.mesh.unwrap_or(default_mesh || xy_off);
    let show_colorbar = opts.colorbar.unwrap_or(draw_filled_patch);
    let discontinuous_z = has_z && opts.z_style == ZStyle::Discontinuous;

    let mut handles: Vec<Arc<GraphicsHandle>> = Vec::new();

    let draw_result = (|| -> Result<()> {
        // Hold off replaces the axes the way plot does (fresh limits, title, and
        // children). `delete(children)` cannot be used: it returns no outputs.
        if !held {
            replace_current_axes(e)?;
        }

        e.call("hold", vec![ax.clone(), Value::Text("on".into())], out)?;

        // 1) ColorMap (set before patch/colorbar so colors map with the requested map)
        if let Some(cmap) = &opts.colormap {
            e.call("colormap", vec![ax.clone(), cmap.clone()], out)?;
        }

        // 2) Patch for XYData and/or ZData
        if draw_patch {
            let cdata = color_data.expect("draw_patch requires color_data");
            let z_slice = if has_z { opts.z_data.as_deref() } else { None };
            let (faces_mat, verts_mat, c_mat) = build_patch_matrices(
                e,
                &data.nodes,
                p1_tris,
                cdata,
                z_slice,
                opts.xy_style,
                opts.z_style,
            )?;
            let face_color = match opts.xy_style {
                XYStyle::Interp => "interp",
                XYStyle::Flat => "flat",
                XYStyle::Off => "none",
            };
            let h = e.call(
                "patch",
                vec![
                    ax.clone(),
                    Value::Text("Faces".into()),
                    Value::Matrix(faces_mat),
                    Value::Text("Vertices".into()),
                    Value::Matrix(verts_mat),
                    Value::Text("FaceVertexCData".into()),
                    Value::Matrix(c_mat),
                    Value::Text("FaceColor".into()),
                    Value::Text(face_color.into()),
                    Value::Text("EdgeColor".into()),
                    Value::Text("none".into()),
                ],
                out,
            )?;
            push_handle(&mut handles, h)?;
        }

        // 3) Iso-contour lines on the P1 triangle split
        if opts.contour {
            let contour_u = opts
                .xy_data
                .as_deref()
                .or(opts.z_data.as_deref())
                .expect("checked above");
            let z_slice = if has_z { opts.z_data.as_deref() } else { None };
            let levels = resolve_levels(&opts.levels, contour_u);
            let contour_color = if draw_filled_patch {
                Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.0, 0.0, 0.0])?)
            } else {
                Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.0, 0.4470, 0.7410])?)
            };
            for &level in &levels {
                let (xs, ys, zs) = extract_contour_segments(
                    e,
                    &data.nodes,
                    p1_tris,
                    contour_u,
                    z_slice,
                    level,
                    discontinuous_z,
                )?;
                if xs.is_empty() {
                    continue;
                }
                let x_val = Value::Matrix(Matrix::from_column_vec(xs.len(), 1, xs)?);
                let y_val = Value::Matrix(Matrix::from_column_vec(ys.len(), 1, ys)?);
                let h = if has_z {
                    let z_val = Value::Matrix(Matrix::from_column_vec(zs.len(), 1, zs)?);
                    e.call(
                        "plot3",
                        vec![
                            ax.clone(),
                            x_val,
                            y_val,
                            z_val,
                            Value::Text("Color".into()),
                            contour_color.clone(),
                        ],
                        out,
                    )?
                } else {
                    e.call(
                        "plot",
                        vec![
                            ax.clone(),
                            x_val,
                            y_val,
                            Value::Text("Color".into()),
                            contour_color.clone(),
                        ],
                        out,
                    )?
                };
                push_handle(&mut handles, h)?;
            }
        }

        // 4) Mesh edges overlay
        if show_mesh {
            let z_slice = if has_z { opts.z_data.as_deref() } else { None };
            let (xs, ys, zs) = build_mesh_edge_lines(e, data, z_slice, discontinuous_z)?;
            if !xs.is_empty() {
                let edge_color = if draw_filled_patch {
                    Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.15, 0.15, 0.15])?)
                } else {
                    Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.0, 0.4470, 0.7410])?)
                };
                let x_val = Value::Matrix(Matrix::from_column_vec(xs.len(), 1, xs)?);
                let y_val = Value::Matrix(Matrix::from_column_vec(ys.len(), 1, ys)?);
                let h = if has_z {
                    let z_val = Value::Matrix(Matrix::from_column_vec(zs.len(), 1, zs)?);
                    e.call(
                        "plot3",
                        vec![
                            ax.clone(),
                            x_val,
                            y_val,
                            z_val,
                            Value::Text("Color".into()),
                            edge_color,
                        ],
                        out,
                    )?
                } else {
                    e.call(
                        "plot",
                        vec![
                            ax.clone(),
                            x_val,
                            y_val,
                            Value::Text("Color".into()),
                            edge_color,
                        ],
                        out,
                    )?
                };
                push_handle(&mut handles, h)?;
            }
        }

        // 5) FlowData quiver arrows at mesh nodes
        if let Some((ref fx, ref fy)) = opts.flow_data {
            let n = data.nodes.len();
            let xs: Vec<f64> = data.nodes.iter().map(|p| p[0]).collect();
            let ys: Vec<f64> = data.nodes.iter().map(|p| p[1]).collect();
            let x_val = Value::Matrix(Matrix::from_column_vec(n, 1, xs)?);
            let y_val = Value::Matrix(Matrix::from_column_vec(n, 1, ys)?);
            let u_val = Value::Matrix(Matrix::from_column_vec(n, 1, fx.clone())?);
            let v_val = Value::Matrix(Matrix::from_column_vec(n, 1, fy.clone())?);
            e.call("axes", vec![ax.clone()], out)?;
            let h = if let Some(z_vals) = opts.z_data.as_deref().filter(|_| has_z) {
                let z_val = Value::Matrix(Matrix::from_column_vec(n, 1, z_vals.to_vec())?);
                let w_val = Value::Matrix(Matrix::from_column_vec(n, 1, vec![0.0; n])?);
                e.call(
                    "quiver3",
                    vec![x_val, y_val, z_val, u_val, v_val, w_val],
                    out,
                )?
            } else {
                e.call("quiver", vec![x_val, y_val, u_val, v_val], out)?
            };
            push_handle(&mut handles, h)?;
        }

        // 6) ColorBar
        if show_colorbar {
            e.call("colorbar", vec![ax.clone()], out)?;
        }

        // 7) Title, view, aspect
        if let Some(title) = &opts.title {
            e.call("title", vec![ax.clone(), Value::Text(title.clone())], out)?;
        }

        e.call("axes", vec![ax.clone()], out)?;
        if has_z {
            e.call("view", vec![Value::Matrix(Matrix::scalar(3.0))], out)?;
        } else {
            e.call("view", vec![Value::Matrix(Matrix::scalar(2.0))], out)?;
            e.call("axis", vec![Value::Text("equal".into())], out)?;
        }
        Ok(())
    })();

    let restore = e.call(
        "hold",
        vec![ax, Value::Text(if held { "on" } else { "off" }.into())],
        out,
    );
    draw_result?;
    restore?;

    let arr = GraphicsArray::from_shared(handles.len(), 1, handles, &e.cancel)?;
    Ok(Value::Container(Array::Graphics(arr)))
}

/// Hold off: replace the current axes the way `plot` does. Identity, placement
/// and figure-level properties stay; title, manual limits, series and the colorbar go.
fn replace_current_axes(e: &mut Engine) -> Result<()> {
    let Some(old) = e.figure.clone() else {
        return Ok(());
    };
    let axes_id = old.axes.identity.clone();
    let palette = old.axes.palette_cycle.map(crate::plot::PaletteCycle::reset);
    let placement = old.axes.placement;
    let mut figure = old;
    figure.axes = crate::plot::Axes {
        palette_cycle: palette,
        identity: axes_id.clone(),
        placement,
        ..crate::plot::Axes::default()
    };
    figure
        .slice_viewers
        .retain(|s| !s.axes_identity.same_object(&axes_id));
    figure
        .orthoslice_viewers
        .retain(|o| !o.axes_identities.iter().any(|a| a.same_object(&axes_id)));
    let defaults = e.graphics_plot_axes_defaults(&mut figure)?;
    figure.validate()?;
    e.graphics_finish_axes_defaults(&figure, defaults);
    e.figure = Some(figure);
    Ok(())
}

fn build_patch_matrices(
    e: &Engine,
    nodes: &[[f64; 2]],
    p1_tris: &[[usize; 3]],
    cdata: &[f64],
    z_slice: Option<&[f64]>,
    xy_style: XYStyle,
    z_style: ZStyle,
) -> Result<(Matrix, Matrix, Matrix)> {
    let n_faces = p1_tris.len();
    let flat = xy_style == XYStyle::Flat;
    let discontinuous = z_slice.is_some() && z_style == ZStyle::Discontinuous;
    // Flat shading reads the first corner, and FaceVertexCData must be one value
    // per vertex, so each face gets its own three vertices at the element mean.
    // Discontinuous height does the same for z (one mean per P1 triangle).
    if flat || discontinuous {
        let n_verts = n_faces
            .checked_mul(3)
            .ok_or("pdeplot: vertex count overflow")?;
        let mut faces = vec![0.0; n_faces * 3];
        let mut verts = vec![0.0; n_verts * 3];
        let mut colors = Vec::with_capacity(n_verts);
        for (f, tri) in p1_tris.iter().enumerate() {
            if f.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let c_mean = (cdata[tri[0]] + cdata[tri[1]] + cdata[tri[2]]) / 3.0;
            let z_mean = z_slice.map(|zv| (zv[tri[0]] + zv[tri[1]] + zv[tri[2]]) / 3.0);
            for corner in 0..3 {
                let v = f * 3 + corner;
                let ni = tri[corner];
                faces[f + corner * n_faces] = (v + 1) as f64;
                verts[v] = nodes[ni][0];
                verts[v + n_verts] = nodes[ni][1];
                verts[v + 2 * n_verts] = if discontinuous {
                    z_mean.unwrap_or(0.0)
                } else {
                    z_slice.map_or(0.0, |zv| zv[ni])
                };
                colors.push(if flat { c_mean } else { cdata[ni] });
            }
        }
        return Ok((
            Matrix::from_column_vec(n_faces, 3, faces)?,
            Matrix::from_column_vec(n_verts, 3, verts)?,
            Matrix::from_column_vec(n_verts, 1, colors)?,
        ));
    }

    let n_verts = nodes.len();
    let mut faces = vec![0.0; n_faces * 3];
    for (f, tri) in p1_tris.iter().enumerate() {
        if f.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        faces[f] = (tri[0] + 1) as f64;
        faces[f + n_faces] = (tri[1] + 1) as f64;
        faces[f + 2 * n_faces] = (tri[2] + 1) as f64;
    }
    let mut verts = vec![0.0; n_verts * 3];
    for (i, p) in nodes.iter().enumerate() {
        if i.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        verts[i] = p[0];
        verts[i + n_verts] = p[1];
        verts[i + 2 * n_verts] = z_slice.map_or(0.0, |zv| zv[i]);
    }
    let colors = Matrix::from_column_vec(n_verts, 1, cdata.to_vec())?;
    Ok((
        Matrix::from_column_vec(n_faces, 3, faces)?,
        Matrix::from_column_vec(n_verts, 3, verts)?,
        colors,
    ))
}

fn build_mesh_edge_lines(
    e: &Engine,
    data: &PlotMeshData,
    z_slice: Option<&[f64]>,
    discontinuous_z: bool,
) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    // Each parent triangle's edges sit at that triangle's mean height, so a
    // shared edge is drawn once per side instead of at the nodal z.
    if discontinuous_z && let Some(zv) = z_slice {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let mut zs = Vec::new();
        for (t, tri) in data.tris.iter().enumerate() {
            if t.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let z_mean = (zv[tri[0]] + zv[tri[1]] + zv[tri[2]]) / 3.0;
            for k in 0..3 {
                if !xs.is_empty() {
                    xs.push(f64::NAN);
                    ys.push(f64::NAN);
                    zs.push(f64::NAN);
                }
                let a = tri[k];
                let b = tri[(k + 1) % 3];
                let pa = data.nodes[a];
                let pb = data.nodes[b];
                if let Some(mids) = &data.mid {
                    let pm = data.nodes[mids[t][k]];
                    xs.extend([pa[0], pm[0], pb[0]]);
                    ys.extend([pa[1], pm[1], pb[1]]);
                    zs.extend([z_mean, z_mean, z_mean]);
                } else {
                    xs.extend([pa[0], pb[0]]);
                    ys.extend([pa[1], pb[1]]);
                    zs.extend([z_mean, z_mean]);
                }
            }
        }
        return Ok((xs, ys, zs));
    }

    let mut edges: BTreeMap<(usize, usize), Option<usize>> = BTreeMap::new();
    for (t, tri) in data.tris.iter().enumerate() {
        if t.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        for k in 0..3 {
            let a = tri[k];
            let b = tri[(k + 1) % 3];
            let key = (a.min(b), a.max(b));
            let mid_node = data.mid.as_ref().map(|m| m[t][k]);
            edges.entry(key).or_insert(mid_node);
        }
    }

    let mut xs = Vec::with_capacity(edges.len() * 3);
    let mut ys = Vec::with_capacity(edges.len() * 3);
    let mut zs = Vec::with_capacity(if z_slice.is_some() {
        edges.len() * 3
    } else {
        0
    });

    for (idx, (&(a, b), &mid_opt)) in edges.iter().enumerate() {
        if idx.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        if idx > 0 {
            xs.push(f64::NAN);
            ys.push(f64::NAN);
            if z_slice.is_some() {
                zs.push(f64::NAN);
            }
        }
        let pa = data.nodes[a];
        let pb = data.nodes[b];
        if let Some(m) = mid_opt {
            let pm = data.nodes[m];
            let mx = 0.5 * (pa[0] + pb[0]);
            let my = 0.5 * (pa[1] + pb[1]);
            let len = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
            let dev = (pm[0] - mx).hypot(pm[1] - my);
            if z_slice.is_some() || dev <= 1e-12 * len.max(1.0) {
                xs.extend([pa[0], pm[0], pb[0]]);
                ys.extend([pa[1], pm[1], pb[1]]);
                if let Some(zv) = z_slice {
                    zs.extend([zv[a], zv[m], zv[b]]);
                }
            } else {
                for step in 0..=CURVED_EDGE_STEPS {
                    let t = step as f64 / CURVED_EDGE_STEPS as f64;
                    let l0 = (1.0 - t) * (1.0 - 2.0 * t);
                    let l1 = 4.0 * t * (1.0 - t);
                    let l2 = t * (2.0 * t - 1.0);
                    xs.push(l0 * pa[0] + l1 * pm[0] + l2 * pb[0]);
                    ys.push(l0 * pa[1] + l1 * pm[1] + l2 * pb[1]);
                }
            }
        } else {
            xs.extend([pa[0], pb[0]]);
            ys.extend([pa[1], pb[1]]);
            if let Some(zv) = z_slice {
                zs.extend([zv[a], zv[b]]);
            }
        }
    }

    Ok((xs, ys, zs))
}

fn push_handle(handles: &mut Vec<Arc<GraphicsHandle>>, v: Value) -> Result<()> {
    if let Value::Container(Array::Graphics(arr)) = v {
        for i in 0..arr.len() {
            handles.push(arr.get_linear(i)?.clone());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Value parsers

fn numeric_matrix(v: &Value, label: &str) -> Result<Matrix> {
    let m = match v {
        Value::Matrix(m) | Value::Container(Array::Numeric(m)) => m.clone(),
        Value::Logical(l) | Value::Container(Array::Logical(l)) => {
            let data = l.data().iter().map(|&b| f64::from(b)).collect();
            Matrix::from_column_vec(l.rows(), l.cols(), data)?
        }
        Value::Container(Array::Typed(a)) => a.to_matrix()?,
        _ => return Err(format!("{label} must be a real numeric matrix")),
    };
    if m.is_nd() || m.data().iter().any(|x| !x.is_finite()) {
        return Err(format!("{label} must be a finite real 2-D matrix"));
    }
    Ok(m)
}

fn parse_nodal_vector(v: &Value, n_nodes: usize, name: &str) -> Result<Option<Vec<f64>>> {
    let m = numeric_matrix(v, &format!("pdeplot: '{name}'"))?;
    if m.is_empty() {
        return Ok(None);
    }
    if (m.rows() != 1 && m.cols() != 1) || m.len() != n_nodes {
        return Err(format!(
            "pdeplot: '{name}' must be a numeric vector of length {n_nodes} (number of mesh nodes)"
        ));
    }
    Ok(Some(m.data().to_vec()))
}

fn parse_flow_data(v: &Value, n_nodes: usize) -> Result<Option<(Vec<f64>, Vec<f64>)>> {
    let m = numeric_matrix(v, "pdeplot: 'FlowData'")?;
    if m.is_empty() {
        return Ok(None);
    }
    if m.rows() == 2 && m.cols() == n_nodes {
        let mut fx = Vec::with_capacity(n_nodes);
        let mut fy = Vec::with_capacity(n_nodes);
        for j in 0..n_nodes {
            fx.push(m.get(0, j));
            fy.push(m.get(1, j));
        }
        Ok(Some((fx, fy)))
    } else if m.rows() == n_nodes && m.cols() == 2 {
        let mut fx = Vec::with_capacity(n_nodes);
        let mut fy = Vec::with_capacity(n_nodes);
        for i in 0..n_nodes {
            fx.push(m.get(i, 0));
            fy.push(m.get(i, 1));
        }
        Ok(Some((fx, fy)))
    } else {
        Err(format!(
            "pdeplot: 'FlowData' must be a 2-by-{n_nodes} or {n_nodes}-by-2 numeric matrix"
        ))
    }
}

fn parse_levels(v: &Value) -> Result<ContourLevels> {
    let m = numeric_matrix(v, "pdeplot: 'Levels'")?;
    if m.is_empty() || (m.rows() != 1 && m.cols() != 1) {
        return Err("pdeplot: 'Levels' must be a positive integer or a numeric vector".into());
    }
    if m.len() == 1 {
        let val = m.data()[0];
        if val >= 1.0 && val.fract() == 0.0 && val <= MAX_CONTOUR_LEVELS as f64 {
            return Ok(ContourLevels::Count(val as usize));
        }
        if val > MAX_CONTOUR_LEVELS as f64 && val.fract() == 0.0 {
            return Err(format!(
                "pdeplot: 'Levels' count must not exceed {MAX_CONTOUR_LEVELS}"
            ));
        }
        return Ok(ContourLevels::Values(vec![val]));
    }
    if m.len() > MAX_CONTOUR_LEVELS {
        return Err(format!(
            "pdeplot: 'Levels' vector must not exceed {MAX_CONTOUR_LEVELS} entries"
        ));
    }
    Ok(ContourLevels::Values(m.data().to_vec()))
}

fn parse_on_off(v: &Value, name: &str) -> Result<bool> {
    if let Some(s) = text_value(v) {
        if s.eq_ignore_ascii_case("on") {
            return Ok(true);
        }
        if s.eq_ignore_ascii_case("off") {
            return Ok(false);
        }
        return Err(format!(
            "pdeplot: '{name}' must be 'on', 'off', true, or false"
        ));
    }
    match v {
        Value::Logical(l) | Value::Container(Array::Logical(l)) if l.len() == 1 => {
            Ok(l.data()[0] != 0)
        }
        Value::Matrix(m) | Value::Container(Array::Numeric(m))
            if m.len() == 1 && (m.data()[0] == 0.0 || m.data()[0] == 1.0) =>
        {
            Ok(m.data()[0] != 0.0)
        }
        _ => Err(format!(
            "pdeplot: '{name}' must be 'on', 'off', true, or false"
        )),
    }
}

fn text_value(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
        Value::Container(Array::Character(c)) if c.rows() <= 1 => {
            Some(String::from_utf16_lossy(c.units()))
        }
        Value::Container(Array::Str(s)) if s.len() == 1 => {
            s.data()[0].as_deref().map(str::to_owned)
        }
        _ => None,
    }
}
