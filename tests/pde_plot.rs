//! Tests for 2-D PDE solution plotting (`pdeplot`, Batch 8b5).
use matrust::{Engine, Matrix};

fn run(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code)
        .unwrap_or_else(|err| panic!("code failed: {err}\n---\n{code}"));
    e
}

fn error(code: &str) -> String {
    Engine::default()
        .execute(code)
        .expect_err(&format!("expected error for:\n{code}"))
}

fn scalar(e: &Engine, name: &str) -> f64 {
    e.variables[name].as_scalar().unwrap()
}

fn matrix(e: &Engine, name: &str) -> Matrix {
    e.variables[name].clone()
}

const UNIT_SQUARE: &str = "\
R1 = [3; 4; 0; 1; 1; 0; 0; 0; 1; 1];\n\
dl = decsg(R1);\n\
model = createpde();\n\
geometryFromEdges(model, dl);\n";

#[test]
fn pdeplot_xydata_linear_quadratic_and_results_idiom() {
    // 1) Linear mesh via model: Faces count == M, FaceVertexCData == u, ColorBar on by default.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         m_lin = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'linear');\n\
         u = m_lin.Nodes(1, :)' + 2 * m_lin.Nodes(2, :)';\n\
         figure;\n\
         h = pdeplot(model, 'XYData', u, 'Title', 'Linear XY', 'ColorMap', 'hot');\n\
         ptype = double(strcmp(get(h(1), 'Type'), 'patch'));\n\
         fcolor = double(strcmp(get(h(1), 'FaceColor'), 'interp'));\n\
         faces = get(h(1), 'Faces');\n\
         verts = get(h(1), 'Vertices');\n\
         cdata = get(h(1), 'FaceVertexCData');\n\
         n_elems = size(m_lin.Elements, 2);\n\
         n_nodes = size(m_lin.Nodes, 2);\n\
         max_c_err = max(abs(cdata - u));\n\
         max_z_err = max(abs(verts(:, 3)));\n\
         v2 = get(gca, 'View');\n\
         tstr = double(strcmp(get(get(gca, 'Title'), 'String'), 'Linear XY'));\n"
    ));

    assert_eq!(scalar(&e, "ptype"), 1.0);
    assert_eq!(scalar(&e, "fcolor"), 1.0);
    assert_eq!(
        matrix(&e, "faces").rows(),
        scalar(&e, "n_elems") as usize,
        "linear mesh patch Faces count must equal element count"
    );
    assert_eq!(matrix(&e, "verts").rows(), scalar(&e, "n_nodes") as usize);
    assert!(scalar(&e, "max_c_err") == 0.0, "CData per vertex equals u");
    assert!(scalar(&e, "max_z_err") == 0.0, "2-D XYData has z == 0");
    assert_eq!(matrix(&e, "v2").data(), &[0.0, 90.0]);
    assert_eq!(scalar(&e, "tstr"), 1.0);
    let fig = e.figure.as_ref().expect("figure created");
    assert!(
        fig.axes.extras.colorbar.is_some(),
        "ColorBar must be on by default for XYData"
    );

    // 2) Quadratic mesh: Faces count == 4 * M, CData per vertex equals u.
    let eq = run(&format!(
        "{UNIT_SQUARE}\
         m_quad = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'quadratic');\n\
         uq = m_quad.Nodes(1, :).^2' + m_quad.Nodes(2, :).^2';\n\
         figure;\n\
         hq = pdeplot(m_quad, 'XYData', uq);\n\
         faces_q = get(hq(1), 'Faces');\n\
         cdata_q = get(hq(1), 'FaceVertexCData');\n\
         fcolor_q = double(strcmp(get(hq(1), 'FaceColor'), 'interp'));\n\
         n_elems_q = size(m_quad.Elements, 2);\n\
         max_cq_err = max(abs(cdata_q - uq));\n"
    ));

    assert_eq!(
        matrix(&eq, "faces_q").rows(),
        4 * scalar(&eq, "n_elems_q") as usize,
        "quadratic mesh patch Faces count must equal 4 * element count"
    );
    assert!(scalar(&eq, "max_cq_err") == 0.0);
    assert_eq!(scalar(&eq, "fcolor_q"), 1.0);

    // 3) MATLAB StationaryResults idiom: pdeplot(results.Mesh, 'XYData', results.NodalSolution)
    //    plus direct (nodes, elements) call.
    let er = run(&format!(
        "{UNIT_SQUARE}\
         generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'linear');\n\
         specifyCoefficients(model, 'm', 0, 'd', 0, 'c', 1, 'a', 0, 'f', 1);\n\
         applyBoundaryCondition(model, 'dirichlet', 'Edge', 1:4, 'u', 0);\n\
         results = solvepde(model);\n\
         figure;\n\
         hr = pdeplot(results.Mesh, 'XYData', results.NodalSolution);\n\
         err_r = max(abs(get(hr(1), 'FaceVertexCData') - results.NodalSolution));\n\
         figure;\n\
         hd = pdeplot(results.Mesh.Nodes, results.Mesh.Elements, 'XYData', results.NodalSolution, 'ColorBar', 'off');\n\
         err_d = max(abs(get(hd(1), 'FaceVertexCData') - results.NodalSolution));\n"
    ));

    assert!(scalar(&er, "err_r") == 0.0);
    assert!(scalar(&er, "err_d") == 0.0);
    let fig_d = er.figure.as_ref().expect("figure created");
    assert!(
        fig_d.axes.extras.colorbar.is_none(),
        "ColorBar 'off' must omit the colorbar"
    );
}

#[test]
fn pdeplot_zdata_sets_vertex_z_and_3d_view() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'linear');\n\
         u = sin(pi * mesh.Nodes(1, :)') .* sin(pi * mesh.Nodes(2, :)');\n\
         figure;\n\
         h = pdeplot(model, 'XYData', u, 'ZData', u);\n\
         verts = get(h(1), 'Vertices');\n\
         max_z_err = max(abs(verts(:, 3) - u));\n\
         v3 = get(gca, 'View');\n\
         % Discontinuous ZStyle creates 3 vertices per face with constant face z\n\
         figure;\n\
         h_disc = pdeplot(model, 'ZData', u, 'ZStyle', 'discontinuous');\n\
         vd = get(h_disc(1), 'Vertices');\n\
         fd = get(h_disc(1), 'Faces');\n\
         disc_rows = size(vd, 1);\n\
         disc_faces = size(fd, 1);\n\
         % ZStyle 'off' keeps 2-D view and z == 0\n\
         figure;\n\
         h_off = pdeplot(model, 'XYData', u, 'ZData', u, 'ZStyle', 'off');\n\
         vo = get(h_off(1), 'Vertices');\n\
         max_zo = max(abs(vo(:, 3)));\n\
         v_off = get(gca, 'View');\n"
    ));

    assert!(
        scalar(&e, "max_z_err") == 0.0,
        "ZData must set vertex z = u"
    );
    assert_eq!(
        matrix(&e, "v3").data(),
        &[-37.5, 30.0],
        "ZData must set 3-D view"
    );
    assert_eq!(
        scalar(&e, "disc_rows") as usize,
        3 * scalar(&e, "disc_faces") as usize
    );
    assert!(scalar(&e, "max_zo") == 0.0);
    assert_eq!(matrix(&e, "v_off").data(), &[0.0, 90.0]);
}

#[test]
fn pdeplot_contour_levels_interpolate_within_1e12() {
    // On an affine field u(x, y) = 0.6*x + 0.4*y, P1 interpolation on every
    // triangle is exact: u_interp(x, y) == 0.6*x + 0.4*y everywhere in [0,1]^2.
    // Every vertex on the level 0.1 and 0.2 contour lines must satisfy
    // |0.6*x + 0.4*y - level| < 1e-12.
    let e = run(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.3, 'GeometricOrder', 'quadratic');\n\
         u = 0.6 * mesh.Nodes(1, :)' + 0.4 * mesh.Nodes(2, :)';\n\
         figure;\n\
         h = pdeplot(mesh, 'XYData', u, 'Contour', 'on', 'Levels', [0.1 0.2]);\n\
         n_handles = numel(h);\n\
         t1 = double(strcmp(get(h(1), 'Type'), 'patch'));\n\
         t2 = double(strcmp(get(h(2), 'Type'), 'line'));\n\
         t3 = double(strcmp(get(h(3), 'Type'), 'line'));\n\
         x1 = get(h(2), 'XData'); y1 = get(h(2), 'YData');\n\
         m1 = ~isnan(x1);\n\
         err1 = max(abs(0.6 * x1(m1) + 0.4 * y1(m1) - 0.1));\n\
         cnt1 = sum(m1);\n\
         x2 = get(h(3), 'XData'); y2 = get(h(3), 'YData');\n\
         m2 = ~isnan(x2);\n\
         err2 = max(abs(0.6 * x2(m2) + 0.4 * y2(m2) - 0.2));\n\
         cnt2 = sum(m2);\n\
         % Also test integer level count\n\
         figure;\n\
         hc = pdeplot(mesh, 'XYData', u, 'XYStyle', 'off', 'Mesh', 'off', 'Contour', 'on', 'Levels', 4);\n\
         n_count_lines = numel(hc);\n"
    ));

    assert_eq!(scalar(&e, "n_handles"), 3.0);
    assert_eq!(scalar(&e, "t1"), 1.0);
    assert_eq!(scalar(&e, "t2"), 1.0);
    assert_eq!(scalar(&e, "t3"), 1.0);
    assert!(scalar(&e, "cnt1") >= 2.0, "level 0.1 must have vertices");
    assert!(scalar(&e, "cnt2") >= 2.0, "level 0.2 must have vertices");
    assert!(
        scalar(&e, "err1") < 1e-12,
        "level 0.1 error {} >= 1e-12",
        scalar(&e, "err1")
    );
    assert!(
        scalar(&e, "err2") < 1e-12,
        "level 0.2 error {} >= 1e-12",
        scalar(&e, "err2")
    );
    assert_eq!(scalar(&e, "n_count_lines"), 4.0);
}

#[test]
fn pdeplot_mesh_overlay_and_flowdata_quiver() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'linear');\n\
         N = size(mesh.Nodes, 2);\n\
         u = mesh.Nodes(1, :)';\n\
         T = mesh.Elements;\n\
         E_pairs = [T([1 2], :) T([2 3], :) T([3 1], :)];\n\
         expected_edges = size(unique(sort(E_pairs, 1)', 'rows'), 1);\n\
         % Mesh 'on' adds edges\n\
         figure;\n\
         hm = pdeplot(mesh, 'XYData', u, 'Mesh', 'on');\n\
         n_hm = numel(hm);\n\
         t_patch = double(strcmp(get(hm(1), 'Type'), 'patch'));\n\
         t_mesh = double(strcmp(get(hm(2), 'Type'), 'line'));\n\
         drawn_edges = sum(isnan(get(hm(2), 'XData'))) + 1;\n\
         % FlowData (2 x N) creates quiver with N arrows (7 points & 2 NaNs per arrow)\n\
         F_2xN = [ones(1, N); zeros(1, N)];\n\
         figure;\n\
         hf1 = pdeplot(mesh, 'FlowData', F_2xN);\n\
         q1_type = double(strcmp(get(hf1(1), 'Type'), 'line'));\n\
         q1_nans = sum(isnan(get(hf1(1), 'XData')));\n\
         % FlowData (N x 2) also supported\n\
         F_Nx2 = [ones(N, 1), -ones(N, 1)];\n\
         figure;\n\
         hf2 = pdeplot(mesh, 'FlowData', F_Nx2);\n\
         q2_nans = sum(isnan(get(hf2(1), 'XData')));\n\
         % Statement form (0 outputs)\n\
         figure;\n\
         pdeplot(model, 'XYData', u, 'ColorBar', 'off');\n"
    ));

    let n = scalar(&e, "N");
    assert_eq!(scalar(&e, "n_hm"), 2.0);
    assert_eq!(scalar(&e, "t_patch"), 1.0);
    assert_eq!(scalar(&e, "t_mesh"), 1.0);
    assert_eq!(scalar(&e, "drawn_edges"), scalar(&e, "expected_edges"));
    assert_eq!(scalar(&e, "q1_type"), 1.0);
    assert_eq!(
        scalar(&e, "q1_nans"),
        2.0 * n,
        "quiver emits 2 NaN-separated segments (shaft + head) per arrow"
    );
    assert_eq!(scalar(&e, "q2_nans"), 2.0 * n);
}

#[test]
fn pdeplot_errors_and_help_example() {
    let setup = format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.5, 'GeometricOrder', 'linear');\n\
         u = mesh.Nodes(1, :)';\n"
    );
    // Bad parameter name uses MATLAB wording
    let err_bad = error(&format!("{setup} pdeplot(model, 'BogusParam', 'on');"));
    assert!(
        err_bad.contains("Unrecognized parameter name 'BogusParam'"),
        "expected MATLAB unrecognized parameter wording, got: {err_bad}"
    );
    // Model without mesh
    assert!(error(&format!("{UNIT_SQUARE} pdeplot(model);")).contains("no mesh"));
    // Wrong XYData length
    assert!(error(&format!("{setup} pdeplot(mesh, 'XYData', [1; 2]);")).contains("XYData"));
    // Wrong FlowData shape
    assert!(error(&format!("{setup} pdeplot(mesh, 'FlowData', ones(3, 3));")).contains("FlowData"));
    // Contour without XYData/ZData
    assert!(error(&format!("{setup} pdeplot(mesh, 'Contour', 'on');")).contains("Contour"));
    // Invalid XYStyle / ZStyle / ColorBar
    assert!(
        error(&format!(
            "{setup} pdeplot(mesh, 'XYData', u, 'XYStyle', 'bogus');"
        ))
        .contains("XYStyle")
    );
    assert!(
        error(&format!(
            "{setup} pdeplot(mesh, 'ZData', u, 'ZStyle', 'bogus');"
        ))
        .contains("ZStyle")
    );
    assert!(
        error(&format!(
            "{setup} pdeplot(mesh, 'XYData', u, 'ColorBar', 'maybe');"
        ))
        .contains("ColorBar")
    );

    // Help example runs and returns true
    let eh = run(
        "R1=[3;4;0;1;1;0;0;0;1;1]; dl=decsg(R1); m=createpde(); geometryFromEdges(m,dl); \
         mesh=generateMesh(m,'Hmax',0.5,'GeometricOrder','linear'); u=mesh.Nodes(1,:)'; \
         h=pdeplot(mesh,'XYData',u,'ColorBar','off'); ok=double(strcmp(get(h(1),'Type'),'patch'));",
    );
    assert_eq!(scalar(&eh, "ok"), 1.0);
}

#[test]
fn pdeplot_hold_off_resets_axes_and_keeps_only_new_objects() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.5, 'GeometricOrder', 'linear');\n\
         u = mesh.Nodes(1, :)';\n\
         % Two pdeplot calls without hold: the second replaces the first.\n\
         figure;\n\
         pdeplot(mesh, 'XYData', u, 'ColorBar', 'off');\n\
         h2 = pdeplot(mesh, 'XYData', u, 'ColorBar', 'off');\n\
         n2 = numel(h2);\n\
         nchild2 = numel(get(gca, 'Children'));\n\
         % plot then pdeplot, still without hold.\n\
         figure;\n\
         plot(1:3);\n\
         hmesh = pdeplot(mesh);\n\
         nmesh = numel(hmesh);\n\
         nchild_mesh = numel(get(gca, 'Children'));\n\
         xd = get(hmesh(1), 'XData');\n\
         xmax = max(xd(~isnan(xd)));\n\
         % Stale title and manual limits do not survive. Title option does.\n\
         figure;\n\
         xlim([5 6]);\n\
         title('OLD');\n\
         pdeplot(mesh, 'XYData', u, 'ColorBar', 'off');\n\
         blank = double(strcmp(get(get(gca, 'Title'), 'String'), ''));\n\
         xl = get(gca, 'XLim');\n\
         stale = double(abs(xl(1) - 5) < 1e-9 && abs(xl(2) - 6) < 1e-9);\n\
         % hold on keeps the previous line.\n\
         figure;\n\
         hold on;\n\
         plot(1:3);\n\
         hheld = pdeplot(mesh, 'XYData', u, 'ColorBar', 'off');\n\
         nheld = numel(hheld);\n\
         nchild_held = numel(get(gca, 'Children'));\n\
         figure;\n\
         xlim([5 6]);\n\
         title('OLD');\n\
         pdeplot(mesh, 'XYData', u, 'Title', 'Kept', 'ColorBar', 'off');\n\
         kept = double(strcmp(get(get(gca, 'Title'), 'String'), 'Kept'));\n"
    ));

    assert_eq!(scalar(&e, "nchild2"), scalar(&e, "n2"));
    assert_eq!(scalar(&e, "n2"), 1.0, "second pdeplot is one patch");
    assert_eq!(scalar(&e, "nchild_mesh"), scalar(&e, "nmesh"));
    assert_eq!(scalar(&e, "nmesh"), 1.0);
    assert!(
        scalar(&e, "xmax") <= 1.5,
        "plot(1:3) must be gone; mesh XData max was {}",
        scalar(&e, "xmax")
    );
    assert_eq!(
        scalar(&e, "blank"),
        1.0,
        "hold-off pdeplot clears the title"
    );
    assert_eq!(scalar(&e, "stale"), 0.0, "manual xlim must not survive");
    assert_eq!(scalar(&e, "kept"), 1.0);
    assert_eq!(
        scalar(&e, "nchild_held"),
        scalar(&e, "nheld") + 1.0,
        "hold on keeps the previous line"
    );
    let fig = e.figure.as_ref().expect("figure");
    assert!(
        fig.axes.xlim.is_none() && fig.axes.ylim.is_none(),
        "limits stay automatic"
    );
    assert_eq!(fig.axes.title, "Kept");
}

#[test]
fn pdeplot_xystyle_flat_uses_element_mean_and_off_draws_mesh() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.5, 'GeometricOrder', 'linear');\n\
         u = 10 * mesh.Nodes(1, :)' + mesh.Nodes(2, :)';\n\
         elems = mesh.Elements;\n\
         figure;\n\
         hf = pdeplot(mesh, 'XYData', u, 'XYStyle', 'flat', 'ColorBar', 'off');\n\
         faces = get(hf(1), 'Faces');\n\
         cdata = get(hf(1), 'FaceVertexCData');\n\
         fcolor = double(strcmp(get(hf(1), 'FaceColor'), 'flat'));\n\
         T = mesh.Elements;\n\
         E_pairs = [T([1 2], :) T([2 3], :) T([3 1], :)];\n\
         expected_edges = size(unique(sort(E_pairs, 1)', 'rows'), 1);\n\
         figure;\n\
         h2 = pdeplot(mesh, 'XYData', u, 'XYStyle', 'off', 'ColorBar', 'off');\n\
         n2 = numel(h2);\n\
         ty2 = double(strcmp(get(h2(1), 'Type'), 'line'));\n\
         drawn2 = sum(isnan(get(h2(1), 'XData'))) + 1;\n\
         v2 = get(gca, 'View');\n\
         figure;\n\
         hz = pdeplot(mesh, 'ZData', u, 'XYStyle', 'off', 'ColorBar', 'off');\n\
         nz = numel(hz);\n\
         tyz = double(strcmp(get(hz(1), 'Type'), 'line'));\n\
         zfinite = sum(~isnan(get(hz(1), 'ZData')));\n\
         vz = get(gca, 'View');\n"
    ));

    assert_eq!(scalar(&e, "fcolor"), 1.0);
    let faces = matrix(&e, "faces");
    let cdata = matrix(&e, "cdata");
    let elems = matrix(&e, "elems");
    let u = matrix(&e, "u");
    let n_faces = faces.rows();
    let rows = elems.rows();
    let n_elems = elems.data().len() / rows;
    assert_eq!(n_faces, n_elems, "one flat face per linear element");
    assert_eq!(cdata.data().len(), 3 * n_faces);
    let mut differs_from_first = false;
    for f in 0..n_faces {
        let node = |k: usize| elems.data()[k + f * rows] as usize - 1;
        let mean = (u.data()[node(0)] + u.data()[node(1)] + u.data()[node(2)]) / 3.0;
        if (u.data()[node(0)] - mean).abs() > 1e-8 {
            differs_from_first = true;
        }
        for corner in 0..3 {
            let v = faces.data()[f + corner * n_faces] as usize - 1;
            let got = cdata.data()[v];
            assert!(
                (got - mean).abs() < 1e-12,
                "face {f} CData {got} != corner mean {mean}"
            );
        }
    }
    assert!(
        differs_from_first,
        "fixture must distinguish the element mean from the first corner"
    );

    assert_eq!(
        scalar(&e, "n2"),
        1.0,
        "XYStyle off draws the mesh and no patch"
    );
    assert_eq!(scalar(&e, "ty2"), 1.0);
    assert_eq!(scalar(&e, "drawn2"), scalar(&e, "expected_edges"));
    assert_eq!(matrix(&e, "v2").data(), &[0.0, 90.0]);
    assert_eq!(scalar(&e, "nz"), 1.0);
    assert_eq!(scalar(&e, "tyz"), 1.0);
    assert!(
        scalar(&e, "zfinite") >= 2.0,
        "ZData + XYStyle off draws surface edges"
    );
    assert_eq!(matrix(&e, "vz").data(), &[-37.5, 30.0]);
}

#[test]
fn pdeplot_default_contour_levels_and_discontinuous_z() {
    let e = run(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.5, 'GeometricOrder', 'linear');\n\
         u = mesh.Nodes(1, :)';\n\
         z = mesh.Nodes(2, :)';\n\
         elems = mesh.Elements;\n\
         figure;\n\
         hd = pdeplot(mesh, 'XYData', u, 'Contour', 'on', 'ColorBar', 'off', 'Mesh', 'off');\n\
         n_default = numel(hd) - 1;\n\
         tpatch = double(strcmp(get(hd(1), 'Type'), 'patch'));\n\
         figure;\n\
         hc = pdeplot(mesh, 'XYData', u, 'ZData', z, 'ZStyle', 'discontinuous', \
                      'Contour', 'on', 'Levels', 0.5, 'Mesh', 'off', 'ColorBar', 'off');\n\
         cz = get(hc(2), 'ZData');\n\
         cy = get(hc(2), 'YData');\n\
         figure;\n\
         hm = pdeplot(mesh, 'ZData', z, 'ZStyle', 'discontinuous', 'Mesh', 'on', \
                      'XYStyle', 'off', 'ColorBar', 'off');\n\
         mz = get(hm(1), 'ZData');\n"
    ));

    assert_eq!(scalar(&e, "tpatch"), 1.0);
    assert_eq!(
        scalar(&e, "n_default"),
        10.0,
        "omitted Levels draws 10 contour lines"
    );

    let elems = matrix(&e, "elems");
    let z = matrix(&e, "z");
    let rows = elems.rows();
    let n_elems = elems.data().len() / rows;
    let mut means = Vec::with_capacity(n_elems);
    let mut max_node = f64::NEG_INFINITY;
    for &node_z in z.data() {
        max_node = max_node.max(node_z);
    }
    for f in 0..n_elems {
        let mut mean = 0.0;
        for k in 0..3 {
            let node = elems.data()[k + f * rows] as usize - 1;
            mean += z.data()[node];
        }
        means.push(mean / 3.0);
    }
    let max_mean = means.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    let cy = matrix(&e, "cy");
    let cz = matrix(&e, "cz");
    let mut span = false;
    let mut i = 0;
    while i < cy.data().len() {
        if !cy.data()[i].is_finite() {
            i += 1;
            continue;
        }
        let y0 = cy.data()[i];
        let z0 = cz.data()[i];
        i += 1;
        if i >= cy.data().len() || !cy.data()[i].is_finite() {
            break;
        }
        let y1 = cy.data()[i];
        let z1 = cz.data()[i];
        if (y0 - y1).abs() > 1e-8 {
            span = true;
            assert!(
                (z0 - z1).abs() < 1e-12,
                "discontinuous contour z must be constant on a triangle, {z0} vs {z1}"
            );
            assert!(
                means.iter().any(|m| (m - z0).abs() < 1e-12),
                "contour z {z0} is not an element mean"
            );
        }
        i += 1;
    }
    assert!(span, "contour segment must span y so nodal z would differ");

    let mz = matrix(&e, "mz");
    let finite: Vec<f64> = mz
        .data()
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    assert!(!finite.is_empty());
    let max_mesh_z = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max_mesh_z < max_node - 1e-9,
        "discontinuous mesh z max {max_mesh_z} still reaches nodal max {max_node}"
    );
    assert!(
        (max_mesh_z - max_mean).abs() < 1e-12,
        "highest mesh z should be the highest element mean"
    );
    for v in &finite {
        assert!(
            means.iter().any(|m| (m - v).abs() < 1e-12),
            "mesh z {v} is not an element mean"
        );
    }
}

#[cfg(feature = "plot-export")]
#[test]
fn pdeplot_svg_export_smoke_xydata_and_contour() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(&format!(
        "{UNIT_SQUARE}\
         mesh = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'linear');\n\
         u = mesh.Nodes(1, :)' + mesh.Nodes(2, :)';\n\
         figure;\n\
         pdeplot(mesh, 'XYData', u, 'Contour', 'on', 'Levels', [0.5 1.0 1.5]);\n\
         exportgraphics(gca, 'pde_plot_smoke.svg');\n"
    ))
    .unwrap();
    let svg = std::fs::read_to_string(dir.path().join("pde_plot_smoke.svg")).unwrap();
    assert!(!svg.is_empty(), "exported SVG must be non-empty");
    assert!(
        svg.contains("<polygon"),
        "exported SVG must contain triangle polygons"
    );
}
