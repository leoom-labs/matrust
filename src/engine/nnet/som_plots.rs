//! SOM graphics use ordinary editable patch, line and text calls. Triangles and
//! NaN-separated boundary/neighbor lines keep default maps within renderer caps.
use super::*;
fn txt(s: &str) -> Value {
    Value::Text(s.into())
}
fn row(v: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(1, v.len(), v)?))
}
#[derive(Default)]
struct Geometry {
    x: Vec<f64>,
    y: Vec<f64>,
}
fn polygon(g: &mut Geometry, x: &[f64], y: &[f64]) {
    // Fan triangulation: four triangles per hexagon, two per rectangle/diamond.
    for k in 1..x.len() - 1 {
        g.x.extend([x[0], x[k], x[k + 1]]);
        g.y.extend([y[0], y[k], y[k + 1]]);
    }
}
fn patch(
    e: &mut Engine,
    out: &mut Execution,
    g: Geometry,
    color: Vec<f64>,
    label: &str,
) -> Result<()> {
    e.call(
        "patch",
        vec![
            txt("XData"),
            Value::Matrix(Matrix::from_column_vec(3, g.x.len() / 3, g.x)?),
            txt("YData"),
            Value::Matrix(Matrix::from_column_vec(3, g.y.len() / 3, g.y)?),
            txt("EdgeColor"),
            txt("none"),
            txt("FaceColor"),
            row(color)?,
            txt("DisplayName"),
            txt(label),
        ],
        out,
    )?;
    Ok(())
}
fn line(
    e: &mut Engine,
    out: &mut Execution,
    g: Geometry,
    label: &str,
    color: Vec<f64>,
) -> Result<()> {
    if !g.x.is_empty() {
        e.call(
            "line",
            vec![
                row(g.x)?,
                row(g.y)?,
                txt("Color"),
                row(color)?,
                txt("DisplayName"),
                txt(label),
            ],
            out,
        )?;
    }
    Ok(())
}
pub(in crate::engine) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    out: &mut Execution,
) -> Result<Value> {
    if name == "plotsompos" {
        if !(1..=2).contains(&args.len()) {
            return Err("plotsompos: expected net[,inputs]".into());
        }
    } else {
        arity(name, args, if name == "plotsomhits" { 2 } else { 1 })?;
    }
    let o = super::super::objects::nnet_value(e, &args[0]).ok_or("SOM plot: network required")?;
    let s = o.som.as_ref().ok_or("SOM plot: selforgmap required")?;
    if s.positions.rows() > 2 || s.topology == "randtop" {
        return Err("SOM plots support one/two-dimensional gridtop or hextop maps".into());
    }
    if !o.core.configured
        && matches!(
            name,
            "plotsomnd" | "plotsomplanes" | "plotsomhits" | "plotsompos"
        )
    {
        return Err("SOM plot: configure network first".into());
    }
    if name == "plotsompos" && o.core.input.size > 2 {
        return Err("plotsompos: at most two input dimensions supported".into());
    }
    let n = s.positions.cols();
    let planes = if name == "plotsomplanes" {
        o.core.input.size
    } else {
        1
    };
    if planes > 16 {
        return Err("plotsomplanes: at most 16 input planes supported".into());
    }
    // Each weight plane needs one outline and up to steps+1 fill groups.
    // Fit the shared figure series budget even when every bin is occupied.
    let plane_steps = (crate::plot::MAX_SERIES / planes - 2).min(30);
    let p = &s.positions;
    let coordinate = |i: usize, r: usize| {
        if name == "plotsompos" {
            if r < o.core.input.size {
                o.core.iw[0].data()[i + r * n]
            } else {
                0.
            }
        } else if r < p.rows() {
            p.data()[r + i * p.rows()]
        } else {
            0.
        }
    };
    let mut edges = vec![];
    for j in 0..n {
        e.check_cancel()?;
        for i in 0..j {
            if s.distances.data()[i + j * n] <= 1. + 1e-10 {
                edges.push((i, j));
            }
        }
    }
    let faces = planes
        .saturating_mul(n)
        .saturating_mul(if s.topology == "hextop" { 4 } else { 2 })
        + if name == "plotsomnd" {
            edges.len() * 2
        } else {
            0
        };
    if faces > 10000 {
        return Err("SOM plot: renderer face limit exceeded".into());
    }
    e.charge(n.saturating_mul(n) + faces * 16)?;
    let input = if args.len() == 2 {
        let x = matrix(e, &args[1])?;
        if x.rows() != o.core.input.size {
            return Err("SOM plot: input dimensions mismatch".into());
        }
        Some(x)
    } else {
        None
    };
    let mut hits = vec![0usize; n];
    if name == "plotsomhits" {
        for i in som::winners(e, &o, input.as_ref().unwrap())? {
            hits[i] += 1;
        }
    }
    let f = e.call("figure", vec![], out)?;
    for plane in 0..planes {
        if planes > 1 {
            let side = (planes as f64).sqrt().ceil();
            e.call(
                "subplot",
                vec![
                    Value::Matrix(Matrix::scalar(side)),
                    Value::Matrix(Matrix::scalar(side)),
                    Value::Matrix(Matrix::scalar((plane + 1) as f64)),
                ],
                out,
            )?;
        }
        e.call("hold", vec![txt("on")], out)?;
        if let Some(x) = &input
            && name == "plotsompos"
        {
            e.call(
                "plot",
                vec![
                    row((0..x.cols()).map(|c| x.data()[c * x.rows()]).collect())?,
                    row((0..x.cols())
                        .map(|c| {
                            if x.rows() > 1 {
                                x.data()[1 + c * x.rows()]
                            } else {
                                0.
                            }
                        })
                        .collect())?,
                    txt("+"),
                    txt("DisplayName"),
                    txt("Data"),
                ],
                out,
            )?;
        }
        let mut neighbor = Geometry::default();
        let mut distances = BTreeMap::<usize, Geometry>::new();
        for &(i, j) in &edges {
            neighbor
                .x
                .extend([coordinate(i, 0), coordinate(j, 0), f64::NAN]);
            neighbor
                .y
                .extend([coordinate(i, 1), coordinate(j, 1), f64::NAN]);
            if name == "plotsomnd" {
                let d = (0..o.core.input.size)
                    .map(|r| {
                        (o.core.iw[0].data()[i + r * n] - o.core.iw[0].data()[j + r * n]).powi(2)
                    })
                    .sum::<f64>()
                    .sqrt();
                let bin = (31. * d / (1. + d)).round() as usize;
                let cx = (coordinate(i, 0) + coordinate(j, 0)) / 2.;
                let cy = (coordinate(i, 1) + coordinate(j, 1)) / 2.;
                polygon(
                    distances.entry(bin).or_default(),
                    &[cx - 0.12, cx, cx + 0.12, cx],
                    &[cy, cy - 0.12, cy, cy + 0.12],
                );
            }
        }
        if name != "plotsomplanes" {
            line(
                e,
                out,
                neighbor,
                "Neighbor",
                if name == "plotsomnd" {
                    vec![1., 0., 0.]
                } else {
                    vec![0.5, 0.5, 0.5]
                },
            )?;
        }
        for (bin, g) in distances {
            let a = bin as f64 / 31.;
            patch(e, out, g, vec![a, a, 0.], "Distance")?;
        }
        if name == "plotsompos" {
            e.call(
                "plot",
                vec![
                    row((0..n).map(|i| coordinate(i, 0)).collect())?,
                    row((0..n).map(|i| coordinate(i, 1)).collect())?,
                    txt("o"),
                    txt("DisplayName"),
                    txt("Neuron"),
                ],
                out,
            )?;
        } else {
            let (lo, hi) = if name == "plotsomplanes" {
                (0..n)
                    .map(|i| o.core.iw[0].data()[i + plane * n])
                    .fold((0f64, 0f64), |(lo, hi), w| (lo.min(w), hi.max(w)))
            } else {
                (0., 1.)
            };
            let maxhits = hits.iter().copied().max().unwrap_or(1).max(1) as f64;
            let mut groups = BTreeMap::<usize, Geometry>::new();
            let mut boundary = Geometry::default();
            for (i, &hit) in hits.iter().enumerate() {
                e.check_cancel()?;
                let cx = coordinate(i, 0);
                let cy = coordinate(i, 1);
                let scale = if name == "plotsomhits" {
                    0.12 + 0.35 * (hit as f64 / maxhits).sqrt()
                } else {
                    0.48
                };
                let (x, y) = if s.topology == "hextop" {
                    let angles = (0..6)
                        .map(|k| std::f64::consts::PI / 6. + k as f64 * std::f64::consts::TAU / 6.)
                        .collect::<Vec<_>>();
                    (
                        angles
                            .iter()
                            .map(|a| cx + scale * a.cos())
                            .collect::<Vec<_>>(),
                        angles
                            .iter()
                            .map(|a| cy + scale * a.sin())
                            .collect::<Vec<_>>(),
                    )
                } else {
                    (
                        vec![cx - scale, cx + scale, cx + scale, cx - scale],
                        vec![cy - scale, cy - scale, cy + scale, cy + scale],
                    )
                };
                let bin = if name == "plotsomplanes" {
                    let w = o.core.iw[0].data()[i + plane * n];
                    let a = if w >= 0. {
                        if hi > 0. { 0.5 + 0.5 * w / hi } else { 0.5 }
                    } else {
                        0.5 - 0.5 * w / lo
                    };
                    (plane_steps as f64 * a).round() as usize
                } else {
                    0
                };
                polygon(groups.entry(bin).or_default(), &x, &y);
                boundary.x.extend(&x);
                boundary.x.extend([x[0], f64::NAN]);
                boundary.y.extend(&y);
                boundary.y.extend([y[0], f64::NAN]);
                if name == "plotsomhits" && n <= 256 {
                    e.call(
                        "text",
                        vec![
                            Value::Matrix(Matrix::scalar(cx)),
                            Value::Matrix(Matrix::scalar(cy)),
                            txt(&hit.to_string()),
                        ],
                        out,
                    )?;
                }
            }
            for (bin, g) in groups {
                let a = bin as f64 / plane_steps as f64;
                let color = if name == "plotsomplanes" {
                    vec![(2. * a).min(1.), (2. * a - 1.).max(0.), 0.]
                } else {
                    vec![0.8, 0.85, 1.]
                };
                patch(e, out, g, color, "Neuron")?;
            }
            line(e, out, boundary, "NeuronOutline", vec![0., 0., 0.])?;
        }
        e.call(
            "title",
            vec![txt(match name {
                "plotsomtop" => "SOM Topology",
                "plotsomnc" => "SOM Neighbor Connections",
                "plotsomnd" => "SOM Neighbor Distances",
                "plotsomplanes" => "SOM Weight Plane",
                "plotsomhits" => "SOM Sample Hits",
                _ => "SOM Weight Positions",
            })],
            out,
        )?;
        e.call("hold", vec![txt("off")], out)?;
    }
    Ok(f)
}
