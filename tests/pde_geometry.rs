//! decsg / csgdel. Expected values are plain geometry, stated per test:
//! polygon areas, circle and ellipse areas (pi r^2, pi a b), circular caps
//! (r^2 acos(h/r) - h sqrt(r^2-h^2)). Region areas are computed here from dl
//! independently of the engine (Green's theorem over each region boundary).
use matrust::{Engine, Matrix};
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};

fn run(src: &str) -> Engine {
    let mut e = Engine::default();
    if let Err(err) = e.execute(src) {
        panic!("{src}\n{err}");
    }
    e
}

fn var(e: &Engine, name: &str) -> Matrix {
    e.variables[name].clone()
}

fn error(src: &str) -> String {
    Engine::default().execute(src).unwrap_err()
}

/// Integral of ((x - ox) dy - (y - oy) dx)/2 along one dl column, start to end.
fn area_term(dl: &Matrix, j: usize, origin: [f64; 2]) -> f64 {
    let (ox, oy) = (origin[0], origin[1]);
    let (x0, x1, y0, y1) = (dl.get(1, j), dl.get(2, j), dl.get(3, j), dl.get(4, j));
    let chord = ((x0 - ox) * (y1 - oy) - (x1 - ox) * (y0 - oy)) / 2.;
    match dl.get(0, j) as i32 {
        2 => chord,
        1 => {
            let (xc, yc, r) = (dl.get(7, j), dl.get(8, j), dl.get(9, j));
            let t0 = (y0 - yc).atan2(x0 - xc);
            let d = ((y1 - yc).atan2(x1 - xc) - t0).rem_euclid(TAU);
            (r * r * d + (xc - ox) * (y1 - y0) - (yc - oy) * (x1 - x0)) / 2.
        }
        4 => {
            let (xc, yc, a, b, phi) = (
                dl.get(7, j),
                dl.get(8, j),
                dl.get(9, j),
                dl.get(10, j),
                dl.get(11, j),
            );
            let t = |x: f64, y: f64| {
                let (dx, dy) = (x - xc, y - yc);
                let (u, v) = (
                    phi.cos() * dx + phi.sin() * dy,
                    -phi.sin() * dx + phi.cos() * dy,
                );
                (v / b).atan2(u / a)
            };
            let d = (t(x1, y1) - t(x0, y0)).rem_euclid(TAU);
            (a * b * d + (xc - ox) * (y1 - y0) - (yc - oy) * (x1 - x0)) / 2.
        }
        k => panic!("segment type {k}"),
    }
}

/// Area of every region 1..=n from the dl boundary.
fn region_areas(dl: &Matrix) -> Vec<f64> {
    let n = (0..dl.cols())
        .map(|j| dl.get(5, j).max(dl.get(6, j)) as usize)
        .max()
        .unwrap_or(0);
    let mut origin: Vec<Option<[f64; 2]>> = vec![None; n];
    for j in 0..dl.cols() {
        let p = [dl.get(1, j), dl.get(3, j)];
        for side in [dl.get(5, j) as usize, dl.get(6, j) as usize] {
            if side > 0 && origin[side - 1].is_none() {
                origin[side - 1] = Some(p);
            }
        }
    }
    let mut area = vec![0.; n];
    for j in 0..dl.cols() {
        let (l, r) = (dl.get(5, j) as usize, dl.get(6, j) as usize);
        if l > 0 {
            area[l - 1] += area_term(dl, j, origin[l - 1].unwrap_or([0., 0.]));
        }
        if r > 0 {
            area[r - 1] -= area_term(dl, j, origin[r - 1].unwrap_or([0., 0.]));
        }
    }
    area
}

fn key(x: f64, y: f64) -> (u64, u64) {
    ((x + 0.).to_bits(), (y + 0.).to_bits())
}

fn segment_length(dl: &Matrix, j: usize) -> f64 {
    let chord = (dl.get(2, j) - dl.get(1, j)).hypot(dl.get(4, j) - dl.get(3, j));
    // Arcs here are at most a quarter turn, so the chord bounds the length.
    chord
}

/// Invariants for every case: each region boundary closes (as many segments
/// leave every point as arrive, end points shared exactly), labels are
/// 1..n, no column has zero length, and csgdel(dl,bt,[]) (read dl into the
/// geometry type and write it back) returns dl and bt unchanged.
fn check_invariants(e: &mut Engine) {
    let dl = var(e, "dl");
    let n = region_areas(&dl).len();
    let mut balance: HashMap<(usize, (u64, u64)), i32> = HashMap::new();
    let mut used = vec![false; n + 1];
    for j in 0..dl.cols() {
        assert!(
            segment_length(&dl, j) > 1e-9,
            "column {} has zero length",
            j + 1
        );
        let (l, r) = (dl.get(5, j) as usize, dl.get(6, j) as usize);
        assert_ne!(l, r, "column {}", j + 1);
        let (s, t) = (
            key(dl.get(1, j), dl.get(3, j)),
            key(dl.get(2, j), dl.get(4, j)),
        );
        for (face, from, to) in [(l, s, t), (r, t, s)] {
            used[face] = true;
            if face > 0 {
                *balance.entry((face, from)).or_default() -= 1;
                *balance.entry((face, to)).or_default() += 1;
            }
        }
    }
    assert!(used[1..].iter().all(|&u| u), "region labels have gaps");
    assert!(
        balance.values().all(|&v| v == 0),
        "a region boundary is open"
    );
    e.execute("[rt_dl, rt_bt] = csgdel(dl, bt, []); rt_btd = double(rt_bt); btd = double(bt);")
        .unwrap();
    let rt = var(e, "rt_dl");
    assert_eq!((rt.rows(), rt.cols()), (dl.rows(), dl.cols()));
    assert!(
        rt.data()
            .iter()
            .zip(dl.data())
            .all(|(a, b)| a.to_bits() == b.to_bits() || a == b)
    );
    assert_eq!(var(e, "rt_btd").data(), var(e, "btd").data());
}

fn bt_rows(e: &mut Engine) -> Vec<String> {
    e.execute("btd = double(bt);").unwrap();
    let bt = var(e, "btd");
    (0..bt.rows())
        .map(|i| {
            (0..bt.cols())
                .map(|j| if bt.get(i, j) != 0. { '1' } else { '0' })
                .collect()
        })
        .collect()
}

const SQUARE_MINUS_DISK: &str =
    "R1=[3;4;-1;1;1;-1;-1;-1;1;1]; C1=[1;0;0;0.5;0;0;0;0;0;0]; gd=[R1 C1]; ns=['R1';'C1']';";

#[test]
fn square_minus_disk_has_one_face_four_lines_and_four_arcs() {
    let mut e = run(&format!(
        "{SQUARE_MINUS_DISK} [dl,bt]=decsg(gd,'R1-C1',ns);"
    ));
    let dl = var(&e, "dl");
    assert_eq!((dl.rows(), dl.cols()), (10, 8));
    let types: Vec<f64> = (0..8).map(|j| dl.get(0, j)).collect();
    assert_eq!(types.iter().filter(|&&t| t == 2.).count(), 4);
    assert_eq!(types.iter().filter(|&&t| t == 1.).count(), 4);
    for j in 0..8 {
        let pair = (dl.get(5, j), dl.get(6, j));
        if dl.get(0, j) == 1. {
            // Arcs run counter-clockwise around the hole: face on the right.
            assert_eq!(pair, (0., 1.), "arc column {}", j + 1);
            assert_eq!((dl.get(7, j), dl.get(8, j), dl.get(9, j)), (0., 0., 0.5));
        } else {
            assert_eq!(pair, (1., 0.), "line column {}", j + 1);
        }
    }
    let area = region_areas(&dl);
    assert_eq!(area.len(), 1);
    eprintln!(
        "square minus disk area {:.17}, expected {:.17}",
        area[0],
        4. - PI / 4.
    );
    assert!((area[0] - (4. - PI / 4.)).abs() <= 1e-12);
    assert_eq!(bt_rows(&mut e), ["10"]);
    check_invariants(&mut e);
}

const TWO_RECTANGLES: &str =
    "R1=[3;4;0;2;2;0;0;0;1;1]; R2=[3;4;1;3;3;1;0.5;0.5;1.5;1.5]; gd=[R1 R2]; ns=['R1';'R2']';";

#[test]
fn union_of_overlapping_rectangles_keeps_three_regions_and_csgdel_joins_them() {
    let mut e = run(&format!("{TWO_RECTANGLES} [dl,bt]=decsg(gd,'R1+R2',ns);"));
    let dl = var(&e, "dl");
    assert_eq!(dl.rows(), 7);
    // R1 and R2 cross at (1,1) and (2,0.5): each splits into 6 sides.
    assert_eq!(dl.cols(), 12);
    let mut rows = bt_rows(&mut e);
    let areas = region_areas(&dl);
    // R1 only 2 - 0.5, R2 only 2 - 0.5, both 1 x 0.5.
    let mut by_pattern: Vec<(String, f64)> =
        rows.iter().cloned().zip(areas.iter().copied()).collect();
    by_pattern.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(by_pattern.len(), 3);
    for ((pattern, area), (want_p, want_a)) in
        by_pattern
            .iter()
            .zip([("01", 1.5), ("10", 1.5), ("11", 0.5)])
    {
        assert_eq!(pattern, want_p);
        assert!((area - want_a).abs() < 1e-14, "{pattern}: {area}");
    }
    rows.sort();
    assert_eq!(rows, ["01", "10", "11"]);
    assert!((areas.iter().sum::<f64>() - 3.5).abs() < 1e-14);
    let internal = (0..12)
        .filter(|&j| dl.get(5, j) != 0. && dl.get(6, j) != 0.)
        .count();
    assert_eq!(internal, 4);
    check_invariants(&mut e);

    e.execute("[dl2,bt2]=csgdel(dl,bt); bt2d=double(bt2);")
        .unwrap();
    let dl2 = var(&e, "dl2");
    assert_eq!(dl2.cols(), 8);
    assert!((0..8).all(|j| dl2.get(5, j) == 0. || dl2.get(6, j) == 0.));
    let areas2 = region_areas(&dl2);
    assert_eq!(areas2.len(), 1);
    assert!((areas2[0] - 3.5).abs() < 1e-14);
    assert_eq!(var(&e, "bt2d").data(), &[1., 1.]);
    e.execute("dl=dl2; bt=bt2;").unwrap();
    check_invariants(&mut e);
}

#[test]
fn csgdel_with_a_border_list_merges_only_those_regions() {
    let mut e = run(&format!("{TWO_RECTANGLES} [dl,bt]=decsg(gd,'R1+R2',ns);"));
    let dl = var(&e, "dl");
    let rows = bt_rows(&mut e);
    let only_r1 = rows.iter().position(|r| r == "10").unwrap() + 1;
    let both = rows.iter().position(|r| r == "11").unwrap() + 1;
    let pair = |j: usize| {
        let (l, r) = (dl.get(5, j) as usize, dl.get(6, j) as usize);
        (l.min(r), l.max(r))
    };
    let want = (only_r1.min(both), only_r1.max(both));
    let shared: Vec<usize> = (0..dl.cols()).filter(|&j| pair(j) == want).collect();
    // R2's bottom and left pieces inside R1.
    assert_eq!(shared.len(), 2);
    let outer = (0..dl.cols())
        .find(|&j| dl.get(5, j) == 0. || dl.get(6, j) == 0.)
        .unwrap();
    // Deleting one of the two borders also drops the other (same region on both sides).
    e.execute(&format!("[dl,bt]=csgdel(dl,bt,{});", shared[0] + 1))
        .unwrap();
    let dl3 = var(&e, "dl");
    assert_eq!(dl3.cols(), 10);
    let mut areas = region_areas(&dl3);
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 2);
    // R1 (2) and R2 only (1.5).
    assert!(
        (areas[0] - 1.5).abs() < 1e-14 && (areas[1] - 2.).abs() < 1e-14,
        "{areas:?}"
    );
    let mut rows = bt_rows(&mut e);
    rows.sort();
    assert_eq!(rows, ["01", "11"]);
    check_invariants(&mut e);
    let err = error(&format!(
        "{TWO_RECTANGLES} [dl,bt]=decsg(gd,'R1+R2',ns); csgdel(dl,bt,{});",
        outer + 1
    ));
    assert!(err.contains("borders the outside"), "{err}");
}

#[test]
fn intersection_with_a_circle_crossing_the_rectangle_edge() {
    // Disk centre (1,0.8), r 0.5, cut by y = 1: area pi r^2 - cap at h = 0.2.
    let mut e = run(
        "R1=[3;4;0;2;2;0;0;0;1;1]; C1=[1;1;0.8;0.5;0;0;0;0;0;0]; [dl,bt]=decsg([R1 C1],'R1*C1',['R1';'C1']');",
    );
    let dl = var(&e, "dl");
    let (r, h) = (0.5f64, 0.2f64);
    let cap = r * r * (h / r).acos() - h * (r * r - h * h).sqrt();
    let area = region_areas(&dl);
    assert_eq!(area.len(), 1);
    assert!((area[0] - (PI * r * r - cap)).abs() < 1e-13, "{}", area[0]);
    // Two quarter arcs below the centre, two cut arcs, one chord of the edge.
    assert_eq!(dl.cols(), 5);
    assert_eq!((0..5).filter(|&j| dl.get(0, j) == 2.).count(), 1);
    let x = 1. + (r * r - h * h).sqrt();
    assert!((0..5).any(|j| (dl.get(1, j) - x).abs() < 1e-15 || (dl.get(2, j) - x).abs() < 1e-15));
    assert_eq!(bt_rows(&mut e), ["11"]);
    check_invariants(&mut e);
}

#[test]
fn difference_with_a_shared_edge_merges_coincident_pieces() {
    // R2 = [1,3]x[0.5,1] shares y = 1 for x in [1,2] with R1 = [0,2]x[0,1].
    let mut e = run(
        "R1=[3;4;0;2;2;0;0;0;1;1]; R2=[3;4;1;3;3;1;0.5;0.5;1;1]; [dl,bt]=decsg([R1 R2],'R1-R2',['R1';'R2']');",
    );
    let dl = var(&e, "dl");
    // L-shaped result: 2 - 0.5, six sides.
    let area = region_areas(&dl);
    assert_eq!(area.len(), 1);
    assert!((area[0] - 1.5).abs() < 1e-15);
    assert_eq!(dl.cols(), 6);
    assert_eq!(bt_rows(&mut e), ["10"]);
    check_invariants(&mut e);
    // Union: the shared piece appears once.
    let mut e = run(
        "R1=[3;4;0;2;2;0;0;0;1;1]; R2=[3;4;1;3;3;1;0.5;0.5;1;1]; [dl,bt]=decsg([R1 R2],'R1+R2',['R1';'R2']');",
    );
    let dl = var(&e, "dl");
    let top = (0..dl.cols())
        .filter(|&j| {
            dl.get(3, j) == 1.
                && dl.get(4, j) == 1.
                && dl.get(1, j).min(dl.get(2, j)) == 1.
                && dl.get(1, j).max(dl.get(2, j)) == 2.
        })
        .count();
    assert_eq!(top, 1);
    let mut areas = region_areas(&dl);
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 3);
    assert!((areas.iter().sum::<f64>() - 2.5).abs() < 1e-15);
    check_invariants(&mut e);
}

#[test]
fn decsg_without_formula_keeps_every_region() {
    let mut e = run(&format!("{SQUARE_MINUS_DISK} [dl,bt]=decsg(gd);"));
    let dl = var(&e, "dl");
    assert_eq!(dl.cols(), 8);
    let mut rows = bt_rows(&mut e);
    let areas = region_areas(&dl);
    for (row, area) in rows.iter().zip(&areas) {
        let want = if row == "11" { PI / 4. } else { 4. - PI / 4. };
        assert!((area - want).abs() < 1e-12, "{row}: {area}");
    }
    rows.sort();
    assert_eq!(rows, ["10", "11"]);
    // Arcs now separate two kept regions.
    assert!(
        (0..8)
            .filter(|&j| dl.get(0, j) == 1.)
            .all(|j| dl.get(5, j) != 0. && dl.get(6, j) != 0.)
    );
    check_invariants(&mut e);
    let mut e = run(&format!("{TWO_RECTANGLES} [dl,bt]=decsg(gd);"));
    assert_eq!(bt_rows(&mut e).len(), 3);
    check_invariants(&mut e);
}

/// Sum of region areas inside each shape equals that shape's area.
fn check_shape_areas(e: &mut Engine, shape_areas: &[f64], tol: f64) {
    let dl = var(e, "dl");
    let areas = region_areas(&dl);
    let rows = bt_rows(e);
    for (j, want) in shape_areas.iter().enumerate() {
        let got: f64 = rows
            .iter()
            .zip(&areas)
            .filter(|(r, _)| r.as_bytes()[j] == b'1')
            .map(|(_, a)| a)
            .sum();
        eprintln!(
            "shape {}: regions sum {got:.15}, shape area {want:.15}",
            j + 1
        );
        assert!((got - want).abs() < tol, "shape {}: {got} vs {want}", j + 1);
    }
}

#[test]
fn ellipse_cases_use_twelve_rows() {
    // Rotated ellipse alone: pi a b.
    let mut e = run("E1=[4;0.2;-0.1;2;1;pi/6]; ns='E1'; [dl,bt]=decsg(E1,'E1',ns');");
    let dl = var(&e, "dl");
    assert_eq!((dl.rows(), dl.cols()), (12, 4));
    assert!((region_areas(&dl)[0] - 2. * PI).abs() < 1e-13);
    assert!((0..4).all(|j| dl.get(0, j) == 4. && dl.get(11, j) == PI / 6.));
    check_invariants(&mut e);
    // Ellipse minus a centred disk: 2 pi - pi/4, eight arcs.
    let mut e = run(
        "E1=[4;0;0;2;1;pi/6]; C1=[1;0;0;0.5;0;0]; [dl,bt]=decsg([E1 C1],'E1-C1',['E1';'C1']');",
    );
    let dl = var(&e, "dl");
    assert_eq!((dl.rows(), dl.cols()), (12, 8));
    assert!((region_areas(&dl)[0] - (2. * PI - PI / 4.)).abs() < 1e-13);
    check_invariants(&mut e);
    // Ellipse crossing a circle: every region found, shape areas add up.
    let mut e = run("E1=[4;0;0;2;1;pi/6]; C1=[1;1.5;0.5;0.8;0;0]; [dl,bt]=decsg([E1 C1]);");
    assert_eq!(var(&e, "dl").rows(), 12);
    let mut rows = bt_rows(&mut e);
    rows.sort();
    assert_eq!(rows, ["01", "10", "11"]);
    check_shape_areas(&mut e, &[2. * PI, PI * 0.64], 1e-12);
    check_invariants(&mut e);
    // Two ellipses crossing four times (numeric conic-conic intersection;
    // dense sampling of E1's implicit along E2 also finds 4 sign changes):
    // E2 pokes out of E1 at both ends, so 5 regions.
    let mut e = run("E1=[4;0;0;2;1;0]; E2=[4;0.5;0.3;1.5;0.7;1]; [dl,bt]=decsg([E1 E2]);");
    let mut rows = bt_rows(&mut e);
    rows.sort();
    assert_eq!(rows, ["01", "01", "10", "10", "11"]);
    check_shape_areas(&mut e, &[2. * PI, PI * 1.05], 1e-12);
    check_invariants(&mut e);
}

#[test]
fn l_shaped_polygon_and_union_with_a_circle() {
    // x=[0 2 2 1 1 0], y=[0 0 1 1 2 2]: area 2 + 1.
    let p1 = "P1=[2;6;0;2;2;1;1;0;0;0;1;1;2;2];";
    let mut e = run(&format!("{p1} ns='P1'; [dl,bt]=decsg(P1,'P1',ns');"));
    let dl = var(&e, "dl");
    assert_eq!((dl.rows(), dl.cols()), (7, 6));
    assert!((region_areas(&dl)[0] - 3.).abs() < 1e-15);
    check_invariants(&mut e);
    // Circle at the inner corner (1,1), r 0.5: three quarters lie in P1.
    let mut e = run(&format!(
        "{p1} C1=[1;1;1;0.5;0;0;0;0;0;0;0;0;0;0]; [dl,bt]=decsg([P1 C1],'P1+C1',['P1';'C1']');"
    ));
    let mut rows = bt_rows(&mut e);
    rows.sort();
    assert_eq!(rows, ["01", "10", "11"]);
    check_shape_areas(&mut e, &[3., PI / 4.], 1e-13);
    let areas = region_areas(&var(&e, "dl"));
    assert!((areas.iter().sum::<f64>() - (3. + PI / 16.)).abs() < 1e-13);
    check_invariants(&mut e);
    // Clockwise vertex order gives the same region.
    let mut e = run("P1=[2;6;0;0;1;1;2;2;0;2;2;1;1;0]; [dl,bt]=decsg(P1);");
    assert!((region_areas(&var(&e, "dl"))[0] - 3.).abs() < 1e-15);
    check_invariants(&mut e);
}

#[test]
fn rectangle_tangent_to_circle_and_nested_shapes() {
    // Circle touching the rectangle's top side at (1,1) from above.
    let mut e =
        run("R1=[3;4;0;2;2;0;0;0;1;1]; C1=[1;1;1.5;0.5;0;0;0;0;0;0]; [dl,bt]=decsg([R1 C1]);");
    let mut rows = bt_rows(&mut e);
    rows.sort();
    assert_eq!(rows, ["01", "10"]);
    check_shape_areas(&mut e, &[2., PI / 4.], 1e-13);
    check_invariants(&mut e);
    // Disk inside a disk inside a square: three nested regions.
    let mut e = run(
        "R1=[3;4;-2;2;2;-2;-2;-2;2;2]; C1=[1;0;0;1.5;0;0;0;0;0;0]; C2=[1;0.3;0;0.5;0;0;0;0;0;0]; [dl,bt]=decsg([R1 C1 C2],'R1-C1+C2',['R1';'C1';'C2']');",
    );
    let mut rows = bt_rows(&mut e);
    rows.sort();
    assert_eq!(rows, ["100", "111"]);
    let mut areas = region_areas(&var(&e, "dl"));
    areas.sort_by(f64::total_cmp);
    assert!(
        (areas[0] - PI / 4.).abs() < 1e-13 && (areas[1] - (16. - 2.25 * PI)).abs() < 1e-12,
        "{areas:?}"
    );
    check_invariants(&mut e);
}

#[test]
fn string_formula_numeric_names_and_single_output() {
    let mut e = run(&format!(
        "{SQUARE_MINUS_DISK} dl=decsg(gd,\"R1 - (C1)\",double(ns)); bt=decsg(gd,'R1-C1',ns)~=0;"
    ));
    assert_eq!(var(&e, "dl").cols(), 8);
    let e2 = run(&format!(
        "{SQUARE_MINUS_DISK} [dl,bt]=decsg(gd,'(R1+C1)-C1',ns);"
    ));
    assert_eq!(var(&e2, "dl").data(), var(&e, "dl").data());
    let e3 = run(&format!(
        "{SQUARE_MINUS_DISK} [dl,bt]=decsg(gd,'R1*C1',ns);"
    ));
    assert!((region_areas(&var(&e3, "dl"))[0] - PI / 4.).abs() < 1e-14);
    let _ = &mut e;
}

#[test]
fn errors_are_explicit() {
    let ns = "ns=['A';'B']';";
    for (src, needle) in [
        ("decsg([5;0;0;1])", "unknown shape code"),
        ("decsg([2;2;0;1;0;1])", "at least 3 vertices"),
        ("decsg([2;4;0;1;1;0;0;1;0;1])", "self-intersecting"),
        ("decsg([1;0;0;0])", "radius must be positive"),
        ("decsg([1;0;0;-1])", "radius must be positive"),
        ("decsg([4;0;0;1;0;0])", "semi-axes must be positive"),
        ("decsg([3;3;0;1;1;0;0;1;1;0])", "rectangle has 4 vertices"),
        (
            &format!("{ns} decsg([1 1;0 3;0 0;1 1],'A+C',ns)"),
            "unknown name 'C'",
        ),
        (
            "ns='A'; decsg([1 1;0 3;0 0;1 1],'A',ns')",
            "1 names but gd has 2 shapes",
        ),
        (
            &format!("{ns} decsg([1 1;0 3;0 0;1 1],'A+',ns)"),
            "malformed set formula",
        ),
        (
            &format!("{ns} decsg([1 1;0 3;0 0;1 1],'(A+B',ns)"),
            "missing ')'",
        ),
        (
            &format!("{ns} decsg([1 1;0 3;0 0;1 1],'A B',ns)"),
            "malformed set formula",
        ),
        (
            &format!("{ns} decsg([1 1;0 3;0 0;1 1],'A&B',ns)"),
            "malformed set formula",
        ),
        ("decsg([1;0;0;1],'A')", "decsg(gd,sf,ns)"),
        ("[a,b,c]=decsg([1;0;0;1])", "too many output arguments"),
        ("csgdel([2;0;1;0;0;1;0],1)", "not closed"),
        ("csgdel([2;0;1;0;0;3;0],1)", "without gaps"),
    ] {
        let err = error(src);
        assert!(err.contains(needle), "{src}: {err}");
    }
}

fn kept_area(e: &Engine) -> f64 {
    region_areas(&var(e, "dl")).iter().sum()
}

#[test]
fn set_formula_precedence_is_minus_then_times_then_plus() {
    // Documented order, highest first: -, *, +; equal operators left to right.
    let ns = "ns=['R1';'C1';'C2']';";
    let r1 = "R1=[3;4;0;2;2;0;0;0;2;2];"; // square [0,2]^2, area 4
    let check = |shapes: &str, sf: &str, same: &str, want: f64| {
        let mut e = run(&format!(
            "{r1} {shapes} gd=[R1 C1 C2]; {ns} [dl,bt]=decsg(gd,'{sf}',ns);"
        ));
        let got = kept_area(&e);
        assert!((got - want).abs() < 1e-12, "{sf}: {got} vs {want}");
        check_invariants(&mut e);
        let e2 = run(&format!(
            "{r1} {shapes} gd=[R1 C1 C2]; {ns} [dl,bt]=decsg(gd,'{same}',ns);"
        ));
        assert_eq!(
            var(&e, "dl").data(),
            var(&e2, "dl").data(),
            "{sf} vs {same}"
        );
    };
    // C1 disjoint disk r=1 (area pi), C2 disk r=0.5 inside R1 (area pi/4):
    // R1+(C1-C2) = R1 u C1 = 4 + pi; the other reading (R1+C1)-C2 = 4 + 3pi/4.
    let disjoint = "C1=[1;5;0;1;0;0;0;0;0;0]; C2=[1;1;1;0.5;0;0;0;0;0;0];";
    check(disjoint, "R1+C1-C2", "R1+(C1-C2)", 4. + PI);
    // C1 disk r=0.5 inside R1, C2 disjoint unit disk: (R1*C1)+C2 = pi/4 + pi;
    // the other reading R1*(C1+C2) = pi/4.
    let inner = "C1=[1;1;1;0.5;0;0;0;0;0;0]; C2=[1;5;0;1;0;0;0;0;0;0];";
    check(inner, "R1*C1+C2", "(R1*C1)+C2", PI / 4. + PI);
    // C1 disk r=0.5 at (1,1), C2 disk r=0.5 at the corner (2,2), C1 and C2
    // disjoint: (R1-C1)*C2 = quarter of C2 = pi/16; R1-(C1*C2) = 4.
    let corner = "C1=[1;1;1;0.5;0;0;0;0;0;0]; C2=[1;2;2;0.5;0;0;0;0;0;0];";
    check(corner, "R1-C1*C2", "(R1-C1)*C2", PI / 16.);
}

/// Ellipse a=5, b=1, phi=0.47 at the origin and a circle of radius 0.05
/// centred on the ellipse boundary at parameter 1.
fn long_ellipse_small_circle(ellipse_first: bool) -> String {
    let (a, b, phi, t, r) = (5f64, 1f64, 0.47f64, 1f64, 0.05f64);
    let (u, v) = (a * t.cos(), b * t.sin());
    let (x, y) = (phi.cos() * u - phi.sin() * v, phi.sin() * u + phi.cos() * v);
    let e = format!("E1=[4;0;0;{a:?};{b:?};{phi:?}];");
    let c = format!("C1=[1;{x:?};{y:?};{r:?};0;0];");
    let gd = if ellipse_first {
        "gd=[E1 C1];"
    } else {
        "gd=[C1 E1];"
    };
    format!("{e} {c} {gd} [dl,bt]=decsg(gd);")
}

#[test]
fn small_circle_on_a_long_ellipse_splits_the_ellipse_boundary() {
    let (r, ab) = (0.05f64, 5f64);
    for first in [true, false] {
        let mut e = run(&long_ellipse_small_circle(first));
        // Both crossings found: three regions (ellipse only, circle only, lens).
        let mut rows = bt_rows(&mut e);
        rows.sort();
        assert_eq!(rows, ["01", "10", "11"]);
        // Regions inside each shape sum to its area (pi a b, pi r^2), so the
        // faces sum to the union area.
        let shape_areas = if first {
            [PI * ab, PI * r * r]
        } else {
            [PI * r * r, PI * ab]
        };
        check_shape_areas(&mut e, &shape_areas, 1e-12);
        // Lens: about half the disk (boundary curvature 0.066 changes it by
        // under 1e-5 at r = 0.05).
        let areas = region_areas(&var(&e, "dl"));
        let lens: f64 = bt_rows(&mut e)
            .iter()
            .zip(&areas)
            .filter(|(row, _)| *row == "11")
            .map(|(_, a)| a)
            .sum();
        assert!((lens - PI * r * r / 2.).abs() < 1e-4, "lens {lens}");
        check_invariants(&mut e);
    }
}

#[test]
fn face_areas_do_not_depend_on_column_order() {
    let sorted_areas = |src: &str| {
        let mut e = run(src);
        check_invariants(&mut e);
        let mut a = region_areas(&var(&e, "dl"));
        a.sort_by(f64::total_cmp);
        a
    };
    let cases = [
        (
            long_ellipse_small_circle(true),
            long_ellipse_small_circle(false),
        ),
        (
            "E1=[4;0;0;2;1;0]; E2=[4;0.5;0.3;1.5;0.7;1]; [dl,bt]=decsg([E1 E2]);".to_string(),
            "E1=[4;0;0;2;1;0]; E2=[4;0.5;0.3;1.5;0.7;1]; [dl,bt]=decsg([E2 E1]);".to_string(),
        ),
        (
            "E1=[4;0;0;2;1;pi/6]; C1=[1;1.5;0.5;0.8;0;0]; [dl,bt]=decsg([E1 C1]);".to_string(),
            "E1=[4;0;0;2;1;pi/6]; C1=[1;1.5;0.5;0.8;0;0]; [dl,bt]=decsg([C1 E1]);".to_string(),
        ),
    ];
    for (one, two) in &cases {
        let (x, y) = (sorted_areas(one), sorted_areas(two));
        assert_eq!(x.len(), y.len(), "{one}");
        for (p, q) in x.iter().zip(&y) {
            assert!((p - q).abs() <= 1e-12, "{one}: {p} vs {q}");
        }
    }
}

fn check_shape_areas_rel(e: &mut Engine, shape_areas: &[f64], rel_tol: f64) {
    let dl = var(e, "dl");
    let areas = region_areas(&dl);
    let rows = bt_rows(e);
    assert_eq!(rows.len(), areas.len());
    for (j, &want) in shape_areas.iter().enumerate() {
        let got: f64 = rows
            .iter()
            .zip(&areas)
            .filter(|(r, _)| r.as_bytes()[j] == b'1')
            .map(|(_, a)| a)
            .sum();
        assert!(
            (got - want).abs() <= rel_tol * want.abs().max(1e-30),
            "shape {}: got {got:.15e}, want {want:.15e}, rel {:.3e}",
            j + 1,
            (got - want).abs() / want.abs().max(1e-30)
        );
    }
}

#[test]
fn tangent_ellipse_and_circle_repros_and_sweep() {
    // Repro 1: circle of radius 0.5 tangent outside E1(a=2, b=1) at t = 2.2.
    let e_area = 2. * PI;
    let r_out = 0.5f64;
    let c_out_area = PI * r_out * r_out;
    for order in ["[E1 C1]", "[C1 E1]"] {
        let src = format!(
            "E1=[4;0;0;2;1;0]; t=2.2; p=[2*cos(t);sin(t)]; n=[cos(t)/2;sin(t)]; n=n/norm(n); \
             C1=[1;p(1)+0.5*n(1);p(2)+0.5*n(2);0.5;0;0]; [dl,bt]=decsg({order});"
        );
        let mut e = run(&src);
        let dl = var(&e, "dl");
        assert_eq!((dl.rows(), dl.cols()), (12, 10), "{order}");
        let mut rows = bt_rows(&mut e);
        rows.sort();
        assert_eq!(rows, ["01", "10"], "{order}");
        let expected = if order == "[E1 C1]" {
            [e_area, c_out_area]
        } else {
            [c_out_area, e_area]
        };
        check_shape_areas_rel(&mut e, &expected, 1e-12);
        for j in 0..dl.cols() {
            assert!(
                segment_length(&dl, j) >= 1e-6,
                "{order} col {}: len {}",
                j + 1,
                segment_length(&dl, j)
            );
        }
        check_invariants(&mut e);
    }

    // Repro 2: circle of radius 0.4 tangent inside E1(a=2, b=1) at (2, 0).
    let r_in = 0.4f64;
    let c_in_area = PI * r_in * r_in;
    for order in ["[E1 C1]", "[C1 E1]"] {
        let src = format!("E1=[4;0;0;2;1;0]; C1=[1;1.6;0;0.4;0;0]; [dl,bt]=decsg({order});");
        let mut e = run(&src);
        let dl = var(&e, "dl");
        assert_eq!((dl.rows(), dl.cols()), (12, 8), "{order}");
        let mut areas = region_areas(&dl);
        areas.sort_by(f64::total_cmp);
        assert_eq!(areas.len(), 2, "{order}");
        assert!(
            (areas[0] - c_in_area).abs() <= 1e-12 * c_in_area
                && (areas[1] - (e_area - c_in_area)).abs() <= 1e-12 * (e_area - c_in_area),
            "{order}: {areas:?}"
        );
        let expected = if order == "[E1 C1]" {
            [e_area, c_in_area]
        } else {
            [c_in_area, e_area]
        };
        check_shape_areas_rel(&mut e, &expected, 1e-12);
        for j in 0..dl.cols() {
            assert!(segment_length(&dl, j) >= 1e-6, "{order} col {}", j + 1);
        }
        check_invariants(&mut e);
    }

    // Sweep of 24 tangent circles around E1(a=2, b=1), both outside (r=0.5)
    // and inside (r=0.4 < minimum radius of curvature b^2/a = 0.5).
    for k in 0..24 {
        let t = 0.13 + TAU * (k as f64) / 24.;
        let (px, py) = (2. * t.cos(), t.sin());
        let (nx0, ny0) = (t.cos() / 2., t.sin());
        let nn = nx0.hypot(ny0);
        let (nx, ny) = (nx0 / nn, ny0 / nn);
        for (outside, r) in [(true, 0.5f64), (false, 0.4f64)] {
            let sign = if outside { 1. } else { -1. };
            let (cx, cy) = (px + sign * r * nx, py + sign * r * ny);
            let c_area = PI * r * r;
            for order in ["[E1 C1]", "[C1 E1]"] {
                let src = format!(
                    "E1=[4;0;0;2;1;0]; C1=[1;{cx:?};{cy:?};{r:?};0;0]; [dl,bt]=decsg({order});"
                );
                let mut e = run(&src);
                let dl = var(&e, "dl");
                assert!(dl.cols() >= 8, "k={k} outside={outside} {order}: empty dl");
                let mut areas = region_areas(&dl);
                areas.sort_by(f64::total_cmp);
                assert_eq!(areas.len(), 2, "k={k} outside={outside} {order}: {areas:?}");
                let (want_lo, want_hi) = if outside {
                    (c_area, e_area)
                } else {
                    (c_area, e_area - c_area)
                };
                assert!(
                    (areas[0] - want_lo).abs() <= 1e-12 * want_lo
                        && (areas[1] - want_hi).abs() <= 1e-12 * want_hi,
                    "k={k} outside={outside} {order}: {areas:?} vs [{want_lo}, {want_hi}]"
                );
                let expected = if order == "[E1 C1]" {
                    [e_area, c_area]
                } else {
                    [c_area, e_area]
                };
                check_shape_areas_rel(&mut e, &expected, 1e-12);
                for j in 0..dl.cols() {
                    let len = segment_length(&dl, j);
                    assert!(
                        len >= 1e-6,
                        "k={k} outside={outside} {order} col {}: short border {len:.3e}",
                        j + 1
                    );
                }
                check_invariants(&mut e);
            }
        }
    }
}

#[test]
fn near_tangent_overlaps_keep_lens_or_collapse_consistently() {
    let t = 2.2f64;
    let (px, py) = (2. * t.cos(), t.sin());
    let (nx0, ny0) = (t.cos() / 2., t.sin());
    let nn = nx0.hypot(ny0);
    let (nx, ny) = (nx0 / nn, ny0 / nn);
    let r = 0.5f64;
    let (e_area, c_area) = (2. * PI, PI * r * r);

    // Depth 1e-7: genuine lens (3 regions), matching in both column orders.
    let d_lens = 1e-7f64;
    let (cx, cy) = (px + (r - d_lens) * nx, py + (r - d_lens) * ny);
    let mut lens_areas = Vec::new();
    for order in ["[E1 C1]", "[C1 E1]"] {
        let src =
            format!("E1=[4;0;0;2;1;0]; C1=[1;{cx:?};{cy:?};{r:?};0;0]; [dl,bt]=decsg({order});");
        let mut e = run(&src);
        let mut rows = bt_rows(&mut e);
        rows.sort();
        assert_eq!(rows, ["01", "10", "11"], "{order}");
        let expected = if order == "[E1 C1]" {
            [e_area, c_area]
        } else {
            [c_area, e_area]
        };
        check_shape_areas_rel(&mut e, &expected, 1e-12);
        let mut a = region_areas(&var(&e, "dl"));
        a.sort_by(f64::total_cmp);
        lens_areas.push(a);
        check_invariants(&mut e);
    }
    for (a, b) in lens_areas[0].iter().zip(&lens_areas[1]) {
        assert!((a - b).abs() <= 1e-12 * a.max(*b), "{a} vs {b}");
    }

    // Depths 1e-9 and 1e-13: collapse to 2 regions (one shared edge), same in both orders.
    for depth in [1e-9f64, 1e-13f64] {
        let (cx, cy) = (px + (r - depth) * nx, py + (r - depth) * ny);
        let mut cols = Vec::new();
        for order in ["[E1 C1]", "[C1 E1]"] {
            let src = format!(
                "E1=[4;0;0;2;1;0]; C1=[1;{cx:?};{cy:?};{r:?};0;0]; [dl,bt]=decsg({order});"
            );
            let mut e = run(&src);
            let dl = var(&e, "dl");
            cols.push(dl.cols());
            let mut rows = bt_rows(&mut e);
            rows.sort();
            assert_eq!(rows, ["01", "10"], "depth={depth:e} {order}");
            let expected = if order == "[E1 C1]" {
                [e_area, c_area]
            } else {
                [c_area, e_area]
            };
            check_shape_areas_rel(&mut e, &expected, 1e-12);
            check_invariants(&mut e);
        }
        assert_eq!(cols[0], cols[1], "depth={depth:e} column count mismatch");
    }
}

#[test]
fn small_circle_on_long_ellipse_down_to_1e_5_in_both_orders() {
    let (a, b, phi, t) = (5f64, 1f64, 0.47f64, 1f64);
    let (u, v) = (a * t.cos(), b * t.sin());
    let (x, y) = (phi.cos() * u - phi.sin() * v, phi.sin() * u + phi.cos() * v);
    let e_area = PI * a * b;
    for r in [1e-3f64, 1e-4f64, 1e-5f64] {
        let c_area = PI * r * r;
        let mut both = Vec::new();
        for order in ["[E1 C1]", "[C1 E1]"] {
            let src = format!(
                "E1=[4;0;0;{a:?};{b:?};{phi:?}]; C1=[1;{x:?};{y:?};{r:?};0;0]; [dl,bt]=decsg({order});"
            );
            let mut e = run(&src);
            let mut rows = bt_rows(&mut e);
            rows.sort();
            assert_eq!(rows, ["01", "10", "11"], "r={r:e} {order}");
            let expected = if order == "[E1 C1]" {
                [e_area, c_area]
            } else {
                [c_area, e_area]
            };
            check_shape_areas_rel(&mut e, &expected, 1e-12);
            let mut areas = region_areas(&var(&e, "dl"));
            areas.sort_by(f64::total_cmp);
            both.push(areas);
            check_invariants(&mut e);
        }
        assert_eq!(both[0].len(), both[1].len(), "r={r:e}");
        // Crossing points carry ~eps*a absolute error whatever the order, which
        // moves an r-sized region's area by ~eps*a*r: allow that on top of 1e-12.
        for (p, q) in both[0].iter().zip(&both[1]) {
            assert!(
                (p - q).abs() <= 1e-12 * p.abs().max(q.abs()) + 64. * f64::EPSILON * a * r,
                "r={r:e}: {p:.15e} vs {q:.15e}"
            );
        }
    }
}

#[test]
fn rotated_ellipse_quarter_arc_bounding_box_catches_extreme_crossing() {
    // E1(a=2, b=1, phi=pi/4) has y-extreme sqrt(5/2) ~= 1.58113883 inside the
    // [0, pi/2] quarter arc; the old 9-sample + 1e-3*a box topped out at ~1.579.
    // A rectangle with bottom edge y = 1.580 crosses strictly inside that cap.
    let e_area = 2. * PI;
    let r_area = 3. * (2. - 1.58);
    let mut both = Vec::new();
    for order in ["[E1 R1]", "[R1 E1]"] {
        let src = format!(
            "E1=[4;0;0;2;1;pi/4;0;0;0;0]; R1=[3;4;0;3;3;0;1.58;1.58;2;2]; [dl,bt]=decsg({order});"
        );
        let mut e = run(&src);
        let mut rows = bt_rows(&mut e);
        rows.sort();
        assert_eq!(rows, ["01", "10", "11"], "{order}");
        let expected = if order == "[E1 R1]" {
            [e_area, r_area]
        } else {
            [r_area, e_area]
        };
        check_shape_areas_rel(&mut e, &expected, 1e-12);
        let mut areas = region_areas(&var(&e, "dl"));
        areas.sort_by(f64::total_cmp);
        both.push(areas);
        check_invariants(&mut e);
    }
    for (p, q) in both[0].iter().zip(&both[1]) {
        assert!((p - q).abs() <= 1e-12 * p.abs().max(q.abs()), "{p} vs {q}");
    }
}
