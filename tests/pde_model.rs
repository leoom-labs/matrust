//! Tests for createpde, geometryFromEdges, and pdegplot (Batch 8a2).
use matrust::{Array, Engine, Matrix};

const SQUARE_MINUS_DISK: &str = "R1=[3;4;-1;1;1;-1;-1;-1;1;1]; C1=[1;0;0;0.5;0;0;0;0;0;0]; gd=[R1 C1]; ns=['R1';'C1']'; dl=decsg(gd,'R1-C1',ns);";
const TWO_RECTANGLES: &str = "R1=[3;4;0;2;2;0;0;0;1;1]; R2=[3;4;1;3;3;1;0.5;0.5;1.5;1.5]; gd=[R1 R2]; ns=['R1';'R2']'; dl=decsg(gd,'R1+R2',ns);";

fn run(src: &str) -> Engine {
    let mut e = Engine::default();
    if let Err(err) = e.execute(src) {
        panic!("{src}\n{err}");
    }
    e
}

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].as_scalar().unwrap()
}

fn error(src: &str) -> String {
    Engine::default().execute(src).unwrap_err()
}

#[test]
fn createpde_defaults_properties_get_isa_and_display() {
    let mut e = Engine::default();
    let out = e
        .execute(
            "m = createpde();\n\
             m3 = createpde(3);\n\
             m_bare = createpde;\n\
             n1 = m.PDESystemSize;\n\
             n3 = get(m3, 'PDESystemSize');\n\
             nb = m_bare.PDESystemSize;\n\
             td = double(m.IsTimeDependent);\n\
             g_empty = double(isempty(m.Geometry));\n\
             mesh_empty = double(isempty(m.Mesh));\n\
             eq_empty = double(isempty(m.EquationCoefficients));\n\
             bc_empty = double(isempty(m.BoundaryConditions));\n\
             ic_empty = double(isempty(m.InitialConditions));\n\
             is_pde = double(isa(m, 'pde.PDEModel'));\n\
             is_handle = double(isa(m, 'handle'));\n\
             s = get(m);\n\
             s_n = s.PDESystemSize;\n\
             disp(m);",
        )
        .unwrap();

    assert_eq!(scalar(&e, "n1"), 1.0);
    assert_eq!(scalar(&e, "n3"), 3.0);
    assert_eq!(scalar(&e, "nb"), 1.0);
    assert_eq!(scalar(&e, "td"), 0.0);
    assert_eq!(scalar(&e, "g_empty"), 1.0);
    assert_eq!(scalar(&e, "mesh_empty"), 1.0);
    assert_eq!(scalar(&e, "eq_empty"), 1.0);
    assert_eq!(scalar(&e, "bc_empty"), 1.0);
    assert_eq!(scalar(&e, "ic_empty"), 1.0);
    assert_eq!(scalar(&e, "is_pde"), 1.0);
    assert_eq!(scalar(&e, "is_handle"), 1.0);
    assert_eq!(scalar(&e, "s_n"), 1.0);

    let disp_text = out.output.join("\n");
    for prop in [
        "PDESystemSize",
        "IsTimeDependent",
        "Geometry",
        "Mesh",
        "EquationCoefficients",
        "BoundaryConditions",
        "InitialConditions",
    ] {
        assert!(
            disp_text.contains(prop),
            "display missing property {prop}:\n{disp_text}"
        );
    }
}

#[test]
fn geometry_from_edges_r1_minus_c1_and_handle_aliasing() {
    let mut e = run(&format!(
        "{SQUARE_MINUS_DISK}\n\
         m = createpde();\n\
         m2 = m;\n\
         g = geometryFromEdges(m2, dl);\n\
         nf = m.Geometry.NumFaces;\n\
         ne = m.Geometry.NumEdges;\n\
         nv = m.Geometry.NumVertices;\n\
         V = m.Geometry.Vertices;\n\
         same_handle = double(m == m2);\n\
         is_geom = double(isa(g, 'pde.AnalyticGeometry'));\n\
         is_geom_handle = double(isa(g, 'handle'));\n\
         g_ne = get(g, 'NumEdges');\n\
         m_nf = get(m, 'Geometry').NumFaces;"
    ));

    assert_eq!(scalar(&e, "nf"), 1.0);
    assert_eq!(scalar(&e, "ne"), 8.0);
    assert_eq!(scalar(&e, "nv"), 8.0);
    assert_eq!(scalar(&e, "same_handle"), 1.0);
    assert_eq!(scalar(&e, "is_geom"), 1.0);
    assert_eq!(scalar(&e, "is_geom_handle"), 0.0);
    assert_eq!(scalar(&e, "g_ne"), 8.0);
    assert_eq!(scalar(&e, "m_nf"), 1.0);

    let v = &e.variables["V"];
    assert_eq!((v.rows(), v.cols()), (8, 2));

    // Verify deterministic first-appearance order along dl columns.
    let dl = &e.variables["dl"];
    let mut expected: Vec<[f64; 2]> = Vec::new();
    for j in 0..dl.cols() {
        for p in [[dl.get(1, j), dl.get(3, j)], [dl.get(2, j), dl.get(4, j)]] {
            if !expected
                .iter()
                .any(|q| (q[0] - p[0]).abs() <= 1e-12 && (q[1] - p[1]).abs() <= 1e-12)
            {
                expected.push(p);
            }
        }
    }
    assert_eq!(expected.len(), 8);
    for (i, want) in expected.iter().enumerate() {
        assert!((v.get(i, 0) - want[0]).abs() <= 1e-12, "vertex {i} x");
        assert!((v.get(i, 1) - want[1]).abs() <= 1e-12, "vertex {i} y");
    }

    // Display text for AnalyticGeometry contains all property names.
    let out = e.execute("disp(g);").unwrap();
    let disp_text = out.output.join("\n");
    for prop in ["NumFaces", "NumEdges", "NumVertices", "Vertices"] {
        assert!(
            disp_text.contains(prop),
            "geometry display missing property {prop}:\n{disp_text}"
        );
    }
}

#[test]
fn pdegplot_r1_minus_c1_lines_labels_and_face_point_inside_not_in_hole() {
    let mut e = run(&format!(
        "{SQUARE_MINUS_DISK}\n\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         h = pdegplot(m, 'EdgeLabels', 'on', 'FaceLabels', 'on', 'VertexLabels', 'on');\n\
         nh = numel(h);\n\
         t1 = double(strcmp(get(h(1), 'Type'), 'line'));"
    ));

    assert_eq!(scalar(&e, "nh"), 8.0);
    assert_eq!(scalar(&e, "t1"), 1.0);

    let fig = e.figure.as_ref().expect("figure created");
    assert_eq!(fig.axes.series.len(), 8, "expected 8 line objects");

    // Check arc sampling: 4 lines (2 pts each) and 4 quarter-circle arcs (>= 33 pts each).
    let mut line_2pt = 0usize;
    let mut arc_sampled = 0usize;
    for s in &fig.axes.series {
        if s.points.len() == 2 {
            line_2pt += 1;
        } else {
            assert!(
                s.points.len() >= 33,
                "quarter-circle arc should have >= 33 points (32 intervals), got {}",
                s.points.len()
            );
            arc_sampled += 1;
        }
    }
    assert_eq!(line_2pt, 4);
    assert_eq!(arc_sampled, 4);

    // Check text objects: exactly 8 E-texts, 1 F-text, 8 V-texts.
    let texts = &fig.axes.extras.texts;
    let mut e_labels = Vec::new();
    let mut f_labels = Vec::new();
    let mut v_labels = Vec::new();
    for t in texts {
        if t.string.starts_with('E') {
            e_labels.push((t.string.clone(), t.position[0], t.position[1]));
        } else if t.string.starts_with('F') {
            f_labels.push((t.string.clone(), t.position[0], t.position[1]));
        } else if t.string.starts_with('V') {
            v_labels.push((t.string.clone(), t.position[0], t.position[1]));
        } else {
            panic!("unexpected text label: {}", t.string);
        }
    }
    assert_eq!(e_labels.len(), 8, "expected 8 E-texts");
    assert_eq!(f_labels.len(), 1, "expected 1 F-text");
    assert_eq!(v_labels.len(), 8, "expected 8 V-texts");
    assert_eq!(f_labels[0].0, "F1");
    let mut e_names: Vec<&str> = e_labels.iter().map(|l| l.0.as_str()).collect();
    e_names.sort_unstable();
    let want: Vec<String> = (1..=8).map(|i| format!("E{i}")).collect();
    assert_eq!(e_names, want, "edge labels must be exactly E1..E8");

    // Verify F1 is strictly inside the outer square and strictly outside the hole using inpolygon.
    let (fx, fy) = (f_labels[0].1, f_labels[0].2);
    e.execute(&format!(
        "fx = {fx:.17}; fy = {fy:.17};\n\
         outer_x = [-1 1 1 -1 -1]; outer_y = [-1 -1 1 1 -1];\n\
         th = linspace(0, 2*pi, 257);\n\
         hole_x = 0.5*cos(th); hole_y = 0.5*sin(th);\n\
         [in_outer, on_outer] = inpolygon(fx, fy, outer_x, outer_y);\n\
         [in_hole, on_hole] = inpolygon(fx, fy, hole_x, hole_y);\n\
         ok_outer = double(in_outer && ~on_outer);\n\
         ok_hole = double(~in_hole && ~on_hole);"
    ))
    .unwrap();
    assert_eq!(
        scalar(&e, "ok_outer"),
        1.0,
        "F1 ({fx}, {fy}) must be strictly inside outer square"
    );
    assert_eq!(
        scalar(&e, "ok_hole"),
        1.0,
        "F1 ({fx}, {fy}) must be strictly outside hole"
    );
}

#[test]
fn pdegplot_accepts_g_and_dl_case_insensitive_options_and_defaults_off() {
    // Default options: no labels, works with g and dl, case-insensitive names/values.
    let e = run(&format!(
        "{TWO_RECTANGLES}\n\
         m = createpde();\n\
         g = geometryFromEdges(m, dl);\n\
         pdegplot(g);"
    ));
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.axes.series.len(), 12);
    assert_eq!(fig.axes.extras.texts.len(), 0);

    // Passing dl directly with boolean and case-insensitive options.
    let e2 = run(&format!(
        "{TWO_RECTANGLES}\n\
         h = pdegplot(dl, 'edgelabels', false, 'FACELABELS', true, 'vertexlabels', 'OFF');\n\
         nh = numel(h);"
    ));
    assert_eq!(scalar(&e2, "nh"), 12.0);
    let fig2 = e2.figure.as_ref().unwrap();
    assert_eq!(fig2.axes.extras.texts.len(), 3);
    let labels: Vec<&str> = fig2
        .axes
        .extras
        .texts
        .iter()
        .map(|t| t.string.as_str())
        .collect();
    assert_eq!(labels, vec!["F1", "F2", "F3"]);
}

#[test]
fn errors_wrong_rows_open_boundary_and_mesh_already_set() {
    // Wrong row count (too few rows: 5 rows).
    let err_few = error("m = createpde(); dl = zeros(5, 4); geometryFromEdges(m, dl);");
    assert!(
        err_few.to_lowercase().contains("row"),
        "expected row count error, got: {err_few}"
    );

    // Wrong row count (too many rows: 13 rows).
    let err_many = error("m = createpde(); dl = zeros(13, 4); geometryFromEdges(m, dl);");
    assert!(
        err_many.to_lowercase().contains("row"),
        "expected row count error, got: {err_many}"
    );

    // Open boundary (drop one column from valid dl).
    let err_open = error(&format!(
        "{SQUARE_MINUS_DISK}\n\
         m = createpde();\n\
         geometryFromEdges(m, dl(:, 1:end-1));"
    ));
    assert!(
        err_open.to_lowercase().contains("closed") || err_open.to_lowercase().contains("open"),
        "expected open boundary error, got: {err_open}"
    );

    // Second call on a model that already has a mesh.
    let mut e = run(&format!(
        "{SQUARE_MINUS_DISK}\n\
         m = createpde();\n\
         geometryFromEdges(m, dl);"
    ));
    matrust::engine::pde_model::attach_mesh_for_test(
        &mut e,
        "m",
        Array::Numeric(Matrix::scalar(1.0)),
    )
    .unwrap();
    let err_mesh = e.execute("geometryFromEdges(m, dl);").unwrap_err();
    assert!(
        err_mesh.to_lowercase().contains("new"),
        "expected message saying to create a new model, got: {err_mesh}"
    );

    // Invalid createpde(N) and pdegplot on empty model.
    assert!(error("createpde(0);").contains("createpde"));
    assert!(error("createpde(-2);").contains("createpde"));
    assert!(error("createpde(1.5);").contains("createpde"));
    assert!(error("m = createpde(); pdegplot(m);").contains("geometry"));
    assert!(
        error(&format!(
            "{SQUARE_MINUS_DISK} pdegplot(dl, 'UnknownOpt', 'on');"
        ))
        .contains("unknown option")
    );
}

#[test]
fn pdegplot_many_edges_combines_into_single_nan_separated_series() {
    let e = run("n = 80;\n\
         th = linspace(0, 2*pi, n + 1);\n\
         th = th(1:n)';\n\
         gd = [2; n; cos(th); sin(th)];\n\
         dl = decsg(gd);\n\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         h = pdegplot(m);\n\
         nh = numel(h);\n\
         is_line = double(strcmp(get(h, 'Type'), 'line'));");

    assert_eq!(scalar(&e, "nh"), 1.0);
    assert_eq!(scalar(&e, "is_line"), 1.0);

    let fig = e.figure.as_ref().expect("figure created");
    assert_eq!(fig.axes.series.len(), 1, "expected 1 combined line series");
    let pts = &fig.axes.series[0].points;
    let nan_seps = pts
        .iter()
        .filter(|p| p[0].is_nan() && p[1].is_nan())
        .count();
    assert_eq!(nan_seps, 79, "expected 79 NaN separators for 80 edges");
    assert_eq!(
        pts.len(),
        80 * 2 + 79,
        "expected 2 endpoints per edge plus 79 NaN separators"
    );
}

#[test]
fn pdegplot_hold_on_respects_free_series_slots_and_reports_full_axes() {
    let e = run(&format!(
        "hold on;\n\
         for k = 1:60\n\
             plot([0 1], [k k]);\n\
         end\n\
         {SQUARE_MINUS_DISK}\n\
         m = createpde();\n\
         geometryFromEdges(m, dl);\n\
         h = pdegplot(m);\n\
         nh = numel(h);\n\
         is_line = double(strcmp(get(h, 'Type'), 'line'));"
    ));

    assert_eq!(scalar(&e, "nh"), 1.0);
    assert_eq!(scalar(&e, "is_line"), 1.0);

    let fig = e.figure.as_ref().expect("figure created");
    assert_eq!(
        fig.axes.series.len(),
        61,
        "expected 60 existing series plus 1 combined geometry line"
    );
    let geom_series = &fig.axes.series[60];
    let nan_seps = geom_series
        .points
        .iter()
        .filter(|p| p[0].is_nan() && p[1].is_nan())
        .count();
    assert_eq!(nan_seps, 7, "expected 7 NaN separators for 8 edges");

    // When all 64 series slots are occupied with hold on, pdegplot reports a specific error naming the limit.
    let mut e_full = run(&format!(
        "hold on;\n\
         for k = 1:64\n\
             plot([0 1], [k k]);\n\
         end\n\
         {SQUARE_MINUS_DISK}\n\
         m = createpde();\n\
         geometryFromEdges(m, dl);"
    ));
    let err = e_full.execute("pdegplot(m);").unwrap_err();
    assert!(
        err.contains("pdegplot") && err.contains("64"),
        "expected pdegplot error naming limit 64, got: {err}"
    );
    assert_eq!(e_full.figure.as_ref().unwrap().axes.series.len(), 64);
}
