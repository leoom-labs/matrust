//! Complete value state for network equality. Matrix payloads remain shared.
use super::*;

fn numbers(values: impl IntoIterator<Item = f64>) -> Result<Array> {
    let values: Vec<_> = values.into_iter().collect();
    Ok(Array::Numeric(Matrix::from_column_vec(
        1,
        values.len(),
        values,
    )?))
}
fn indices(values: &[usize]) -> Result<Array> {
    numbers(values.iter().map(|&x| x as f64))
}
fn port_state(p: &network::Port, c: &AtomicBool) -> Result<Array> {
    let settings = p
        .process_settings
        .iter()
        .map(|p| {
            record(
                vec![
                    ("name", string(&p.name, c)?),
                    ("xrows", num(p.xrows as f64)),
                    ("xcols", num(p.xcols as f64)),
                    ("offset", numbers(p.offset.iter().copied())?),
                    ("gain", numbers(p.gain.iter().copied())?),
                    ("xmin", numbers(p.xmin.iter().copied())?),
                    ("xmax", numbers(p.xmax.iter().copied())?),
                    ("keep", indices(&p.keep)?),
                    ("yoffset", num(p.yoffset)),
                    ("yscale", num(p.yscale)),
                ],
                c,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    record(
        vec![
            ("size", num(p.size as f64)),
            ("range", numbers(p.range.iter().flat_map(|&(a, b)| [a, b]))?),
            ("processFcns", strings(&p.process_fcns, c)?),
            (
                "processParams",
                numbers(p.process_params.iter().flat_map(|&(a, b)| [a, b]))?,
            ),
            ("processSettings", cell(1, settings.len(), settings, c)?),
        ],
        c,
    )
}

pub(in crate::engine) fn state(o: &Object, c: &AtomicBool) -> Result<Array> {
    let net = &o.core;
    let matrices = |v: &[Matrix]| {
        cell(
            1,
            v.len(),
            v.iter().cloned().map(Array::Numeric).collect(),
            c,
        )
    };
    let flags = |v: &[bool]| numbers(v.iter().map(|&x| f64::from(x)));
    let layers = net
        .layers
        .iter()
        .map(|l| {
            record(
                vec![
                    ("size", num(l.size as f64)),
                    ("transferFcn", string(&l.transfer_fcn, c)?),
                    ("initFcn", string(&l.init_fcn, c)?),
                    ("name", string(&l.name, c)?),
                ],
                c,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let dynamic = if let Some(d) = &o.dynamic {
        record(
            vec![
                ("inputDelays", indices(&d.input_delays)?),
                ("feedbackDelays", indices(&d.feedback_delays)?),
                ("closed", num(f64::from(d.closed))),
                ("inputSize", num(d.input_size as f64)),
                ("feedbackSize", num(d.feedback_size as f64)),
            ],
            c,
        )?
    } else {
        empty()?
    };
    let som = if let Some(s) = &o.som {
        record(
            vec![
                ("dimensions", indices(&s.dimensions)?),
                ("topology", string(&s.topology, c)?),
                ("distance", string(&s.distance, c)?),
                ("positions", Array::Numeric(s.positions.clone())),
                ("distances", Array::Numeric(s.distances.clone())),
                ("cover", num(s.cover as f64)),
                ("neighbor", num(s.neighbor)),
                ("initial", Array::Numeric(s.initial.clone())),
            ],
            c,
        )?
    } else {
        empty()?
    };
    record(
        vec![
            (
                "properties",
                record(
                    o.props
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.clone()))
                        .collect(),
                    c,
                )?,
            ),
            ("layers", cell(1, layers.len(), layers, c)?),
            ("input", port_state(&net.input, c)?),
            ("output", port_state(&net.output, c)?),
            ("biasConnect", flags(&net.bias_connect)?),
            ("inputConnect", flags(&net.input_connect)?),
            ("layerConnect", flags(&net.layer_connect)?),
            ("IW", matrices(&net.iw)?),
            ("LW", matrices(&net.lw)?),
            ("b", matrices(&net.b)?),
            ("configured", num(f64::from(net.configured))),
            ("dynamic", dynamic),
            ("som", som),
        ],
        c,
    )
}
