//! surf, mesh and shading: one Patch series of quads per surface with
//! per-face colours, the painter's depth sort of the projected display, the
//! surface object's properties, bounds and .mrfig round trips. Behaviour
//! follows the published MathWorks documentation (never MATLAB output); the
//! flat-colour rule and face counts are checked against
//! tests/fixtures/surf/*.json, written by GNU Octave 10.3 with
//! tool/refs/surf/make_refs.m.
use matrust::{
    Array, Engine, Matrix, files,
    plot::{Figure, PlotKind, Series, view3d},
};
use std::collections::BTreeMap;

fn engine(source: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(source).unwrap();
    e
}
fn run(source: &str) -> Figure {
    engine(source).figure.unwrap()
}
fn error(source: &str) -> String {
    Engine::default().execute(source).unwrap_err().to_string()
}
fn num(e: &Engine, name: &str) -> Vec<f64> {
    e.variables[name].data().to_vec()
}
fn matrix(e: &Engine, name: &str) -> Matrix {
    e.variables[name].clone()
}
fn text(e: &mut Engine, expression: &str) -> String {
    e.execute(&format!("zz__ = double({expression});")).unwrap();
    e.variables["zz__"]
        .data()
        .iter()
        .map(|&c| char::from_u32(c as u32).unwrap())
        .collect()
}
fn surfaces(f: &Figure) -> Vec<&Series> {
    f.series
        .iter()
        .filter(|s| s.kind == PlotKind::Patch)
        .collect()
}
/// Colormap row `row` (0-based) of the 256-row map `cm` as 8-bit RGB.
fn row_rgb(cm: &Matrix, row: usize) -> [u8; 3] {
    [0, 1, 2].map(|k| (cm.get(row, k) * 255.).round() as u8)
}
/// MATLAB's documented scaled mapping, written out: row fix(m (v - cmin) / (cmax - cmin)).
fn scaled_row(m: usize, clim: [f64; 2], v: f64) -> usize {
    let t = (v - clim[0]) / (clim[1] - clim[0]);
    ((t * m as f64).floor().max(0.) as usize).min(m - 1)
}
fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}
/// Fill colours of the SVG's polygons, in paint order.
fn polygon_fills(svg: &str) -> Vec<String> {
    svg.match_indices("<polygon fill=\"")
        .map(|(i, m)| svg[i + m.len()..i + m.len() + 7].to_owned())
        .collect()
}
/// The SVG's patch faces in paint order: each polygon's fill, its
/// fill-opacity (None when opaque) and the stroke of the face outline path
/// right after it (None when the face draws no outline).
fn painted_faces(svg: &str) -> Vec<(String, Option<String>, Option<String>)> {
    let attr = |element: &str, name: &str| {
        let key = format!(" {name}=\"");
        element.find(&key).map(|i| {
            let rest = &element[i + key.len()..];
            rest[..rest.find('"').unwrap()].to_owned()
        })
    };
    svg.match_indices("<polygon fill=\"")
        .map(|(i, _)| {
            let end = i + svg[i..].find("/>").unwrap() + 2;
            let polygon = &svg[i..end];
            let next = &svg[end..];
            let outline = next
                .starts_with("<path fill=\"none\"")
                .then(|| attr(&next[..next.find("/>").unwrap()], "stroke"))
                .flatten();
            (
                attr(polygon, "fill").unwrap(),
                attr(polygon, "fill-opacity"),
                outline,
            )
        })
        .collect()
}
const PEAKS: &str = "[X,Y] = meshgrid(linspace(-3,3,20)); \
    Z = 3*(1-X).^2.*exp(-X.^2-(Y+1).^2) - 10*(X/5-X.^3-Y.^5).*exp(-X.^2-Y.^2) - exp(-(X+1).^2-Y.^2)/3;";

#[test]
fn surf_of_a_peaks_grid_is_one_patch_series_with_first_vertex_colours() {
    let mut e = engine(&format!(
        "{PEAKS} h = surf(Z); cm = colormap; cl = clim; v = view; t = h.Type;"
    ));
    let f = e.figure.clone().unwrap();
    assert_eq!(f.series.len(), 1);
    let s = &f.series[0];
    assert_eq!(s.kind, PlotKind::Patch);
    assert_eq!(s.patch.unwrap().vertices_per_face(), 4);
    assert_eq!(s.patch_face_count(), 361);
    assert_eq!(s.z.len(), 361 * 4);
    assert_eq!(num(&e, "v"), [-37.5, 30.]);
    assert_eq!(text(&mut e, "t"), "surface");
    let z = matrix(&e, "Z");
    let (lo, hi) = z
        .data()
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    assert_eq!(num(&e, "cl"), [lo, hi]);
    let cm = matrix(&e, "cm");
    assert_eq!(cm.rows(), 256);
    let colors = s.face_colors.as_ref().unwrap();
    // Faces are column-major: face (i, j) is number i + 19 j (0-based).
    for (i, j) in [(0, 0), (7, 3), (18, 18)] {
        let face = i + 19 * j;
        let want = row_rgb(&cm, scaled_row(256, [lo, hi], z.get(i, j)));
        assert_eq!(colors[face], want, "face ({i},{j})");
        // The face's first vertex is grid point (i, j).
        assert_eq!(s.points.get(face * 4).unwrap(), [z_x(j), z_x(i)]);
        assert_eq!(s.z[face * 4], z.get(i, j));
    }
    // Default surf style: black edges, '-' line style, flat faces.
    assert_eq!(s.face_edge_color, Some([0; 3]));
    assert!(s.face_edge_colors.is_none() && s.face_alpha.is_none());
    for (property, want) in [("FaceColor", "flat"), ("LineStyle", "-")] {
        assert_eq!(text(&mut e, &format!("get(h, '{property}')")), want);
    }
}
/// surf(Z) puts column j at x = j + 1 and row i at y = i + 1.
fn z_x(k: usize) -> f64 {
    (k + 1) as f64
}

#[derive(serde::Deserialize)]
struct Reference {
    source: String,
    z: Vec<Vec<Option<f64>>>,
    c: Vec<Vec<Option<f64>>>,
    clim: [f64; 2],
    faces: usize,
    rows256: Vec<usize>,
}
fn reference(name: &str) -> Reference {
    let path = format!("tests/fixtures/surf/{name}.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}
fn grid(rows: &[Vec<Option<f64>>]) -> Matrix {
    let (nr, nc) = (rows.len(), rows[0].len());
    let mut data = Vec::new();
    for j in 0..nc {
        for row in rows {
            data.push(row[j].unwrap_or(f64::NAN));
        }
    }
    Matrix::from_column_vec(nr, nc, data).unwrap()
}

#[test]
fn flat_colour_rule_and_face_counts_match_the_reference_fixtures() {
    for (name, with_c) in [("grid3x4", false), ("grid3x4_c", true), ("nan5", false)] {
        let r = reference(name);
        // Written by GNU Octave 10.3 (tool/refs/surf/make_refs.m).
        assert!(r.source.starts_with("octave 10.3"), "{name}: {}", r.source);
        let mut e = Engine::default();
        e.merge_workspace(BTreeMap::from([
            ("Z".to_owned(), Array::Numeric(grid(&r.z))),
            ("C".to_owned(), Array::Numeric(grid(&r.c))),
        ]))
        .unwrap();
        let call = if with_c { "surf(Z, C)" } else { "surf(Z)" };
        e.execute(&format!("h = {call}; cl = clim; cm = colormap;"))
            .unwrap();
        assert_eq!(num(&e, "cl"), r.clim, "{name}");
        let cm = matrix(&e, "cm");
        let f = e.figure.clone().unwrap();
        let s = &f.series[0];
        assert_eq!(s.patch_face_count(), r.faces, "{name}");
        let want: Vec<[u8; 3]> = r.rows256.iter().map(|&k| row_rgb(&cm, k - 1)).collect();
        assert_eq!(
            &**s.face_colors.as_ref().unwrap(),
            want.as_slice(),
            "{name}"
        );
    }
}

#[test]
fn surf_with_c_manual_clim_colormap_change_and_colorbar() {
    let mut e = engine(
        "Z = [0 1 2; 3 4 5; 6 7 8]; C = [8 7 6; 5 4 3; 2 1 0] * 10; h = surf(1:3, (1:3)', Z, C); \
         cl = clim; cm = colormap;",
    );
    assert_eq!(num(&e, "cl"), [0., 80.]);
    let f = e.figure.clone().unwrap();
    let cm = matrix(&e, "cm");
    // Face (0, 0) takes C(1,1) = 80: the last row, not Z's colour.
    assert_eq!(
        f.series[0].face_colors.as_ref().unwrap()[0],
        row_rgb(&cm, 255)
    );
    // A manual CLim is respected and recolours.
    e.execute("caxis([0 160]); cl2 = clim;").unwrap();
    assert_eq!(num(&e, "cl2"), [0., 160.]);
    let f = e.figure.clone().unwrap();
    assert_eq!(
        f.series[0].face_colors.as_ref().unwrap()[0],
        row_rgb(&cm, 128)
    );
    // A new colormap recolours the faces.
    e.execute("colormap(gray); cm2 = colormap;").unwrap();
    let gray = matrix(&e, "cm2");
    let f = e.figure.clone().unwrap();
    assert_eq!(
        f.series[0].face_colors.as_ref().unwrap()[0],
        row_rgb(&gray, 128)
    );
    // Back to automatic; a colorbar spans the CData.
    e.execute("clim('auto'); cb = colorbar; lim = cb.Limits; cl3 = clim;")
        .unwrap();
    assert_eq!(num(&e, "cl3"), [0., 80.]);
    assert_eq!(num(&e, "lim"), [0., 80.]);
    let f = e.figure.clone().unwrap();
    assert_eq!(
        f.series[0].face_colors.as_ref().unwrap()[0],
        row_rgb(&gray, 255)
    );
    // set(h, 'CData', ...) redraws.
    e.execute("set(h, 'CData', zeros(3)); h.CData(1,1) = 1; c = h.CData;")
        .unwrap();
    let f = e.figure.clone().unwrap();
    let faces = f.series[0].face_colors.as_ref().unwrap();
    assert_eq!(faces[0], row_rgb(&gray, 255));
    assert_eq!(faces[1], row_rgb(&gray, 0));
}

#[test]
fn mesh_faces_take_the_background_and_edges_the_face_colours() {
    let mut e =
        engine("Z = magic(4); h = mesh(Z); cm = colormap; fc = h.FaceColor; ec = h.EdgeColor;");
    assert_eq!(num(&e, "fc"), [1., 1., 1.]);
    assert_eq!(text(&mut e, "ec"), "flat");
    let f = e.figure.clone().unwrap();
    let s = &f.series[0];
    assert_eq!(s.color, [255; 3]);
    assert!(s.face_colors.is_none());
    let cm = matrix(&e, "cm");
    let edges = s.face_edge_colors.as_ref().unwrap();
    assert_eq!(edges.len(), 9);
    // magic(4)(1,1) = 16 is the maximum, (2,1) = 5.
    assert_eq!(edges[0], row_rgb(&cm, 255));
    assert_eq!(edges[1], row_rgb(&cm, scaled_row(256, [1., 16.], 5.)));
    assert_eq!(f.view, view3d::VIEW_3D);
}

#[test]
fn shading_sets_the_documented_face_and_edge_colours() {
    let mut e = engine("Z = magic(4); h = surf(Z);");
    for (mode, face, edge) in [
        ("flat", "flat", "none"),
        ("interp", "interp", "none"),
        ("faceted", "flat", "black"),
    ] {
        e.execute(&format!("shading {mode}")).unwrap();
        assert_eq!(text(&mut e, "h.FaceColor"), face, "{mode}");
        if edge == "black" {
            e.execute("ec = h.EdgeColor;").unwrap();
            assert_eq!(num(&e, "ec"), [0.; 3]);
        } else {
            assert_eq!(text(&mut e, "h.EdgeColor"), edge, "{mode}");
        }
    }
    // interp: the mean of the four vertices' mapped colours.
    let mut e = engine("Z = [0 1; 2 3]; h = surf(Z); shading(gca, 'interp'); cm = colormap;");
    let cm = matrix(&e, "cm");
    let f = e.figure.clone().unwrap();
    let rows = [0., 2., 3., 1.].map(|v| row_rgb(&cm, scaled_row(256, [0., 3.], v)));
    let mean: [u8; 3] = std::array::from_fn(|k| {
        (rows.iter().map(|c| f64::from(c[k])).sum::<f64>() / 4.).round() as u8
    });
    assert_eq!(f.series[0].face_colors.as_ref().unwrap()[0], mean);
    assert!(f.series[0].face_edge_color.is_none());
    // A mesh keeps its faces; shading sets its EdgeColor.
    e.execute("h2 = mesh(magic(3)); shading interp").unwrap();
    assert_eq!(text(&mut e, "h2.EdgeColor"), "interp");
    e.execute("fc = h2.FaceColor;").unwrap();
    assert_eq!(num(&e, "fc"), [1., 1., 1.]);
    e.execute("shading flat").unwrap();
    assert_eq!(text(&mut e, "h2.EdgeColor"), "flat");
}

#[test]
fn face_color_none_and_face_alpha_reach_the_svg() {
    let mut e = engine("h = surf(magic(3)); h.FaceAlpha = 0.5;");
    let svg = e.figure.as_ref().unwrap().svg();
    assert!(svg.contains("fill-opacity=\"0.5\""));
    e.execute("set(h, 'FaceColor', 'none'); a = h.FaceAlpha;")
        .unwrap();
    assert_eq!(num(&e, "a"), [0.5]);
    let f = e.figure.clone().unwrap();
    assert_eq!(f.series[0].face_alpha, Some(0.));
    assert!(polygon_fills(&f.svg()).is_empty());
    // Edges are still drawn.
    assert!(f.svg().contains("stroke=\"#000000\""));
    e.execute("set(h, 'FaceColor', [1 0 0], 'EdgeColor', 'none', 'FaceAlpha', 1);")
        .unwrap();
    let f = e.figure.clone().unwrap();
    assert!(polygon_fills(&f.svg()).iter().all(|c| c == "#ff0000"));
    assert_eq!(polygon_fills(&f.svg()).len(), 4);
}

#[test]
fn a_nan_hole_skips_exactly_the_faces_touching_it() {
    // 4x5 grid: 12 faces; an interior NaN touches 4 of them, a corner one 1.
    for (index, holes) in [("2,3", 4), ("1,1", 1), ("4,5", 1), ("1,3", 2)] {
        let f = run(&format!(
            "Z = reshape(1:20, 4, 5); Z({index}) = NaN; surf(Z);"
        ));
        assert_eq!(f.series[0].patch_face_count(), 12 - holes, "{index}");
        assert!(f.series[0].z.iter().all(|v| v.is_finite()));
    }
    // A NaN in C (or X) is a hole too.
    let f = run("C = magic(4); C(1,1) = NaN; surf(1:4, 1:4, magic(4), C);");
    assert_eq!(f.series[0].patch_face_count(), 8);
    let f = run("x = 1:4; x(4) = NaN; surf(x, 1:4, magic(4));");
    assert_eq!(f.series[0].patch_face_count(), 6);
}

/// Depth of a unit-box point under MATLAB's viewmtx (third row), written out.
fn depth(az: f64, el: f64, p: [f64; 3]) -> f64 {
    let (sa, ca) = az.to_radians().sin_cos();
    let (se, ce) = el.to_radians().sin_cos();
    ce * sa * p[0] - ce * ca * p[1] + se * p[2]
}

/// The four faces of a 3x3 grid on x, y in [1 3] (unit box [0 1]) with the
/// face's z level `level` in unit coordinates and the given colormap rows.
fn expected_order(az: f64, el: f64, faces: &[([f64; 3], &str)]) -> Vec<String> {
    let mut keyed: Vec<(f64, &str)> = faces
        .iter()
        .map(|(centre, color)| (depth(az, el, *centre), *color))
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    keyed.into_iter().map(|(_, c)| c.to_owned()).collect()
}

const DEPTH_SETUP: &str = "colormap([1 0 0; 0 1 0; 0 0 1; 1 1 0; 1 0 1; 0 1 1; 0 0 0; 1 1 1]); \
    clim([0 8]); xlim([1 3]); ylim([1 3]); zlim([0 1]); shading flat";

#[test]
fn faces_are_painted_far_to_near_and_reverse_with_the_view() {
    // C(i,j) for faces (i,j): 0..3 picks colormap rows 1..4 with clim [0 8]
    // and 8 rows (row = fix(8 c / 8)).
    let source = format!(
        "C = [0 2 0; 1 3 0; 0 0 0]; surf(1:3, 1:3, zeros(3), C); {DEPTH_SETUP}; view(30, 40);"
    );
    let mut e = engine(&source);
    // Face (i, j) centre: x = (j + 0.5) / 2, y = (i + 0.5) / 2, z = 0.
    let faces = [
        ([0.25, 0.25, 0.], "#ff0000"),
        ([0.25, 0.75, 0.], "#00ff00"),
        ([0.75, 0.25, 0.], "#0000ff"),
        ([0.75, 0.75, 0.], "#ffff00"),
    ];
    let svg = e.figure.as_ref().unwrap().svg();
    let want = expected_order(30., 40., &faces);
    assert_eq!(polygon_fills(&svg), want);
    e.execute("view(210, 40);").unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    let mut reversed = want.clone();
    reversed.reverse();
    assert_eq!(polygon_fills(&svg), reversed);
    assert_eq!(polygon_fills(&svg), expected_order(210., 40., &faces));
    // A 2-D view keeps the original (column-major) order.
    e.execute("view(2);").unwrap();
    let svg = e.figure.as_ref().unwrap().svg();
    assert_eq!(
        polygon_fills(&svg),
        ["#ff0000", "#00ff00", "#0000ff", "#ffff00"]
    );
}

#[test]
fn two_overlapping_surfaces_interleave_by_depth() {
    // Surface A at z = 0 (rows 1..4), surface B at z = 0.5 (rows 5..8),
    // seen from the side: their faces alternate in depth.
    let source = format!(
        "surf(1:3, 1:3, zeros(3), [0 2 0; 1 3 0; 0 0 0]); hold on; \
         surf(1:3, 1:3, 0.5 * ones(3), [4 6 0; 5 7 0; 0 0 0]); {DEPTH_SETUP}; view(0, 10);"
    );
    let f = run(&source);
    assert_eq!(surfaces(&f).len(), 2);
    let faces = [
        ([0.25, 0.25, 0.], "#ff0000"),
        ([0.25, 0.75, 0.], "#00ff00"),
        ([0.75, 0.25, 0.], "#0000ff"),
        ([0.75, 0.75, 0.], "#ffff00"),
        ([0.25, 0.25, 0.5], "#ff00ff"),
        ([0.25, 0.75, 0.5], "#00ffff"),
        ([0.75, 0.25, 0.5], "#000000"),
        ([0.75, 0.75, 0.5], "#ffffff"),
    ];
    let fills = polygon_fills(&f.svg());
    assert_eq!(fills, expected_order(0., 10., &faces));
    // Interleaved: not all of A before all of B.
    let a = ["#ff0000", "#00ff00", "#0000ff", "#ffff00"];
    let first_b = fills.iter().position(|c| !a.contains(&c.as_str())).unwrap();
    assert!(
        fills[first_b..].iter().any(|c| a.contains(&c.as_str())),
        "{fills:?}"
    );
    // The display keeps one legend entry per surface.
    let display = view3d::display_view(&f.axes);
    assert_eq!(
        display
            .series
            .iter()
            .filter(|s| s.kind == PlotKind::Patch && !s.legend_hidden)
            .count(),
        2
    );
    let mut e = engine(&source);
    e.execute("legend('a', 'b');").unwrap();
    assert!(e.figure.as_ref().unwrap().svg().contains(">b</text>"));
}

/// Items of `faces` (unit-box centre, value) sorted far to near; ties keep
/// their order.
fn by_depth<T: Clone>(az: f64, el: f64, faces: &[([f64; 3], T)]) -> Vec<T> {
    let mut keyed: Vec<(f64, T)> = faces
        .iter()
        .map(|(centre, item)| (depth(az, el, *centre), item.clone()))
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
    keyed.into_iter().map(|(_, item)| item).collect()
}
/// A painted face: its fill and one more attribute (outline or opacity).
type FaceKey = (String, Option<String>);
/// Surface A at z = 0 (colormap rows 1..4) and the unit-box face centres of
/// a second surface at z = 0.5 (C rows 5..8: magenta, cyan, black, white).
const LOWER: &str = "surf(1:3, 1:3, zeros(3), [0 2 0; 1 3 0; 0 0 0]); hold on;";
const UPPER_C: &str = "[4 6 0; 5 7 0; 0 0 0]";
const LOWER_FACES: [([f64; 3], &str); 4] = [
    ([0.25, 0.25, 0.], "#ff0000"),
    ([0.25, 0.75, 0.], "#00ff00"),
    ([0.75, 0.25, 0.], "#0000ff"),
    ([0.75, 0.75, 0.], "#ffff00"),
];
const UPPER_FACES: [([f64; 3], &str); 4] = [
    ([0.25, 0.25, 0.5], "#ff00ff"),
    ([0.25, 0.75, 0.5], "#00ffff"),
    ([0.75, 0.25, 0.5], "#000000"),
    ([0.75, 0.75, 0.5], "#ffffff"),
];

#[test]
fn a_surf_and_a_mesh_interleave_by_depth_with_their_own_styles() {
    // view(0, 10): depth -0.985 y + 0.174 z, so the faces go A A B B A A B B.
    // The surf's faces (shading flat) have no outline; the mesh's faces are
    // filled with the axes background and outlined in their flat colours.
    let f = run(&format!(
        "{LOWER} mesh(1:3, 1:3, 0.5 * ones(3), {UPPER_C}); {DEPTH_SETUP}; view(0, 10);"
    ));
    assert_eq!(surfaces(&f).len(), 2);
    let mut faces: Vec<([f64; 3], FaceKey)> = LOWER_FACES
        .iter()
        .map(|&(c, fill)| (c, (fill.to_owned(), None)))
        .collect();
    faces.extend(
        UPPER_FACES
            .iter()
            .map(|&(c, edge)| (c, ("#ffffff".to_owned(), Some(edge.to_owned())))),
    );
    let want = by_depth(0., 10., &faces);
    let got: Vec<FaceKey> = painted_faces(&f.svg())
        .into_iter()
        .map(|(fill, opacity, outline)| {
            assert_eq!(opacity, None);
            (fill, outline)
        })
        .collect();
    assert_eq!(got, want);
    // Interleaved: a mesh face is painted between surf faces.
    let first_mesh = got.iter().position(|f| f.1.is_some()).unwrap();
    assert!(got[first_mesh..].iter().any(|f| f.1.is_none()), "{got:?}");
}

#[test]
fn a_translucent_and_an_opaque_surf_interleave_by_depth() {
    let f = run(&format!(
        "{LOWER} surf(1:3, 1:3, 0.5 * ones(3), {UPPER_C}, 'FaceAlpha', 0.5); {DEPTH_SETUP}; view(0, 10);"
    ));
    let mut faces: Vec<([f64; 3], FaceKey)> = LOWER_FACES
        .iter()
        .map(|&(c, fill)| (c, (fill.to_owned(), None)))
        .collect();
    faces.extend(
        UPPER_FACES
            .iter()
            .map(|&(c, fill)| (c, (fill.to_owned(), Some("0.5".to_owned())))),
    );
    let want = by_depth(0., 10., &faces);
    let got: Vec<FaceKey> = painted_faces(&f.svg())
        .into_iter()
        .map(|(fill, opacity, _)| (fill, opacity))
        .collect();
    assert_eq!(got, want);
    let first_b = got.iter().position(|f| f.1.is_some()).unwrap();
    assert!(got[first_b..].iter().any(|f| f.1.is_none()), "{got:?}");
}

#[test]
fn the_legend_of_two_meshes_shows_each_one_s_own_stroked_swatch() {
    let f = run(&format!(
        "mesh(1:3, 1:3, zeros(3), [0 2 0; 1 3 0; 0 0 0]); hold on; \
         mesh(1:3, 1:3, 0.5 * ones(3), {UPPER_C}); {DEPTH_SETUP}; view(0, 10); legend('a', 'b');"
    ));
    let edges: Vec<String> = surfaces(&f)
        .iter()
        .map(|s| hex(s.face_edge_color.unwrap()))
        .collect();
    // The first visible faces' flat colours: C = 0 (red) and C = 4 (magenta).
    assert_eq!(edges, ["#ff0000", "#ff00ff"]);
    let svg = f.svg();
    let swatch = "width=\"24\" height=\"10\" fill=\"none\" stroke=\"";
    assert_eq!(svg.matches(swatch).count(), 2, "{svg}");
    for edge in edges {
        assert!(svg.contains(&format!("{swatch}{edge}\"")), "{edge}");
    }
}

#[test]
fn patch_after_a_large_surface_or_contourf_counts_only_patch_object_faces() {
    // 119 x 119 = 14,161 surface faces: the 10,000-face bound of low-level
    // patch objects does not count them (the figure cap is 100,000).
    let triangle = "'XData',[0;1;1],'YData',[0;0;1],'FaceColor',[1 0 0],'EdgeColor','none'";
    let mut e = engine(&format!(
        "[X,Y] = meshgrid(1:120); surf(X, Y, sin(X/9).*cos(Y/7)); hold on; p = patch({triangle});"
    ));
    let f = e.figure.clone().unwrap();
    assert_eq!(f.series[0].patch_face_count(), 14_161);
    assert_eq!(f.series[1].patch_face_count(), 1);
    // The patch-object bound itself still holds: 1 + 9,999 fit, one more fails.
    let n = 9_999;
    let x = Matrix::from_column_vec(3, n, (0..n).flat_map(|_| [0., 1., 0.]).collect()).unwrap();
    let y = Matrix::from_column_vec(3, n, (0..n).flat_map(|_| [0., 0., 1.]).collect()).unwrap();
    e.merge_workspace(BTreeMap::from([
        ("PX".to_owned(), Array::Numeric(x)),
        ("PY".to_owned(), Array::Numeric(y)),
    ]))
    .unwrap();
    e.execute("patch('XData',PX,'YData',PY,'FaceColor',[0 0 1],'EdgeColor','none');")
        .unwrap();
    let before = e.figure.clone().unwrap();
    let err = e.execute(&format!("patch({triangle});")).unwrap_err();
    assert!(err.contains("face budget"), "{err}");
    assert!(before.same_snapshot(e.figure.as_ref().unwrap()));
    // contourf bands (more than 10,000 faces) are not patch objects either.
    let mut e = engine(
        "[X,Y] = meshgrid(linspace(-3,3,300)); \
         Z = 3*(1-X).^2.*exp(-X.^2-(Y+1).^2) - 10*(X/5-X.^3-Y.^5).*exp(-X.^2-Y.^2) - exp(-(X+1).^2-Y.^2)/3; \
         contourf(Z, 10); hold on;",
    );
    let bands: usize = e
        .figure
        .as_ref()
        .unwrap()
        .series
        .iter()
        .map(Series::patch_face_count)
        .sum();
    assert!(bands > 10_000, "{bands}");
    e.execute(&format!("patch({triangle});")).unwrap();
}

#[test]
fn bounds_error_before_anything_is_built() {
    let e = error("surf(zeros(1001, 1000));");
    assert!(e.contains("Z exceeds 1000000 elements"), "{e}");
    let e = error("mesh(zeros(400, 300));");
    assert!(e.contains("100000 patch faces"), "{e}");
    let mut engine = Engine::default();
    engine.execute("surf(zeros(250)); hold on;").unwrap();
    let before = engine.figure.clone().unwrap();
    let e = engine.execute("surf(ones(250));").unwrap_err();
    assert!(e.contains("100000 patch faces"), "{e}");
    assert!(before.same_snapshot(engine.figure.as_ref().unwrap()));
    for (source, expect) in [
        ("surf([1 2 3]);", "at least 2-by-2"),
        ("surf(1:4, 1:3, magic(3));", "X must be"),
        ("surf(1:3, 1:4, magic(3));", "Y must be"),
        ("surf(magic(3), ones(2));", "C must be the same size"),
        (
            "surf(magic(3), 'Lighting', 'gouraud');",
            "unsupported property",
        ),
        ("shading bumpy", "shading accepts"),
    ] {
        let e = error(source);
        assert!(e.contains(expect), "{source}: {e}");
    }
}

#[test]
fn contourf_of_a_large_grid_now_fits_the_raised_face_cap() {
    // 300x300 peaks with 10 levels needs more than the old 10,000 faces
    // (30 levels hit the separate 64-series cap).
    let f = run("[X,Y] = meshgrid(linspace(-3,3,300)); \
         Z = 3*(1-X).^2.*exp(-X.^2-(Y+1).^2) - 10*(X/5-X.^3-Y.^5).*exp(-X.^2-Y.^2) - exp(-(X+1).^2-Y.^2)/3; \
         contourf(Z, 10);");
    let faces: usize = f.series.iter().map(Series::patch_face_count).sum();
    assert!(faces > 10_000 && faces <= 100_000, "{faces}");
}

#[test]
fn surface_properties_get_set_and_hold_rules() {
    let mut e = engine(
        "M = magic(3); h = surf([1 2], [3 4 5], M(:, 1:2), 'EdgeColor', 'r', 'LineStyle', '--', 'LineWidth', 2, 'DisplayName', 'top'); \
         x = h.XData; y = h.YData; z = h.ZData; w = h.LineWidth; ec = h.EdgeColor; n = h.DisplayName;",
    );
    assert_eq!(num(&e, "x"), [1., 2.]);
    assert_eq!(num(&e, "y"), [3., 4., 5.]);
    assert_eq!(num(&e, "w"), [2.]);
    assert_eq!(num(&e, "ec"), [1., 0., 0.]);
    assert_eq!(text(&mut e, "n"), "top");
    assert_eq!(text(&mut e, "h.LineStyle"), "--");
    let f = e.figure.clone().unwrap();
    assert_eq!(f.series[0].face_edge_color, Some([255, 0, 0]));
    assert_eq!(f.series[0].line_width, Some(2.));
    // ZData set redraws; Visible goes through the generic series path.
    e.execute("h.ZData = zeros(3, 2); set(h, 'Visible', 'off', 'FaceColor', 'g');")
        .unwrap();
    let f = e.figure.clone().unwrap();
    assert!(f.series[0].z.iter().all(|&v| v == 0.));
    assert!(!f.series[0].visible);
    assert_eq!(f.series[0].color, [0, 255, 0]);
    assert!(e.execute("h.ZData = zeros(4);").is_err());
    // hold on keeps a 2-D axes 2-D; hold off replaces it in view(3).
    let f = run("plot(1:3); hold on; surf(magic(3));");
    assert_eq!(f.view, view3d::VIEW_2D);
    assert_eq!(f.series.len(), 2);
    let f = run("plot(1:3); surf(magic(3));");
    assert_eq!(f.view, view3d::VIEW_3D);
    assert_eq!(f.series.len(), 1);
    // surf(ax, ...) and an axes-targeted shading.
    let f = run("ax = gca; h = surf(ax, magic(3)); shading(ax, 'interp');");
    assert!(f.series[0].face_edge_color.is_none());
}

#[test]
fn mrfig_round_trip_and_crafted_colour_lists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("surf.mrfig");
    let f = run(&format!(
        "{PEAKS} h = surf(Z); h.FaceAlpha = 0.75; hold on; mesh(Z + 5); view(30, 15);"
    ));
    files::save_figure(&path, &f).unwrap();
    let loaded = files::load_figure(&path).unwrap();
    assert_eq!(loaded.view, [30., 15.]);
    assert_eq!(loaded.series.len(), 2);
    for (a, b) in f.series.iter().zip(&loaded.series) {
        assert_eq!(a.face_colors, b.face_colors);
        assert_eq!(a.face_edge_colors, b.face_edge_colors);
        assert_eq!(a.face_alpha, b.face_alpha);
        assert_eq!(&*a.z, &*b.z);
    }
    assert_eq!(loaded.series[0].face_alpha, Some(0.75));
    assert_eq!(loaded.svg(), f.svg());
    // A colour list whose length is not the face count is an invalid file.
    let bytes = std::fs::read(&path).unwrap();
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    for (field, value) in [
        ("face_colors", serde_json::json!(360)),
        ("face_edge_colors", serde_json::json!(5)),
        ("face_alpha", serde_json::json!(2.0)),
    ] {
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&bytes[12..12 + size]).unwrap();
        metadata["series"][0][field] = value;
        let json = serde_json::to_vec(&metadata).unwrap();
        let mut crafted = bytes[..8].to_vec();
        crafted.extend((json.len() as u32).to_le_bytes());
        crafted.extend(json);
        crafted.extend_from_slice(&bytes[12 + size..]);
        std::fs::write(&path, crafted).unwrap();
        assert!(files::load_figure(&path).is_err(), "{field}");
    }
    // Figures without surfaces carry none of the new fields.
    for source in [
        "plot([1 2 3], [4 5 6]); grid on;",
        "plot3([0 1], [0 1], [0 1]); hold on; fill([0 1 1], [0 0 1], 'r');",
    ] {
        let path = dir.path().join("plain.mrfig");
        files::save_figure(&path, &run(source)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let json = std::str::from_utf8(&bytes[12..12 + size]).unwrap();
        assert!(
            !json.contains("face_colors") && !json.contains("face_alpha"),
            "{source}"
        );
    }
    // 2-D files still save byte-identical to the fixtures written before 3-D.
    let path = dir.path().join("plain_2d.mrfig");
    files::save_figure(&path, &run("plot([1 2 3], [4 5 6]); grid on; xlabel('x');")).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        include_bytes!("fixtures/plot3d/plain_2d.mrfig")
    );
}

#[test]
fn svg_pdf_and_png_exports_draw_the_surface() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = engine(&format!("{PEAKS} surf(Z); colorbar;"));
    for ext in ["svg", "pdf", "png"] {
        let path = dir.path().join(format!("s.{ext}"));
        let quoted = path.display().to_string().replace('\'', "''");
        e.execute(&format!("exportgraphics(gcf, '{quoted}');"))
            .unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 1000, "{ext}");
    }
    let svg = e.figure.as_ref().unwrap().svg();
    assert_eq!(polygon_fills(&svg).len(), 361);
    let f = e.figure.as_ref().unwrap();
    let first: Vec<String> = f.series[0]
        .face_colors
        .as_ref()
        .unwrap()
        .iter()
        .map(|&c| hex(c))
        .collect();
    let mut drawn = polygon_fills(&svg);
    let mut expected = first.clone();
    drawn.sort();
    expected.sort();
    assert_eq!(drawn, expected);
}

fn flag(e: &mut Engine, expr: &str) -> bool {
    e.execute(&format!("zz_flag = double(logical({expr}));"))
        .unwrap();
    num(e, "zz_flag") == [1.]
}

fn corners(s: &Series) -> Vec<[f64; 3]> {
    (0..s.z.len())
        .map(|i| {
            let p = s.points.get(i).unwrap();
            [p[0], p[1], s.z[i]]
        })
        .collect()
}

/// Two quads of Z = [1 4 2; 8 3 7] on x = [10 40 20], y = [100; 300].
/// Vertex order is (i,j), (i+1,j), (i+1,j+1), (i,j+1).
fn asymmetric_quads() -> Vec<[f64; 3]> {
    vec![
        [10., 100., 1.],
        [10., 300., 8.],
        [40., 300., 3.],
        [40., 100., 4.],
        [40., 100., 4.],
        [40., 300., 3.],
        [20., 300., 7.],
        [20., 100., 2.],
    ]
}

fn polygon_bodies(svg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(i) = rest.find("<polygon ") {
        let tag = &rest[i..];
        let end = tag.find("/>").unwrap();
        let tag = &tag[..end];
        let key = "points=\"";
        let start = tag.find(key).unwrap() + key.len();
        let body = &tag[start..start + tag[start..].find('"').unwrap()];
        out.push(body.to_owned());
        rest = &rest[i + end..];
    }
    out
}

fn parse_points(body: &str) -> Vec<[f64; 2]> {
    body.split_whitespace()
        .map(|pair| {
            let (x, y) = pair.split_once(',').unwrap();
            [x.parse().unwrap(), y.parse().unwrap()]
        })
        .collect()
}

fn unique_sorted(values: impl IntoIterator<Item = f64>, tol: f64) -> Vec<f64> {
    let mut values: Vec<f64> = values.into_iter().collect();
    values.sort_by(f64::total_cmp);
    values.dedup_by(|a, b| (*a - *b).abs() <= tol);
    values
}

/// 2-D SVG keeps data x left-to-right and data y bottom-to-top, and each
/// polygon walks the grid vertices in order (not reversed).
fn assert_svg_orientation(svg: &str, faces: &[[f64; 3]]) {
    let polys: Vec<Vec<[f64; 2]>> = polygon_bodies(svg)
        .iter()
        .map(|b| parse_points(b))
        .collect();
    assert_eq!(polys.len(), faces.len() / 4, "{polys:?}");
    let data_x = unique_sorted(faces.iter().map(|p| p[0]), 0.);
    let data_y = unique_sorted(faces.iter().map(|p| p[1]), 0.);
    let screen_x = unique_sorted(polys.iter().flatten().map(|p| p[0]), 0.02);
    let screen_y = unique_sorted(polys.iter().flatten().map(|p| p[1]), 0.02);
    assert_eq!(screen_x.len(), data_x.len(), "{screen_x:?} vs {data_x:?}");
    assert_eq!(screen_y.len(), data_y.len(), "{screen_y:?} vs {data_y:?}");
    let map = |x: f64, y: f64| -> [f64; 2] {
        let ix = data_x.iter().position(|v| v == &x).unwrap();
        let iy = data_y.iter().position(|v| v == &y).unwrap();
        // Screen y grows downward, so the smallest data y is the largest screen y.
        [screen_x[ix], screen_y[screen_y.len() - 1 - iy]]
    };
    let close = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 0.02 && (a[1] - b[1]).abs() < 0.02;
    let mut used = vec![false; polys.len()];
    for face in faces.chunks(4) {
        let expect: Vec<[f64; 2]> = face.iter().map(|p| map(p[0], p[1])).collect();
        let reversed = vec![expect[0], expect[3], expect[2], expect[1]];
        let hit = polys.iter().position(|poly| {
            poly.len() == 4 && poly.iter().zip(&expect).all(|(a, b)| close(*a, *b))
        });
        let Some(hit) = hit else {
            panic!("no polygon in vertex order {expect:?} among {polys:?}");
        };
        assert!(!used[hit], "polygon reused");
        used[hit] = true;
        let rev =
            polys[hit].len() == 4 && polys[hit].iter().zip(&reversed).all(|(a, b)| close(*a, *b));
        assert!(!rev, "polygon is reversed: {:?}", polys[hit]);
    }
}

const GRID: &str = "x = [10 40 20]; y = [100; 300]; Z = [1 4 2; 8 3 7];";

#[test]
fn low_level_surface_grid_matches_vector_and_matrix_forms() {
    let quads = asymmetric_quads();
    // Statement path: a fresh surface is 2-D, and a second call does not replace it.
    let mut stated = engine(&format!("{GRID} surface(x, y, Z);"));
    let stated_fig = stated.figure.clone().unwrap();
    assert_eq!(stated_fig.view, view3d::VIEW_2D);
    assert!(!stated_fig.hold);
    assert_eq!(stated_fig.series.len(), 1);
    assert_eq!(corners(&stated_fig.series[0]), quads);
    stated.execute("surface(x, y, Z);").unwrap();
    let stated_fig = stated.figure.unwrap();
    assert_eq!(stated_fig.view, view3d::VIEW_2D);
    assert!(!stated_fig.hold);
    assert_eq!(stated_fig.series.len(), 2);
    assert_eq!(corners(&stated_fig.series[0]), quads);
    assert_eq!(corners(&stated_fig.series[1]), quads);

    // Nested call returns the handle the same way an assignment does.
    let mut nested = engine(&format!("{GRID} ty = get(surface(x, y, Z), 'Type');"));
    assert_eq!(text(&mut nested, "ty"), "surface");
    let nested_fig = nested.figure.unwrap();
    assert_eq!(nested_fig.series.len(), 1);
    assert_eq!(corners(&nested_fig.series[0]), quads);
    assert_eq!(nested_fig.view, view3d::VIEW_2D);

    // Handle path: scalar graphics handle, CData defaults to Z, vectors stay vectors.
    let mut vector = engine(&format!(
        "{GRID} h = surface(x, y, Z); xlim([0 50]); ylim([0 400]); \
         cm = colormap; cl = clim;"
    ));
    assert!(flag(&mut vector, "isequal(size(h), [1 1])"));
    assert!(flag(
        &mut vector,
        "strcmp(class(h), 'graphics handle') && strcmp(get(h, 'Type'), 'surface')"
    ));
    assert!(flag(
        &mut vector,
        "isequal(get(h, 'CData'), Z) && isequal(get(h, 'ZData'), Z)"
    ));
    assert!(flag(
        &mut vector,
        "isequal(size(get(h, 'XData')), [1 3]) && isequal(get(h, 'XData'), x)"
    ));
    assert!(flag(
        &mut vector,
        "isequal(size(get(h, 'YData')), [2 1]) && isequal(get(h, 'YData'), y)"
    ));
    assert_eq!(num(&vector, "cl"), [1., 8.]);
    let cm = matrix(&vector, "cm");
    let vector_fig = vector.figure.clone().unwrap();
    let s = &vector_fig.series[0];
    assert_eq!(s.kind, PlotKind::Patch);
    assert_eq!(s.patch_face_count(), 2);
    assert_eq!(corners(s), quads);
    let colors = s.face_colors.as_ref().unwrap();
    assert_eq!(colors[0], row_rgb(&cm, scaled_row(256, [1., 8.], 1.)));
    assert_eq!(colors[1], row_rgb(&cm, scaled_row(256, [1., 8.], 4.)));
    let svg = vector_fig.svg();
    let fills = polygon_fills(&svg);
    assert_eq!(fills.len(), 2);
    let mut drawn = fills.clone();
    let mut expected = colors.iter().map(|&c| hex(c)).collect::<Vec<_>>();
    drawn.sort();
    expected.sort();
    assert_eq!(drawn, expected);
    assert_svg_orientation(&svg, &quads);

    // The same Z with the matching X/Y matrices is the same two quads and the same SVG.
    let mut matrix_form = engine(
        "X = [10 40 20; 10 40 20]; Y = [100 100 100; 300 300 300]; Z = [1 4 2; 8 3 7]; \
         h = surface(X, Y, Z); xlim([0 50]); ylim([0 400]);",
    );
    assert!(flag(
        &mut matrix_form,
        "isequal(size(h), [1 1]) && strcmp(get(h, 'Type'), 'surface') && isequal(get(h, 'CData'), Z)"
    ));
    assert!(flag(
        &mut matrix_form,
        "isequal(size(get(h, 'XData')), [2 3]) && isequal(get(h, 'XData'), X) \
         && isequal(size(get(h, 'YData')), [2 3]) && isequal(get(h, 'YData'), Y)"
    ));
    let matrix_fig = matrix_form.figure.unwrap();
    assert_eq!(corners(&matrix_fig.series[0]), quads);
    assert_eq!(matrix_fig.series[0].z.as_ref(), s.z.as_ref());
    let mut vector_polys = polygon_bodies(&svg);
    let mut matrix_polys = polygon_bodies(&matrix_fig.svg());
    vector_polys.sort();
    matrix_polys.sort();
    assert_eq!(matrix_polys, vector_polys);

    // A non-separable X uses X(i, j), so the second vertex of the first quad moves.
    let mut shifted = engine(
        "X = [10 40 20; 15 40 20]; Y = [100 100 100; 300 300 300]; Z = [1 4 2; 8 3 7]; \
         h = surface(X, Y, Z);",
    );
    assert!(flag(
        &mut shifted,
        "isequal(get(h, 'XData'), X) && isequal(get(h, 'CData'), Z)"
    ));
    let shifted_fig = shifted.figure.unwrap();
    let shifted_corners = corners(&shifted_fig.series[0]);
    assert_ne!(shifted_corners, quads);
    assert_eq!(shifted_corners[0][0], 10.);
    assert_eq!(shifted_corners[1], [15., 300., 8.]);
    assert_eq!(shifted_corners[2][0], 40.);
}

#[test]
fn low_level_surface_pending_axes_defaults() {
    let mut e = engine(
        "f=figure(1); set(f,'DefaultAxesFontSize',23); s=surface([1 2],[1 2],[1 3;2 4]); fs=get(gca,'FontSize');",
    );
    assert_eq!(num(&e, "fs"), [23.]);
    e.execute("set(f,'DefaultAxesFontSize',30); surface([1 2;3 4]); fs_after=get(gca,'FontSize');")
        .unwrap();
    assert_eq!(num(&e, "fs_after"), [23.]);

    e.execute("figure(2); plot([1 2],[4 5]); ax=gca; set(ax,'FontSize',17); view(12,34); f2=gcf; set(f2,'DefaultAxesFontSize',31); surface([1 2;3 4]); fs2=get(ax,'FontSize');").unwrap();
    assert_eq!(num(&e, "fs2"), [17.]);
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.series.len(), 2);
    assert_eq!(fig.series[0].kind, PlotKind::Line);
    assert_eq!(fig.series[0].points.get(0).unwrap(), [1., 4.]);
    assert_eq!(fig.series[0].points.get(1).unwrap(), [2., 5.]);
    assert_eq!(fig.view, [12., 34.]);
    assert!(!fig.hold);
}

#[test]
fn low_level_surface_lighting_property_is_surface_only() {
    let mut e = engine(
        "s=surface([1 2;3 4]); hold on; q=patch([0 1 0],[0 0 1],'r'); assert(isprop(s,'FaceLighting')); assert(~isprop(q,'FaceLighting'));",
    );
    assert_eq!(text(&mut e, "get(s,'FaceLighting')"), "flat");
    e.execute("set(s,'FaceLighting','phong');").unwrap();
    assert_eq!(text(&mut e, "get(s,'FaceLighting')"), "phong");
    assert!(e.execute("get(q,'FaceLighting');").is_err());
    assert!(e.execute("set(q,'FaceLighting','phong');").is_err());
    assert!(
        e.execute("s.FaceLighting='invalid';")
            .unwrap_err()
            .to_string()
            .contains("FaceLighting must be")
    );
    assert_eq!(text(&mut e, "get(s,'FaceLighting')"), "phong");
}

#[test]
fn low_level_surface_keeps_line_and_view_and_rejects_bad_input() {
    let mut e = engine("plot([1 2 3], [4 5 6]); view(12, 34);");
    let axes_id = e.figure.as_ref().unwrap().axes.identity.clone();
    let line: Vec<[f64; 2]> = (0..3)
        .map(|i| e.figure.as_ref().unwrap().series[0].points.get(i).unwrap())
        .collect();
    assert_eq!(line, [[1., 4.], [2., 5.], [3., 6.]]);
    let before = e.figure.clone().unwrap();
    for (source, expect) in [
        ("surface([1 2], [100; 300], [1 4 2; 8 3 7]);", "X must be"),
        ("surface([10 40 20], [1 2 3], [1 4 2; 8 3 7]);", "Y must be"),
        (
            "surface(ones(2, 2), [100; 300], [1 4 2; 8 3 7]);",
            "X must be",
        ),
        ("surface([1 2 3]);", "at least 2-by-2"),
        (
            "surface([1 4 2; 8 3 7], 'FaceLighting', 'lambert');",
            "FaceLighting must be",
        ),
        (
            "surface([1 4 2; 8 3 7], 'FaceLighting', 1);",
            "FaceLighting must be",
        ),
        (
            "surface([1 4 2; 8 3 7], 'NotAName', 1);",
            "unsupported property",
        ),
        ("surface([1 4 2; 8 3 7], 'EdgeColor');", "Name,Value"),
        (
            "surf(magic(3), 'Lighting', 'gouraud');",
            "unsupported property",
        ),
    ] {
        let err = e.execute(source).unwrap_err().to_string();
        assert!(err.contains(expect), "{source}: {err}");
        if !source.contains("FaceLighting") {
            assert!(
                err.contains("surface:") || source.starts_with("surf("),
                "{source}: {err}"
            );
        }
        let fig = e.figure.as_ref().unwrap();
        assert!(before.same_snapshot(fig), "{source}");
        assert_eq!(fig.series.len(), 1, "{source}");
        assert_eq!(fig.view, [12., 34.], "{source}");
        assert!(!fig.hold, "{source}");
        assert!(fig.axes.identity.same_object(&axes_id), "{source}");
    }
    e.execute("surface([10 40 20], [100; 300], [1 4 2; 8 3 7]);")
        .unwrap();
    let fig = e.figure.as_ref().unwrap();
    assert_eq!(fig.series.len(), 2);
    assert_eq!(fig.series[0].kind, PlotKind::Line);
    let kept: Vec<[f64; 2]> = (0..3)
        .map(|i| fig.series[0].points.get(i).unwrap())
        .collect();
    assert_eq!(kept, line);
    assert_eq!(corners(&fig.series[1]), asymmetric_quads());
    assert_eq!(fig.view, [12., 34.]);
    assert_ne!(fig.view, view3d::VIEW_3D);
    assert!(!fig.hold);
    assert!(fig.axes.identity.same_object(&axes_id));

    let mut bare = Engine::default();
    let err = bare.execute("surface([1 2 3]);").unwrap_err().to_string();
    assert!(bare.figure.is_none(), "{err}");
    assert!(err.contains("surface:"), "{err}");
}

#[test]
fn low_level_surface_facelighting_is_stored_and_faces_stay_unlit() {
    let mut plain = engine(
        "X = [10 20 30]; Y = [100; 200]; Z = [11 22 33; 44 55 66]; \
         h = surface(X, Y, Z); fl = get(h, 'FaceLighting');",
    );
    assert_eq!(text(&mut plain, "fl"), "flat");
    let plain_fig = plain.figure.clone().unwrap();
    let plain_colors = plain_fig.series[0].face_colors.clone().unwrap();
    assert_eq!(plain_colors.len(), 2);
    assert_ne!(plain_colors[0], plain_colors[1]);
    let plain_faces = painted_faces(&plain_fig.svg());
    assert_eq!(plain_faces.len(), 2);
    assert!(
        plain_faces
            .iter()
            .all(|(_, _, edge)| edge.as_deref() == Some("#000000"))
    );

    let mut e = engine(
        "ca = [1 4; 2 5; 3 6]; cb = [10 40; 20 50; 30 60]; \
         X = [10 20 30]; Y = [100; 200]; \
         h = surface(X, Y, ca'+cb', 'EdgeColor', 'none', 'LineStyle', 'none', 'FaceLighting', 'phong'); \
         ec = get(h, 'EdgeColor'); ls = get(h, 'LineStyle'); fl = get(h, 'FaceLighting');",
    );
    assert_eq!(text(&mut e, "ec"), "none");
    assert_eq!(text(&mut e, "ls"), "none");
    assert_eq!(text(&mut e, "fl"), "phong");
    assert!(flag(
        &mut e,
        "isequal(get(h, 'CData'), [11 22 33; 44 55 66]) && isequal(get(h, 'ZData'), ca'+cb')"
    ));
    let fig = e.figure.clone().unwrap();
    assert_eq!(fig.series[0].patch_face_count(), 2);
    assert_eq!(
        corners(&fig.series[0]),
        vec![
            [10., 100., 11.],
            [10., 200., 44.],
            [20., 200., 55.],
            [20., 100., 22.],
            [20., 100., 22.],
            [20., 200., 55.],
            [30., 200., 66.],
            [30., 100., 33.],
        ]
    );
    assert_eq!(fig.series[0].face_colors.as_ref().unwrap(), &plain_colors);
    assert!(fig.series[0].face_edge_color.is_none());
    let svg = fig.svg();
    assert_eq!(polygon_fills(&svg).len(), 2);
    let faces = painted_faces(&svg);
    assert_eq!(faces.len(), 2);
    assert!(faces.iter().all(|(_, _, edge)| edge.is_none()));
    let mut lit = polygon_fills(&svg);
    let mut unlit = polygon_fills(&plain_fig.svg());
    lit.sort();
    unlit.sort();
    assert_eq!(lit, unlit);

    e.execute("set(h, 'FaceLighting', 'gouraud');").unwrap();
    assert_eq!(text(&mut e, "get(h, 'FaceLighting')"), "gouraud");
    assert_eq!(
        e.figure.as_ref().unwrap().series[0]
            .face_colors
            .as_ref()
            .unwrap(),
        &plain_colors
    );
    let before = e.figure.clone().unwrap();
    for source in [
        "set(h, 'FaceLighting', 'banana');",
        "set(h, 'FaceLighting', 1);",
        "h.FaceLighting = 'lambert';",
        "set(h, 'Lighting', 'phong');",
    ] {
        let err = e.execute(source).unwrap_err().to_string();
        assert!(!err.is_empty(), "{source}");
        assert!(
            err.contains("FaceLighting must be")
                || err.contains("unsupported")
                || err.contains("Unrecognized")
                || err.contains("Patch set supports"),
            "{source}: {err}"
        );
        assert!(before.same_snapshot(e.figure.as_ref().unwrap()), "{source}");
        assert_eq!(text(&mut e, "get(h, 'FaceLighting')"), "gouraud");
    }

    let mut upper = engine("h = surface([1 2; 3 4], 'FaceLighting', 'PHONG');");
    assert_eq!(text(&mut upper, "h.FaceLighting"), "phong");
    let mut flat = engine("h = surface([1 2; 3 4], 'FaceLighting', 'Flat');");
    assert_eq!(text(&mut flat, "get(h, 'FaceLighting')"), "flat");
    let err = error("surface([1 2; 3 4], 'FaceLighting', 'banana');");
    assert!(err.contains("FaceLighting must be"), "{err}");
    let err = error("surface([1 2; 3 4], 'Nope', 1);");
    assert!(err.contains("unsupported property"), "{err}");
}
