use super::checkpoint as check_cancel;
use super::xml::Element;
use crate::{
    Result,
    simulation::{Block, Model, Node, ProductInput, SwitchCriteria},
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};
type Properties = BTreeMap<String, String>;

fn required<'a>(p: &'a Properties, name: &str) -> Result<&'a str> {
    p.get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("missing saved {name}; no implicit default assumed"))
}
fn choice(p: &Properties, name: &str, allowed: &[&str]) -> Result<()> {
    let value = required(p, name)?;
    if !allowed.contains(&value) {
        return Err(format!("unsupported {name}={value}"));
    }
    Ok(())
}
fn optional_choice(p: &Properties, name: &str, allowed: &[&str]) -> Result<()> {
    if p.contains_key(name) {
        choice(p, name, allowed)?;
    }
    Ok(())
}
fn number(text: &str) -> Result<f64> {
    let text = text.trim();
    if text.is_empty()
        || text.len() > 128
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
    {
        return Err(format!("numeric scalar literal required: {text}"));
    }
    let n: f64 = text.parse().map_err(|_| "invalid numeric literal")?;
    if !n.is_finite() {
        return Err("finite real scalar literal required".into());
    }
    Ok(n)
}
fn scalar(p: &Properties, name: &str) -> Result<f64> {
    number(required(p, name)?)
}

/// Exact decimal-rational admission; no rounding an unsupported stop onto the grid.
fn decimal(text: &str) -> Result<(u128, u128)> {
    let text = text.trim().trim_start_matches('+');
    let (mantissa, exp) = match text.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().map_err(|_| "invalid time exponent")?),
        None => (text, 0),
    };
    if !(-18..=18).contains(&exp) {
        return Err("time exponent outside supported exact-decimal range".into());
    }
    let mut value = 0u128;
    let mut places = 0u32;
    let mut dot = false;
    let mut digits = 0;
    for b in mantissa.bytes() {
        if b == b'.' && !dot {
            dot = true;
            continue;
        }
        if !b.is_ascii_digit() {
            return Err("nonnegative decimal time required".into());
        }
        digits += 1;
        if digits > 18 {
            return Err("time literal exceeds 18 significant storage digits".into());
        }
        value = value * 10 + u128::from(b - b'0');
        if dot {
            places += 1;
        }
    }
    if digits == 0 {
        return Err("empty time literal".into());
    }
    let scale = places as i32 - exp;
    if scale >= 0 {
        Ok((value, 10u128.pow(scale as u32)))
    } else {
        Ok((
            value
                .checked_mul(10u128.pow((-scale) as u32))
                .ok_or("time overflow")?,
            1,
        ))
    }
}
fn timing(config: &Element) -> Result<(f64, usize)> {
    if config.name != "ConfigSet" {
        return Err("SLX: unsupported configuration root".into());
    }
    let names = [
        "Solver",
        "FixedStep",
        "StartTime",
        "StopTime",
        "SolverMode",
        "ConcurrentTasks",
        "LoadExternalInput",
        "LoadInitialState",
    ];
    fn visit(n: &Element, names: &[&str], p: &mut Properties) -> Result<()> {
        if n.name == "P"
            && let Some(key) = n.attr("Name")
            && names.contains(&key)
            && p.insert(key.to_owned(), n.text.trim().to_owned()).is_some()
        {
            return Err(format!("SLX: duplicate configuration {key}"));
        }
        for child in &n.children {
            visit(child, names, p)?;
        }
        Ok(())
    }
    let mut p = Properties::new();
    visit(config, &names, &mut p)?;
    choice(&p, "Solver", &["FixedStepDiscrete"])?;
    choice(&p, "SolverMode", &["SingleTasking", "Auto"])?;
    for name in ["ConcurrentTasks", "LoadExternalInput", "LoadInitialState"] {
        choice(&p, name, &["off"])?;
    }
    let start = decimal(required(&p, "StartTime")?)?;
    if start.0 != 0 {
        return Err("SLX: only zero start time is supported".into());
    }
    let fixed = required(&p, "FixedStep")?;
    let dt = number(fixed)?;
    if dt <= 0. {
        return Err("SLX: positive explicit fixed step required".into());
    }
    let step = decimal(fixed)?;
    let stop = decimal(required(&p, "StopTime")?)?;
    let numerator = stop
        .0
        .checked_mul(step.1)
        .ok_or("SLX: time ratio overflow")?;
    let denominator = stop
        .1
        .checked_mul(step.0)
        .ok_or("SLX: time ratio overflow")?;
    if denominator == 0 || !numerator.is_multiple_of(denominator) {
        return Err("SLX: stop time must lie exactly on the fixed-step decimal grid".into());
    }
    let intervals = numerator / denominator;
    if intervals >= 1_000_000 {
        return Err("SLX: inclusive time output exceeds one million samples".into());
    }
    Ok((dt, intervals as usize + 1))
}
fn inert_callbacks(n: &Element) -> Result<()> {
    if n.name == "P"
        && let Some(name) = n.attr("Name")
        && (name.ends_with("Fcn") || name == "MaskInitialization")
        && !n.text.trim().is_empty()
    {
        return Err(format!(
            "SLX: callback {name} is not executed by read-only import"
        ));
    }
    for c in &n.children {
        inert_callbacks(c)?;
    }
    Ok(())
}
fn vector(text: &str) -> Result<Vec<f64>> {
    let text = text.trim();
    let inner = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or("bracketed literal vector required")?;
    let mut out = Vec::new();
    for item in inner
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
    {
        if out.len() >= 32 {
            return Err("literal vector exceeds 32 entries".into());
        }
        out.push(number(item)?);
    }
    Ok(out)
}
fn block(kind: &str, p: &Properties, dt: f64) -> Result<(Block, usize)> {
    // Reject unexplained parameters, rather than assuming unknown execution controls inert.
    let common = [
        "Position",
        "Ports",
        "ZOrder",
        "ForegroundColor",
        "BackgroundColor",
        "DropShadow",
        "NamePlacement",
        "FontName",
        "FontSize",
        "FontWeight",
        "FontAngle",
        "ShowName",
        "BlockRotation",
        "BlockMirror",
        "SampleTime",
        "OutDataTypeStr",
        "OutMin",
        "OutMax",
        "LockScale",
        "RndMeth",
        "SaturateOnIntegerOverflow",
        "Description",
        "Tag",
    ];
    let specific: &[&str] = match kind {
        "Constant" => &[
            "Value",
            "VectorParams1D",
            "SamplingMode",
            "FramePeriod",
            "PreserveConstantTs",
        ],
        "Gain" => &[
            "Gain",
            "Multiplication",
            "ParamDataTypeStr",
            "ParamMin",
            "ParamMax",
        ],
        "Sum" => &[
            "Inputs",
            "IconShape",
            "CollapseMode",
            "CollapseDim",
            "InputSameDT",
            "AccumDataTypeStr",
        ],
        "UnitDelay" => &[
            "InitialCondition",
            "InputProcessing",
            "StateMustResolveToSignalObject",
            "CodeGenStateStorageClass",
            "HasFrameUpgradeWarning",
            "StateName",
        ],
        "Abs" | "UnaryMinus" => &[],
        "Bias" => &["Bias"],
        "Saturate" => &["UpperLimit", "LowerLimit", "LinearizeAsGain", "ZeroCross"],
        "Product" => &[
            "Inputs",
            "Multiplication",
            "InputSameDT",
            "AccumDataTypeStr",
            "CollapseMode",
            "CollapseDim",
        ],
        "Switch" => &[
            "Criteria",
            "Threshold",
            "InputSameDT",
            "ZeroCross",
            "AllowDiffInputSizes",
        ],
        "Scope" => &[
            "NumInputPorts",
            "Floating",
            "DefaultConfigurationName",
            "ScopeSpecificationString",
            "IOType",
        ],
        _ => return Err(format!("unsupported block type {kind}")),
    };
    for key in p.keys() {
        if !common.contains(&key.as_str()) && !specific.contains(&key.as_str()) {
            return Err(format!("unsupported saved {kind} parameter {key}"));
        }
    }
    choice(p, "BlockRotation", &["0"])?;
    choice(p, "BlockMirror", &["off"])?;
    optional_choice(p, "OutMin", &["[]"])?;
    optional_choice(p, "OutMax", &["[]"])?;
    optional_choice(p, "ParamMin", &["[]"])?;
    optional_choice(p, "ParamMax", &["[]"])?;
    optional_choice(
        p,
        "InputProcessing",
        &["Inherited", "Elements as channels (sample based)"],
    )?;
    optional_choice(p, "SamplingMode", &["Sample based"])?;
    optional_choice(p, "StateMustResolveToSignalObject", &["off"])?;
    optional_choice(p, "StateName", &[""])?;
    optional_choice(
        p,
        "ParamDataTypeStr",
        &[
            "double",
            "Inherit: Inherit via internal rule",
            "Inherit: Same as input",
        ],
    )?;
    optional_choice(
        p,
        "AccumDataTypeStr",
        &[
            "double",
            "Inherit: Inherit via internal rule",
            "Inherit: Same as first input",
        ],
    )?;
    if kind != "Scope" {
        let st = required(p, "SampleTime")?;
        if st != "-1"
            && !(kind == "Constant" && st.eq_ignore_ascii_case("inf"))
            && number(st)? != dt
        {
            return Err(format!(
                "single rate with zero offset required; SampleTime={st}"
            ));
        }
        if kind != "UnitDelay" {
            let allowed = if kind == "Constant" {
                vec!["double", "Inherit: Inherit from 'Constant value'"]
            } else {
                vec![
                    "double",
                    "Inherit: Same as input",
                    "Inherit: Same as first input",
                    "Inherit: Inherit via internal rule",
                ]
            };
            choice(p, "OutDataTypeStr", &allowed)?;
        } else {
            optional_choice(p, "OutDataTypeStr", &["double", "Inherit: Same as input"])?;
        }
    }
    let result = match kind {
        "Constant" => (
            Block::Constant {
                value: scalar(p, "Value")?,
            },
            0,
        ),
        "Gain" => {
            choice(
                p,
                "Multiplication",
                &["Element-wise(K.*u)", "Matrix(K*u)", "Matrix(u*K)"],
            )?;
            (
                Block::Gain {
                    gain: scalar(p, "Gain")?,
                },
                1,
            )
        }
        "Sum" => {
            let s = required(p, "Inputs")?;
            let count = if s.bytes().all(|b| b == b'+' || b == b'|') {
                s.bytes().filter(|&b| b == b'+').count()
            } else {
                s.parse()
                    .map_err(|_| "Sum supports positive input signs only")?
            };
            if !(2..=32).contains(&count) {
                return Err("Sum requires 2..32 scalar positive inputs".into());
            }
            (Block::Sum, count)
        }
        "UnitDelay" => (
            Block::UnitDelay {
                initial: scalar(p, "InitialCondition")?,
            },
            1,
        ),
        "Abs" => (Block::Abs, 1),
        "UnaryMinus" => (Block::UnaryMinus, 1),
        "Bias" => (
            Block::Bias {
                bias: scalar(p, "Bias")?,
            },
            1,
        ),
        "Saturate" => (
            Block::Saturation {
                lower: scalar(p, "LowerLimit")?,
                upper: scalar(p, "UpperLimit")?,
            },
            1,
        ),
        "Product" => {
            choice(p, "Multiplication", &["Element-wise(.*)"])?;
            let s = required(p, "Inputs")?;
            let operations = if s.bytes().all(|b| b == b'*' || b == b'/') {
                s.bytes()
                    .map(|b| {
                        if b == b'*' {
                            ProductInput::Multiply
                        } else {
                            ProductInput::Divide
                        }
                    })
                    .collect::<Vec<_>>()
            } else {
                let count: usize = s.parse().map_err(|_| "invalid Product inputs")?;
                if !(1..=32).contains(&count) {
                    return Err("Product input limit".into());
                }
                vec![ProductInput::Multiply; count]
            };
            if !(1..=32).contains(&operations.len()) {
                return Err("Product input limit".into());
            }
            let n = operations.len();
            (Block::Product { operations }, n)
        }
        "Switch" => {
            let criteria = match required(p, "Criteria")? {
                "u2 > Threshold" => SwitchCriteria::Greater,
                "u2 >= Threshold" => SwitchCriteria::GreaterEqual,
                "u2 ~= 0" => SwitchCriteria::Nonzero,
                _ => return Err("unsupported Switch criteria".into()),
            };
            (
                Block::Switch {
                    criteria,
                    threshold: scalar(p, "Threshold")?,
                },
                3,
            )
        }
        "Scope" => {
            choice(p, "NumInputPorts", &["1"])?;
            choice(p, "Floating", &["off"])?;
            optional_choice(p, "IOType", &[""])?;
            (Block::Scope, 1)
        }
        _ => unreachable!(),
    };
    if let Some(ports) = p.get("Ports") {
        let mut ports = vector(ports)?;
        let outputs = usize::from(kind != "Scope");
        let mut expected = vec![result.1 as f64, outputs as f64];
        // The reference's ScopeA saves [1], omitting its zero output count.
        while ports.last() == Some(&0.) {
            ports.pop();
        }
        while expected.last() == Some(&0.) {
            expected.pop();
        }
        if ports != expected {
            return Err("saved ports disagree with supported scalar block ports".into());
        }
    }
    Ok(result)
}

/// Prove only the documented equal-input discrete-rate propagation rule.
/// Explicit block rates seed the proof; the solver step alone is not a seed.
/// Each edge is visited once. Constant/unknown inputs do not invent a clock.
fn inherited_state_rates(
    nodes: &[Node],
    mut discrete: Vec<bool>,
    cancel: &AtomicBool,
) -> Result<()> {
    let mut consumers = vec![Vec::new(); nodes.len()];
    let mut remaining: Vec<usize> = nodes.iter().map(|n| n.inputs.len()).collect();
    let mut ready = Vec::with_capacity(nodes.len());
    for (i, node) in nodes.iter().enumerate() {
        check_cancel(cancel)?;
        if discrete[i] {
            ready.push(i);
        }
        for &source in &node.inputs {
            consumers[source].push(i);
        }
    }
    let mut cursor = 0;
    while cursor < ready.len() {
        check_cancel(cancel)?;
        let source = ready[cursor];
        cursor += 1;
        for &target in &consumers[source] {
            check_cancel(cancel)?;
            if discrete[target] {
                continue;
            }
            remaining[target] -= 1;
            if remaining[target] == 0 {
                discrete[target] = true;
                ready.push(target);
            }
        }
    }
    for (i, node) in nodes.iter().enumerate() {
        check_cancel(cancel)?;
        if matches!(node.block, Block::UnitDelay { .. }) && !discrete[i] {
            return Err(format!(
                "SLX block {}: unsupported unresolved inherited UnitDelay sample time; saved explicit upstream rates do not establish the fixed discrete rate. Specify an explicit block SampleTime or provide a supported discrete-rate path; the solver step is not substituted for Simulink propagation heuristics",
                node.name
            ));
        }
    }
    Ok(())
}

pub(super) fn decode(diagram: &Element, config: &Element, cancel: &AtomicBool) -> Result<Model> {
    check_cancel(cancel)?;
    let (dt, steps) = timing(config).map_err(|e| format!("SLX configuration: {e}"))?;
    if diagram.name != "ModelInformation" {
        return Err("SLX: unsupported diagram root".into());
    }
    let model = diagram.one("Model")?;
    inert_callbacks(model)?;
    let props = model.properties()?;
    optional_choice(&props, "DataTypeOverride", &["UseLocalSettings"])?;
    optional_choice(&props, "DisableAllScopes", &["off"])?;
    let common = model.one("BlockDefaults")?.properties()?;
    let mut defaults = BTreeMap::new();
    for d in &model.one("BlockParameterDefaults")?.children {
        if d.name != "Block" {
            return Err("SLX: unsupported parameter-default element".into());
        }
        let kind = d.required_attr("BlockType")?;
        if defaults.insert(kind, d.properties()?).is_some() {
            return Err(format!("SLX: duplicate {kind} defaults"));
        }
    }
    let system = model.one("System")?;
    let mut ids = BTreeMap::new();
    let mut nodes = Vec::new();
    let mut positions = Vec::new();
    let mut discrete = Vec::new();
    for b in system.children.iter().filter(|c| c.name == "Block") {
        check_cancel(cancel)?;
        if nodes.len() >= 512 {
            return Err("SLX: model exceeds 512 blocks".into());
        }
        let sid = b.required_attr("SID")?;
        let name = b.required_attr("Name")?;
        let kind = b.required_attr("BlockType")?;
        if sid.len() > 32
            || sid.is_empty()
            || !sid.bytes().all(|b| b.is_ascii_digit())
            || ids.insert(sid, nodes.len()).is_some()
        {
            return Err("SLX: invalid/duplicate block SID".into());
        }
        if name.is_empty() || name.len() > 128 {
            return Err("SLX: block name exceeds native name limit".into());
        }
        for child in b.children.iter().filter(|n| n.name != "P") {
            if child.name != "Port" || child.children.iter().any(|n| n.name != "P") {
                return Err(format!(
                    "SLX block {sid} ({name}): nested block/mask metadata requires unsupported semantics"
                ));
            }
            let port = child.properties()?;
            if required(&port, "PortNumber")? != "1" {
                return Err(format!("SLX block {sid}: unsupported signal metadata port"));
            }
            for key in port.keys() {
                if ![
                    "PortNumber",
                    "Name",
                    "RTWStorageClass",
                    "DataLoggingNameMode",
                    "DataLoggingName",
                    "DataLogging",
                    "DataLoggingDecimateData",
                    "DataLoggingDecimation",
                    "DataLoggingLimitDataPoints",
                    "DataLoggingMaxPoints",
                ]
                .contains(&key.as_str())
                {
                    return Err(format!(
                        "SLX block {sid}: unsupported signal metadata {key}"
                    ));
                }
            }
            optional_choice(&port, "RTWStorageClass", &["Auto"])?;
        }
        let mut p = common.clone();
        if let Some(d) = defaults.get(kind) {
            p.extend(d.clone());
        }
        p.extend(b.properties()?);
        let (value, arity) =
            block(kind, &p, dt).map_err(|e| format!("SLX block {sid} ({name}): {e}"))?;
        // block() has already admitted only -1, Constant inf, or explicit dt.
        discrete.push(
            kind != "Scope"
                && p.get("SampleTime")
                    .is_some_and(|s| s != "-1" && !s.eq_ignore_ascii_case("inf")),
        );
        let pos = vector(required(&p, "Position")?)?;
        if pos.len() != 4
            || pos[2] < pos[0]
            || pos[3] < pos[1]
            || pos.iter().any(|x| x.abs() > 1_000_000.)
        {
            return Err(format!("SLX block {sid}: unsupported Position"));
        }
        positions.push([pos[0], pos[1]]);
        nodes.push(Node {
            name: name.into(),
            block: value,
            inputs: vec![usize::MAX; arity],
        });
    }
    for n in &system.children {
        if !matches!(n.name.as_str(), "P" | "Block" | "Line" | "Annotation") {
            return Err(format!("SLX: unsupported System child {}", n.name));
        }
    }
    fn endpoint<'a>(s: &'a str, direction: &str) -> Result<(&'a str, usize)> {
        let (id, port) = s.split_once('#').ok_or("SLX: invalid endpoint")?;
        let number = port
            .strip_prefix(direction)
            .ok_or("SLX: unsupported endpoint direction")?;
        let port: usize = number.parse().map_err(|_| "SLX: invalid port number")?;
        if !(1..=32).contains(&port) {
            return Err("SLX: port number outside 1..32".into());
        }
        Ok((id, port))
    }
    fn wire(
        n: &Element,
        inherited: Option<usize>,
        ids: &BTreeMap<&str, usize>,
        nodes: &mut [Node],
        edges: &mut usize,
        cancel: &AtomicBool,
    ) -> Result<()> {
        check_cancel(cancel)?;
        let p = n.properties()?;
        let source = if let Some(src) = p.get("Src") {
            if inherited.is_some() {
                return Err("SLX: Branch overrides its source".into());
            }
            let (id, port) = endpoint(src, "out:")?;
            let i = *ids.get(id).ok_or("SLX: unknown source SID")?;
            if port != 1 || matches!(nodes[i].block, Block::Scope) {
                return Err("SLX: invalid scalar source port".into());
            }
            i
        } else {
            inherited.ok_or("SLX: line has no source")?
        };
        if let Some(dst) = p.get("Dst") {
            *edges += 1;
            if *edges > 16384 {
                return Err("SLX: edge limit".into());
            }
            let (id, port) = endpoint(dst, "in:")?;
            let i = *ids.get(id).ok_or("SLX: unknown destination SID")?;
            let slot = nodes[i]
                .inputs
                .get_mut(port - 1)
                .ok_or("SLX: destination port does not exist")?;
            if *slot != usize::MAX {
                return Err("SLX: multiple drivers for destination port".into());
            }
            *slot = source;
        }
        for key in p.keys() {
            if !["Src", "Dst", "Name", "ZOrder", "Points", "Labels"].contains(&key.as_str()) {
                return Err(format!("SLX: unsupported line parameter {key}"));
            }
        }
        let mut branches = 0;
        for c in &n.children {
            if c.name == "Branch" {
                branches += 1;
                wire(c, Some(source), ids, nodes, edges, cancel)?;
            } else if c.name != "P" {
                return Err("SLX: unsupported line structure".into());
            }
        }
        if !p.contains_key("Dst") && branches == 0 {
            return Err("SLX: dangling line".into());
        }
        Ok(())
    }
    let mut edges = 0;
    for line in system.children.iter().filter(|n| n.name == "Line") {
        wire(line, None, &ids, &mut nodes, &mut edges, cancel)?;
    }
    if nodes.iter().any(|n| n.inputs.contains(&usize::MAX)) {
        return Err("SLX: unwired required input port".into());
    }
    inherited_state_rates(&nodes, discrete, cancel)?;
    let minx = positions.iter().map(|p| p[0]).fold(0., f64::min);
    let miny = positions.iter().map(|p| p[1]).fold(0., f64::min);
    let positions = positions
        .into_iter()
        .map(|p| {
            let x = p[0] - minx;
            let y = p[1] - miny;
            if x > 1_000_000. || y > 1_000_000. {
                return Err("SLX: translated native layout exceeds bounds".into());
            }
            Ok([x as f32, y as f32])
        })
        .collect::<Result<Vec<_>>>()?;
    let result = Model {
        version: 2,
        sample_time: dt,
        steps,
        nodes,
        positions,
    };
    check_cancel(cancel)?;
    result.validate()?;
    check_cancel(cancel)?;
    Ok(result)
}
