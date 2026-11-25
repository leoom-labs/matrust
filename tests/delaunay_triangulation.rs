//! The unit square and its centre give four triangles. Their circumcentres
//! are the four side midpoints, which bound the central Voronoi cell.
use matrust::{
    Array, Engine, Matrix,
    plot::{LinePattern, PlotKind},
};

const SQUARE: &str = "p=[0 0; 1 0; 1 1; 0 1; 0.5 0.5]; x=p(:,1); y=p(:,2);";

fn eval(e: &mut Engine, script: &str) {
    e.execute(script)
        .unwrap_or_else(|error| panic!("{script}: {error}"));
}

fn matrix(e: &Engine, name: &str) -> Matrix {
    match e.array(name) {
        Some(Array::Numeric(m)) => m,
        other => panic!("{name}: expected a numeric matrix, found {other:?}"),
    }
}

#[test]
fn delaunay_triangulation_properties_and_indexing() {
    let mut e = Engine::default();
    eval(&mut e, SQUARE);
    eval(
        &mut e,
        "
        dt = delaunayTriangulation(p);
        from_vectors = delaunayTriangulation(x.', y);
        assert(strcmp(class(dt), 'delaunayTriangulation'));
        assert(isobject(dt) && isa(dt, 'triangulation') && ~isa(dt, 'handle'));
        assert(isprop(dt, 'Points') && ~isprop(dt, 'Missing'));
        assert(isequal(dt.Points, p) && isempty(dt.Constraints));
        t = dt.ConnectivityList;
        assert(isequal(sortrows(sort(t,2)), [1 2 5; 1 4 5; 2 3 5; 3 4 5]));
        assert(isequal(sortrows(sort(from_vectors.ConnectivityList,2)), sortrows(sort(t,2))));
        assert(isequal(dt(2:end, [3 1]), t(2:end, [3 1])));
        assert(isequal(dt.ConnectivityList(1,:), dt(1,:)));
        assert(isequal(dt(:), t(:)));
        assert(isequal(size(dt), [4 3]) && size(dt,2)==3 && size(dt,3)==1);
        [r,c,z] = size(dt); assert(isequal([r c z], [4 3 1]));
        s = struct('Option',dt); assert(isequal(s.Option(:,2), t(:,2)));
        copy = dt; q = copy.Points; q(1,1) = 10;
        assert(isequal(copy.Points, p));
        duplicates = delaunayTriangulation([p(1,:); p; p(3,:)]);
        assert(isequal(duplicates.Points, p));
        assert(isequal(sortrows(sort(duplicates.ConnectivityList,2)), sortrows(sort(t,2))));
    ",
    );
    for (source, expected) in [
        ("copy.Points = [1 2]", "read-only"),
        ("dt(1,1) = 2", "Indexed assignment"),
        ("delaunayTriangulation([p, x])", "three-dimensional"),
        ("delaunayTriangulation(x,y,x)", "3-D"),
        ("delaunayTriangulation([1 Inf; 2 3])", "finite"),
        ("delaunayTriangulation([0 1], [0 1 0])", "same number"),
        ("delaunayTriangulation(zeros(200001,2))", "200000"),
    ] {
        let error = e.execute(source).unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn delaunay_triangulation_queries_use_connectivity_ids_and_vertex_order() {
    let mut e = Engine::default();
    eval(&mut e, SQUARE);
    eval(
        &mut e,
        "
        dt = delaunayTriangulation(p);
        t = dt.ConnectivityList;
        q = [.5 .125; .875 .5; 3 3; .5 .5; .25 .25; .5 0; NaN 0];
        [id,b] = pointLocation(dt, q);
        assert(isequaln(id, pointLocation(dt, q)));
        [dotid,dotb] = dt.pointLocation(q);
        assert(isequaln(id,dotid) && isequaln(b,dotb));
        assert(isequal(size(id), [7 1]) && isequal(size(b), [7 3]));
        assert(isnan(id(3)) && all(isnan(b(3,:))) && isnan(id(7)));
        assert(isequal(sort(t(id(1),:)), [1 2 5]));
        assert(isequal(sort(t(id(2),:)), [2 3 5]));
        assert(isequal(sort(b(1,:)), [.25 .375 .375]));
        assert(isequal(sort(b(2,:)), [.25 .375 .375]));
        assert(id(4) == size(t,1) && isequal(sort(b(4,:)), [0 0 1]));
        assert(id(5) == max(find(any(t==1,2))) && isequal(sort(b(5,:)), [0 .5 .5]));
        assert(isequal(sort(t(id(6),:)), [1 2 5]) && isequal(sort(b(6,:)), [0 .5 .5]));
        assert(isequal(b(1,:)*p(t(id(1),:),:), q(1,:)));
        assert(isequal(b(2,:)*p(t(id(2),:),:), q(2,:)));
        [v,d] = nearestNeighbor(dt, [.125 .25; .5 .5; 2 2; .5 0; NaN 0]);
        assert(isequaln(v,[1;5;3;1;NaN]));
        assert(max(abs(d(1:4) - [sqrt(5)/8;0;sqrt(2);.5])) < 1e-14 && isnan(d(5)));
        assert(nearestNeighbor(dt, [.5 .5]) == 5);
        [empty_id,empty_b] = pointLocation(dt, zeros(0,2));
        assert(isequal(size(empty_id), [0 1]) && isequal(size(empty_b), [0 3]));
        line_dt = delaunayTriangulation([0 0; 1 0; 2 0]);
        assert(isequal(size(line_dt), [0 3]) && isnan(pointLocation(line_dt, [1 0])));
        assert(nearestNeighbor(line_dt, [1 0]) == 2);
    ",
    );
}

#[test]
fn delaunay_triangulation_edges_neighbors_and_hull() {
    let mut e = Engine::default();
    eval(&mut e, SQUARE);
    eval(
        &mut e,
        "
        dt = delaunayTriangulation(p);
        t = dt.ConnectivityList;
        es = edges(dt);
        assert(isequal(sortrows(sort(es,2)), [1 2; 1 4; 1 5; 2 3; 2 5; 3 4; 3 5; 4 5]));
        n = neighbors(dt);
        [h,a] = convexHull(dt);
        assert(isequal(h, [1;2;3;4;1]) && a==1);
        [h2,a2] = dt.convexHull();
        assert(isequal(h,h2) && a==a2);
        % A collinear vertex on the boundary also belongs to the hull cycle.
        h3 = convexHull(delaunayTriangulation([p; .5 0]));
        assert(isequal(h3, [1;6;2;3;4;1]));
    ",
    );
    let (t, n) = (matrix(&e, "t"), matrix(&e, "n"));
    assert_eq!((n.rows(), n.cols()), (4, 3));
    for i in 0..4 {
        for j in 0..3 {
            let across = n.get(i, j);
            let edge = [t.get(i, (j + 1) % 3), t.get(i, (j + 2) % 3)];
            if edge.contains(&5.) {
                assert!(across.is_finite());
                let other = across as usize - 1;
                assert_ne!(i, other);
                let slot = (0..3).find(|&k| !edge.contains(&t.get(other, k))).unwrap();
                assert_eq!(n.get(other, slot), (i + 1) as f64);
                assert!(edge.contains(&t.get(other, (slot + 1) % 3)));
                assert!(edge.contains(&t.get(other, (slot + 2) % 3)));
            } else {
                assert!(across.is_nan());
            }
        }
    }
}

fn segment_set(segments: impl IntoIterator<Item = [[f64; 2]; 2]>) -> Vec<[[i32; 2]; 2]> {
    let mut segments: Vec<_> = segments
        .into_iter()
        .map(|segment| {
            let mut segment = segment.map(|p| {
                p.map(|v| {
                    assert!(v.is_finite() && v * 2. == (v * 2.).round(), "{v}");
                    (v * 2.) as i32
                })
            });
            segment.sort_unstable();
            segment
        })
        .collect();
    segments.sort_unstable();
    segments
}

#[test]
fn delaunay_triangulation_voronoi_finite_edges_and_plot() {
    let mut e = Engine::default();
    eval(&mut e, SQUARE);
    eval(
        &mut e,
        "[vx,vy]=voronoi(x,y); [v,c]=voronoin(p);
        assert(all(isinf(v(1,:))) && isequal(size(c), [5 1]));
        assert(isequal(sortrows(v(c{5},:)), [0 .5; .5 0; .5 1; 1 .5]));
        assert(any(c{1}==1) && any(c{2}==1) && any(c{3}==1) && any(c{4}==1));
    ",
    );
    assert!(e.figure.is_none());
    let (vx, vy) = (matrix(&e, "vx"), matrix(&e, "vy"));
    assert_eq!((vx.rows(), vx.cols(), vy.rows(), vy.cols()), (2, 4, 2, 4));
    // Doubled coordinates make the set comparison exact and independent of order.
    let expected = vec![
        [[0, 1], [1, 0]],
        [[0, 1], [1, 2]],
        [[1, 0], [2, 1]],
        [[1, 2], [2, 1]],
    ];
    assert_eq!(
        segment_set((0..4).map(|j| [0, 1].map(|i| [vx.get(i, j), vy.get(i, j)]))),
        expected
    );
    eval(&mut e, "voronoi(x,y);");
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.series.len(), 2);
    assert_eq!(figure.series[0].kind, PlotKind::Scatter);
    assert!(figure.series[0].markers);
    assert_eq!(figure.series[0].points.len(), 5);
    let path = &figure.series[1];
    assert_eq!(path.kind, PlotKind::Line);
    assert_eq!(path.pattern, LinePattern::Solid);
    assert_eq!(path.points.len(), 12);
    let (segments, remainder) = path.points.dense().unwrap().as_chunks::<3>();
    assert!(remainder.is_empty());
    assert_eq!(
        segment_set(segments.iter().map(|chunk| {
            assert!(chunk[2].iter().all(|x| x.is_nan()));
            [chunk[0], chunk[1]]
        })),
        expected
    );
    eval(&mut e, "hold on; h=voronoi(x,y); assert(numel(h)==2);");
    let figure = e.figure.as_ref().unwrap();
    assert_eq!(figure.series.len(), 4);
    assert!(figure.hold);
}

#[test]
fn delaunay_triangulation_voronoi_skips_cocircular_diagonals_and_hull_rays() {
    let mut e = Engine::default();
    eval(
        &mut e,
        "
        [vx,vy] = voronoi([0 1 1 0], [0 0 1 1]);
        assert(isequal(size(vx), [2 0]) && isequal(size(vy), [2 0]));
        [tx,ty] = voronoi([0 1 0], [0 0 1]);
        assert(isequal(size(tx), [2 0]) && isequal(size(ty), [2 0]));
        voronoi([0 1 1 0], [0 0 1 1]);
    ",
    );
    assert_eq!(e.figure.as_ref().unwrap().series[0].points.len(), 4);
}

#[test]
fn delaunay_triangulation_ordered_grid_reuses_retired_triangles() {
    let mut e = Engine::default();
    eval(
        &mut e,
        "
        [x,y] = meshgrid(1:100);
        dt = delaunayTriangulation(x(:), y(:));
        assert(isequal(size(dt), [19602 3]));
        [h,a] = convexHull(dt);
        assert(length(h)==397 && a==9801);
        [vx,vy] = voronoi(x(:),y(:));
        assert(isequal(size(vx), [2 19404]) && isequal(size(vy), [2 19404]));
    ",
    );
}

/// An interior point splits an asymmetric triangle into three faces; the
/// attachments are the ConnectivityList rows that contain each vertex.
#[test]
fn delaunay_triangulation_vertex_attachments_follow_connectivity_rows() {
    let mut e = Engine::default();
    eval(
        &mut e,
        "
        dt = delaunayTriangulation([0 0; 4 0; 1 3; 1.5 1]);
        t = dt.ConnectivityList;
        C = vertexAttachments(dt);
        assert(iscell(C) && isequal(size(C), [4 1]) && isequal(size(t), [3 3]));
        assert(isequal(C, dt.vertexAttachments()));
        assert(numel(C{4}) == 3 && numel(C{1}) == 2);
        for v = 1:4
            assert(isequal(C{v}, find(any(t == v, 2)).'));
        end
        joined = [C{4} C{1}];
        rows = t(joined, :);
    ",
    );
    let t = matrix(&e, "t");
    let joined = matrix(&e, "joined");
    let rows = matrix(&e, "rows");
    // Independent oracle: scan the stored rows for each vertex.
    let attached = |v: f64| -> Vec<f64> {
        (0..t.rows())
            .filter(|&i| (0..3).any(|k| t.get(i, k) == v))
            .map(|i| (i + 1) as f64)
            .collect()
    };
    for v in 1..=4 {
        let Some(Array::Cell(c)) = e.array("C") else {
            panic!("C is not a cell")
        };
        let Array::Numeric(m) = c.get_linear(v - 1).unwrap().as_ref() else {
            panic!("C{{{v}}} is not numeric")
        };
        assert_eq!(m.rows(), 1);
        let ids: Vec<f64> = (0..m.cols()).map(|j| m.get(0, j)).collect();
        assert_eq!(ids, attached(v as f64), "vertex {v}");
        let expected = if v == 4 { 3 } else { 2 };
        assert_eq!(ids.len(), expected, "vertex {v}");
    }
    let mut expected = attached(4.);
    expected.extend(attached(1.));
    assert_eq!((joined.rows(), joined.cols()), (1, 5));
    let ids: Vec<f64> = (0..5).map(|j| joined.get(0, j)).collect();
    assert_eq!(ids, expected);
    // Both faces at vertex 1 also touch vertex 4, so they occur twice.
    for id in attached(1.) {
        assert_eq!(ids.iter().filter(|&&x| x == id).count(), 2);
    }
    assert_eq!((rows.rows(), rows.cols()), (5, 3));
    for (r, id) in ids.iter().enumerate() {
        for k in 0..3 {
            assert_eq!(rows.get(r, k), t.get(*id as usize - 1, k));
        }
    }
    let error = e.execute("vertexAttachments(dt, 1)").unwrap_err();
    assert!(error.contains("not supported"), "{error}");
}
