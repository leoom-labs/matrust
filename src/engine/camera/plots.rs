//! Camera calibration piece 7d3c: showReprojectionErrors and showExtrinsics
//! (public MathWorks reference pages), drawn with the existing graphics
//! builtins (bar, plot, plot3, surf, text, legend, ...); no new primitives.
//!
//! - showReprojectionErrors 'BarGraph': one bar per pattern = the mean of
//!   hypot(ReprojectionErrors) over that pattern's points, a dashed line at
//!   MeanReprojectionError with the legend "Overall Mean Error: %.2f pixels".
//!   Highlighted patterns are a second bar series (other bars NaN) on top.
//! - 'ScatterPlot': one '+' series per pattern (x error, y error); a
//!   highlighted pattern uses 'o' markers in the highlight colour.
//! - showExtrinsics 'CameraCentric': each board (bounding rectangle of the
//!   planar WorldPoints) placed in the camera frame, [Xc Yc Zc] = [X Y 0]*R + t,
//!   as a surf; a camera glyph at the origin. 'PatternCentric': the board at
//!   the origin, one glyph per pattern at Xw = (Xc - t)*R' (centre -t*R').
//!   Plotted in natural camera/world coordinates (the engine has no ZDir).
//!   Glyph line: first vertex is the camera centre.
use super::{Camera, Params, err, num, text_arg};
use crate::engine::{Engine, Execution, Value};
use crate::{Array, Matrix, Result};

pub(super) const NAMES: [&str; 2] = ["showReprojectionErrors", "showExtrinsics"];

/// MATLAB's default line colour order (lines(7)).
const PALETTE: [[f64; 3]; 7] = [
    [0., 0.447, 0.741],
    [0.85, 0.325, 0.098],
    [0.929, 0.694, 0.125],
    [0.494, 0.184, 0.556],
    [0.466, 0.674, 0.188],
    [0.301, 0.745, 0.933],
    [0.635, 0.078, 0.184],
];
const HIGHLIGHT: [f64; 3] = [1., 0., 0.];
const BAR: [f64; 3] = [0., 0.447, 0.741];
const MEAN_LINE: [f64; 3] = [0., 0., 0.];
const BOARD: [f64; 3] = [0.5, 0.5, 0.5];

struct Options {
    view: usize,
    highlight: Vec<bool>,
    parent: Option<Value>,
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn color(c: [f64; 3]) -> Value {
    Value::Matrix(Matrix::from_column_vec(1, 3, c.to_vec()).expect("1x3"))
}

fn column(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(v.len(), 1, v)?))
}

fn scalar(x: f64) -> Value {
    Value::Matrix(Matrix::scalar(x))
}

fn options(
    e: &mut Engine,
    who: &str,
    views: [&str; 2],
    patterns: usize,
    rest: &[Value],
    out: &mut Execution,
) -> Result<Options> {
    let (view, pairs) = if rest.len() % 2 == 1 {
        let name = text_arg(e, &rest[0]).ok_or_else(|| {
            err(
                who,
                &format!("view must be '{}' or '{}'", views[0], views[1]),
            )
        })?;
        let k = views
            .iter()
            .position(|v| v.eq_ignore_ascii_case(&name))
            .ok_or_else(|| {
                err(
                    who,
                    &format!(
                        "unknown view '{name}'; view must be '{}' or '{}'",
                        views[0], views[1]
                    ),
                )
            })?;
        (k, &rest[1..])
    } else {
        (0, rest)
    };
    let mut o = Options {
        view,
        highlight: vec![false; patterns],
        parent: None,
    };
    for (name, v) in super::objects::pairs(e, who, pairs, &["HighlightIndex", "Parent"])? {
        if name == "HighlightIndex" {
            let n = num(e, who, "HighlightIndex", v)?;
            o.highlight = vec![false; patterns];
            for x in n.data {
                if !(x.fract() == 0. && x >= 1. && x <= patterns as f64) {
                    return Err(err(
                        who,
                        &format!("HighlightIndex must hold integers from 1 to {patterns}"),
                    ));
                }
                o.highlight[x as usize - 1] = true;
            }
        } else {
            let axes = matches!(v, Value::Container(Array::Graphics(_)))
                && matches!(
                    e.call("get", vec![v.clone(), text("Type")], out),
                    Ok(t) if text_arg(e, &t).as_deref() == Some("axes")
                );
            if !axes {
                return Err(err(who, "Parent must be an axes handle"));
            }
            o.parent = Some(v.clone());
        }
    }
    Ok(o)
}

/// `showReprojectionErrors(params, ...)` / `showExtrinsics(params, ...)`;
/// `rest` follows the params argument.
pub(super) fn show(
    e: &mut Engine,
    cam: Option<&Camera>,
    name: &str,
    rest: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    super::max_outputs(nargout, 1)?;
    let Some(Camera::Params(p)) = cam else {
        return Err(err(
            name,
            "the first argument must be a cameraParameters object (from estimateCameraParameters)",
        ));
    };
    let views = if name == "showExtrinsics" {
        ["CameraCentric", "PatternCentric"]
    } else {
        ["BarGraph", "ScatterPlot"]
    };
    // HighlightIndex counts what is drawn: the patterns for showExtrinsics,
    // the ReprojectionErrors pages for showReprojectionErrors (params made
    // from ReprojectionErrors alone have no RotationVectors).
    let count = if name == "showExtrinsics" {
        p.patterns()
    } else if p.error_points == 0 {
        0
    } else {
        p.errors.len() / (2 * p.error_points)
    };
    let opts = options(e, name, views, count, rest, out)?;
    if name == "showExtrinsics" {
        if p.patterns() == 0 || p.world.is_empty() {
            return Err(err(
                name,
                "the cameraParameters has no patterns or WorldPoints",
            ));
        }
    } else if p.errors.is_empty() || p.error_points == 0 {
        return Err(err(
            name,
            "the cameraParameters has no ReprojectionErrors (make it with estimateCameraParameters)",
        ));
    }
    let old = (e.current_figure, e.figure.clone(), e.other_figures.clone());
    let result = draw(e, name, p, &opts, out);
    if result.is_err() {
        (e.current_figure, e.figure, e.other_figures) = old;
    }
    let ax = result?;
    Ok(if nargout == 1 { vec![ax] } else { vec![] })
}

fn draw(e: &mut Engine, name: &str, p: &Params, o: &Options, out: &mut Execution) -> Result<Value> {
    let ax = match &o.parent {
        Some(ax) => ax.clone(),
        None => e.call("gca", vec![], out)?,
    };
    let next = e.call("get", vec![ax.clone(), text("NextPlot")], out)?;
    let held = text_arg(e, &next).as_deref() == Some("add");
    let mut first = true;
    let mut hold_after_first = |e: &mut Engine, out: &mut Execution| -> Result<()> {
        if first {
            first = false;
            e.call("hold", vec![ax.clone(), text("on")], out)?;
        }
        Ok(())
    };
    let r = match (name, o.view) {
        ("showReprojectionErrors", 0) => bar_graph(e, &ax, p, o, &mut hold_after_first, out),
        ("showReprojectionErrors", _) => scatter(e, &ax, p, o, &mut hold_after_first, out),
        (_, view) => extrinsics(e, &ax, p, o, view == 1, &mut hold_after_first, out),
    };
    let restore = e.call(
        "hold",
        vec![ax.clone(), text(if held { "on" } else { "off" })],
        out,
    );
    r?;
    restore?;
    Ok(ax)
}

/// `axis equal` on the target axes (Parent or gca): `axes(ax)` selects it,
/// which drawing into it has already done, so no other state changes.
fn axis_equal(e: &mut Engine, ax: &Value, out: &mut Execution) -> Result<()> {
    e.call("axes", vec![ax.clone()], out)?;
    e.call("axis", vec![text("equal")], out)?;
    Ok(())
}

type Hold<'a> = dyn FnMut(&mut Engine, &mut Execution) -> Result<()> + 'a;

fn labels(e: &mut Engine, ax: &Value, items: &[(&str, &str)], out: &mut Execution) -> Result<()> {
    for (f, s) in items {
        e.call(f, vec![ax.clone(), text(s)], out)?;
    }
    Ok(())
}

/// Per-pattern and overall means of hypot(ReprojectionErrors) over finite pairs.
fn pattern_means(p: &Params) -> (Vec<f64>, f64) {
    let m = p.error_points;
    let mut means = Vec::with_capacity(p.errors.len() / (2 * m));
    let (mut total, mut count) = (0., 0);
    for page in p.errors.chunks_exact(2 * m) {
        let (x, y) = page.split_at(m);
        let (mut sum, mut points) = (0., 0);
        for (&dx, &dy) in x.iter().zip(y) {
            if dx.is_finite() && dy.is_finite() {
                let distance = dx.hypot(dy);
                sum += distance;
                total += distance;
                points += 1;
            }
        }
        means.push(sum / points as f64);
        count += points;
    }
    (means, total / count as f64)
}

fn bar_graph(
    e: &mut Engine,
    ax: &Value,
    p: &Params,
    o: &Options,
    hold: &mut Hold,
    out: &mut Execution,
) -> Result<()> {
    let (means, overall) = pattern_means(p);
    let n = means.len();
    let x: Vec<f64> = (1..=n).map(|k| k as f64).collect();
    e.call(
        "bar",
        vec![
            ax.clone(),
            column(x.clone())?,
            column(means.clone())?,
            text("FaceColor"),
            color(BAR),
        ],
        out,
    )?;
    hold(e, out)?;
    if o.highlight.iter().any(|h| *h) {
        let y = means
            .iter()
            .zip(&o.highlight)
            .map(|(m, h)| if *h { *m } else { f64::NAN })
            .collect();
        e.call(
            "bar",
            vec![
                ax.clone(),
                column(x)?,
                column(y)?,
                text("FaceColor"),
                color(HIGHLIGHT),
            ],
            out,
        )?;
    }
    let line = e.call(
        "plot",
        vec![
            ax.clone(),
            column(vec![0.5, n as f64 + 0.5])?,
            column(vec![overall, overall])?,
            text("--"),
            text("Color"),
            color(MEAN_LINE),
            text("LineWidth"),
            scalar(1.5),
        ],
        out,
    )?;
    e.call(
        "legend",
        vec![
            line,
            text(&format!("Overall Mean Error: {overall:.2} pixels")),
        ],
        out,
    )?;
    labels(
        e,
        ax,
        &[
            ("title", "Mean Reprojection Error per Image"),
            ("xlabel", "Images"),
            ("ylabel", "Mean Error in Pixels"),
        ],
        out,
    )
}

fn scatter(
    e: &mut Engine,
    ax: &Value,
    p: &Params,
    o: &Options,
    hold: &mut Hold,
    out: &mut Execution,
) -> Result<()> {
    let m = p.error_points;
    let pages = p.errors.len() / (2 * m);
    for k in 0..pages {
        e.check_cancel()?;
        let page = &p.errors[k * 2 * m..(k + 1) * 2 * m];
        let lit = o.highlight.get(k).copied().unwrap_or(false);
        e.call(
            "plot",
            vec![
                ax.clone(),
                column(page[..m].to_vec())?,
                column(page[m..].to_vec())?,
                text(if lit { "o" } else { "+" }),
                text("Color"),
                color(if lit { HIGHLIGHT } else { PALETTE[k % 7] }),
                text("DisplayName"),
                text(&format!("Image {}", k + 1)),
            ],
            out,
        )?;
        hold(e, out)?;
    }
    axis_equal(e, ax, out)?;
    labels(
        e,
        ax,
        &[
            ("title", "Reprojection Errors"),
            ("xlabel", "X in pixels"),
            ("ylabel", "Y in pixels"),
        ],
        out,
    )
}

type V3 = [f64; 3];

/// Row point times postmultiply matrix: [x y z] * R.
fn row_times(x: V3, r: &super::projection::Mat3) -> V3 {
    [0, 1, 2].map(|j| x[0] * r[0][j] + x[1] * r[1][j] + x[2] * r[2][j])
}

/// Row point times R' (the inverse rotation).
fn row_times_t(x: V3, r: &super::projection::Mat3) -> V3 {
    [0, 1, 2].map(|j| x[0] * r[j][0] + x[1] * r[j][1] + x[2] * r[j][2])
}

/// Board rectangle corners [xmin ymin], [xmax ymin], [xmin ymax], [xmax ymax]
/// (z = 0) and the board size.
fn board(p: &Params) -> ([V3; 4], f64) {
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for w in &p.world {
        for j in 0..2 {
            lo[j] = lo[j].min(w[j]);
            hi[j] = hi[j].max(w[j]);
        }
    }
    let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    (
        [
            [lo[0], lo[1], 0.],
            [hi[0], lo[1], 0.],
            [lo[0], hi[1], 0.],
            [hi[0], hi[1], 0.],
        ],
        if size > 0. { size } else { 1. },
    )
}

/// Pyramid glyph in camera coordinates; the first vertex is the centre.
fn glyph(size: f64) -> Vec<V3> {
    let (d, a, b) = (size / 2., size / 4., size * 3. / 16.);
    let o = [0., 0., 0.];
    let c = [[-a, -b, d], [a, -b, d], [a, b, d], [-a, b, d]];
    vec![o, c[0], c[1], c[2], c[3], c[0], o, c[1], o, c[2], o, c[3]]
}

fn surf_board(
    e: &mut Engine,
    ax: &Value,
    corners: [V3; 4],
    col: [f64; 3],
    alpha: f64,
    label: &str,
    out: &mut Execution,
) -> Result<()> {
    // 2x2 grids: [c0 c1; c2 c3] row-wise, column-major data c0 c2 c1 c3.
    let grid = |j: usize| {
        Value::Matrix(
            Matrix::from_column_vec(
                2,
                2,
                vec![corners[0][j], corners[2][j], corners[1][j], corners[3][j]],
            )
            .expect("2x2"),
        )
    };
    e.call(
        "surf",
        vec![
            ax.clone(),
            grid(0),
            grid(1),
            grid(2),
            text("FaceColor"),
            color(col),
            text("FaceAlpha"),
            scalar(alpha),
            text("EdgeColor"),
            color(col),
            text("DisplayName"),
            text(label),
        ],
        out,
    )?;
    Ok(())
}

fn line3(
    e: &mut Engine,
    ax: &Value,
    pts: &[V3],
    style: &str,
    col: [f64; 3],
    label: &str,
    out: &mut Execution,
) -> Result<()> {
    let coord = |j: usize| column(pts.iter().map(|q| q[j]).collect());
    e.call(
        "plot3",
        vec![
            ax.clone(),
            coord(0)?,
            coord(1)?,
            coord(2)?,
            text(style),
            text("Color"),
            color(col),
            text("DisplayName"),
            text(label),
        ],
        out,
    )?;
    Ok(())
}

fn extrinsics(
    e: &mut Engine,
    ax: &Value,
    p: &Params,
    o: &Options,
    pattern_centric: bool,
    hold: &mut Hold,
    out: &mut Execution,
) -> Result<()> {
    let planar = p.world_cols == 2;
    let (corners, size) = board(p);
    let cam = glyph(size);
    let style = |k: usize| {
        if o.highlight[k] {
            (HIGHLIGHT, 0.8)
        } else {
            (PALETTE[k % 7], 0.3)
        }
    };
    if pattern_centric {
        if planar {
            surf_board(e, ax, corners, BOARD, 0.5, "Pattern", out)?;
        } else {
            line3(e, ax, &p.world, ".", BOARD, "Pattern", out)?;
        }
        hold(e, out)?;
        for k in 0..p.patterns() {
            e.check_cancel()?;
            let (r, t) = (p.rotation(k), p.translation_vectors[k]);
            let pts: Vec<V3> = cam
                .iter()
                .map(|c| row_times_t([c[0] - t[0], c[1] - t[1], c[2] - t[2]], &r))
                .collect();
            line3(
                e,
                ax,
                &pts,
                "-",
                style(k).0,
                &format!("Camera {}", k + 1),
                out,
            )?;
            let c = pts[0];
            e.call(
                "text",
                vec![
                    ax.clone(),
                    scalar(c[0]),
                    scalar(c[1]),
                    scalar(c[2]),
                    text(&format!("{}", k + 1)),
                ],
                out,
            )?;
        }
    } else {
        line3(e, ax, &cam, "-", [0., 0., 0.], "Camera", out)?;
        hold(e, out)?;
        for k in 0..p.patterns() {
            e.check_cancel()?;
            let (r, t) = (p.rotation(k), p.translation_vectors[k]);
            let place = |w: V3| {
                let q = row_times(w, &r);
                [q[0] + t[0], q[1] + t[1], q[2] + t[2]]
            };
            let label = format!("Pattern {}", k + 1);
            let (col, alpha) = style(k);
            if planar {
                surf_board(e, ax, corners.map(place), col, alpha, &label, out)?;
            } else {
                let pts: Vec<V3> = p.world.iter().map(|w| place(*w)).collect();
                line3(e, ax, &pts, ".", col, &label, out)?;
            }
            let c = place(corners[0]);
            e.call(
                "text",
                vec![
                    ax.clone(),
                    scalar(c[0]),
                    scalar(c[1]),
                    scalar(c[2]),
                    text(&format!("{}", k + 1)),
                ],
                out,
            )?;
        }
    }
    let units = if p.units.is_empty() {
        "mm"
    } else {
        p.units.as_str()
    };
    e.call("view", vec![ax.clone(), scalar(3.)], out)?;
    e.call("grid", vec![ax.clone(), text("on")], out)?;
    axis_equal(e, ax, out)?;
    labels(
        e,
        ax,
        &[
            ("title", "Extrinsic Parameters Visualization"),
            ("xlabel", &format!("X ({units})")),
            ("ylabel", &format!("Y ({units})")),
            ("zlabel", &format!("Z ({units})")),
        ],
        out,
    )
}
