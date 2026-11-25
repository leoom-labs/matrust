//! Display formatting for `mpc` controller objects.

use super::object::{ControlHorizon, MpcObject};

fn fmt_vec(data: &[f64]) -> String {
    if data.len() == 1 {
        format!("{}", data[0])
    } else {
        let items: Vec<String> = data.iter().map(|x| format!("{x}")).collect();
        format!("[{}]", items.join(" "))
    }
}

pub(in crate::engine) fn body(m: &MpcObject) -> String {
    let ch_str = match &m.control_horizon {
        ControlHorizon::Scalar(k) => format!("{k}"),
        ControlHorizon::Blocking(v) => {
            let parts: Vec<String> = v.iter().map(|x| x.to_string()).collect();
            format!("[{}]", parts.join(" "))
        }
    };

    let mv_bounded = m
        .mv
        .iter()
        .filter(|v| {
            v.min.data().iter().any(|x| x.is_finite()) || v.max.data().iter().any(|x| x.is_finite())
        })
        .count();
    let mv_rate_bounded =
        m.mv.iter()
            .filter(|v| {
                v.rate_min.data().iter().any(|x| x.is_finite())
                    || v.rate_max.data().iter().any(|x| x.is_finite())
            })
            .count();
    let ov_bounded = m
        .ov
        .iter()
        .filter(|v| {
            v.min.data().iter().any(|x| x.is_finite()) || v.max.data().iter().any(|x| x.is_finite())
        })
        .count();

    format!(
        "\
  mpc controller with properties:

                       Ts: {}
        PredictionHorizon: {}
           ControlHorizon: {}
                  Weights: [MV: {}, MVRate: {}, OV: {}, ECR: {}]
     ManipulatedVariables: [1x{} struct] ({} bounded, {} rate-bounded)
          OutputVariables: [1x{} struct] ({} bounded)
     DisturbanceVariables: [1x{} struct]
                    Model: [Plant: {} states, {} inputs ({} MV, {} MD, {} UD), {} outputs ({} MO, {} UO)]
                Optimizer: [Algorithm: '{}', MaxIterations: {}, ConstraintTolerance: {}]
                    Notes: {}
                 UserData: {}
                  History: '{}'
",
        m.ts,
        m.prediction_horizon,
        ch_str,
        fmt_vec(m.weights.mv.data()),
        fmt_vec(m.weights.mv_rate.data()),
        fmt_vec(m.weights.ov.data()),
        m.weights.ecr,
        m.n_mv(),
        mv_bounded,
        mv_rate_bounded,
        m.n_ov(),
        ov_bounded,
        m.n_dv(),
        m.nx(),
        m.nu(),
        m.mv_idx.len(),
        m.md_idx.len(),
        m.ud_idx.len(),
        m.ny(),
        m.mo_idx.len(),
        m.uo_idx.len(),
        m.optimizer.algorithm,
        m.optimizer.max_iterations,
        m.optimizer.constraint_tolerance,
        if m.notes.is_empty() { "{}" } else { "{...}" },
        if m.user_data.is_empty() {
            "[]"
        } else {
            "[data]"
        },
        m.history,
    )
}
