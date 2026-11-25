use matrust::{
    Engine,
    engine::{MeshOpts, TriMesh, pde_model::attach_tri_mesh_for_test},
};

pub(super) const UNIT_DISK: &str = "C1 = [1; 0; 0; 1]; dl = decsg(C1);\n";

pub(super) fn disk_model(nodes: &[Vec<f64>], elements: &[Vec<usize>]) -> Engine {
    let mut engine = Engine::default();
    engine
        .execute(&format!(
            "{UNIT_DISK}m = createpde(); geometryFromEdges(m, dl);"
        ))
        .unwrap();

    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].len(), nodes[1].len());
    let nodes: Vec<_> = nodes[0]
        .iter()
        .zip(&nodes[1])
        .map(|(&x, &y)| [x, y])
        .collect();
    assert!(matches!(elements.len(), 3 | 6));
    let n_elems = elements[0].len();
    assert!(elements.iter().all(|row| row.len() == n_elems));
    let triples = |row: usize| {
        (0..n_elems)
            .map(|i| std::array::from_fn(|j| elements[row + j][i] - 1))
            .collect()
    };
    let quadratic = elements.len() == 6;

    // The disk's four counterclockwise arcs start at (1, 0). Boundary
    // midsides also lie on the circle. Mark the axis vertices so FEMesh
    // includes each in both incident arcs.
    let node_edge = nodes
        .iter()
        .map(|&[x, y]| {
            ((x * x + y * y - 1.0).abs() < 1e-12).then_some(match (x >= 0.0, y >= 0.0) {
                (true, true) => 1,
                (false, true) => 2,
                (false, false) => 3,
                (true, false) => 4,
            })
        })
        .collect();
    let node_vertex = nodes
        .iter()
        .map(|&[x, y]| (x.abs() == 1.0 && y == 0.0) || (x == 0.0 && y.abs() == 1.0))
        .collect();
    let tri = TriMesh {
        nodes,
        tris: triples(0),
        mid: quadratic.then(|| triples(3)),
        tri_face: vec![1; n_elems],
        node_edge,
        node_vertex,
    };
    let opts = MeshOpts {
        hmax: 0.2,
        hmin: 0.05,
        hgrad: 1.5,
        quadratic,
    };
    attach_tri_mesh_for_test(&mut engine, "m", tri, opts).unwrap();
    engine
}
