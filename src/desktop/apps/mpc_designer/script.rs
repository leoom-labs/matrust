//! The single controller script builder, shared by preview, export and save.
use super::*;
use matrust::syntax::{Expr, StatementKind};

pub fn checked_name(name: &str) -> Result<(), String> {
    if !matrust::files::valid_name(name) || name.starts_with("app_mpc_") {
        return Err("Use a variable name outside app_mpc_ (at most 63 characters)".into());
    }
    super::super::single_expression(name)?;
    matrust::syntax::parse(&format!("{name}=0;"))?;
    super::super::check_export_name(name)
}
/// Literal-only numeric fields use the real parser, then a small AST allowlist.
/// No function, workspace lookup, range or executable expression is admitted.
pub fn literal(text: &str) -> Result<Vec<Vec<f64>>, String> {
    if text.len() > 32_768 {
        return Err("Numeric field exceeds 32768 bytes".into());
    }
    super::super::single_expression(text)?;
    let ast = matrust::syntax::parse(&format!("app_mpc_literal=({text});"))?;
    let StatementKind::Evaluate(ev) = &ast[0].kind else {
        return Err("Use numeric literals".into());
    };
    fn scalar(e: &Expr) -> Result<f64, String> {
        match e {
            Expr::Number(x) => Ok(*x),
            Expr::Name(n) if n == "Inf" || n == "inf" => Ok(f64::INFINITY),
            Expr::Unary(neg, e) => scalar(e).map(|x| if *neg { -x } else { x }),
            _ => Err("Use numeric literals, for example [1 2]".into()),
        }
    }
    let rows = match &ev.expr {
        Expr::Matrix(rows) => rows
            .iter()
            .map(|r| r.iter().map(scalar).collect())
            .collect::<Result<Vec<Vec<_>>, _>>()?,
        e => vec![vec![scalar(e)?]],
    };
    let n = rows.first().map_or(0, Vec::len);
    if n == 0 || rows.iter().any(|r| r.len() != n) || rows.len().saturating_mul(n) > 10_000 {
        return Err("Use a nonempty rectangular numeric array with at most 10000 entries".into());
    }
    Ok(rows)
}
pub fn scalar(text: &str, what: &str) -> Result<f64, String> {
    let v = literal(text).map_err(|e| format!("{what}: {e}"))?;
    if v.len() != 1 || v[0].len() != 1 {
        return Err(format!("{what}: use one scalar"));
    }
    Ok(v[0][0])
}
fn vector(text: &str, what: &str) -> Result<Vec<f64>, String> {
    let v = literal(text).map_err(|e| format!("{what}: {e}"))?;
    if v.len() != 1 && v[0].len() != 1 {
        return Err(format!("{what}: use a scalar or vector"));
    }
    Ok(v.into_iter().flatten().collect())
}
pub(super) fn canonical(text: &str) -> Result<String, String> {
    Ok(format!(
        "[{}]",
        literal(text)?
            .iter()
            .map(|r| r.iter().map(|x| num(*x)).collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("; ")
    ))
}
pub fn num(x: f64) -> String {
    if x == f64::INFINITY {
        "Inf".into()
    } else if x == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        super::super::number(x)
    }
}
pub fn matrix(a: &Array) -> Result<String, String> {
    let Array::Numeric(m) = a else {
        return Err("Expected a real numeric controller property".into());
    };
    Ok(format!(
        "[{}]",
        (0..m.rows())
            .map(|r| (0..m.cols())
                .map(|c| num(m.data()[r + c * m.rows()]))
                .collect::<Vec<_>>()
                .join(" "))
            .collect::<Vec<_>>()
            .join("; ")
    ))
}
fn quote(s: &str) -> String {
    if s.chars().any(char::is_control) {
        return format!(
            "char([{}])",
            s.encode_utf16()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    format!("'{}'", s.replace('\'', "''"))
}
fn indices(roles: &[usize], role: usize) -> String {
    format!(
        "[{}]",
        roles
            .iter()
            .enumerate()
            .filter(|(_, r)| **r == role)
            .map(|(i, _)| (i + 1).to_string())
            .collect::<Vec<_>>()
            .join(" ")
    )
}
pub fn validate(s: &Spec) -> Result<(), String> {
    checked_name(&s.name)?;
    checked_name(&s.source)?;
    let ts = scalar(&s.ts, "Sample time")?;
    if !ts.is_finite() || ts <= 0. {
        return Err("Sample time must be finite and positive".into());
    }
    let p = scalar(&s.prediction, "Prediction horizon")?;
    if !(1. ..=10_000.).contains(&p) || p.fract() != 0. {
        return Err("Prediction horizon must be an integer from 1 to 10000".into());
    }
    let h = vector(&s.control, "Control horizon")?;
    if h.iter()
        .any(|x| !x.is_finite() || x.fract() != 0. || *x < 1. || *x > p)
        || (h.len() > 1 && h.iter().sum::<f64>() != p)
    {
        return Err("Control horizon: positive integer scalar <= prediction horizon, or blocking vector summing to it".into());
    }
    if !s.inputs.is_empty()
        && (!s.inputs.contains(&0)
            || !s.outputs.contains(&0)
            || s.inputs.len() > 128
            || s.outputs.len() > 128
            || s.inputs.iter().any(|r| *r > 2)
            || s.outputs.iter().any(|r| *r > 1))
    {
        return Err("Signal roles need at least one MV and MO, with at most 128 channels".into());
    }
    let ecr = scalar(&s.ecr, "ECR weight")?;
    if !ecr.is_finite() || ecr <= 0. {
        return Err("ECR weight must be positive and finite".into());
    }
    if s.mv.is_empty() && s.ov.is_empty() {
        return Ok(());
    } // initial engine defaults
    for (label, text, count) in [
        ("OV weights", &s.weights[0], s.ov.len()),
        ("MV weights", &s.weights[1], s.mv.len()),
        ("MVRate weights", &s.weights[2], s.mv.len()),
    ] {
        let v = literal(text).map_err(|e| format!("{label}: {e}"))?;
        if v.iter().flatten().any(|x| !x.is_finite() || *x < 0.)
            || !((v.len() == 1 && v[0].len() == 1)
                || ((v.len() == 1 || v.len() == p as usize) && v[0].len() == count))
        {
            return Err(format!(
                "{label}: finite nonnegative scalar, 1-by-{count}, or {p}-by-{count} array required"
            ));
        }
    }
    for row in s.mv.iter().chain(&s.ov) {
        for (i, text) in row.bounds.iter().enumerate() {
            let v = vector(text, BOUNDS[i])?;
            if v.len() > p as usize
                || v.iter().any(|x| {
                    x.is_nan()
                        || (i >= 4 && (!x.is_finite() || *x < 0.))
                        || (i < 4
                            && (if i.is_multiple_of(2) {
                                *x == f64::INFINITY
                            } else {
                                *x == f64::NEG_INFINITY
                            }))
                })
            {
                return Err(format!("{}: invalid bound or softness value", BOUNDS[i]));
            }
        }
        for (lo, hi) in [(0, 1), (2, 3)] {
            let a = vector(&row.bounds[lo], BOUNDS[lo])?;
            let b = vector(&row.bounds[hi], BOUNDS[hi])?;
            if (0..a.len().max(b.len())).any(|i| a[i.min(a.len() - 1)] > b[i.min(b.len() - 1)]) {
                return Err(format!("{} must be <= {}", BOUNDS[lo], BOUNDS[hi]));
            }
        }
    }
    Ok(())
}
/// The builder is deliberately independent of views and Runner bookkeeping.
pub fn build_script(specs: &[Spec]) -> Result<String, String> {
    if specs.is_empty() {
        return Err("Create a controller first".into());
    }
    if specs.len() > 16 {
        return Err("Keep at most 16 controllers".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for s in specs {
        validate(s)?;
        if !names.insert(&s.name) {
            return Err("Controller names must be unique".into());
        }
        if specs.iter().any(|other| s.name == other.source) {
            return Err("Controller names must differ from all source variables".into());
        }
    }
    let mut code = String::new();
    for s in specs {
        let n = &s.name;
        let source_plant = if s.imported {
            format!("{}.Model.Plant", s.source)
        } else {
            s.source.clone()
        };
        if s.initial && s.imported {
            code.push_str(&format!("{n} = {};\n", s.source));
            continue;
        }
        let same_roles = s.imported && s.original_roles == (s.inputs.clone(), s.outputs.clone());
        if same_roles {
            // Clear horizon-sized weights only when resizing the horizon.
            // An unchanged import must never pass through temporary tuning.
            code.push_str(&format!("{n} = {};\nif {n}.PredictionHorizon ~= {}\n{n}.ControlHorizon = 1;\n{n}.Weights.MV = 0;\n{n}.Weights.MVRate = 0.1;\n{n}.Weights.OV = 1;\nend\n{n}.Ts = {};\n{n}.PredictionHorizon = {};\n{n}.ControlHorizon = {};\n",s.source,canonical(&s.prediction)?,canonical(&s.ts)?,canonical(&s.prediction)?,canonical(&s.control)?));
        } else {
            let plant = if s.inputs.is_empty() {
                source_plant
            } else {
                format!(
                    "setmpcsignals({source_plant},'MV',{},'MD',{},'UD',{},'MO',{},'UO',{})",
                    indices(&s.inputs, 0),
                    indices(&s.inputs, 1),
                    indices(&s.inputs, 2),
                    indices(&s.outputs, 0),
                    indices(&s.outputs, 1)
                )
            };
            code.push_str(&format!(
                "{n} = mpc({plant},{},{},{});\n",
                canonical(&s.ts)?,
                canonical(&s.prediction)?,
                canonical(&s.control)?
            ));
            if s.imported {
                for p in ["Model.Nominal", "Optimizer", "Notes", "UserData"] {
                    code.push_str(&format!("{n}.{p} = {}.{p};\n", s.source));
                }
                // Carry unexposed signal settings (targets, scaling and rate
                // softness) along physical channels when their roles change.
                let old_mv = s
                    .original_roles
                    .0
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| **r == 0)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                let old_ov = (0..s.original_roles.1.len()).collect::<Vec<_>>();
                for (kind, rows, old) in [("MV", &s.mv, &old_mv), ("OV", &s.ov, &old_ov)] {
                    for (i, row) in rows.iter().enumerate() {
                        if let Some(j) = old.iter().position(|channel| *channel == row.channel) {
                            code.push_str(&format!(
                                "{n}.{kind}({}) = {}.{kind}({});\n",
                                i + 1,
                                s.source,
                                j + 1
                            ));
                        }
                    }
                }
            }
        }
        if !s.mv.is_empty() || !s.ov.is_empty() {
            for (p, w) in ["OV", "MV", "MVRate"].into_iter().zip(&s.weights) {
                code.push_str(&format!("{n}.Weights.{p} = {};\n", canonical(w)?));
            }
            code.push_str(&format!("{n}.Weights.ECR = {};\n", canonical(&s.ecr)?));
            for (kind, rows) in [("MV", &s.mv), ("OV", &s.ov)] {
                for (i, row) in rows.iter().enumerate() {
                    // Reset the opposing bounds first so valid shifted intervals
                    // never fail at an intermediate setter when editing an import.
                    for (p, value) in [("Min", "-Inf"), ("Max", "Inf")] {
                        code.push_str(&format!("{n}.{kind}({}).{p} = {value};\n", i + 1));
                    }
                    if kind == "MV" {
                        code.push_str(&format!(
                            "{n}.MV({}).RateMin=-Inf;\n{n}.MV({}).RateMax=Inf;\n",
                            i + 1,
                            i + 1
                        ));
                    }
                    for (j, p) in BOUNDS.iter().enumerate() {
                        if kind == "OV" && (j == 2 || j == 3) {
                            continue;
                        }
                        code.push_str(&format!(
                            "{n}.{kind}({}).{p} = {};\n",
                            i + 1,
                            canonical(&row.bounds[j])?
                        ));
                    }
                    code.push_str(&format!(
                        "{n}.{kind}({}).Name = {};\n{n}.{kind}({}).Units = {};\n",
                        i + 1,
                        quote(&row.name),
                        i + 1,
                        quote(&row.units)
                    ));
                }
            }
        }
    }
    if code.len() > 1024 * 1024 {
        return Err("Controller script exceeds 1 MiB".into());
    }
    Ok(code)
}

/// Canonical scenario: used unchanged by Runner, preview, saved script and export.
/// Sources are explicit workspace dependencies; no private Runner snapshots occur.
pub fn build_scenario_script(
    specs: &[Spec],
    scenario: &Scenario,
    prefix: &str,
) -> Result<String, String> {
    if !(1..=2).contains(&specs.len()) {
        return Err("Run one controller or compare two".into());
    }
    scenario.validate(specs)?;
    checked_name(prefix)?;
    let mut unique = Vec::new();
    for s in specs {
        if !unique.iter().any(|u: &Spec| u.name == s.name) {
            unique.push(s.clone());
        }
    }
    let names = scenario::output_names(prefix, specs.len());
    for name in &names {
        checked_name(name)?;
        if specs.iter().any(|s| s.name == *name || s.source == *name) {
            return Err(format!(
                "Scenario name {name} collides with a controller or source"
            ));
        }
    }
    let mut code = format!(
        "% Requires source variables: {}\n",
        unique
            .iter()
            .map(|s| s.source.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    code += &build_script(&unique)?;
    let first = &specs[0];
    let literal_or = |text: &str, default: String| -> Result<String, String> {
        Ok(if scenario::finite_literal(text)?.is_some() {
            canonical(text)?
        } else {
            default
        })
    };
    // Resolve empty signals once, so both alternatives receive identical physical inputs,
    // including when their controller nominal offsets differ.
    let r = literal_or(
        &scenario.reference,
        format!("{}.Model.Nominal.Y'", first.name),
    )?;
    let md = indices(&first.inputs, 1);
    let v = literal_or(
        &scenario.disturbance,
        if first.inputs.contains(&1) {
            format!("{}.Model.Nominal.U({md})'", first.name)
        } else {
            "[]".into()
        },
    )?;
    let x0 = literal_or(&scenario.initial, "[]".into())?;
    code += &format!(
        "{prefix}_r = {r};\n{prefix}_v = {v};\n{prefix}_x0 = {x0};\n{prefix}_options = mpcsimopt;\n{prefix}_options.PlantInitialState = {prefix}_x0;\n{prefix}_options.RefLookAhead = '{}';\n{prefix}_options.MDLookAhead = '{}';\n",
        if scenario.reference_preview {
            "on"
        } else {
            "off"
        },
        if scenario.disturbance_preview {
            "on"
        } else {
            "off"
        }
    );
    for (i, s) in specs.iter().enumerate() {
        let suffix = if i == 0 {
            String::new()
        } else {
            (i + 1).to_string()
        };
        code += &format!(
            "[{prefix}_y{suffix},{prefix}_t{suffix},{prefix}_u{suffix}] = sim({},{},{prefix}_r,{prefix}_v,{prefix}_options);\n",
            s.name,
            scenario.count()?
        );
    }
    if code.len() > 1024 * 1024 {
        return Err("Scenario script exceeds 1 MiB".into());
    }
    Ok(code)
}
