//! Tests for generateMesh, pde.FEMesh, findNodes, findElements, and pdemesh (Batch 8a4).
use matrust::Engine;

const R1_MINUS_C1: &str = "\
R1 = [3; 4; -1; 1; 1; -1; -1; -1; 1; 1];\n\
C1 = [1; 0; 0; 0.5; 0; 0; 0; 0; 0; 0];\n\
gd = [R1 C1];\n\
ns = ['R1'; 'C1']';\n\
dl = decsg(gd, 'R1-C1', ns);\n";

const TWO_RECTANGLES: &str = "\
R1 = [3; 4; 0; 2; 2; 0; 0; 0; 1; 1];\n\
R2 = [3; 4; 1; 3; 3; 1; 0.5; 0.5; 1.5; 1.5];\n\
gd = [R1 R2];\n\
ns = ['R1'; 'R2']';\n\
dl = decsg(gd, 'R1+R2', ns);\n";

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
fn generate_mesh_r1_minus_c1_linear_quality_and_properties() {
    let mut e = run(&format!(
        "{R1_MINUS_C1}\
         model = createpde;\n\
         geometryFromEdges(model, dl);\n\
         m = generateMesh(model, 'Hmax', 0.2, 'GeometricOrder', 'linear');\n\
         P = m.Nodes;\n\
         T = m.Elements;\n\
         nr = size(P, 1);\n\
         N = size(P, 2);\n\
         er = size(T, 1);\n\
         M = size(T, 2);\n\
         in_range = double(all(T(:) >= 1) && all(T(:) <= N) && all(T(:) == floor(T(:))));\n\
         % Compute minimum triangle angle (degrees), minimum signed area, and max edge length in MATLAB code\n\
         min_ang = 180;\n\
         min_area = inf;\n\
         max_edge = 0;\n\
         for k = 1:M\n\
             p1 = P(:, T(1, k));\n\
             p2 = P(:, T(2, k));\n\
             p3 = P(:, T(3, k));\n\
             v12 = p2 - p1; v23 = p3 - p2; v31 = p1 - p3;\n\
             area2 = v12(1)*(p3(2) - p1(2)) - v12(2)*(p3(1) - p1(1));\n\
             if area2 < min_area, min_area = area2; end\n\
             a = norm(v23); b = norm(v31); c = norm(v12);\n\
             if max([a b c]) > max_edge, max_edge = max([a b c]); end\n\
             ang1 = acosd(max(-1, min(1, (b^2 + c^2 - a^2) / (2*b*c))));\n\
             ang2 = acosd(max(-1, min(1, (c^2 + a^2 - b^2) / (2*c*a))));\n\
             ang3 = acosd(max(-1, min(1, (a^2 + b^2 - c^2) / (2*a*b))));\n\
             min_ang = min([min_ang, ang1, ang2, ang3]);\n\
         end\n\
         same_model_mesh = double(isequal(model.Mesh, m));\n\
         is_femesh = double(isa(m, 'pde.FEMesh'));\n\
         is_handle = double(isa(m, 'handle'));\n\
         cls_ok = double(strcmp(class(m), 'pde.FEMesh'));\n\
         hmax_v = m.MaxElementSize;\n\
         hmin_v = m.MinElementSize;\n\
         hgrad_v = m.MeshGradation;\n\
         ord_ok = double(strcmp(m.GeometricOrder, 'linear'));\n\
         s = get(m);\n\
         s_hmax = s.MaxElementSize;\n\
         g_ord = double(strcmp(get(m, 'GeometricOrder'), 'linear'));\n\
         props_n = numel(properties(m));\n\
         m_fine = generateMesh(model, 'Hmax', 0.1, 'GeometricOrder', 'linear');\n\
         ratio = size(m_fine.Elements, 2) / M;\n\
         m_again = generateMesh(model, 'Hmax', 0.2, 'GeometricOrder', 'linear');\n\
         det_ok = double(isequal(m, m_again));\n"
    ));

    assert_eq!(scalar(&e, "nr"), 2.0);
    assert_eq!(scalar(&e, "er"), 3.0);
    assert!(scalar(&e, "N") > 50.0);
    assert!(scalar(&e, "M") > 50.0);
    assert_eq!(scalar(&e, "in_range"), 1.0);
    assert!(
        scalar(&e, "min_area") > 0.0,
        "all triangles must be counter-clockwise with positive area"
    );
    let min_ang = scalar(&e, "min_ang");
    assert!(
        min_ang >= 20.0,
        "expected min angle >= 20 deg, got {min_ang}"
    );
    let max_edge = scalar(&e, "max_edge");
    assert!(
        max_edge <= 0.2 * 1.1,
        "expected longest edge <= 0.22, got {max_edge}"
    );
    let ratio = scalar(&e, "ratio");
    assert!(
        (3.2..=4.8).contains(&ratio),
        "halving Hmax element ratio {ratio} outside [3.2, 4.8]"
    );
    assert_eq!(scalar(&e, "same_model_mesh"), 1.0);
    assert_eq!(scalar(&e, "is_femesh"), 1.0);
    assert_eq!(scalar(&e, "is_handle"), 0.0);
    assert_eq!(scalar(&e, "cls_ok"), 1.0);
    assert!((scalar(&e, "hmax_v") - 0.2).abs() < 1e-12);
    assert!((scalar(&e, "hmin_v") - 0.05).abs() < 1e-12);
    assert!((scalar(&e, "hgrad_v") - 1.5).abs() < 1e-12);
    assert_eq!(scalar(&e, "ord_ok"), 1.0);
    assert!((scalar(&e, "s_hmax") - 0.2).abs() < 1e-12);
    assert_eq!(scalar(&e, "g_ord"), 1.0);
    assert_eq!(scalar(&e, "props_n"), 6.0);
    assert_eq!(scalar(&e, "det_ok"), 1.0);

    // Display lists all 6 FEMesh properties.
    let out = e.execute("disp(m);").unwrap();
    let disp_text = out.output.join("\n");
    for prop in [
        "Nodes",
        "Elements",
        "MaxElementSize",
        "MinElementSize",
        "MeshGradation",
        "GeometricOrder",
    ] {
        assert!(
            disp_text.contains(prop),
            "FEMesh display missing {prop}:\n{disp_text}"
        );
    }
}

#[test]
fn generate_mesh_defaults_quadratic_and_arc_midside_nodes() {
    let e = run(&format!(
        "{R1_MINUS_C1}\
         model = createpde;\n\
         geometryFromEdges(model, dl);\n\
         m = generateMesh(model);\n\
         er = size(m.Elements, 1);\n\
         ord_ok = double(strcmp(m.GeometricOrder, 'quadratic'));\n\
         expected_hmax = hypot(2, 2) / 10;\n\
         hmax_err = abs(m.MaxElementSize - expected_hmax);\n\
         hmin_err = abs(m.MinElementSize - expected_hmax / 4);\n\
         hgrad_v = m.MeshGradation;\n\
         % Identify arc edges geometrically (dl row 1 == 1 for circle arcs)\n\
         arc_edges = find(dl(1, :) == 1);\n\
         n_arcs = numel(arc_edges);\n\
         arc_nodes = findNodes(m, 'region', 'Edge', arc_edges);\n\
         P = m.Nodes;\n\
         r_arc = hypot(P(1, arc_nodes), P(2, arc_nodes));\n\
         max_r_err = max(abs(r_arc - 0.5));\n\
         % Verify midside node ordering: rows 4,5,6 are between (1,2),(2,3),(3,1)\n\
         T = m.Elements;\n\
         mid_ok = 1;\n\
         for k = 1:size(T, 2)\n\
             for j = 1:3\n\
                 a = T(j, k); b = T(mod(j, 3) + 1, k); mid = T(3 + j, k);\n\
                 pa = P(:, a); pb = P(:, b); pm = P(:, mid);\n\
                 % Midside node must be closer to both endpoints than the full edge length\n\
                 elen = norm(pb - pa);\n\
                 if norm(pm - pa) >= elen || norm(pm - pb) >= elen\n\
                     mid_ok = 0;\n\
                 end\n\
             end\n\
         end\n"
    ));

    assert_eq!(scalar(&e, "er"), 6.0);
    assert_eq!(scalar(&e, "ord_ok"), 1.0);
    assert!(scalar(&e, "hmax_err") < 1e-12);
    assert!(scalar(&e, "hmin_err") < 1e-12);
    assert!((scalar(&e, "hgrad_v") - 1.5).abs() < 1e-12);
    assert_eq!(scalar(&e, "n_arcs"), 4.0);
    assert!(
        scalar(&e, "max_r_err") < 1e-12,
        "quadratic arc nodes radius error: {}",
        scalar(&e, "max_r_err")
    );
    assert_eq!(scalar(&e, "mid_ok"), 1.0);
}

#[test]
fn find_nodes_region_box_radius_and_nearest() {
    let e = run(&format!(
        "{R1_MINUS_C1}\
         model = createpde;\n\
         geometryFromEdges(model, dl);\n\
         m_lin = generateMesh(model, 'Hmax', 0.2, 'GeometricOrder', 'linear');\n\
         m_quad = generateMesh(model, 'Hmax', 0.2, 'GeometricOrder', 'quadratic');\n\
         arc_edges = find(dl(1, :) == 1);\n\
         % Linear: region Edge on arcs == brute-force |r - 0.5| < 1e-9\n\
         arc_lin = findNodes(m_lin, 'region', 'Edge', arc_edges);\n\
         P = m_lin.Nodes;\n\
         bf_lin = find(abs(hypot(P(1,:), P(2,:)) - 0.5) < 1e-9);\n\
         eq_arc_lin = double(isequal(arc_lin, bf_lin));\n\
         % Quadratic: region Edge on arcs == brute-force |r - 0.5| < 1e-9\n\
         arc_quad = findNodes(m_quad, 'region', 'Edge', arc_edges);\n\
         Pq = m_quad.Nodes;\n\
         bf_quad = find(abs(hypot(Pq(1,:), Pq(2,:)) - 0.5) < 1e-9);\n\
         eq_arc_quad = double(isequal(arc_quad, bf_quad));\n\
         % Single arc edge must include both of its endpoints (shared geometry vertices)\n\
         e_first = arc_edges(1);\n\
         n_single = findNodes(m_lin, 'region', 'Edge', e_first);\n\
         ep1 = [dl(2, e_first); dl(4, e_first)];\n\
         ep2 = [dl(3, e_first); dl(5, e_first)];\n\
         has_ep1 = double(any(hypot(P(1, n_single) - ep1(1), P(2, n_single) - ep1(2)) < 1e-12));\n\
         has_ep2 = double(any(hypot(P(1, n_single) - ep2(1), P(2, n_single) - ep2(2)) < 1e-12));\n\
         % region Face 1 covers all nodes on single-face domain\n\
         f1_nodes = findNodes(m_lin, 'region', 'Face', 1);\n\
         eq_face_all = double(isequal(f1_nodes, 1:size(P, 2)));\n\
         % box query matches brute force\n\
         box_n = findNodes(m_lin, 'box', [-0.6 0.6], [0.0 0.8]);\n\
         bf_box = find(P(1,:) >= -0.6 & P(1,:) <= 0.6 & P(2,:) >= 0.0 & P(2,:) <= 0.8);\n\
         eq_box = double(isequal(box_n, bf_box));\n\
         % radius query matches brute force\n\
         rad_n = findNodes(m_lin, 'radius', [0.5 0.5], 0.45);\n\
         bf_rad = find(hypot(P(1,:) - 0.5, P(2,:) - 0.5) <= 0.45);\n\
         eq_rad = double(isequal(rad_n, bf_rad));\n\
         % nearest preserves query order and matches brute force\n\
         qpts = [0.95 -0.95 0.0; 0.95 -0.95 0.5];\n\
         near_n = findNodes(m_lin, 'nearest', qpts);\n\
         bf_near = zeros(1, size(qpts, 2));\n\
         for j = 1:size(qpts, 2)\n\
             [~, bf_near(j)] = min((P(1,:) - qpts(1,j)).^2 + (P(2,:) - qpts(2,j)).^2);\n\
         end\n\
         eq_near = double(isequal(near_n, bf_near));\n\
         near_is_row = double(size(near_n, 1) == 1 && size(near_n, 2) == 3);\n"
    ));

    assert_eq!(scalar(&e, "eq_arc_lin"), 1.0);
    assert_eq!(scalar(&e, "eq_arc_quad"), 1.0);
    assert_eq!(scalar(&e, "has_ep1"), 1.0);
    assert_eq!(scalar(&e, "has_ep2"), 1.0);
    assert_eq!(scalar(&e, "eq_face_all"), 1.0);
    assert_eq!(scalar(&e, "eq_box"), 1.0);
    assert_eq!(scalar(&e, "eq_rad"), 1.0);
    assert_eq!(scalar(&e, "eq_near"), 1.0);
    assert_eq!(scalar(&e, "near_is_row"), 1.0);
}

#[test]
fn find_elements_box_radius_attached_and_multiface_region() {
    let e = run(&format!(
        "{R1_MINUS_C1}\
         model = createpde;\n\
         geometryFromEdges(model, dl);\n\
         m_lin = generateMesh(model, 'Hmax', 0.2, 'GeometricOrder', 'linear');\n\
         m_quad = generateMesh(model, 'Hmax', 0.25, 'GeometricOrder', 'quadratic');\n\
         % Linear box == brute-force all-nodes-inside\n\
         P = m_lin.Nodes; T = m_lin.Elements;\n\
         eb_lin = findElements(m_lin, 'box', [-1 0.2], [-1 0.2]);\n\
         bf_box_lin = [];\n\
         for k = 1:size(T, 2)\n\
             pts = P(:, T(:, k));\n\
             if all(pts(1,:) >= -1 & pts(1,:) <= 0.2 & pts(2,:) >= -1 & pts(2,:) <= 0.2)\n\
                 bf_box_lin(end+1) = k;\n\
             end\n\
         end\n\
         eq_box_lin = double(isequal(eb_lin, bf_box_lin));\n\
         % Quadratic box == brute-force all-6-nodes-inside\n\
         Pq = m_quad.Nodes; Tq = m_quad.Elements;\n\
         eb_quad = findElements(m_quad, 'box', [0 1], [0 1]);\n\
         bf_box_quad = [];\n\
         for k = 1:size(Tq, 2)\n\
             pts = Pq(:, Tq(:, k));\n\
             if all(pts(1,:) >= 0 & pts(1,:) <= 1 & pts(2,:) >= 0 & pts(2,:) <= 1)\n\
                 bf_box_quad(end+1) = k;\n\
             end\n\
         end\n\
         eq_box_quad = double(isequal(eb_quad, bf_box_quad));\n\
         % Radius == brute-force all-nodes-inside\n\
         er_lin = findElements(m_lin, 'radius', [0.5 0.5], 0.6);\n\
         bf_rad_lin = [];\n\
         for k = 1:size(T, 2)\n\
             pts = P(:, T(:, k));\n\
             if all(hypot(pts(1,:) - 0.5, pts(2,:) - 0.5) <= 0.6)\n\
                 bf_rad_lin(end+1) = k;\n\
             end\n\
         end\n\
         eq_rad_lin = double(isequal(er_lin, bf_rad_lin));\n\
         % Attached == brute-force any-node-in-set\n\
         qnodes = [1 5 12];\n\
         att = findElements(m_lin, 'attached', qnodes);\n\
         bf_att = [];\n\
         for k = 1:size(T, 2)\n\
             if any(ismember(T(:, k), qnodes))\n\
                 bf_att(end+1) = k;\n\
             end\n\
         end\n\
         eq_att = double(isequal(att, bf_att));\n\
         % Multi-face region query partitions all elements\n\
         {TWO_RECTANGLES}\
         m2 = createpde;\n\
         geometryFromEdges(m2, dl);\n\
         mesh2 = generateMesh(m2, 'Hmax', 0.3, 'GeometricOrder', 'linear');\n\
         nf = m2.Geometry.NumFaces;\n\
         all_f = [];\n\
         for f = 1:nf\n\
             ef = findElements(mesh2, 'region', 'Face', f);\n\
             all_f = [all_f ef];\n\
         end\n\
         part_ok = double(isequal(sort(all_f), 1:size(mesh2.Elements, 2)));\n\
         multi_ok = double(isequal(findElements(mesh2, 'region', 'Face', 1:nf), 1:size(mesh2.Elements, 2)));\n"
    ));

    assert_eq!(scalar(&e, "eq_box_lin"), 1.0);
    assert_eq!(scalar(&e, "eq_box_quad"), 1.0);
    assert_eq!(scalar(&e, "eq_rad_lin"), 1.0);
    assert_eq!(scalar(&e, "eq_att"), 1.0);
    assert_eq!(scalar(&e, "part_ok"), 1.0);
    assert_eq!(scalar(&e, "multi_ok"), 1.0);
}

#[test]
fn pdemesh_draws_expected_edges_curved_quadratic_and_labels() {
    let mut e = run(&format!(
        "{R1_MINUS_C1}\
         model = createpde;\n\
         geometryFromEdges(model, dl);\n\
         m_lin = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'linear');\n\
         % Count unique triangle edges from Elements\n\
         T = m_lin.Elements;\n\
         E_pairs = [T([1 2], :) T([2 3], :) T([3 1], :)];\n\
         E_sorted = sort(E_pairs, 1)';\n\
         E_uniq = unique(E_sorted, 'rows');\n\
         expected_edges = size(E_uniq, 1);\n\
         % 1) pdemesh(model) with labels\n\
         figure;\n\
         h = pdemesh(model, 'NodeLabels', 'on', 'ElementLabels', 'on');\n\
         is_line = double(strcmp(get(h, 'Type'), 'line'));\n\
         xd = get(h, 'XData');\n\
         drawn_edges = sum(isnan(xd)) + 1;\n\
         expected_texts = size(m_lin.Nodes, 2) + size(m_lin.Elements, 2);\n\
"
    ));

    assert_eq!(scalar(&e, "is_line"), 1.0);
    assert_eq!(scalar(&e, "drawn_edges"), scalar(&e, "expected_edges"));
    let fig = e.figure.as_ref().expect("figure created");
    // `axis equal` state (get(gca,'DataAspectRatio') is not readable yet).
    assert!(fig.axes.extras.equal_aspect, "pdemesh sets axis equal");
    assert_eq!(
        fig.axes.extras.texts.len(),
        scalar(&e, "expected_texts") as usize,
        "expected N node labels + M element labels"
    );

    e.execute(
        "% 2) pdemesh(nodes, elements) direct matrix call\n\
         figure;\n\
         h2 = pdemesh(m_lin.Nodes, m_lin.Elements);\n\
         drawn_edges_2 = sum(isnan(get(h2, 'XData'))) + 1;\n\
         % 3) Quadratic mesh draws curved edges through midside nodes\n\
         m_quad = generateMesh(model, 'Hmax', 0.4, 'GeometricOrder', 'quadratic');\n\
         figure;\n\
         hq = pdemesh(m_quad);\n\
         xdq = get(hq, 'XData');\n\
         ydq = get(hq, 'YData');\n\
         Tq = m_quad.Elements;\n\
         Eq_pairs = [Tq([1 2], :) Tq([2 3], :) Tq([3 1], :)];\n\
         expected_edges_q = size(unique(sort(Eq_pairs, 1)', 'rows'), 1);\n\
         drawn_edges_q = sum(isnan(xdq)) + 1;\n\
         % Verify every arc midside node appears in the drawn polyline\n\
         arc_edges = find(dl(1, :) == 1);\n\
         arc_nodes = findNodes(m_quad, 'region', 'Edge', arc_edges);\n\
         mid_ids = unique(Tq(4:6, :));\n\
         arc_mids = intersect(arc_nodes, mid_ids);\n\
         Pq = m_quad.Nodes;\n\
         all_mids_drawn = 1;\n\
         for idx = arc_mids(:).'\n\
             d = hypot(xdq - Pq(1, idx), ydq - Pq(2, idx));\n\
             if min(d) > 1e-12\n\
                 all_mids_drawn = 0;\n\
             end\n\
         end\n\
         % Statement form (0 outputs) succeeds\n\
         figure;\n\
         pdemesh(model);\n",
    )
    .unwrap();

    assert_eq!(scalar(&e, "drawn_edges_2"), scalar(&e, "expected_edges"));
    assert_eq!(scalar(&e, "drawn_edges_q"), scalar(&e, "expected_edges_q"));
    assert_eq!(scalar(&e, "all_mids_drawn"), 1.0);
}

#[test]
fn pde_mesh_error_handling() {
    // generateMesh without geometry
    assert!(
        error("m = createpde; generateMesh(m);").contains("no geometry"),
        "expected no geometry error"
    );
    // Hgrad out of (1, 2)
    let setup = format!("{R1_MINUS_C1} m = createpde; geometryFromEdges(m, dl);");
    assert!(error(&format!("{setup} generateMesh(m, 'Hgrad', 1);")).contains("Hgrad"));
    assert!(error(&format!("{setup} generateMesh(m, 'Hgrad', 2);")).contains("Hgrad"));
    assert!(error(&format!("{setup} generateMesh(m, 'Hgrad', 0.5);")).contains("Hgrad"));
    // Hmax / Hmin range checks
    assert!(error(&format!("{setup} generateMesh(m, 'Hmax', 0);")).contains("Hmax"));
    assert!(error(&format!("{setup} generateMesh(m, 'Hmax', -0.1);")).contains("Hmax"));
    assert!(error(&format!("{setup} generateMesh(m, 'Hmin', 0);")).contains("Hmin"));
    assert!(
        error(&format!(
            "{setup} generateMesh(m, 'Hmax', 0.2, 'Hmin', 0.3);"
        ))
        .contains("Hmin")
    );
    // Bad GeometricOrder / unknown option
    assert!(
        error(&format!(
            "{setup} generateMesh(m, 'GeometricOrder', 'cubic');"
        ))
        .contains("GeometricOrder")
    );
    assert!(error(&format!("{setup} generateMesh(m, 'Bogus', 1);")).contains("unknown option"));
    // Read-only FEMesh property
    let with_mesh = format!("{setup} mesh = generateMesh(m, 'Hmax', 0.4);");
    assert!(error(&format!("{with_mesh} mesh.Nodes = 1;")).contains("read-only"));
    // geometryFromEdges rejected once mesh exists
    assert!(error(&format!("{with_mesh} geometryFromEdges(m, dl);")).contains("mesh"));
    // findNodes / findElements errors
    assert!(
        error(&format!(
            "{with_mesh} findNodes(mesh, 'region', 'Edge', 99);"
        ))
        .contains("ID")
    );
    assert!(
        error(&format!(
            "{with_mesh} findNodes(mesh, 'region', 'Cell', 1);"
        ))
        .contains("region")
    );
    assert!(
        error(&format!(
            "{with_mesh} findNodes(mesh, 'box', [1 0], [0 1]);"
        ))
        .contains("bound")
    );
    assert!(
        error(&format!(
            "{with_mesh} findNodes(mesh, 'radius', [0 0], -1);"
        ))
        .contains("radius")
    );
    assert!(error(&format!("{with_mesh} findNodes(mesh, 'nearest', [1;2;3]);")).contains("2-by-P"));
    assert!(
        error(&format!(
            "{with_mesh} findElements(mesh, 'region', 'Edge', 1);"
        ))
        .contains("Face")
    );
    assert!(
        error(&format!(
            "{with_mesh} findElements(mesh, 'attached', 999999);"
        ))
        .contains("ID")
    );
    // pdemesh errors
    assert!(error(&format!("{setup} pdemesh(m);")).contains("no mesh"));
    assert!(error("pdemesh([0 1 0; 0 0 1], [1; 2; 4]);").contains("index"));
    assert!(
        error(&format!("{with_mesh} pdemesh(mesh, 'Bogus', 'on');")).contains("unknown option")
    );
}

#[test]
fn pde_mesh_help_examples_execute() {
    let e1 = run(
        "R1=[3;4;-1;1;1;-1;-1;-1;1;1]; dl=decsg(R1); m=createpde(); geometryFromEdges(m,dl); mesh=generateMesh(m,'Hmax',0.5,'GeometricOrder','linear'); r=size(mesh.Elements,1);",
    );
    assert_eq!(scalar(&e1, "r"), 3.0);

    let e2 = run(
        "R1=[3;4;0;1;1;0;0;0;1;1]; dl=decsg(R1); m=createpde(); geometryFromEdges(m,dl); mesh=generateMesh(m,'Hmax',0.5,'GeometricOrder','linear'); r=double(numel(findNodes(mesh,'box',[0 1],[0 1])) == size(mesh.Nodes,2));",
    );
    assert_eq!(scalar(&e2, "r"), 1.0);

    let e3 = run(
        "R1=[3;4;0;1;1;0;0;0;1;1]; dl=decsg(R1); m=createpde(); geometryFromEdges(m,dl); mesh=generateMesh(m,'Hmax',0.5,'GeometricOrder','linear'); r=double(numel(findElements(mesh,'region','Face',1)) == size(mesh.Elements,2));",
    );
    assert_eq!(scalar(&e3, "r"), 1.0);

    let e4 = run(
        "R1=[3;4;0;1;1;0;0;0;1;1]; dl=decsg(R1); m=createpde(); geometryFromEdges(m,dl); generateMesh(m,'Hmax',0.5,'GeometricOrder','linear'); h=pdemesh(m); r=double(strcmp(get(h,'Type'),'line'));",
    );
    assert_eq!(scalar(&e4, "r"), 1.0);
}
