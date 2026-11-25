//! Triangle mesher core (internal test hook `pde_mesh_from_dl`). Expected
//! values are plain geometry computed here independently of the engine:
//! triangle areas and angles from the node coordinates, distances to the
//! analytic lines, circles and ellipses of each dl column, region areas by
//! Green's theorem and point-in-region by ray casting.
use matrust::engine::{MeshOpts, TriMesh, pde_mesh_from_dl};
use matrust::{Engine, Matrix};
use std::f64::consts::PI;

const SQUARE_MINUS_DISK: &str = "R1=[3;4;-1;1;1;-1;-1;-1;1;1]; C1=[1;0;0;0.5;0;0;0;0;0;0]; dl=decsg([R1 C1],'R1-C1',['R1';'C1']');";
const TWO_RECTANGLES: &str = "R1=[3;4;0;2;2;0;0;0;1;1]; R2=[3;4;1;3;3;1;0.5;0.5;1.5;1.5]; dl=decsg([R1 R2],'R1+R2',['R1';'R2']');";
const L_SHAPE: &str = "P1=[2;6;0;2;2;1;1;0;0;0;1;1;2;2]; dl=decsg(P1,'P1',['P1']');";
const ELLIPSE: &str = "E1=[4;0.2;-0.1;2;1;pi/6]; dl=decsg(E1,'E1',['E1']');";

fn dl_of(src: &str) -> Matrix {
    let mut e = Engine::default();
    if let Err(err) = e.execute(src) {
        panic!("{src}\n{err}");
    }
    e.variables["dl"].clone()
}

fn mesh(dl: &Matrix, opts: MeshOpts) -> TriMesh {
    pde_mesh_from_dl(dl, &opts).unwrap_or_else(|e| panic!("mesh failed: {e}"))
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn cross(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn len(a: [f64; 2]) -> f64 {
    a[0].hypot(a[1])
}

/// Smallest angle of a triangle in degrees.
fn min_angle(p: [[f64; 2]; 3]) -> f64 {
    (0..3)
        .map(|i| {
            let (u, v) = (sub(p[(i + 1) % 3], p[i]), sub(p[(i + 2) % 3], p[i]));
            cross(u, v)
                .abs()
                .atan2(u[0] * v[0] + u[1] * v[1])
                .to_degrees()
        })
        .fold(f64::INFINITY, f64::min)
}

fn corners(m: &TriMesh, t: usize) -> [[f64; 2]; 3] {
    m.tris[t].map(|v| m.nodes[v])
}

/// Distance of p from the analytic carrier curve of dl column j.
fn curve_distance(dl: &Matrix, j: usize, p: [f64; 2]) -> f64 {
    let (x0, x1, y0, y1) = (dl.get(1, j), dl.get(2, j), dl.get(3, j), dl.get(4, j));
    match dl.get(0, j) as i32 {
        2 => cross(sub([x1, y1], [x0, y0]), sub(p, [x0, y0])).abs() / len(sub([x1, y1], [x0, y0])),
        1 => (len(sub(p, [dl.get(7, j), dl.get(8, j)])) - dl.get(9, j)).abs(),
        4 => {
            let (xc, yc, a, b, phi) = (
                dl.get(7, j),
                dl.get(8, j),
                dl.get(9, j),
                dl.get(10, j),
                dl.get(11, j),
            );
            let (dx, dy) = (p[0] - xc, p[1] - yc);
            let (u, v) = (
                phi.cos() * dx + phi.sin() * dy,
                -phi.sin() * dx + phi.cos() * dy,
            );
            let g = (u / a).powi(2) + (v / b).powi(2) - 1.;
            g.abs() / (2. * u / (a * a)).hypot(2. * v / (b * b))
        }
        k => panic!("segment type {k}"),
    }
}

/// Area enclosed by the dl boundary of every face (Green's theorem, lines only).
fn polygon_face_area(dl: &Matrix, f: usize) -> f64 {
    (0..dl.cols())
        .map(|j| {
            let t = (dl.get(1, j) * dl.get(4, j) - dl.get(2, j) * dl.get(3, j)) / 2.;
            let l = dl.get(5, j) as usize;
            let r = dl.get(6, j) as usize;
            (if l == f { t } else { 0. }) - (if r == f { t } else { 0. })
        })
        .sum()
}

/// Ray casting against the line columns bounding face f.
fn in_polygon_face(dl: &Matrix, f: usize, p: [f64; 2]) -> bool {
    let mut inside = false;
    for j in 0..dl.cols() {
        let (l, r) = (dl.get(5, j) as usize, dl.get(6, j) as usize);
        if (l == f) == (r == f) {
            continue;
        }
        let (a, b) = ([dl.get(1, j), dl.get(3, j)], [dl.get(2, j), dl.get(4, j)]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
            if x > p[0] {
                inside = !inside;
            }
        }
    }
    inside
}

struct Stats {
    min_angle: f64,
    longest: f64,
}

/// Checks shared by every case; returns the smallest angle and longest edge.
fn check_mesh(dl: &Matrix, m: &TriMesh, hmax: f64) -> Stats {
    let n = m.nodes.len();
    assert_eq!(m.tri_face.len(), m.tris.len());
    assert_eq!(m.node_edge.len(), n);
    assert_eq!(m.node_vertex.len(), n);
    let mut min = f64::INFINITY;
    let mut longest: f64 = 0.;
    let mut edges = Vec::new();
    for t in 0..m.tris.len() {
        let p = corners(m, t);
        let area = cross(sub(p[1], p[0]), sub(p[2], p[0])) / 2.;
        assert!(area > 0., "triangle {t} area {area}");
        min = min.min(min_angle(p));
        for k in 0..3 {
            longest = longest.max(len(sub(p[(k + 1) % 3], p[k])));
            let (a, b) = (m.tris[t][k], m.tris[t][(k + 1) % 3]);
            edges.push((a.min(b), a.max(b), m.tri_face[t]));
        }
    }
    // Conforming: an edge has at most two triangles; edges with one
    // triangle or two faces lie on geometry edges.
    edges.sort();
    let mut i = 0;
    while i < edges.len() {
        let mut j = i;
        while j < edges.len() && edges[j].0 == edges[i].0 && edges[j].1 == edges[i].1 {
            j += 1;
        }
        assert!(j - i <= 2, "edge {:?} used {} times", edges[i], j - i);
        if j - i == 1 || edges[i].2 != edges[i + 1].2 {
            let (a, b) = (edges[i].0, edges[i].1);
            assert!(
                m.node_edge[a].is_some() && m.node_edge[b].is_some(),
                "free edge {a}-{b} inside"
            );
        }
        i = j;
    }
    // Every boundary node is on its edge.
    for v in 0..n {
        if let Some(e) = m.node_edge[v] {
            let d = curve_distance(dl, e - 1, m.nodes[v]);
            assert!(d <= 1e-12, "node {v} is {d} from edge {e}");
        }
    }
    assert!(longest <= 1.1 * hmax, "longest edge {longest}");
    Stats {
        min_angle: min,
        longest,
    }
}

#[test]
fn square_minus_disk_quality_size_and_refinement_ratio() {
    let dl = dl_of(SQUARE_MINUS_DISK);
    let coarse = mesh(&dl, MeshOpts::new(0.2));
    let s = check_mesh(&dl, &coarse, 0.2);
    let fine = mesh(&dl, MeshOpts::new(0.1));
    let s2 = check_mesh(&dl, &fine, 0.1);
    let ratio = fine.tris.len() as f64 / coarse.tris.len() as f64;
    eprintln!(
        "R1-C1 hmax 0.2: {} elements, {} nodes, min angle {:.2}, longest {:.4}; hmax 0.1: {} elements, min angle {:.2}; ratio {ratio:.3}",
        coarse.tris.len(),
        coarse.nodes.len(),
        s.min_angle,
        s.longest,
        fine.tris.len(),
        s2.min_angle
    );
    assert!(
        s.min_angle >= 20. && s2.min_angle >= 20.,
        "{} {}",
        s.min_angle,
        s2.min_angle
    );
    assert!((3.2..=4.8).contains(&ratio), "ratio {ratio}");
    // Triangle areas add up to the polygon of the boundary chords, which is
    // inside 4 - pi/4 by less than the hole's chord sagittas.
    for m in [&coarse, &fine] {
        let area: f64 = (0..m.tris.len())
            .map(|t| {
                let p = corners(m, t);
                cross(sub(p[1], p[0]), sub(p[2], p[0])) / 2.
            })
            .sum();
        // Hole = polygon of its boundary nodes in angle order (shoelace).
        let mut hole: Vec<[f64; 2]> = (0..m.nodes.len())
            .filter(|&v| m.node_edge[v].is_some_and(|e| dl.get(0, e - 1) == 1.))
            .map(|v| m.nodes[v])
            .collect();
        hole.sort_by(|a, b| a[1].atan2(a[0]).total_cmp(&b[1].atan2(b[0])));
        let hole_area: f64 = (0..hole.len())
            .map(|i| cross(hole[i], hole[(i + 1) % hole.len()]) / 2.)
            .sum();
        assert!(
            (area - (4. - hole_area)).abs() < 1e-12,
            "area {area} hole {hole_area}"
        );
        assert!(
            hole_area < PI / 4. && hole_area > PI / 4. - 0.03,
            "hole {hole_area}"
        );
        assert!(m.tri_face.iter().all(|&f| f == 1));
        // Corners: 4 square corners plus the 4 arc end points.
        assert_eq!(m.node_vertex.iter().filter(|&&c| c).count(), 8);
    }
}

#[test]
fn same_input_gives_bit_identical_meshes() {
    let dl = dl_of(SQUARE_MINUS_DISK);
    let a = mesh(&dl, MeshOpts::new(0.15));
    let b = mesh(&dl, MeshOpts::new(0.15));
    let bits = |m: &TriMesh| {
        m.nodes
            .iter()
            .map(|p| (p[0].to_bits(), p[1].to_bits()))
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&a), bits(&b));
    assert_eq!(a.tris, b.tris);
    assert_eq!(a, b);
}

#[test]
#[allow(clippy::needless_range_loop)]
fn quadratic_midside_nodes_lie_on_the_arcs() {
    let dl = dl_of(SQUARE_MINUS_DISK);
    let linear = mesh(&dl, MeshOpts::new(0.2));
    let m = mesh(
        &dl,
        MeshOpts {
            quadratic: true,
            ..MeshOpts::new(0.2)
        },
    );
    check_mesh(&dl, &m, 0.2);
    assert_eq!(m.tris, linear.tris);
    let mid = m.mid.as_ref().expect("midside nodes");
    assert_eq!(mid.len(), m.tris.len());
    let mut on_arc = 0;
    for (t, row) in mid.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (m.tris[t][k], m.tris[t][(k + 1) % 3]);
            let p = m.nodes[row[k]];
            match m.node_edge[row[k]] {
                Some(e) if dl.get(0, e - 1) == 1. => {
                    let r = len(p);
                    assert!((r - 0.5).abs() <= 1e-12, "radius {r}");
                    on_arc += 1;
                }
                Some(_) => {}
                None => {
                    let (pa, pb) = (m.nodes[a], m.nodes[b]);
                    assert_eq!(p, [(pa[0] + pb[0]) / 2., (pa[1] + pb[1]) / 2.]);
                }
            }
        }
    }
    // Shared between neighbours: one midside node per mesh edge
    // (Euler: E = V + F - 1 for one face with one hole).
    let corners_n = linear.nodes.len();
    let edges = m.nodes.len() - corners_n;
    assert_eq!(edges, corners_n + m.tris.len());
    assert!(on_arc >= 16, "{on_arc}");
}

#[test]
fn two_rectangles_mesh_conformingly_with_faces_by_centroid() {
    let dl = dl_of(TWO_RECTANGLES);
    let m = mesh(&dl, MeshOpts::new(0.15));
    let s = check_mesh(&dl, &m, 0.15);
    assert!(s.min_angle >= 20., "{}", s.min_angle);
    let faces = (0..dl.cols())
        .map(|j| dl.get(5, j).max(dl.get(6, j)) as usize)
        .max()
        .unwrap();
    assert_eq!(faces, 3);
    let mut area = vec![0.; faces];
    for t in 0..m.tris.len() {
        let p = corners(&m, t);
        let c = [
            (p[0][0] + p[1][0] + p[2][0]) / 3.,
            (p[0][1] + p[1][1] + p[2][1]) / 3.,
        ];
        let hits: Vec<usize> = (1..=faces)
            .filter(|&f| in_polygon_face(&dl, f, c))
            .collect();
        assert_eq!(hits, [m.tri_face[t]], "triangle {t} centroid {c:?}");
        area[m.tri_face[t] - 1] += cross(sub(p[1], p[0]), sub(p[2], p[0])) / 2.;
    }
    for f in 1..=faces {
        assert!(
            (area[f - 1] - polygon_face_area(&dl, f)).abs() < 1e-12,
            "face {f}"
        );
    }
    // Interface nodes are shared: each interface edge's nodes are used by
    // triangles of both faces.
    let interface: Vec<usize> = (0..dl.cols())
        .filter(|&j| dl.get(5, j) != 0. && dl.get(6, j) != 0.)
        .collect();
    assert_eq!(interface.len(), 4);
    for v in 0..m.nodes.len() {
        let Some(e) = m.node_edge[v] else { continue };
        if !interface.contains(&(e - 1)) || m.node_vertex[v] {
            continue;
        }
        let mut seen: Vec<usize> = (0..m.tris.len())
            .filter(|&t| m.tris[t].contains(&v))
            .map(|t| m.tri_face[t])
            .collect();
        seen.sort();
        seen.dedup();
        let want = {
            let mut w = vec![dl.get(5, e - 1) as usize, dl.get(6, e - 1) as usize];
            w.sort();
            w
        };
        assert_eq!(seen, want, "node {v} on interface edge {e}");
    }
}

#[test]
fn l_shape_with_re_entrant_corner() {
    let dl = dl_of(L_SHAPE);
    for h in [0.3, 0.1] {
        let m = mesh(&dl, MeshOpts::new(h));
        let s = check_mesh(&dl, &m, h);
        assert!(s.min_angle >= 20., "{h}: {}", s.min_angle);
        let area: f64 = (0..m.tris.len())
            .map(|t| {
                let p = corners(&m, t);
                cross(sub(p[1], p[0]), sub(p[2], p[0])) / 2.
            })
            .sum();
        assert!((area - 3.).abs() < 1e-12, "{area}");
        // The re-entrant corner (1,1) is a mesh node.
        assert!(m.nodes.contains(&[1., 1.]));
    }
}

#[test]
fn rotated_ellipse_linear_and_quadratic() {
    let dl = dl_of(ELLIPSE);
    let m = mesh(
        &dl,
        MeshOpts {
            quadratic: true,
            ..MeshOpts::new(0.25)
        },
    );
    let s = check_mesh(&dl, &m, 0.25);
    assert!(s.min_angle >= 20., "{}", s.min_angle);
    let mid = m.mid.as_ref().unwrap();
    let mut boundary = 0;
    for row in mid {
        for &v in row {
            if let Some(e) = m.node_edge[v] {
                assert!(curve_distance(&dl, e - 1, m.nodes[v]) <= 1e-12);
                boundary += 1;
            }
        }
    }
    assert!(boundary > 0);
    let area: f64 = (0..m.tris.len())
        .map(|t| {
            let p = corners(&m, t);
            cross(sub(p[1], p[0]), sub(p[2], p[0])) / 2.
        })
        .sum();
    // Inscribed chord polygon: just under pi a b.
    assert!(area < 2. * PI && area > 2. * PI - 0.05, "{area}");
}

#[test]
fn hgrad_grades_pieces_away_from_a_short_edge() {
    // Square [0,2]^2 with the corner (0,2) cut by a 0.028 edge: pieces on
    // the top side start small at the cut and grow by at most hgrad.
    let dl = dl_of("P1=[2;5;0;2;2;0.02;0;0;0;2;2;1.98]; dl=decsg(P1,'P1',['P1']');");
    let m = mesh(&dl, MeshOpts::new(0.4));
    let s = check_mesh(&dl, &m, 0.4);
    assert!(s.min_angle >= 20., "{}", s.min_angle);
    let pieces = top_pieces(&m, 2.);
    eprintln!("top pieces {pieces:?}");
    assert!(pieces[0] < 0.06, "{pieces:?}");
    assert!(pieces.iter().all(|&p| p <= 0.4 * (1. + 1e-12)));
    assert_top_ratio(&pieces, 1.5);
}

fn top_pieces(m: &TriMesh, y: f64) -> Vec<f64> {
    let mut top: Vec<f64> = (0..m.nodes.len())
        .filter(|&v| m.nodes[v][1] == y)
        .map(|v| m.nodes[v][0])
        .collect();
    top.sort_by(f64::total_cmp);
    top.windows(2).map(|w| w[1] - w[0]).collect()
}

/// Neighbouring pieces differ by at most hgrad (the mesher keeps them
/// within hgrad^0.9 up to table resolution; the bound allows 10 %).
fn assert_top_ratio(pieces: &[f64], hgrad: f64) {
    for w in pieces.windows(2) {
        let r = (w[1] / w[0]).max(w[0] / w[1]);
        assert!(r <= hgrad * 1.10, "neighbour ratio {r} in {pieces:?}");
    }
}

#[test]
fn hgrad_bounds_neighbour_pieces_after_refinement_splits() {
    // Square [0,2]^2, Hmax 0.4, Hmin 0.004, Hgrad 1.1: refinement splits
    // some top-edge pieces; the graded rebuild keeps neighbours within 1.1.
    let dl = dl_of("R1=[3;4;0;2;2;0;0;0;2;2]; dl=decsg(R1);");
    let opts = MeshOpts {
        hmin: 0.004,
        hgrad: 1.1,
        ..MeshOpts::new(0.4)
    };
    let m = mesh(&dl, opts);
    let s = check_mesh(&dl, &m, 0.4);
    assert!(s.min_angle >= 20., "{}", s.min_angle);
    let pieces = top_pieces(&m, 2.);
    eprintln!("hgrad 1.1 top pieces {pieces:?}");
    assert!((pieces.iter().sum::<f64>() - 2.).abs() < 1e-12);
    assert_top_ratio(&pieces, 1.1);
    // Same on the bottom edge, and the rerun is bit-identical.
    assert_top_ratio(&top_pieces(&m, 0.), 1.1);
    assert_eq!(mesh(&dl, opts), m);
}

#[test]
fn bad_options_are_rejected() {
    let dl = dl_of(SQUARE_MINUS_DISK);
    for (opts, msg) in [
        (MeshOpts::new(0.), "Hmax"),
        (MeshOpts::new(f64::NAN), "Hmax"),
        (
            MeshOpts {
                hmin: 1.,
                ..MeshOpts::new(0.2)
            },
            "Hmin",
        ),
        (
            MeshOpts {
                hgrad: 1.,
                ..MeshOpts::new(0.2)
            },
            "Hgrad",
        ),
        (MeshOpts::new(1e-5), "too small"),
    ] {
        let err = pde_mesh_from_dl(&dl, &opts).unwrap_err();
        assert!(err.contains(msg), "{err}");
    }
}
