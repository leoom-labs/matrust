//! 2-D PDE finite-element mesh object (`pde.FEMesh`), mesh generation
//! (`generateMesh`), node/element queries (`findNodes`, `findElements`), and
//! mesh plotting (`pdemesh`).
use super::{
    Engine, Execution, Value, objects,
    pde_geometry::Geometry2,
    pde_mesher::{self, MeshOpts, TriMesh},
};
use crate::{Array, Matrix, Result, StructArray};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(crate) const MESH_CLASS: &str = "pde.FEMesh";

pub(crate) const MESH_PROPERTIES: [&str; 6] = [
    "Nodes",
    "Elements",
    "MaxElementSize",
    "MinElementSize",
    "MeshGradation",
    "GeometricOrder",
];

/// Number of sub-segments used to draw each curved quadratic triangle edge.
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
    let val = match name {
        "generateMesh" => generate_mesh(e, args)?,
        "findNodes" => find_nodes(e, args)?,
        "findElements" => find_elements(e, args)?,
        "pdemesh" => pdemesh(e, args, out)?,
        _ => return Err(format!("Unknown PDE mesh function {name}")),
    };
    Ok(if count == 0 { vec![] } else { vec![val] })
}

// ---------------------------------------------------------------------------
// pde.FEMesh (value semantics)

#[derive(Clone, Debug)]
pub(crate) struct FeMesh {
    pub(crate) nodes_mat: Matrix,
    pub(crate) elements_mat: Matrix,
    pub(crate) nodes: Vec<[f64; 2]>,
    pub(crate) tris: Vec<[usize; 3]>,
    pub(crate) mid: Option<Vec<[usize; 3]>>,
    pub(crate) edge_nodes: Vec<Vec<usize>>,
    pub(crate) face_nodes: Vec<Vec<usize>>,
    pub(crate) face_elements: Vec<Vec<usize>>,
    pub(crate) hmax: f64,
    pub(crate) hmin: f64,
    pub(crate) hgrad: f64,
    pub(crate) quadratic: bool,
}

impl PartialEq for FeMesh {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes
            && self.tris == other.tris
            && self.mid == other.mid
            && self.edge_nodes == other.edge_nodes
            && self.face_nodes == other.face_nodes
            && self.face_elements == other.face_elements
            && self.hmax == other.hmax
            && self.hmin == other.hmin
            && self.hgrad == other.hgrad
            && self.quadratic == other.quadratic
    }
}

impl FeMesh {
    pub(crate) fn from_tri_mesh(geom: &Geometry2, tri: TriMesh, opts: MeshOpts) -> Result<Self> {
        let n_nodes = tri.nodes.len();
        let n_tris = tri.tris.len();

        // Build 2 x N Nodes matrix (column-major: [x0, y0, x1, y1, ...]).
        let mut nodes_data = Vec::with_capacity(n_nodes * 2);
        for p in &tri.nodes {
            nodes_data.push(p[0]);
            nodes_data.push(p[1]);
        }
        let nodes_mat = Matrix::from_column_vec(2, n_nodes, nodes_data)?;

        // Build 3 x M or 6 x M Elements matrix (1-based indices).
        let rows = if tri.mid.is_some() { 6 } else { 3 };
        let mut elems_data = Vec::with_capacity(n_tris * rows);
        match &tri.mid {
            Some(mids) => {
                for (t, m) in tri.tris.iter().zip(mids.iter()) {
                    elems_data.push((t[0] + 1) as f64);
                    elems_data.push((t[1] + 1) as f64);
                    elems_data.push((t[2] + 1) as f64);
                    elems_data.push((m[0] + 1) as f64);
                    elems_data.push((m[1] + 1) as f64);
                    elems_data.push((m[2] + 1) as f64);
                }
            }
            None => {
                for t in &tri.tris {
                    elems_data.push((t[0] + 1) as f64);
                    elems_data.push((t[1] + 1) as f64);
                    elems_data.push((t[2] + 1) as f64);
                }
            }
        }
        let elements_mat = Matrix::from_column_vec(rows, n_tris, elems_data)?;

        // Precompute sorted 1-based node IDs per geometry edge.
        // Note: TriMesh::node_edge records only the lowest-numbered edge for a
        // geometry corner vertex, so for vertex nodes we also match against all
        // geometry edge endpoints so every incident edge includes its corners.
        let num_edges = geom.edges.len();
        let mut scale = 0.0f64;
        for edge in &geom.edges {
            for p in [edge.start, edge.end] {
                scale = scale.max(p[0].abs()).max(p[1].abs());
            }
        }
        let tol = 1e-12 * scale.max(1.0);
        let mut edge_nodes: Vec<Vec<usize>> = vec![Vec::new(); num_edges];
        for (i, &p) in tri.nodes.iter().enumerate() {
            if let Some(e1) = tri.node_edge[i]
                && e1 >= 1
                && e1 <= num_edges
            {
                edge_nodes[e1 - 1].push(i + 1);
            }
            if tri.node_vertex[i] {
                for (j, edge) in geom.edges.iter().enumerate() {
                    let at_start =
                        (p[0] - edge.start[0]).abs() <= tol && (p[1] - edge.start[1]).abs() <= tol;
                    let at_end =
                        (p[0] - edge.end[0]).abs() <= tol && (p[1] - edge.end[1]).abs() <= tol;
                    if at_start || at_end {
                        edge_nodes[j].push(i + 1);
                    }
                }
            }
        }
        for list in &mut edge_nodes {
            list.sort_unstable();
            list.dedup();
        }

        // Precompute sorted 1-based element IDs and node IDs per face.
        let num_faces = geom.num_faces;
        let mut face_elements: Vec<Vec<usize>> = vec![Vec::new(); num_faces];
        let mut face_nodes: Vec<Vec<usize>> = vec![Vec::new(); num_faces];
        for (t, &f1) in tri.tri_face.iter().enumerate() {
            if f1 >= 1 && f1 <= num_faces {
                let fi = f1 - 1;
                face_elements[fi].push(t + 1);
                for &n in &tri.tris[t] {
                    face_nodes[fi].push(n + 1);
                }
                if let Some(mids) = &tri.mid {
                    for &n in &mids[t] {
                        face_nodes[fi].push(n + 1);
                    }
                }
            }
        }
        for list in &mut face_nodes {
            list.sort_unstable();
            list.dedup();
        }

        Ok(Self {
            nodes_mat,
            elements_mat,
            nodes: tri.nodes,
            tris: tri.tris,
            mid: tri.mid,
            edge_nodes,
            face_nodes,
            face_elements,
            hmax: opts.hmax,
            hmin: opts.hmin,
            hgrad: opts.hgrad,
            quadratic: opts.quadratic,
        })
    }

    pub(crate) fn bytes(&self) -> usize {
        let vecs = self.nodes.len() * std::mem::size_of::<[f64; 2]>()
            + self.tris.len() * std::mem::size_of::<[usize; 3]>()
            + self
                .mid
                .as_ref()
                .map_or(0, |m| m.len() * std::mem::size_of::<[usize; 3]>())
            + self
                .edge_nodes
                .iter()
                .map(|v| v.len() * std::mem::size_of::<usize>())
                .sum::<usize>()
            + self
                .face_nodes
                .iter()
                .map(|v| v.len() * std::mem::size_of::<usize>())
                .sum::<usize>()
            + self
                .face_elements
                .iter()
                .map(|v| v.len() * std::mem::size_of::<usize>())
                .sum::<usize>();
        std::mem::size_of::<Self>()
            + self.nodes_mat.len() * std::mem::size_of::<f64>()
            + self.elements_mat.len() * std::mem::size_of::<f64>()
            + vecs
    }

    pub(crate) fn order_str(&self) -> &'static str {
        if self.quadratic {
            "quadratic"
        } else {
            "linear"
        }
    }

    pub(crate) fn property(&self, name: &str) -> Result<Value> {
        match name {
            "Nodes" => Ok(Value::Matrix(self.nodes_mat.clone())),
            "Elements" => Ok(Value::Matrix(self.elements_mat.clone())),
            "MaxElementSize" => Ok(Value::Matrix(Matrix::scalar(self.hmax))),
            "MinElementSize" => Ok(Value::Matrix(Matrix::scalar(self.hmin))),
            "MeshGradation" => Ok(Value::Matrix(Matrix::scalar(self.hgrad))),
            "GeometricOrder" => Ok(Value::Text(self.order_str().into())),
            _ => Err(format!(
                "No appropriate method, property, or field '{name}' for class '{MESH_CLASS}'."
            )),
        }
    }

    pub(crate) fn property_ci(&self, name: &str) -> Result<Value> {
        if let Some(&canonical) = MESH_PROPERTIES
            .iter()
            .find(|p| p.eq_ignore_ascii_case(name))
        {
            self.property(canonical)
        } else {
            self.property(name)
        }
    }

    pub(crate) fn get_method(&self, e: &Engine, rest: &[Value]) -> Result<Value> {
        match rest {
            [] => {
                let mut fields = Vec::with_capacity(MESH_PROPERTIES.len());
                for &prop in &MESH_PROPERTIES {
                    let val = self.property(prop)?.array(&e.cancel)?;
                    fields.push((prop.to_owned(), Arc::new(val)));
                }
                let names = fields.iter().map(|(n, _)| n.clone()).collect();
                let slots = fields.into_iter().map(|(_, v)| v).collect();
                Ok(Value::Container(Array::Struct(StructArray::from_slots(
                    1, 1, names, slots, &e.cancel,
                )?)))
            }
            [prop] => {
                let name = text_value(prop).ok_or("pde.FEMesh: property name must be text")?;
                self.property_ci(&name)
            }
            _ => Err("pde.FEMesh: get expects zero or one property name".into()),
        }
    }

    pub(crate) fn body(&self) -> String {
        let elem_rows = if self.quadratic { 6 } else { 3 };
        format!(
            "  FEMesh with properties:\n\n\
             {:>18}: [2x{} double]\n\
             {:>18}: [{}x{} double]\n\
             {:>18}: {}\n\
             {:>18}: {}\n\
             {:>18}: {}\n\
             {:>18}: '{}'\n",
            "Nodes",
            self.nodes.len(),
            "Elements",
            elem_rows,
            self.tris.len(),
            "MaxElementSize",
            self.hmax,
            "MinElementSize",
            self.hmin,
            "MeshGradation",
            self.hgrad,
            "GeometricOrder",
            self.order_str(),
        )
    }

    /// Iterate all 0-based node indices belonging to element `t` (3 for linear, 6 for quadratic).
    fn elem_nodes(&self, t: usize) -> ([usize; 6], usize) {
        let c = self.tris[t];
        match &self.mid {
            Some(mids) => {
                let m = mids[t];
                ([c[0], c[1], c[2], m[0], m[1], m[2]], 6)
            }
            None => ([c[0], c[1], c[2], 0, 0, 0], 3),
        }
    }
}

// ---------------------------------------------------------------------------
// generateMesh

fn geometry_bbox_diagonal(g: &Geometry2) -> f64 {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for edge in &g.edges {
        for k in 0..=16 {
            let p = edge.point_at(f64::from(k) / 16.0);
            lo[0] = lo[0].min(p[0]);
            lo[1] = lo[1].min(p[1]);
            hi[0] = hi[0].max(p[0]);
            hi[1] = hi[1].max(p[1]);
        }
    }
    if !(lo[0].is_finite() && lo[1].is_finite() && hi[0].is_finite() && hi[1].is_finite()) {
        return 0.0;
    }
    (hi[0] - lo[0]).hypot(hi[1] - lo[1])
}

fn generate_mesh(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Err("generateMesh: expected generateMesh(model, ...)".into());
    }
    let model_id = objects::pde_model_id(e, &args[0], "generateMesh")?;
    let analytic = objects::pde_model_get(e, model_id)
        .ok_or("generateMesh: invalid or deleted pde.PDEModel")?
        .geometry
        .as_ref()
        .map(|(g, _)| g.clone())
        .ok_or("generateMesh: model has no geometry; call geometryFromEdges first")?;

    let opt_args = &args[1..];
    if !opt_args.len().is_multiple_of(2) {
        return Err("generateMesh: options must be name-value pairs".into());
    }

    let mut hmax_opt: Option<f64> = None;
    let mut hmin_opt: Option<f64> = None;
    let mut hgrad: f64 = 1.5;
    let mut quadratic: bool = true;

    for pair in opt_args.as_chunks::<2>().0 {
        let name = text_value(&pair[0])
            .ok_or("generateMesh: option names must be character vectors or strings")?;
        if name.eq_ignore_ascii_case("Hmax") {
            let v = finite_scalar(&pair[1], "generateMesh: Hmax")?;
            if v <= 0.0 {
                return Err("generateMesh: Hmax must be a positive finite scalar".into());
            }
            hmax_opt = Some(v);
        } else if name.eq_ignore_ascii_case("Hmin") {
            let v = finite_scalar(&pair[1], "generateMesh: Hmin")?;
            if v <= 0.0 {
                return Err("generateMesh: Hmin must be a positive finite scalar".into());
            }
            hmin_opt = Some(v);
        } else if name.eq_ignore_ascii_case("Hgrad") {
            let v = finite_scalar(&pair[1], "generateMesh: Hgrad")?;
            if !(v > 1.0 && v < 2.0) {
                return Err("generateMesh: Hgrad must satisfy 1 < Hgrad < 2".into());
            }
            hgrad = v;
        } else if name.eq_ignore_ascii_case("GeometricOrder") {
            let s = text_value(&pair[1])
                .ok_or("generateMesh: GeometricOrder must be 'linear' or 'quadratic'")?;
            if s.eq_ignore_ascii_case("linear") {
                quadratic = false;
            } else if s.eq_ignore_ascii_case("quadratic") {
                quadratic = true;
            } else {
                return Err("generateMesh: GeometricOrder must be 'linear' or 'quadratic'".into());
            }
        } else {
            return Err(format!(
                "generateMesh: unknown option '{name}' (expected 'Hmax', 'Hmin', 'Hgrad', or 'GeometricOrder')"
            ));
        }
    }

    let hmax = match hmax_opt {
        Some(v) => v,
        None => {
            let diag = geometry_bbox_diagonal(&analytic.geom);
            if !(diag.is_finite() && diag > 0.0) {
                return Err(
                    "generateMesh: cannot determine default Hmax for degenerate geometry".into(),
                );
            }
            diag / 10.0
        }
    };
    let hmin = hmin_opt.unwrap_or(hmax / 4.0);
    if hmin > hmax {
        return Err("generateMesh: Hmin must be less than or equal to Hmax".into());
    }

    let opts = MeshOpts {
        hmax,
        hmin,
        hgrad,
        quadratic,
    };
    let tri =
        pde_mesher::mesh(&analytic.geom, &opts).map_err(|msg| format!("generateMesh: {msg}"))?;
    let fe_mesh = FeMesh::from_tri_mesh(&analytic.geom, tri, opts)?;
    let mesh_val = objects::pde_mesh_new(e, Arc::new(fe_mesh))?;
    let mesh_arr = Arc::new(mesh_val.clone().array(&e.cancel)?);
    objects::pde_model_update(e, model_id, |m| {
        m.mesh = Some(mesh_arr);
        Ok(())
    })?;
    Ok(mesh_val)
}

// ---------------------------------------------------------------------------
// findNodes

fn find_nodes(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err("findNodes: expected findNodes(mesh, queryType, ...)".into());
    }
    let mesh = objects::pde_mesh_value(e, &args[0])
        .ok_or("findNodes: first argument must be a pde.FEMesh object")?;
    let query = text_value(&args[1])
        .ok_or("findNodes: second argument must be 'region', 'box', 'radius', or 'nearest'")?;

    if query.eq_ignore_ascii_case("region") {
        if args.len() != 4 {
            return Err(
                "findNodes: 'region' requires findNodes(mesh, 'region', 'Edge'|'Face', ids)".into(),
            );
        }
        let kind = text_value(&args[2]).ok_or("findNodes: region type must be 'Edge' or 'Face'")?;
        if kind.eq_ignore_ascii_case("Edge") {
            let ids = positive_int_ids(&args[3], mesh.edge_nodes.len(), "findNodes: Edge ids")?;
            let mut out = Vec::new();
            for id in ids {
                out.extend_from_slice(&mesh.edge_nodes[id - 1]);
            }
            out.sort_unstable();
            out.dedup();
            return ids_row_vector(&out);
        } else if kind.eq_ignore_ascii_case("Face") {
            let ids = positive_int_ids(&args[3], mesh.face_nodes.len(), "findNodes: Face ids")?;
            let mut out = Vec::new();
            for id in ids {
                out.extend_from_slice(&mesh.face_nodes[id - 1]);
            }
            out.sort_unstable();
            out.dedup();
            return ids_row_vector(&out);
        } else {
            return Err(format!(
                "findNodes: unsupported region type '{kind}' (expected 'Edge' or 'Face')"
            ));
        }
    }

    if query.eq_ignore_ascii_case("box") {
        if args.len() != 4 {
            return Err("findNodes: 'box' requires findNodes(mesh, 'box', xlim, ylim)".into());
        }
        let xlim = limit_pair(&args[2], "findNodes: xlim")?;
        let ylim = limit_pair(&args[3], "findNodes: ylim")?;
        let mut out = Vec::new();
        for (i, p) in mesh.nodes.iter().enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            if p[0] >= xlim[0] && p[0] <= xlim[1] && p[1] >= ylim[0] && p[1] <= ylim[1] {
                out.push(i + 1);
            }
        }
        return ids_row_vector(&out);
    }

    if query.eq_ignore_ascii_case("radius") {
        if args.len() != 4 {
            return Err("findNodes: 'radius' requires findNodes(mesh, 'radius', center, r)".into());
        }
        let center = point_pair(&args[2], "findNodes: center")?;
        let r = finite_scalar(&args[3], "findNodes: radius")?;
        if r < 0.0 {
            return Err("findNodes: radius must be nonnegative".into());
        }
        let mut out = Vec::new();
        for (i, p) in mesh.nodes.iter().enumerate() {
            if i.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            if (p[0] - center[0]).hypot(p[1] - center[1]) <= r {
                out.push(i + 1);
            }
        }
        return ids_row_vector(&out);
    }

    if query.eq_ignore_ascii_case("nearest") {
        if args.len() != 3 {
            return Err("findNodes: 'nearest' requires findNodes(mesh, 'nearest', points)".into());
        }
        let pts = numeric_matrix(&args[2], "findNodes: points")?;
        if pts.is_empty() && (pts.rows() == 0 || pts.rows() == 2) {
            return ids_row_vector(&[]);
        }
        if pts.rows() != 2 {
            return Err("findNodes: points must be a 2-by-P numeric matrix".into());
        }
        if mesh.nodes.is_empty() {
            return Err("findNodes: mesh has no nodes".into());
        }
        let mut out = Vec::with_capacity(pts.cols());
        for col in 0..pts.cols() {
            e.check_cancel()?;
            let qx = pts.get(0, col);
            let qy = pts.get(1, col);
            let mut best_idx = 0usize;
            let mut best_d2 = f64::INFINITY;
            for (i, p) in mesh.nodes.iter().enumerate() {
                let dx = p[0] - qx;
                let dy = p[1] - qy;
                let d2 = dx * dx + dy * dy;
                if d2 < best_d2 {
                    best_d2 = d2;
                    best_idx = i;
                }
            }
            out.push(best_idx + 1);
        }
        return ids_row_vector(&out);
    }

    Err(format!(
        "findNodes: unknown query type '{query}' (expected 'region', 'box', 'radius', or 'nearest')"
    ))
}

// ---------------------------------------------------------------------------
// findElements

fn find_elements(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 2 {
        return Err("findElements: expected findElements(mesh, queryType, ...)".into());
    }
    let mesh = objects::pde_mesh_value(e, &args[0])
        .ok_or("findElements: first argument must be a pde.FEMesh object")?;
    let query = text_value(&args[1])
        .ok_or("findElements: second argument must be 'region', 'box', 'radius', or 'attached'")?;

    if query.eq_ignore_ascii_case("region") {
        if args.len() != 4 {
            return Err(
                "findElements: 'region' requires findElements(mesh, 'region', 'Face', ids)".into(),
            );
        }
        let kind = text_value(&args[2]).ok_or("findElements: region type must be 'Face'")?;
        if !kind.eq_ignore_ascii_case("Face") {
            return Err(format!(
                "findElements: unsupported region type '{kind}' (expected 'Face')"
            ));
        }
        let ids = positive_int_ids(&args[3], mesh.face_elements.len(), "findElements: Face ids")?;
        let mut out = Vec::new();
        for id in ids {
            out.extend_from_slice(&mesh.face_elements[id - 1]);
        }
        out.sort_unstable();
        out.dedup();
        return ids_row_vector(&out);
    }

    if query.eq_ignore_ascii_case("box") {
        if args.len() != 4 {
            return Err(
                "findElements: 'box' requires findElements(mesh, 'box', xlim, ylim)".into(),
            );
        }
        let xlim = limit_pair(&args[2], "findElements: xlim")?;
        let ylim = limit_pair(&args[3], "findElements: ylim")?;
        let mut out = Vec::new();
        for t in 0..mesh.tris.len() {
            if t.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let (idxs, k) = mesh.elem_nodes(t);
            let all_inside = idxs[..k].iter().all(|&ni| {
                let p = mesh.nodes[ni];
                p[0] >= xlim[0] && p[0] <= xlim[1] && p[1] >= ylim[0] && p[1] <= ylim[1]
            });
            if all_inside {
                out.push(t + 1);
            }
        }
        return ids_row_vector(&out);
    }

    if query.eq_ignore_ascii_case("radius") {
        if args.len() != 4 {
            return Err(
                "findElements: 'radius' requires findElements(mesh, 'radius', center, r)".into(),
            );
        }
        let center = point_pair(&args[2], "findElements: center")?;
        let r = finite_scalar(&args[3], "findElements: radius")?;
        if r < 0.0 {
            return Err("findElements: radius must be nonnegative".into());
        }
        let mut out = Vec::new();
        for t in 0..mesh.tris.len() {
            if t.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let (idxs, k) = mesh.elem_nodes(t);
            let all_inside = idxs[..k].iter().all(|&ni| {
                let p = mesh.nodes[ni];
                (p[0] - center[0]).hypot(p[1] - center[1]) <= r
            });
            if all_inside {
                out.push(t + 1);
            }
        }
        return ids_row_vector(&out);
    }

    if query.eq_ignore_ascii_case("attached") {
        if args.len() != 3 {
            return Err(
                "findElements: 'attached' requires findElements(mesh, 'attached', nodeIDs)".into(),
            );
        }
        let ids = positive_int_ids(&args[2], mesh.nodes.len(), "findElements: nodeIDs")?;
        if ids.is_empty() {
            return ids_row_vector(&[]);
        }
        let mut mask = vec![false; mesh.nodes.len()];
        for id in ids {
            mask[id - 1] = true;
        }
        let mut out = Vec::new();
        for t in 0..mesh.tris.len() {
            if t.is_multiple_of(4096) {
                e.check_cancel()?;
            }
            let (idxs, k) = mesh.elem_nodes(t);
            if idxs[..k].iter().any(|&ni| mask[ni]) {
                out.push(t + 1);
            }
        }
        return ids_row_vector(&out);
    }

    Err(format!(
        "findElements: unknown query type '{query}' (expected 'region', 'box', 'radius', or 'attached')"
    ))
}

// ---------------------------------------------------------------------------
// pdemesh

#[derive(Default)]
struct MeshPlotOptions {
    node_labels: bool,
    element_labels: bool,
}

fn parse_mesh_plot_options(args: &[Value]) -> Result<MeshPlotOptions> {
    if !args.len().is_multiple_of(2) {
        return Err("pdemesh: options must be name-value pairs".into());
    }
    let mut opts = MeshPlotOptions::default();
    for pair in args.as_chunks::<2>().0 {
        let name = text_value(&pair[0])
            .ok_or("pdemesh: option names must be character vectors or strings")?;
        if name.eq_ignore_ascii_case("NodeLabels") {
            opts.node_labels = parse_on_off(&pair[1], "NodeLabels")?;
        } else if name.eq_ignore_ascii_case("ElementLabels") {
            opts.element_labels = parse_on_off(&pair[1], "ElementLabels")?;
        } else {
            return Err(format!(
                "pdemesh: unknown option '{name}' (expected 'NodeLabels' or 'ElementLabels')"
            ));
        }
    }
    Ok(opts)
}

struct PlotMeshData {
    nodes: Vec<[f64; 2]>,
    tris: Vec<[usize; 3]>,
    mid: Option<Vec<[usize; 3]>>,
}

fn extract_mesh_for_plot<'a>(e: &Engine, args: &'a [Value]) -> Result<(PlotMeshData, &'a [Value])> {
    if args.is_empty() {
        return Err("pdemesh: expected pdemesh(model | mesh | nodes, elements, ...)".into());
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
        let id = objects::pde_model_id(e, &args[0], "pdemesh")?;
        let model =
            objects::pde_model_get(e, id).ok_or("pdemesh: invalid or deleted pde.PDEModel")?;
        let mesh_arr = model
            .mesh
            .as_ref()
            .ok_or("pdemesh: model has no mesh; call generateMesh first")?;
        let mesh_val = Value::Container(mesh_arr.as_ref().clone());
        let m = objects::pde_mesh_value(e, &mesh_val)
            .ok_or("pdemesh: model.Mesh is not a valid pde.FEMesh")?;
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
        return Err("pdemesh: expected pdemesh(model | mesh | nodes, elements, ...)".into());
    }
    let nodes_mat = numeric_matrix(&args[0], "pdemesh: nodes")?;
    let elems_mat = numeric_matrix(&args[1], "pdemesh: elements")?;
    if nodes_mat.rows() != 2 {
        return Err("pdemesh: nodes must be a 2-by-N numeric matrix".into());
    }
    let n_nodes = nodes_mat.cols();
    if elems_mat.rows() != 3 && elems_mat.rows() != 6 {
        return Err("pdemesh: elements must be a 3-by-M or 6-by-M index matrix".into());
    }
    let mut nodes = Vec::with_capacity(n_nodes);
    for j in 0..n_nodes {
        nodes.push([nodes_mat.get(0, j), nodes_mat.get(1, j)]);
    }
    let n_elems = elems_mat.cols();
    let is_quad = elems_mat.rows() == 6;
    let mut tris = Vec::with_capacity(n_elems);
    let mut mids = is_quad.then(|| Vec::with_capacity(n_elems));
    let check_idx = |v: f64| -> Result<usize> {
        if v.fract() != 0.0 || v < 1.0 || v > n_nodes as f64 {
            return Err(format!(
                "pdemesh: element node index must be an integer in 1..={n_nodes}"
            ));
        }
        Ok(v as usize - 1)
    };
    for j in 0..n_elems {
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

fn pdemesh(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Value> {
    let (data, opt_args) = extract_mesh_for_plot(e, args)?;
    let opts = parse_mesh_plot_options(opt_args)?;

    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let res = draw_mesh(e, &data, &opts, out);
    if res.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    res
}

fn draw_mesh(
    e: &mut Engine,
    data: &PlotMeshData,
    opts: &MeshPlotOptions,
    out: &mut Execution,
) -> Result<Value> {
    // Collect unique edges (canonical (min, max) -> optional midside node).
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
    for (idx, (&(a, b), &mid_opt)) in edges.iter().enumerate() {
        if idx.is_multiple_of(4096) {
            e.check_cancel()?;
        }
        if idx > 0 {
            xs.push(f64::NAN);
            ys.push(f64::NAN);
        }
        let pa = data.nodes[a];
        let pb = data.nodes[b];
        if let Some(m) = mid_opt {
            let pm = data.nodes[m];
            let mx = 0.5 * (pa[0] + pb[0]);
            let my = 0.5 * (pa[1] + pb[1]);
            let len = (pb[0] - pa[0]).hypot(pb[1] - pa[1]);
            let dev = (pm[0] - mx).hypot(pm[1] - my);
            if dev <= 1e-12 * len.max(1.0) {
                xs.push(pa[0]);
                ys.push(pa[1]);
                xs.push(pm[0]);
                ys.push(pm[1]);
                xs.push(pb[0]);
                ys.push(pb[1]);
            } else {
                // Quadratic Lagrange curve passing through pa (t=0), pm (t=0.5), pb (t=1).
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
            xs.push(pa[0]);
            ys.push(pa[1]);
            xs.push(pb[0]);
            ys.push(pb[1]);
        }
    }

    let ax = e.call("gca", vec![], out)?;
    let next = e.call("get", vec![ax.clone(), Value::Text("NextPlot".into())], out)?;
    let held = text_value(&next).as_deref() == Some("add");

    let edge_color = Value::Matrix(Matrix::from_column_vec(1, 3, vec![0.0, 0.4470, 0.7410])?);
    let x_val = Value::Matrix(Matrix::from_column_vec(xs.len(), 1, xs)?);
    let y_val = Value::Matrix(Matrix::from_column_vec(ys.len(), 1, ys)?);

    let mut line_handle: Option<Value> = None;
    let draw_result = (|| -> Result<()> {
        let h = e.call(
            "plot",
            vec![
                ax.clone(),
                x_val,
                y_val,
                Value::Text("Color".into()),
                edge_color,
            ],
            out,
        )?;
        line_handle = Some(h);

        if opts.node_labels || opts.element_labels {
            e.call("hold", vec![ax.clone(), Value::Text("on".into())], out)?;
        }

        if opts.node_labels {
            for (i, p) in data.nodes.iter().enumerate() {
                if i.is_multiple_of(256) {
                    e.check_cancel()?;
                }
                e.call(
                    "text",
                    vec![
                        ax.clone(),
                        Value::Matrix(Matrix::scalar(p[0])),
                        Value::Matrix(Matrix::scalar(p[1])),
                        Value::Text(format!("{}", i + 1)),
                    ],
                    out,
                )?;
            }
        }

        if opts.element_labels {
            for (t, tri) in data.tris.iter().enumerate() {
                if t.is_multiple_of(256) {
                    e.check_cancel()?;
                }
                let p0 = data.nodes[tri[0]];
                let p1 = data.nodes[tri[1]];
                let p2 = data.nodes[tri[2]];
                let cx = (p0[0] + p1[0] + p2[0]) / 3.0;
                let cy = (p0[1] + p1[1] + p2[1]) / 3.0;
                e.call(
                    "text",
                    vec![
                        ax.clone(),
                        Value::Matrix(Matrix::scalar(cx)),
                        Value::Matrix(Matrix::scalar(cy)),
                        Value::Text(format!("{}", t + 1)),
                    ],
                    out,
                )?;
            }
        }

        e.call("axes", vec![ax.clone()], out)?;
        e.call("axis", vec![Value::Text("equal".into())], out)?;
        Ok(())
    })();

    let restore = e.call(
        "hold",
        vec![ax, Value::Text(if held { "on" } else { "off" }.into())],
        out,
    );
    draw_result?;
    restore?;

    Ok(line_handle.unwrap_or(Value::Void))
}

// ---------------------------------------------------------------------------
// Value helpers

fn ids_row_vector(ids: &[usize]) -> Result<Value> {
    let data: Vec<f64> = ids.iter().map(|&id| id as f64).collect();
    Ok(Value::Matrix(Matrix::from_column_vec(1, ids.len(), data)?))
}

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

fn finite_scalar(v: &Value, label: &str) -> Result<f64> {
    let m = numeric_matrix(v, label)?;
    if m.len() != 1 {
        return Err(format!("{label} must be a finite real scalar"));
    }
    Ok(m.data()[0])
}

fn limit_pair(v: &Value, label: &str) -> Result<[f64; 2]> {
    let m = numeric_matrix(v, label)?;
    if m.len() != 2 {
        return Err(format!("{label} must be a 2-element finite real vector"));
    }
    let a = m.data()[0];
    let b = m.data()[1];
    if a > b {
        return Err(format!("{label}: lower bound must not exceed upper bound"));
    }
    Ok([a, b])
}

fn point_pair(v: &Value, label: &str) -> Result<[f64; 2]> {
    let m = numeric_matrix(v, label)?;
    if m.len() != 2 {
        return Err(format!("{label} must be a 2-element finite real vector"));
    }
    Ok([m.data()[0], m.data()[1]])
}

fn positive_int_ids(v: &Value, max_id: usize, label: &str) -> Result<Vec<usize>> {
    let m = numeric_matrix(v, label)?;
    let mut ids = Vec::with_capacity(m.len());
    for &x in m.data() {
        if x.fract() != 0.0 || x < 1.0 || x > max_id as f64 {
            return Err(format!(
                "{label}: every ID must be an integer between 1 and {max_id}"
            ));
        }
        ids.push(x as usize);
    }
    Ok(ids)
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
            "pdemesh: '{name}' must be 'on', 'off', true, or false"
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
            "pdemesh: '{name}' must be 'on', 'off', true, or false"
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
