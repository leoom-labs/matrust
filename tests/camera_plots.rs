//! Camera calibration piece 7d3c: showReprojectionErrors and showExtrinsics.
//!
//! The params come from estimateCameraParameters on the k2 fixture
//! (tests/fixtures/calib/estimate_k2.json, OpenCV-checked in camera_estimate).
//! Expected values are the documented formulas on the params' own
//! properties: per-pattern mean of hypot(ReprojectionErrors), boards at
//! [X Y 0]*R + t, camera centres at -t*R' (row convention).
use super::camera_estimate::load_rendered_boards;
use matrust::{Array, Engine, Matrix};
use serde_json::Value as Json;

fn matrix(e: &Engine, name: &str) -> Matrix {
    if let Some(m) = e.variables.get(name) {
        return m.clone();
    }
    match e.array(name) {
        Some(Array::Numeric(m)) => m.clone(),
        other => panic!("missing numeric {name}: {other:?}"),
    }
}

fn run(e: &mut Engine, code: &str) {
    if let Err(error) = e.execute(code) {
        panic!("{code}\n  failed: {error}");
    }
}

fn error_has(e: &mut Engine, code: &str, part: &str) {
    match e.execute(code) {
        Ok(_) => panic!("{code}\n  succeeded; expected an error"),
        Err(message) => assert!(
            message.contains(part),
            "{code}\n  error {message:?} does not contain {part:?}"
        ),
    }
}

fn values(e: &mut Engine, expr: &str) -> Vec<f64> {
    run(e, &format!("zz_v = double({expr}); zz_v = zz_v(:)';"));
    matrix(e, "zz_v").data().to_vec()
}

/// [x y z] of the first vertex of graphics object `h`, then `suffix`.
fn first_vertex(e: &mut Engine, h: &str, suffix: &str) -> Vec<f64> {
    run(
        e,
        &format!("gx = get({h}, 'XData'); gy = get({h}, 'YData'); gz = get({h}, 'ZData');"),
    );
    values(e, &format!("[gx(1) gy(1) gz(1)]{suffix}"))
}

/// `name` = the child of axes `ax` whose DisplayName is `label`.
fn named(e: &mut Engine, ax: &str, label: &str, name: &str) {
    run(
        e,
        &format!(
            "zz_c = get({ax}, 'Children'); zz_n = 0; for zz_i = 1:numel(zz_c), try, if strcmp(get(zz_c(zz_i), 'DisplayName'), '{label}'), {name} = zz_c(zz_i); zz_n = zz_n + 1; end, catch, end, end; assert(zz_n == 1);"
        ),
    );
}

fn close(got: &[f64], want: &[f64], tol: f64, what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        let ok = (g.is_nan() && w.is_nan()) || (g - w).abs() <= tol * w.abs().max(1.);
        assert!(ok, "{what}: {got:?} vs {want:?}");
    }
}

fn rows_literal(rows: &Json) -> String {
    let rows: Vec<String> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|v| format!("{}", v.as_f64().unwrap()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    format!("[{}]", rows.join("; "))
}

/// An engine with `params` calibrated from the k2 fixture and P in `np`.
fn calibrated() -> Engine {
    let json: Json = serde_json::from_str(include_str!("fixtures/calib/estimate_k2.json")).unwrap();
    let mut e = Engine::default();
    run(
        &mut e,
        &format!("w = {};", rows_literal(&json["world_points"])),
    );
    let views = json["views"].as_array().unwrap();
    run(
        &mut e,
        &format!(
            "ip = zeros({}, 2, {});",
            views[0].as_array().unwrap().len(),
            views.len()
        ),
    );
    for (k, v) in views.iter().enumerate() {
        run(
            &mut e,
            &format!("ip(:, :, {}) = {};", k + 1, rows_literal(v)),
        );
    }
    run(
        &mut e,
        "params = estimateCameraParameters(ip, w, 'ImageSize', [480 640]); np = params.NumPatterns; err = params.ReprojectionErrors;",
    );
    e
}

fn patterns(e: &mut Engine) -> usize {
    values(e, "np")[0] as usize
}

/// Per-pattern means computed here from the ReprojectionErrors values.
fn expected_means(e: &mut Engine) -> Vec<f64> {
    let p = patterns(e);
    (1..=p)
        .map(|k| {
            let x = values(e, &format!("err(:, 1, {k})"));
            let y = values(e, &format!("err(:, 2, {k})"));
            let (sum, count) = x
                .iter()
                .zip(&y)
                .filter(|(a, b)| a.is_finite() && b.is_finite())
                .fold((0., 0), |(sum, count), (a, b)| {
                    (sum + a.hypot(*b), count + 1)
                });
            sum / count as f64
        })
        .collect()
}

#[test]
fn camera_plots_bar_graph_means_and_mean_line() {
    let mut e = calibrated();
    let p = patterns(&mut e);
    assert!(p >= 3);
    run(
        &mut e,
        "figure; ax = showReprojectionErrors(params); c = get(ax, 'Children');",
    );
    assert_eq!(values(&mut e, "numel(c)"), vec![2.]);
    close(
        &values(&mut e, "get(c(2), 'YData')"),
        &expected_means(&mut e),
        1e-12,
        "bar YData",
    );
    let mean = values(&mut e, "params.MeanReprojectionError")[0];
    close(
        &values(&mut e, "get(c(1), 'YData')"),
        &[mean, mean],
        1e-12,
        "mean line",
    );
    run(
        &mut e,
        "s = get(c(1), 'LineStyle'); t = get(get(ax, 'Title'), 'String'); xl = get(get(ax, 'XLabel'), 'String'); yl = get(get(ax, 'YLabel'), 'String');",
    );
    run(
        &mut e,
        "assert(strcmp(s, '--')); assert(strcmp(t, 'Mean Reprojection Error per Image')); assert(strcmp(xl, 'Images')); assert(strcmp(yl, 'Mean Error in Pixels'));",
    );
    // Statement form (no output) and the explicit view name.
    run(
        &mut e,
        "figure; showReprojectionErrors(params, 'bargraph'); assert(numel(get(gca, 'Children')) == 2);",
    );
}

#[test]
fn camera_plots_bar_graph_omits_missing_corners_from_means() {
    let mut e = Engine::default();
    load_rendered_boards(&mut e);
    run(
        &mut e,
        "ip(1, :, 1) = NaN; \
         params = estimateCameraParameters(ip, w, 'ImageSize', [480 640], 'EstimateTangentialDistortion', true); \
         np = params.NumPatterns; err = params.ReprojectionErrors; \
         figure; ax = showReprojectionErrors(params); c = get(ax, 'Children');",
    );
    assert_eq!(values(&mut e, "isnan(err(1, :, 1))"), vec![1., 1.]);
    let bars = values(&mut e, "get(c(2), 'YData')");
    assert!(bars.iter().all(|y| y.is_finite()), "bars {bars:?}");
    close(&bars, &expected_means(&mut e), 1e-12, "bar YData");
    let mean = values(&mut e, "params.MeanReprojectionError")[0];
    let line = values(&mut e, "get(c(1), 'YData')");
    assert!(line.iter().all(|y| y.is_finite()), "mean line {line:?}");
    close(&line, &[mean, mean], 1e-12, "mean line");
}

#[test]
fn camera_plots_scatter_one_series_per_pattern() {
    let mut e = calibrated();
    let p = patterns(&mut e);
    run(
        &mut e,
        "figure; ax = showReprojectionErrors(params, 'ScatterPlot'); c = get(ax, 'Children'); c = c(end:-1:1);",
    );
    assert_eq!(values(&mut e, "numel(c)"), vec![p as f64]);
    for k in 1..=p {
        let gx = values(&mut e, &format!("get(c({k}), 'XData')"));
        let wx = values(&mut e, &format!("err(:, 1, {k})"));
        close(&gx, &wx, 0., "scatter x");
        let gy = values(&mut e, &format!("get(c({k}), 'YData')"));
        let wy = values(&mut e, &format!("err(:, 2, {k})"));
        close(&gy, &wy, 0., "scatter y");
    }
    // One colour per pattern.
    let c1 = values(&mut e, "get(c(1), 'Color')");
    let c2 = values(&mut e, "get(c(2), 'Color')");
    assert_ne!(c1, c2);
}

#[test]
fn camera_plots_camera_centric_boards() {
    let mut e = calibrated();
    let p = patterns(&mut e);
    run(
        &mut e,
        "figure; ax = showExtrinsics(params); c = get(ax, 'Children');",
    );
    // Camera glyph, then a board surface and its number per pattern.
    assert_eq!(values(&mut e, "numel(c)"), vec![(1 + 2 * p) as f64]);
    named(&mut e, "ax", "Camera", "g");
    close(
        &first_vertex(&mut e, "g", ""),
        &[0., 0., 0.],
        0.,
        "camera at origin",
    );
    run(
        &mut e,
        "wp = params.WorldPoints; q = [min(wp(:,1)) min(wp(:,2)); max(wp(:,1)) min(wp(:,2)); min(wp(:,1)) max(wp(:,2)); max(wp(:,1)) max(wp(:,2))]; q = [q zeros(4, 1)];",
    );
    for k in 1..=p {
        run(
            &mut e,
            &format!(
                "R = params.RotationMatrices(:, :, {k}); t = params.TranslationVectors({k}, :); b = q * R + repmat(t, 4, 1);"
            ),
        );
        named(&mut e, "ax", &format!("Pattern {k}"), "s");
        for (j, axis) in ["XData", "YData", "ZData"].iter().enumerate() {
            let got = values(&mut e, &format!("get(s, '{axis}')"));
            // surf grid [c1 c2; c3 c4] column-major: c1 c3 c2 c4.
            let want = values(&mut e, &format!("b([1 3 2 4], {})", j + 1));
            close(&got, &want, 1e-9, &format!("pattern {k} {axis}"));
        }
        // Independent of the matrix form: the board corner (0,0) is t.
        let origin = first_vertex(&mut e, "s", "");
        let t = values(&mut e, "t");
        if values(&mut e, "all(q(1, :) == 0)")[0] == 1. {
            close(&origin, &t, 1e-9, "board origin at t");
        }
    }
    run(
        &mut e,
        "assert(strcmp(get(get(ax, 'Title'), 'String'), 'Extrinsic Parameters Visualization')); assert(strcmp(get(get(ax, 'XLabel'), 'String'), 'X (mm)'));",
    );
}

#[test]
fn camera_plots_pattern_centric_camera_centres() {
    let mut e = calibrated();
    let p = patterns(&mut e);
    run(
        &mut e,
        "figure; ax = showExtrinsics(params, 'PatternCentric'); c = get(ax, 'Children');",
    );
    assert_eq!(values(&mut e, "numel(c)"), vec![(1 + 2 * p) as f64]);
    named(&mut e, "ax", "Pattern", "b");
    assert_eq!(values(&mut e, "get(b, 'ZData')"), vec![0.; 4]);
    for k in 1..=p {
        run(
            &mut e,
            &format!(
                "R = params.RotationMatrices(:, :, {k}); t = params.TranslationVectors({k}, :);"
            ),
        );
        named(&mut e, "ax", &format!("Camera {k}"), "g");
        let got = first_vertex(&mut e, "g", "");
        let want = values(&mut e, "-t * R'");
        close(&got, &want, 1e-9, &format!("camera {k} centre"));
        // The centre maps to the camera origin: centre*R + t = 0.
        let back = first_vertex(&mut e, "g", " * R + t");
        for x in back {
            assert!(
                x.abs() <= 1e-9 * want.iter().fold(1., |a: f64, b| a.max(b.abs())),
                "camera {k}: {x}"
            );
        }
    }
}

#[test]
fn camera_plots_parent_axes() {
    let mut e = calibrated();
    run(
        &mut e,
        "figure; a1 = gca; figure; a2 = gca; ax = showReprojectionErrors(params, 'Parent', a1);",
    );
    assert_eq!(values(&mut e, "isequal(ax, a1)"), vec![1.]);
    assert_eq!(values(&mut e, "numel(get(a1, 'Children'))"), vec![2.]);
    assert_eq!(values(&mut e, "numel(get(a2, 'Children'))"), vec![0.]);
    run(
        &mut e,
        "ax = showExtrinsics(params, 'PatternCentric', Parent=a1);",
    );
    assert_eq!(values(&mut e, "isequal(ax, a1)"), vec![1.]);
    assert_eq!(values(&mut e, "numel(get(a2, 'Children'))"), vec![0.]);
}

/// `axis equal` lands on the Parent axes (as without Parent), and only there.
#[test]
fn camera_plots_parent_axis_equal() {
    let mut e = calibrated();
    let equal = |e: &mut Engine, ax: &str| -> bool {
        run(e, &format!("axes({ax});"));
        e.figure.as_ref().unwrap().axes.extras.equal_aspect
    };
    run(&mut e, "figure; a0 = showExtrinsics(params);");
    assert!(equal(&mut e, "a0"), "no Parent: axes not equal");
    for call in [
        "showReprojectionErrors(params, 'ScatterPlot', 'Parent', a1)",
        "showExtrinsics(params, Parent=a1)",
        "showExtrinsics(params, 'PatternCentric', Parent=a1)",
    ] {
        run(
            &mut e,
            &format!(
                "figure; subplot(1, 2, 1); a1 = gca; subplot(1, 2, 2); a2 = gca; figure; a3 = gca; {call};"
            ),
        );
        assert!(equal(&mut e, "a1"), "{call}: Parent axes not equal");
        assert!(!equal(&mut e, "a2"), "{call}: sibling axes changed");
        assert!(!equal(&mut e, "a3"), "{call}: current axes changed");
    }
}

/// HighlightIndex counts ReprojectionErrors pages, so params made from
/// ReprojectionErrors alone (no RotationVectors) accept it.
#[test]
fn camera_plots_highlight_errors_only_params() {
    let mut e = calibrated();
    let p = patterns(&mut e);
    run(
        &mut e,
        "pe = cameraParameters('ReprojectionErrors', err); figure; ax = showReprojectionErrors(pe, HighlightIndex=2); c = get(ax, 'Children');",
    );
    assert_eq!(values(&mut e, "pe.NumPatterns"), vec![0.]);
    assert_eq!(values(&mut e, "numel(c)"), vec![3.]);
    let means = expected_means(&mut e);
    let lit: Vec<f64> = (0..p)
        .map(|k| if k == 1 { means[1] } else { f64::NAN })
        .collect();
    close(
        &values(&mut e, "get(c(2), 'YData')"),
        &lit,
        1e-12,
        "highlight bar",
    );
    run(
        &mut e,
        &format!("figure; showReprojectionErrors(pe, 'ScatterPlot', 'HighlightIndex', {p});"),
    );
    error_has(
        &mut e,
        &format!("showReprojectionErrors(pe, 'HighlightIndex', {})", p + 1),
        &format!("from 1 to {p}"),
    );
}

#[test]
fn camera_plots_highlight_one_pattern() {
    let mut e = calibrated();
    let p = patterns(&mut e);
    // Bar graph: a second series holds only the highlighted bar.
    run(
        &mut e,
        "figure; ax = showReprojectionErrors(params, HighlightIndex=2); c = get(ax, 'Children');",
    );
    assert_eq!(values(&mut e, "numel(c)"), vec![3.]);
    let means = expected_means(&mut e);
    let lit: Vec<f64> = (0..p)
        .map(|k| if k == 1 { means[1] } else { f64::NAN })
        .collect();
    close(
        &values(&mut e, "get(c(2), 'YData')"),
        &lit,
        1e-12,
        "highlight bar",
    );
    close(
        &values(&mut e, "get(c(3), 'YData')"),
        &means,
        1e-12,
        "base bars",
    );
    let base = values(&mut e, "get(c(3), 'FaceColor')");
    let high = values(&mut e, "get(c(2), 'FaceColor')");
    assert_ne!(base, high);
    // Scatter: only pattern 2 changes colour.
    run(
        &mut e,
        "figure; a = showReprojectionErrors(params, 'ScatterPlot'); c0 = get(a, 'Children'); figure; a = showReprojectionErrors(params, 'ScatterPlot', 'HighlightIndex', 2); c1 = get(a, 'Children');",
    );
    for k in 1..=p {
        let before = values(&mut e, &format!("get(c0(end - {k} + 1), 'Color')"));
        let after = values(&mut e, &format!("get(c1(end - {k} + 1), 'Color')"));
        assert_eq!(before == after, k != 2, "scatter pattern {k}");
    }
    // Extrinsics: only board 2 changes colour.
    run(
        &mut e,
        "figure; a0 = showExtrinsics(params); figure; a1 = showExtrinsics(params, 'CameraCentric', 'HighlightIndex', [2]);",
    );
    for k in 1..=p {
        named(&mut e, "a0", &format!("Pattern {k}"), "s0");
        named(&mut e, "a1", &format!("Pattern {k}"), "s1");
        let before = values(&mut e, "get(s0, 'FaceColor')");
        let after = values(&mut e, "get(s1, 'FaceColor')");
        assert_eq!(before == after, k != 2, "board {k}");
    }
}

#[test]
fn camera_plots_errors() {
    let mut e = calibrated();
    error_has(&mut e, "showReprojectionErrors(5)", "cameraParameters");
    error_has(
        &mut e,
        "showExtrinsics(cameraIntrinsics([800 800], [320 240], [480 640]))",
        "cameraParameters",
    );
    error_has(
        &mut e,
        "showReprojectionErrors(params, 'Pie')",
        "unknown view 'Pie'",
    );
    error_has(
        &mut e,
        "showExtrinsics(params, 'BarGraph')",
        "unknown view 'BarGraph'",
    );
    error_has(
        &mut e,
        "showReprojectionErrors(params, 'HighlightIndex', 99)",
        "HighlightIndex",
    );
    error_has(
        &mut e,
        "showExtrinsics(params, 'Parent', 3)",
        "Parent must be an axes",
    );
    error_has(
        &mut e,
        "showExtrinsics(params, 'Colour', 3)",
        "unknown name-value",
    );
    error_has(
        &mut e,
        "showReprojectionErrors(cameraParameters)",
        "no ReprojectionErrors",
    );
    error_has(&mut e, "[a, b] = showExtrinsics(params)", "");
}
