//! Plot commands composed from existing ones: `line`, `quiver`, `quiver3`,
//! `fsurf`, `fimplicit`, `animatedline` (with `addpoints`, `getpoints`,
//! `clearpoints`) and `print`.
//!
//! Each builds its data and then calls `plot`, `plot3`, `surf`, `contourc`,
//! `set`/`get` or `exportgraphics`, so the figure model, renderers and
//! property access stay those of the commands they reuse. Quiver arrows follow
//! GNU Octave 10.3's `__quiver__` (scale 0.9 of the grid spacing over the
//! longest vector; heads 1/3 of the arrow, shafts and heads in one line).
//! `line` and `animatedline` add to the current axes whatever `hold` says.
use super::{Engine, Execution, Value};
use crate::{Array, GraphicsKind, Matrix, ObjectIdentity, Result};

pub(super) const NAMES: &[&str] = &[
    "line",
    "quiver",
    "quiver3",
    "fsurf",
    "fimplicit",
    "animatedline",
    "addpoints",
    "getpoints",
    "clearpoints",
    "print",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}

/// Names a statement runs without showing a result.
pub(super) fn statement(name: &str) -> bool {
    handles(name) && name != "getpoints"
}

/// Commands without an output value.
pub(super) fn no_output(name: &str) -> bool {
    matches!(name, "addpoints" | "clearpoints" | "print")
}

/// MaximumNumPoints of live animated lines, bounded like the chart records.
#[derive(Clone, Default)]
pub(crate) struct ComposeState {
    animated: Vec<(crate::GraphicsHandle, usize)>,
}
const MAX_ANIMATED: usize = 1024;
const DEFAULT_MAX_POINTS: usize = crate::plot::MAX_POINTS;

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let max = match name {
        "getpoints" => 3,
        "addpoints" | "clearpoints" | "print" => 0,
        _ => 1,
    };
    if count > max {
        return Err(match max {
            0 => format!("{name} returns no output"),
            1 => format!("{name} returns at most one output"),
            _ => format!("{name} returns at most {max} outputs"),
        });
    }
    let mut args = e.text_arguments(args.to_vec())?;
    // Every figure and the selection: a failed call (also after a leading
    // axes handle switched the current figure) leaves them as they were.
    let old = Snapshot::of(e);
    let result = (|| {
        if !matches!(name, "addpoints" | "getpoints" | "clearpoints" | "print")
            && let Some(Value::Container(Array::Graphics(h))) = args.first()
            && h.len() == 1
            && h.scalar_handle()?.kind() == GraphicsKind::Axes
        {
            let target = args.remove(0);
            let Value::Container(Array::Graphics(h)) = &target else {
                unreachable!()
            };
            let h = h.scalar_handle()?;
            if super::ui_components::is_uiaxes_fig_id(e.locate_graphics(h)?.figure) {
                // axes(uiaxes) cannot select a uiaxes host; draw through it instead.
                return e.with_axes(h, |e| dispatch(e, name, &args, count, out));
            }
            e.call("axes", vec![target], out)?;
        }
        dispatch(e, name, &args, count, out)
    })();
    match result {
        Ok(mut v) => {
            v.truncate(count);
            Ok(v)
        }
        Err(error) => {
            old.restore(e);
            Err(error)
        }
    }
}

struct Snapshot {
    figure: Option<crate::plot::Figure>,
    others: std::collections::BTreeMap<u32, crate::plot::Figure>,
    closed: std::collections::BTreeSet<u32>,
    current: u32,
    p2: super::plots_2d_more::P2State,
    surf: super::graphics_handles::SurfaceState,
    compose: ComposeState,
}
impl Snapshot {
    fn of(e: &Engine) -> Self {
        Self {
            figure: e.figure.clone(),
            others: e.other_figures.clone(),
            closed: e.closed_figures.clone(),
            current: e.current_figure,
            p2: e.p2.clone(),
            surf: e.surf.clone(),
            compose: e.compose.clone(),
        }
    }
    fn restore(self, e: &mut Engine) {
        e.figure = self.figure;
        e.other_figures = self.others;
        e.closed_figures = self.closed;
        e.current_figure = self.current;
        e.p2 = self.p2;
        e.surf = self.surf;
        e.compose = self.compose;
    }
}

fn dispatch(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    match name {
        "line" => line(e, args, out),
        "quiver" => quiver(e, args, false, out),
        "quiver3" => quiver(e, args, true, out),
        "fsurf" => fsurf(e, args, out),
        "fimplicit" => fimplicit(e, args, out),
        "animatedline" => animatedline(e, args, out),
        "addpoints" => addpoints(e, args, out).map(|()| Vec::new()),
        "getpoints" => getpoints(e, args, count.max(1), out),
        "clearpoints" => clearpoints(e, args, out).map(|()| Vec::new()),
        "print" => print(e, args, out).map(|()| Vec::new()),
        _ => Err(format!("{name} is not a composed plot builtin")),
    }
}

// ------------------------------------------------------------- helpers

fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) => Some(s),
        _ => None,
    }
}

fn numeric(e: &Engine, v: &Value, who: &str) -> Result<Matrix> {
    match v {
        Value::Matrix(m) => Ok(m.clone()),
        Value::Logical(l) => l.to_numeric(&e.cancel),
        Value::Complex(_) => Err(format!("{who}: coordinates must be real")),
        Value::Text(_) | Value::Container(_) => {
            Err(format!("{who}: coordinates must be real numeric arrays"))
        }
        other => other
            .clone()
            .matrix(&e.cancel)
            .map_err(|_| format!("{who}: coordinates must be real numeric arrays")),
    }
}

fn row(values: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(
        1,
        values.len(),
        values,
    )?))
}

fn column(values: &[f64]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_slice(
        values.len(),
        1,
        values,
    )?))
}

fn is_vector(m: &Matrix) -> bool {
    m.rows() <= 1 || m.cols() <= 1
}

fn on_off(v: &Value, who: &str, what: &str) -> Result<bool> {
    match v {
        Value::Text(s) if s.eq_ignore_ascii_case("on") => Ok(true),
        Value::Text(s) if s.eq_ignore_ascii_case("off") => Ok(false),
        Value::Matrix(m) if m.len() == 1 => Ok(m.data()[0] != 0.),
        Value::Logical(l) if l.len() == 1 => Ok(l.data()[0] != 0),
        _ => Err(format!("{who}: {what} must be 'on' or 'off'")),
    }
}

fn scalar(e: &Engine, v: &Value, who: &str, what: &str) -> Result<f64> {
    let m = numeric(e, v, who).map_err(|_| format!("{who}: {what} must be a real scalar"))?;
    if m.len() != 1 || !m.data()[0].is_finite() {
        return Err(format!("{who}: {what} must be a finite real scalar"));
    }
    Ok(m.data()[0])
}

/// Run `f` with the current figure held, so it adds to the current axes;
/// the figure's own hold state is restored afterwards.
fn held<T>(e: &mut Engine, f: impl FnOnce(&mut Engine) -> Result<T>) -> Result<T> {
    let had = e.figure.as_ref().map(|f| f.hold);
    if let Some(figure) = e.figure.as_mut() {
        figure.hold = true;
    }
    let result = f(e);
    if let (Some(hold), Some(figure)) = (had, e.figure.as_mut()) {
        figure.hold = hold;
    }
    result
}

/// Name,Value pairs split off the first text argument that is one of `names`
/// (case-insensitive) or, failing that, the first text argument after the
/// positional data.
fn split_at_names<'a>(args: &'a [Value], names: &[&str]) -> (&'a [Value], &'a [Value]) {
    let at = args
        .iter()
        .position(|a| text(a).is_some_and(|t| names.iter().any(|n| n.eq_ignore_ascii_case(t))))
        .unwrap_or(args.len());
    args.split_at(at)
}

// ------------------------------------------------------------- line

/// `line(x,y)`, `line(x,y,z)`, `line(..., Name,Value)` and the all-property
/// form `line('XData',x,'YData',y,...)`: always adds to the current axes.
fn line(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let positional = args
        .iter()
        .position(|a| text(a).is_some())
        .unwrap_or(args.len());
    let (data, rest) = args.split_at(positional);
    if rest.len() % 2 != 0 {
        return Err("line: properties must be Name,Value pairs".into());
    }
    let mut coords: Vec<Option<Value>> = match data {
        [] => vec![None, None, None],
        [x, y] => vec![Some(x.clone()), Some(y.clone()), None],
        [x, y, z] => vec![Some(x.clone()), Some(y.clone()), Some(z.clone())],
        _ => return Err("Use line(x,y), line(x,y,z) or line('XData',x,'YData',y,...)".into()),
    };
    let mut props = Vec::new();
    for pair in rest.chunks(2) {
        let name = text(&pair[0]).ok_or("line: property names must be text")?;
        let slot = match name.to_ascii_lowercase().as_str() {
            "xdata" => Some(0),
            "ydata" => Some(1),
            "zdata" => Some(2),
            "parent" => {
                return Err("line: 'Parent' is not supported yet; select the axes first".into());
            }
            _ => None,
        };
        match slot {
            Some(k) => coords[k] = Some(pair[1].clone()),
            None => props.extend_from_slice(pair),
        }
    }
    let [x, y, z]: [Option<Value>; 3] = coords.try_into().expect("three coordinates");
    // XData and YData default to [0 1] (Octave and MATLAB).
    let unit = || Ok::<Value, String>(Value::Matrix(Matrix::from_column_slice(1, 2, &[0., 1.])?));
    let (x, y) = (x.map_or_else(unit, Ok)?, y.map_or_else(unit, Ok)?);
    let mut call_args = vec![x, y];
    let three = z.is_some();
    if let Some(z) = z {
        call_args.push(z);
    }
    call_args.extend(props);
    let command = if three { "plot3" } else { "plot" };
    let h = held(e, |e| {
        e.graphics_call_with_line_defaults(command, call_args, out, true)
    })?;
    Ok(vec![h])
}

// ------------------------------------------------------------- quiver

const QUIVER_NAMES: &[&str] = &[
    "AutoScale",
    "AutoScaleFactor",
    "MaxHeadSize",
    "ShowArrowHead",
    "Color",
    "LineStyle",
    "LineWidth",
    "DisplayName",
    "Visible",
    "Alignment",
];

/// `quiver(U,V)`, `quiver(X,Y,U,V)`, `quiver3(Z,U,V,W)`,
/// `quiver3(X,Y,Z,U,V,W)`, then an optional scale (0 = none), LineSpec and
/// Name,Value pairs. Returns one line handle holding shafts and heads.
fn quiver(e: &mut Engine, args: &[Value], three: bool, out: &mut Execution) -> Result<Vec<Value>> {
    let who = if three { "quiver3" } else { "quiver" };
    let (positional, rest) = split_at_names(args, QUIVER_NAMES);
    let mut auto = true;
    let mut numbers = Vec::new();
    let mut spec = None;
    for a in positional {
        match text(a) {
            Some(t) if t.eq_ignore_ascii_case("filled") => {}
            // quiver(..., 'off') turns autoscaling off, as a scale of 0 does.
            Some(t) if t.eq_ignore_ascii_case("off") && spec.is_none() => auto = false,
            Some(t) if t.eq_ignore_ascii_case("on") && spec.is_none() => {}
            Some(t) if spec.is_none() => spec = Some(t.to_string()),
            Some(_) => return Err(format!("{who}: expected one line specification")),
            None if spec.is_some() => {
                return Err(format!("{who}: data must precede the line specification"));
            }
            None => numbers.push(numeric(e, a, who)?),
        }
    }
    // A marker in the LineSpec marks the arrow bases and hides the heads
    // (Octave 10.3; MATLAB documents markers at the bases).
    let (spec, marker) = match spec {
        Some(spec) => {
            let (line, marker) = split_marker(&spec);
            (Some(line), marker)
        }
        None => (None, None),
    };
    let data_count = if three { [4, 6] } else { [2, 4] };
    let mut scale = 0.9;
    if !data_count.contains(&numbers.len()) {
        if data_count.contains(&(numbers.len().saturating_sub(1)))
            && numbers.last().is_some_and(|m| m.len() == 1)
        {
            scale = numbers.pop().expect("scale").data()[0];
            if !(scale.is_finite() && scale >= 0.) {
                return Err(format!(
                    "{who}: the scale must be a non-negative real scalar"
                ));
            }
            auto &= scale != 0.;
        } else {
            return Err(if three {
                "Use quiver3(Z,U,V,W) or quiver3(X,Y,Z,U,V,W), then an optional scale".into()
            } else {
                "Use quiver(U,V) or quiver(X,Y,U,V), then an optional scale".into()
            });
        }
    }
    let mut head = 0.2;
    let mut arrows = true;
    let mut pass = Vec::new();
    if rest.len() % 2 != 0 {
        return Err(format!("{who}: properties must be Name,Value pairs"));
    }
    for pair in rest.chunks(2) {
        let name = text(&pair[0]).ok_or(format!("{who}: property names must be text"))?;
        match name.to_ascii_lowercase().as_str() {
            "autoscale" => auto = on_off(&pair[1], who, "AutoScale")?,
            "autoscalefactor" => {
                scale = scalar(e, &pair[1], who, "AutoScaleFactor")?;
                if scale < 0. {
                    return Err(format!("{who}: AutoScaleFactor must be non-negative"));
                }
            }
            "maxheadsize" => {
                head = scalar(e, &pair[1], who, "MaxHeadSize")?;
                if head < 0. {
                    return Err(format!("{who}: MaxHeadSize must be non-negative"));
                }
            }
            "showarrowhead" => arrows = on_off(&pair[1], who, "ShowArrowHead")?,
            "alignment" => {
                if !text(&pair[1]).is_some_and(|t| t.eq_ignore_ascii_case("tail")) {
                    return Err(format!("{who}: only Alignment 'tail' is supported yet"));
                }
            }
            _ => pass.extend_from_slice(pair),
        }
    }
    // Coordinates: (x, y, z) and (u, v, w) as flat columns of one length.
    let (base, vectors): (Vec<Matrix>, Vec<Matrix>) = {
        let n = numbers.len();
        let (lead, vecs) = numbers.split_at(n - if three { 3 } else { 2 });
        (lead.to_vec(), vecs.to_vec())
    };
    let shape = (vectors[0].rows(), vectors[0].cols());
    if vectors.iter().any(|m| m.len() != vectors[0].len()) {
        return Err(format!(
            "{who}: U, V{} must be the same size",
            if three { " and W" } else { "" }
        ));
    }
    let count = vectors[0].len();
    let grid = |rows: usize, cols: usize| -> (Vec<f64>, Vec<f64>) {
        let mut x = Vec::with_capacity(rows * cols);
        let mut y = Vec::with_capacity(rows * cols);
        for c in 0..cols {
            for r in 0..rows {
                x.push((c + 1) as f64);
                y.push((r + 1) as f64);
            }
        }
        (x, y)
    };
    let (x, y, z, x_matrix) = match (three, base.as_slice()) {
        (false, []) => {
            let (x, y) = grid(shape.0, shape.1);
            (x, y, None, !is_vector(&vectors[0]))
        }
        (true, [z]) => {
            let (x, y) = grid(shape.0, shape.1);
            (x, y, Some(z.data().to_vec()), !is_vector(&vectors[0]))
        }
        (_, [x, y, rest @ ..]) => {
            let u = &vectors[0];
            let (x, y, matrix) = if is_vector(x) && is_vector(y) && !is_vector(u) {
                // Vectors along a matrix: meshgrid(x, y).
                if x.len() != u.cols() || y.len() != u.rows() {
                    return Err(format!(
                        "{who}: X must have one entry per column of U and Y one per row"
                    ));
                }
                let mut gx = Vec::with_capacity(count);
                let mut gy = Vec::with_capacity(count);
                for c in 0..u.cols() {
                    for r in 0..u.rows() {
                        gx.push(x.data()[c]);
                        gy.push(y.data()[r]);
                    }
                }
                (gx, gy, true)
            } else {
                (x.data().to_vec(), y.data().to_vec(), !is_vector(x))
            };
            (x, y, rest.first().map(|z| z.data().to_vec()), matrix)
        }
        _ => unreachable!("argument count checked"),
    };
    // A scalar Z is the height of every arrow (quiver3(0, U, V, W)).
    let z = z.map(|z| match z.as_slice() {
        [h] if count > 1 => vec![*h; count],
        _ => z,
    });
    if x.len() != count || y.len() != count || z.as_ref().is_some_and(|z| z.len() != count) {
        return Err(format!(
            "{who}: the coordinates and the vectors must have the same number of elements"
        ));
    }
    let u = vectors[0].data();
    let v = vectors[1].data();
    let w = vectors.get(2).map(|m| m.data());
    // Octave's autoscale: s = factor * |grid spacing| / longest vector.
    let len = (0..count)
        .map(|k| {
            let ww = w.map_or(0., |w| w[k]);
            (u[k] * u[k] + v[k] * v[k] + ww * ww).sqrt()
        })
        .filter(|l| !l.is_nan())
        .fold(0., f64::max);
    let mut s = 1.;
    if auto && len > 0. && scale != 0. {
        let (nx, ny) = if x_matrix {
            (shape.1 as f64, shape.0 as f64)
        } else {
            let n = (count as f64).sqrt();
            (n, n)
        };
        let range = |d: &[f64]| {
            let (lo, hi) = d
                .iter()
                .filter(|v| !v.is_nan())
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                    (a.min(v), b.max(v))
                });
            if hi >= lo { hi - lo } else { 0. }
        };
        let dx = range(&x) / nx;
        let dy = range(&y) / ny;
        let dz = z.as_ref().map_or(0., |z| range(z) / nx.max(ny));
        let sd = (dx * dx + dy * dy + dz * dz).sqrt() / len;
        s = if sd != 0. { scale * sd } else { scale };
    }
    // Octave normalizes MaxHeadSize 0.2 to an arrowhead 1/3 of the arrow.
    let arrow = head / (0.2 * 3.);
    let arrows = arrows && marker.is_none();
    let per = if arrows { 7 } else { 3 };
    if count.saturating_mul(per) > crate::plot::MAX_POINTS {
        return Err(format!(
            "{who}: {count} arrows exceed the figure's point limit"
        ));
    }
    let mut px = Vec::with_capacity(count * per);
    let mut py = Vec::with_capacity(count * per);
    let mut pz = Vec::with_capacity(if three { count * per } else { 0 });
    for k in 0..count {
        let (uu, vv) = (s * u[k], s * v[k]);
        let ww = w.map_or(0., |w| s * w[k]);
        let z0 = z.as_ref().map_or(0., |z| z[k]);
        let (xe, ye, ze) = (x[k] + uu, y[k] + vv, z0 + ww);
        px.extend([x[k], xe, f64::NAN]);
        py.extend([y[k], ye, f64::NAN]);
        if three {
            pz.extend([z0, ze, f64::NAN]);
        }
        if arrows {
            let xt = x[k] + uu * (1. - arrow);
            let yt = y[k] + vv * (1. - arrow);
            if three {
                let xy = (uu * uu + vv * vv + ww * ww).sqrt()
                    / ((uu * uu + vv * vv).sqrt() + f64::EPSILON);
                let (hx, hy) = (vv * xy * arrow / 4., uu * xy * arrow / 4.);
                let zt = ze - ww * arrow;
                px.extend([xt + hx, xe, xt - hx, f64::NAN]);
                py.extend([yt - hy, ye, yt + hy, f64::NAN]);
                pz.extend([zt, ze, zt, f64::NAN]);
            } else {
                let (hx, hy) = (vv * arrow / 3., uu * arrow / 3.);
                px.extend([xt + hx, xe, xt - hx, f64::NAN]);
                py.extend([yt - hy, ye, yt + hy, f64::NAN]);
            }
        }
    }
    let mut call_args = vec![column(&px)?, column(&py)?];
    if three {
        call_args.push(column(&pz)?);
    }
    if let Some(spec) = spec.filter(|s| !s.is_empty()) {
        call_args.push(Value::Text(spec));
    }
    call_args.extend(pass);
    let command = if three { "plot3" } else { "plot" };
    let h = e.call(command, call_args, out)?;
    if let Some(marker) = marker {
        let color = e.call("get", vec![h.clone(), Value::Text("Color".into())], out)?;
        let mut base = vec![column(&x)?, column(&y)?];
        if three {
            base.push(column(z.as_deref().unwrap_or(&[]))?);
        }
        base.extend([
            Value::Text("LineStyle".into()),
            Value::Text("none".into()),
            Value::Text("Marker".into()),
            Value::Text(marker),
            Value::Text("Color".into()),
            color,
        ]);
        held(e, |e| e.call(command, base, out))?;
    }
    Ok(vec![h])
}

/// A LineSpec without its marker, and the marker.
fn split_marker(spec: &str) -> (String, Option<String>) {
    let mut line = String::new();
    let mut marker = None;
    let mut rest = spec;
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("--") || rest.starts_with("-.") {
            line.push_str(&rest[..2]);
            rest = &rest[2..];
            continue;
        }
        if "o+*.xsd^v<>ph".contains(c) {
            marker = Some(c.to_string());
        } else {
            line.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    (line, marker)
}

// ------------------------------------------------------------- function plots

/// Evaluate `f` on equal-size grids: one vectorized call, else point by point.
/// The vectorized call is trusted only after a 2-by-3 trial of six grid
/// points agrees with scalar calls there: on the square grid `x*y` is a
/// matrix product, on the trial it errors or has the wrong size.
fn eval_grid(
    e: &mut Engine,
    f: &crate::FunctionHandle,
    grids: &[&Matrix],
    who: &str,
    out: &mut Execution,
) -> Result<Matrix> {
    let (rows, cols) = (grids[0].rows(), grids[0].cols());
    let n = rows * cols;
    let picks: Vec<usize> = if n >= 6 {
        vec![0, n / 5, 2 * n / 5, 3 * n / 5, 4 * n / 5, n - 1]
    } else {
        (0..n).collect()
    };
    let (tr, tc) = if picks.len() == 6 {
        (2, 3)
    } else {
        (1, picks.len())
    };
    let trial: Vec<Matrix> = grids
        .iter()
        .map(|g| {
            let d: Vec<f64> = picks.iter().map(|&k| g.data()[k]).collect();
            Matrix::from_column_slice(tr, tc, &d)
        })
        .collect::<Result<_>>()?;
    let trial_refs: Vec<&Matrix> = trial.iter().collect();
    let mut vectorized = false;
    if let Some(values) = try_vector(e, f, &trial_refs, who, out)? {
        vectorized = true;
        for (i, &k) in picks.iter().enumerate() {
            let v = eval_point(e, f, grids, k, who, out)?;
            let w = values[i];
            let same = (v.is_nan() && w.is_nan())
                || v == w
                || (v - w).abs() <= 1e-9 * v.abs().max(w.abs());
            if !same {
                vectorized = false;
                break;
            }
        }
    }
    if vectorized && let Some(values) = try_vector(e, f, grids, who, out)? {
        return Matrix::from_column_slice(rows, cols, &values);
    }
    let mut data = Vec::with_capacity(n);
    for k in 0..n {
        data.push(eval_point(e, f, grids, k, who, out)?);
    }
    Matrix::from_column_slice(rows, cols, &data)
}

/// One call of f on whole equal-size arrays: its values when it returns one
/// real value per element (or one constant), None when it errors or returns
/// another size. As in fplot the failed call leaves no error state behind;
/// cancellation and run limits still stop.
fn try_vector(
    e: &mut Engine,
    f: &crate::FunctionHandle,
    grids: &[&Matrix],
    who: &str,
    out: &mut Execution,
) -> Result<Option<Vec<f64>>> {
    let n = grids[0].len();
    let inputs: Vec<Value> = grids.iter().map(|m| Value::Matrix((*m).clone())).collect();
    let failure = e.functions.failure.clone();
    let file_diagnostic = e.file_diagnostic.clone();
    let value = match e.call_handle(f, inputs, 1, None, out) {
        Ok(r) => r.into_iter().next(),
        Err(message) if e.is_catchable(&message) => {
            e.functions.failure = failure;
            e.file_diagnostic = file_diagnostic;
            e.raised = None;
            e.functions.lng.thrown_stack = None;
            e.functions.lng.thrown_object = None;
            None
        }
        Err(message) => return Err(message),
    };
    Ok(match value {
        Some(Value::Complex(_)) => return Err(format!("{who} requires a real-valued function")),
        Some(v @ (Value::Matrix(_) | Value::Logical(_))) => {
            let m = numeric(e, &v, who)?;
            if m.len() == n {
                Some(m.data().to_vec())
            } else if m.len() == 1 {
                // A constant function: f = @(x,y) 2.
                Some(vec![m.data()[0]; n])
            } else {
                None
            }
        }
        _ => None,
    })
}

/// f at grid point k, called with scalars.
fn eval_point(
    e: &mut Engine,
    f: &crate::FunctionHandle,
    grids: &[&Matrix],
    k: usize,
    who: &str,
    out: &mut Execution,
) -> Result<f64> {
    e.check_cancel()?;
    let point: Vec<Value> = grids
        .iter()
        .map(|m| Value::Matrix(Matrix::scalar(m.data()[k])))
        .collect();
    let r = e.call_handle(f, point, 1, None, out)?;
    let v = r
        .into_iter()
        .next()
        .ok_or(format!("{who}: the function returned no value"))?;
    let m = numeric(e, &v, who).map_err(|_| format!("{who} requires a real numeric function"))?;
    if m.len() != 1 {
        return Err(format!(
            "{who}: the function must return one value per point"
        ));
    }
    Ok(m.data()[0])
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|k| {
            if k + 1 == n {
                b
            } else {
                a + (b - a) * k as f64 / (n - 1) as f64
            }
        })
        .collect()
}

/// meshgrid(xs, ys) as two rows-by-cols matrices.
fn meshgrid(xs: &[f64], ys: &[f64]) -> Result<(Matrix, Matrix)> {
    let (rows, cols) = (ys.len(), xs.len());
    let mut gx = Vec::with_capacity(rows * cols);
    let mut gy = Vec::with_capacity(rows * cols);
    for &x in xs {
        for &y in ys {
            gx.push(x);
            gy.push(y);
        }
    }
    Ok((
        Matrix::from_column_slice(rows, cols, &gx)?,
        Matrix::from_column_slice(rows, cols, &gy)?,
    ))
}

/// Leading function handles, an optional [min max] or [xmin xmax ymin ymax]
/// interval, then everything else (LineSpec, Name,Value pairs), with the
/// MeshDensity pair taken out.
struct FunctionArgs {
    functions: Vec<crate::FunctionHandle>,
    interval: [f64; 4],
    density: usize,
    rest: Vec<Value>,
}

fn function_args(
    e: &Engine,
    args: &[Value],
    default_density: usize,
    who: &str,
) -> Result<FunctionArgs> {
    let mut k = 0;
    let mut functions = Vec::new();
    while let Some(Value::Container(Array::Handle(f))) = args.get(k) {
        functions.push(f.clone());
        k += 1;
    }
    if functions.is_empty() {
        return Err(format!(
            "{who} requires a function handle, for example {who}(@(x,y) x.^2 + y.^2)"
        ));
    }
    let mut interval = [-5., 5., -5., 5.];
    if let Some(a) = args.get(k)
        && text(a).is_none()
    {
        let m = numeric(e, a, who)?;
        let d = m.data();
        interval = match d.len() {
            2 => [d[0], d[1], d[0], d[1]],
            4 => [d[0], d[1], d[2], d[3]],
            _ => {
                return Err(format!(
                    "{who}: the interval must be [min max] or [xmin xmax ymin ymax]"
                ));
            }
        };
        if interval.iter().any(|v| !v.is_finite())
            || interval[0] >= interval[1]
            || interval[2] >= interval[3]
        {
            return Err(format!("{who}: the interval must be finite and increasing"));
        }
        k += 1;
    }
    let mut density = default_density;
    let mut rest = Vec::new();
    let mut i = k;
    while i < args.len() {
        if text(&args[i]).is_some_and(|t| t.eq_ignore_ascii_case("MeshDensity")) {
            let value = args
                .get(i + 1)
                .ok_or(format!("{who}: MeshDensity needs a value"))?;
            let d = scalar(e, value, who, "MeshDensity")?;
            if !(2. ..=1000.).contains(&d) || d.fract() != 0. {
                return Err(format!("{who}: MeshDensity must be an integer in 2..1000"));
            }
            density = d as usize;
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    Ok(FunctionArgs {
        functions,
        interval,
        density,
        rest,
    })
}

/// `fsurf(f)`, `fsurf(f, [xmin xmax ymin ymax])`, `fsurf(fx, fy, fz, [umin
/// umax vmin vmax])`, then surf options and MeshDensity (default 35).
fn fsurf(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "fsurf";
    let a = function_args(e, args, 35, who)?;
    if !matches!(a.functions.len(), 1 | 3) {
        return Err(
            "fsurf takes one function f(x,y) or three parametric functions x(u,v), y(u,v), z(u,v)"
                .into(),
        );
    }
    let [x0, x1, y0, y1] = a.interval;
    let (gx, gy) = meshgrid(&linspace(x0, x1, a.density), &linspace(y0, y1, a.density))?;
    let (x, y, z) = if a.functions.len() == 1 {
        let z = eval_grid(e, &a.functions[0], &[&gx, &gy], who, out)?;
        (gx, gy, z)
    } else {
        let x = eval_grid(e, &a.functions[0], &[&gx, &gy], who, out)?;
        let y = eval_grid(e, &a.functions[1], &[&gx, &gy], who, out)?;
        let z = eval_grid(e, &a.functions[2], &[&gx, &gy], who, out)?;
        (x, y, z)
    };
    let mut call_args = vec![Value::Matrix(x), Value::Matrix(y), Value::Matrix(z)];
    call_args.extend(a.rest);
    let h = e.call("surf", call_args, out)?;
    Ok(vec![h])
}

/// `fimplicit(f)`, `fimplicit(f, [xmin xmax ymin ymax])`, then a LineSpec,
/// plot options and MeshDensity (default 151): the zero contour of f as one
/// line (NaN-separated pieces).
fn fimplicit(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "fimplicit";
    let a = function_args(e, args, 151, who)?;
    if a.functions.len() != 1 {
        return Err("fimplicit takes one function f(x,y)".into());
    }
    let [x0, x1, y0, y1] = a.interval;
    let xs = linspace(x0, x1, a.density);
    let ys = linspace(y0, y1, a.density);
    let (gx, gy) = meshgrid(&xs, &ys)?;
    let f = eval_grid(e, &a.functions[0], &[&gx, &gy], who, out)?;
    let c = e.call(
        "contourc",
        vec![row(&xs)?, row(&ys)?, Value::Matrix(f), row(&[0., 0.])?],
        out,
    )?;
    let Value::Matrix(c) = c else {
        return Err("fimplicit: contourc returned no contour matrix".into());
    };
    let (mut px, mut py) = (Vec::new(), Vec::new());
    let mut k = 0;
    while k < c.cols() {
        let n = c.get(1, k) as usize;
        if !px.is_empty() {
            px.push(f64::NAN);
            py.push(f64::NAN);
        }
        for j in k + 1..(k + 1 + n).min(c.cols()) {
            px.push(c.get(0, j));
            py.push(c.get(1, j));
        }
        k += n + 1;
    }
    if px.is_empty() {
        px.push(f64::NAN);
        py.push(f64::NAN);
    }
    let mut call_args = vec![row(&px)?, row(&py)?];
    call_args.extend(a.rest);
    let h = e.call("plot", call_args, out)?;
    // The axes show the whole interval, not just the curve.
    e.call("xlim", vec![row(&[x0, x1])?], out)?;
    e.call("ylim", vec![row(&[y0, y1])?], out)?;
    Ok(vec![h])
}

// ------------------------------------------------------------- animatedline

fn series_handle(e: &Engine, v: &Value, who: &str) -> Result<ObjectIdentity> {
    let Value::Container(Array::Graphics(h)) = v else {
        return Err(format!(
            "{who}: the first argument must be an animated line handle"
        ));
    };
    let h = h
        .scalar_handle()
        .map_err(|_| format!("{who}: expected one animated line handle"))?;
    if h.kind() != GraphicsKind::Series {
        return Err(format!(
            "{who}: the first argument must be an animated line handle"
        ));
    }
    let id = h.object.clone();
    if !e
        .compose
        .animated
        .iter()
        .any(|(a, _)| a.object.same_object(&id))
    {
        return Err(format!("{who}: the handle is not an animated line"));
    }
    Ok(id)
}

/// `animatedline`, `animatedline(x,y[,z])`, then Name,Value pairs with
/// MaximumNumPoints; adds an (initially empty) line to the current axes.
fn animatedline(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<Vec<Value>> {
    let who = "animatedline";
    let positional = args
        .iter()
        .position(|a| text(a).is_some())
        .unwrap_or(args.len());
    let (data, rest) = args.split_at(positional);
    if rest.len() % 2 != 0 {
        return Err("animatedline: properties must be Name,Value pairs".into());
    }
    let mut max_points = DEFAULT_MAX_POINTS;
    let mut props = Vec::new();
    for pair in rest.chunks(2) {
        let name = text(&pair[0]).ok_or("animatedline: property names must be text")?;
        if name.eq_ignore_ascii_case("MaximumNumPoints") {
            let n = scalar(e, &pair[1], who, "MaximumNumPoints")?;
            if n < 1. || n.fract() != 0. {
                return Err("animatedline: MaximumNumPoints must be a positive integer".into());
            }
            max_points = (n as usize).min(DEFAULT_MAX_POINTS);
        } else {
            props.extend_from_slice(pair);
        }
    }
    let initial: Vec<Matrix> = data
        .iter()
        .map(|a| numeric(e, a, who))
        .collect::<Result<_>>()?;
    if !matches!(initial.len(), 0 | 2 | 3) {
        return Err("Use animatedline, animatedline(x,y) or animatedline(x,y,z)".into());
    }
    let three = initial.len() == 3;
    let mut call_args = vec![Value::Matrix(Matrix::scalar(f64::NAN)); if three { 3 } else { 2 }];
    call_args.extend(props);
    let command = if three { "plot3" } else { "plot" };
    let h = held(e, |e| {
        e.graphics_call_with_line_defaults(command, call_args, out, true)
    })?;
    let Value::Container(Array::Graphics(g)) = &h else {
        return Err("animatedline: plot returned no handle".into());
    };
    let handle = g.scalar_handle()?.clone();
    // Records of deleted lines go first; live ones are never forgotten.
    let records = std::mem::take(&mut e.compose.animated);
    let mut live: Vec<(crate::GraphicsHandle, usize)> = records
        .into_iter()
        .filter(|(a, _)| !a.same_object(&handle) && e.figures().any(|(_, f)| a.is_live_in(f)))
        .collect();
    if live.len() >= MAX_ANIMATED {
        e.compose.animated = live;
        return Err(format!(
            "animatedline: at most {MAX_ANIMATED} animated lines can exist at once"
        ));
    }
    live.push((handle, max_points));
    e.compose.animated = live;
    set_points(e, &h, &[Vec::new(), Vec::new(), Vec::new()], three, out)?;
    if !initial.is_empty() {
        let mut add = vec![h.clone()];
        add.extend(initial.into_iter().map(Value::Matrix));
        addpoints(e, &add, out)?;
    }
    Ok(vec![h])
}

fn set_points(
    e: &mut Engine,
    h: &Value,
    xyz: &[Vec<f64>; 3],
    three: bool,
    out: &mut Execution,
) -> Result<()> {
    let mut args = vec![
        h.clone(),
        Value::Text("XData".into()),
        row(&xyz[0])?,
        Value::Text("YData".into()),
        row(&xyz[1])?,
    ];
    if three {
        args.push(Value::Text("ZData".into()));
        args.push(row(&xyz[2])?);
    }
    e.call("set", args, out)?;
    Ok(())
}

fn get_points(e: &mut Engine, h: &Value, out: &mut Execution) -> Result<[Vec<f64>; 3]> {
    let mut get = |e: &mut Engine, name: &str| -> Result<Vec<f64>> {
        let v = e.call("get", vec![h.clone(), Value::Text(name.into())], out)?;
        Ok(numeric(e, &v, "getpoints")?.data().to_vec())
    };
    Ok([get(e, "XData")?, get(e, "YData")?, get(e, "ZData")?])
}

/// `addpoints(h, x, y[, z])`: append, keeping the newest MaximumNumPoints.
fn addpoints(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<()> {
    let who = "addpoints";
    if !matches!(args.len(), 3 | 4) {
        return Err("Use addpoints(h, x, y) or addpoints(h, x, y, z)".into());
    }
    let id = series_handle(e, &args[0], who)?;
    let max = e
        .compose
        .animated
        .iter()
        .find(|(a, _)| a.object.same_object(&id))
        .map_or(DEFAULT_MAX_POINTS, |(_, n)| *n);
    let new: Vec<Vec<f64>> = args[1..]
        .iter()
        .map(|a| numeric(e, a, who).map(|m| m.data().to_vec()))
        .collect::<Result<_>>()?;
    if new.iter().any(|v| v.len() != new[0].len()) {
        return Err("addpoints: x, y and z must have the same number of elements".into());
    }
    let mut points = get_points(e, &args[0], out)?;
    let three = new.len() == 3 || !points[2].is_empty();
    let n_old = points[0].len();
    for (k, p) in points.iter_mut().enumerate() {
        if k == 2 && !three {
            break;
        }
        if k == 2 && p.len() < n_old {
            p.resize(n_old, 0.);
        }
        match new.get(k) {
            Some(v) => p.extend_from_slice(v),
            None => p.extend(std::iter::repeat_n(0., new[0].len())),
        }
        if p.len() > max {
            p.drain(..p.len() - max);
        }
    }
    set_points(e, &args[0], &points, three, out)
}

/// `[x, y, z] = getpoints(h)`.
fn getpoints(
    e: &mut Engine,
    args: &[Value],
    count: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let [h] = args else {
        return Err("Use [x, y, z] = getpoints(h)".into());
    };
    series_handle(e, h, "getpoints")?;
    let mut points = get_points(e, h, out)?;
    // A 2-D line's z is zeros, one per point.
    if points[2].len() != points[0].len() {
        points[2] = vec![0.; points[0].len()];
    }
    points.iter().take(count).map(|p| row(p)).collect()
}

/// `clearpoints(h)`.
fn clearpoints(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<()> {
    let [h] = args else {
        return Err("Use clearpoints(h)".into());
    };
    series_handle(e, h, "clearpoints")?;
    let three = !get_points(e, h, out)?[2].is_empty();
    set_points(e, h, &[Vec::new(), Vec::new(), Vec::new()], three, out)
}

// ------------------------------------------------------------- print

/// `print(filename, '-dformat', '-rN')`, `print(fig, ...)`, `print -dpng out`:
/// PNG, JPEG, TIFF, PDF, SVG (through exportgraphics) and EPSC (saveas).
fn print(e: &mut Engine, args: &[Value], out: &mut Execution) -> Result<()> {
    let mut figure = None;
    let mut device: Option<String> = None;
    let mut resolution: Option<f64> = None;
    let mut filename: Option<String> = None;
    for a in args {
        match a {
            Value::Container(Array::Graphics(_)) if figure.is_none() => figure = Some(a.clone()),
            Value::Text(t) if t.starts_with("-d") => device = Some(t[2..].to_ascii_lowercase()),
            Value::Text(t) if t.starts_with("-r") => {
                let r: f64 = t[2..]
                    .parse()
                    .map_err(|_| format!("print: invalid resolution '{t}'"))?;
                if !(0. ..=1200.).contains(&r) {
                    return Err("print: the resolution must be in 0..1200 dpi".into());
                }
                resolution = Some(if r == 0. { 96. } else { r });
            }
            Value::Text(t) if t.starts_with("-f") && t.len() > 2 => {
                let n: f64 = t[2..]
                    .parse()
                    .map_err(|_| format!("print: invalid figure option '{t}'"))?;
                if figure.is_some() {
                    return Err("print: give the figure once".into());
                }
                figure = Some(Value::Matrix(Matrix::scalar(n)));
            }
            Value::Text(t)
                if matches!(
                    t.to_ascii_lowercase().as_str(),
                    "-painters" | "-opengl" | "-vector" | "-image" | "-noui" | "-bestfit" | "-fillpage" | "-loose"
                ) => {}
            Value::Text(t) if t.starts_with('-') => {
                return Err(format!("print: option '{t}' is not supported yet"));
            }
            Value::Text(t) if filename.is_none() => filename = Some(t.clone()),
            // A figure number; the writers check that the figure exists.
            Value::Matrix(m) if m.len() == 1 && figure.is_none() => figure = Some(a.clone()),
            _ => return Err("Use print(filename, '-dformat'), print(fig, filename, '-dformat') or print -dpng filename".into()),
        }
    }
    let filename = filename.ok_or(
        "print: printing to a printer is not supported yet; give a filename, for example print('-dpng','out.png')",
    )?;
    let path = std::path::Path::new(&filename);
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase);
    let format = match device.as_deref() {
        Some("png") => "png",
        Some("jpeg" | "jpg") => "jpg",
        Some("tiff" | "tiffn" | "tif") => "tif",
        Some("pdf") => "pdf",
        Some("svg") => "svg",
        Some("epsc") => "eps",
        Some(other) => {
            return Err(format!(
                "print: device '-d{other}' is not supported yet; use -dpng, -djpeg, -dtiff, -dpdf, -dsvg or -depsc"
            ));
        }
        None => match ext.as_deref() {
            Some(e @ ("png" | "pdf" | "svg" | "eps")) => e,
            Some("jpg" | "jpeg") => "jpg",
            Some("tif" | "tiff") => "tif",
            _ => "png",
        },
    };
    // The device's extension is added when the name has none, as MATLAB and
    // Octave do; a name with another extension is kept exactly. The writers
    // pick the format by extension, so such a file is written under a
    // temporary name with the device's extension and renamed.
    let matches = |x: &str| match format {
        "jpg" => matches!(x, "jpg" | "jpeg"),
        "tif" => matches!(x, "tif" | "tiff"),
        f => x == f,
    };
    let (filename, rename) = match ext.as_deref() {
        None => (format!("{filename}.{format}"), None),
        Some(x) if matches(x) => (filename, None),
        Some(_) => (format!("{filename}.matrust-print.{format}"), Some(filename)),
    };
    let result = write_figure(e, figure, &filename, format, resolution, out);
    let Some(target) = rename else {
        return result;
    };
    let from = e.file_path(&filename, "")?;
    if let Err(error) = result {
        let _ = std::fs::remove_file(&from);
        return Err(error);
    }
    let to = e.file_path(&target, "")?;
    std::fs::rename(&from, &to).map_err(|err| {
        let _ = std::fs::remove_file(&from);
        format!("print: cannot write {target}: {err}")
    })
}

fn write_figure(
    e: &mut Engine,
    figure: Option<Value>,
    filename: &str,
    format: &str,
    resolution: Option<f64>,
    out: &mut Execution,
) -> Result<()> {
    let filename = filename.to_string();
    let figure = match figure {
        Some(f) => f,
        None => e.call("gcf", Vec::new(), out)?,
    };
    if format == "eps" {
        if resolution.is_some() {
            return Err("print: -r is not supported with -depsc yet".into());
        }
        let values = e.graphics_file_arguments(vec![
            figure,
            Value::Text(filename),
            Value::Text("epsc".into()),
        ])?;
        return e.figure_export_command(values);
    }
    let mut call_args = vec![figure, Value::Text(filename)];
    if let Some(r) = resolution {
        call_args.push(Value::Text("Resolution".into()));
        call_args.push(Value::Matrix(Matrix::scalar(r.round())));
    }
    e.export_graphics_command(call_args)
}
