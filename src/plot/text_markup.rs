//! Bounded, forgiving subset of the graphics TeX interpreter.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextSource {
    Cell(Vec<String>),
    Strings {
        rows: usize,
        cols: usize,
        values: Vec<Option<String>>,
    },
    Character {
        rows: usize,
        cols: usize,
        units: Vec<u16>,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Interpreter {
    #[default]
    Tex,
    None,
    Latex,
}
impl Interpreter {
    pub fn parse(s: &str) -> crate::Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "tex" => Ok(Self::Tex),
            "none" => Ok(Self::None),
            "latex" => Ok(Self::Latex),
            _ => Err("Interpreter must be tex, none or latex".into()),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tex => "tex",
            Self::None => "none",
            Self::Latex => "latex",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub scale: f32,
    pub shift: f32,
    pub bold: bool,
    pub italic: bool,
}
fn symbol(s: &str) -> Option<&'static str> {
    Some(match s {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "omicron" => "ο",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Alpha" => "Α",
        "Beta" => "Β",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Epsilon" => "Ε",
        "Zeta" => "Ζ",
        "Eta" => "Η",
        "Theta" => "Θ",
        "Iota" => "Ι",
        "Kappa" => "Κ",
        "Lambda" => "Λ",
        "Mu" => "Μ",
        "Nu" => "Ν",
        "Xi" => "Ξ",
        "Omicron" => "Ο",
        "Pi" => "Π",
        "Rho" => "Ρ",
        "Sigma" => "Σ",
        "Tau" => "Τ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Chi" => "Χ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "pm" => "±",
        "times" => "×",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "infty" => "∞",
        "circ" => "∘",
        "deg" => "°",
        "partial" => "∂",
        "nabla" => "∇",
        "cdot" => "·",
        "rightarrow" => "→",
        "leftarrow" => "←",
        "aleph" => "ℵ",
        "wp" => "℘",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "forall" => "∀",
        "exists" => "∃",
        "ni" => "∋",
        "cong" => "≅",
        "sim" => "∼",
        "equiv" => "≡",
        "propto" => "∝",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "cap" => "∩",
        "cup" => "∪",
        "vee" => "∨",
        "wedge" => "∧",
        "neg" => "¬",
        "int" => "∫",
        "sum" => "∑",
        "prod" => "∏",
        "sqrt" => "√",
        "otimes" => "⊗",
        "oplus" => "⊕",
        "oslash" => "⊘",
        "perp" => "⊥",
        "angle" => "∠",
        "prime" => "′",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "leftrightarrow" => "↔",
        "bullet" => "•",
        "ldots" => "…",
        "div" => "÷",
        "neq" => "≠",
        "approx" => "≈",
        "in" => "∈",
        _ => return None,
    })
}
/// Fraction size and displacement relative to the parent font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FractionScale {
    factor: f32,
    pub(super) shift: f32,
}

impl std::ops::Mul<FractionScale> for f32 {
    type Output = f32;

    fn mul(self, scale: FractionScale) -> Self::Output {
        self * scale.factor
    }
}

/// A parsed markup node; fractions and square roots are preserved for stacked/radical layout.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Run(Run),
    Fraction {
        num: Vec<Node>,
        den: Vec<Node>,
        scale: FractionScale,
        bold: bool,
        italic: bool,
    },
    Sqrt {
        body: Vec<Node>,
        scale: f32,
        bold: bool,
        italic: bool,
    },
}

fn latex_symbol(s: &str) -> Option<&'static str> {
    match s {
        "varepsilon" => Some("ε"),
        "varphi" => Some("φ"),
        "varrho" => Some("ρ"),
        "varsigma" => Some("ς"),
        _ => symbol(s),
    }
}

fn read_latex_command(chars: &[char], i: &mut usize) -> Option<String> {
    if *i >= chars.len() {
        return None;
    }
    let start = *i;
    if chars[*i].is_ascii_alphabetic() {
        while *i < chars.len() && chars[*i].is_ascii_alphabetic() {
            *i += 1;
        }
    } else {
        *i += 1;
    }
    Some(chars[start..*i].iter().collect())
}

fn read_latex_delimiter(chars: &[char], i: &mut usize) -> Option<&'static str> {
    while *i < chars.len() && chars[*i].is_ascii_whitespace() {
        *i += 1;
    }
    if *i >= chars.len() {
        return None;
    }
    let c = chars[*i];
    *i += 1;
    match c {
        '(' => Some("("),
        ')' => Some(")"),
        '[' => Some("["),
        ']' => Some("]"),
        '|' => Some("|"),
        '.' => Some(""),
        '\\' if *i < chars.len() => {
            let next = chars[*i];
            *i += 1;
            match next {
                '{' => Some("{"),
                '}' => Some("}"),
                '|' => Some("‖"),
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_latex_arg(chars: &[char], i: &mut usize, style: Run, depth: usize) -> Option<Vec<Node>> {
    if depth >= 32 {
        return None;
    }
    while *i < chars.len() && chars[*i].is_ascii_whitespace() {
        *i += 1;
    }
    if *i >= chars.len() {
        return None;
    }
    if chars[*i] == '{' {
        *i += 1;
        let mut out = Vec::new();
        parse_latex_seq(chars, i, &mut out, style, true, depth + 1)?;
        return Some(merge_nodes(out));
    }
    if chars[*i] == '\\' {
        *i += 1;
        let cmd = read_latex_command(chars, i)?;
        let mut out = Vec::new();
        apply_latex_command(&cmd, chars, i, &mut out, style, depth + 1)?;
        return Some(out);
    }
    let c = chars[*i];
    if matches!(c, '}' | '_' | '^') {
        return None;
    }
    *i += 1;
    let mut run = style;
    run.text = c.to_string();
    Some(vec![Node::Run(run)])
}

fn apply_latex_command(
    cmd: &str,
    chars: &[char],
    i: &mut usize,
    out: &mut Vec<Node>,
    style: Run,
    depth: usize,
) -> Option<()> {
    match cmd {
        "frac" => {
            let mut sub_style = style.clone();
            sub_style.scale *= 0.7;
            sub_style.shift = 0.;
            let num = parse_latex_arg(chars, i, sub_style.clone(), depth + 1)?;
            let den = parse_latex_arg(chars, i, sub_style, depth + 1)?;
            out.push(Node::Fraction {
                num,
                den,
                scale: FractionScale {
                    factor: style.scale,
                    shift: style.shift,
                },
                bold: style.bold,
                italic: style.italic,
            });
        }
        "sqrt" => {
            let body = parse_latex_arg(chars, i, style.clone(), depth + 1)?;
            out.push(Node::Sqrt {
                body,
                scale: style.scale,
                bold: style.bold,
                italic: style.italic,
            });
        }
        "mathrm" => {
            let mut rm = style;
            rm.bold = false;
            rm.italic = false;
            out.extend(parse_latex_arg(chars, i, rm, depth + 1)?);
        }
        "mathbf" => {
            let mut bf = style;
            bf.bold = true;
            bf.italic = false;
            out.extend(parse_latex_arg(chars, i, bf, depth + 1)?);
        }
        "left" | "right" => {
            let delim = read_latex_delimiter(chars, i)?;
            if !delim.is_empty() {
                let mut run = style;
                run.text = delim.to_owned();
                out.push(Node::Run(run));
            }
        }
        "{" | "}" | "\\" | "_" | " " | "%" => {
            let mut run = style;
            run.text = cmd.to_owned();
            out.push(Node::Run(run));
        }
        _ => {
            let sym = latex_symbol(cmd)?;
            let mut run = style;
            run.text = sym.to_owned();
            out.push(Node::Run(run));
        }
    }
    Some(())
}

fn parse_latex_seq(
    chars: &[char],
    i: &mut usize,
    out: &mut Vec<Node>,
    style: Run,
    group: bool,
    depth: usize,
) -> Option<()> {
    if depth >= 32 {
        return None;
    }
    while *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        if c == '}' {
            return group.then_some(());
        }
        if c == '{' {
            parse_latex_seq(chars, i, out, style.clone(), true, depth + 1)?;
            continue;
        }
        if c == '_' || c == '^' {
            let mut sub = style.clone();
            sub.scale *= 0.7;
            sub.shift += if c == '^' { 0.5 } else { -0.25 };
            let nodes = parse_latex_arg(chars, i, sub, depth + 1)?;
            out.extend(nodes);
            continue;
        }
        if c == '\\' {
            let cmd = read_latex_command(chars, i)?;
            apply_latex_command(&cmd, chars, i, out, style.clone(), depth + 1)?;
            continue;
        }
        let mut run = style.clone();
        run.text = c.to_string();
        out.push(Node::Run(run));
    }
    if group { None } else { Some(()) }
}

fn merge_nodes(nodes: Vec<Node>) -> Vec<Node> {
    let mut merged: Vec<Node> = Vec::new();
    for node in nodes {
        match (merged.last_mut(), node) {
            (Some(Node::Run(last)), Node::Run(run))
                if last.scale == run.scale
                    && last.shift == run.shift
                    && last.bold == run.bold
                    && last.italic == run.italic =>
            {
                last.text.push_str(&run.text);
            }
            (_, node) => merged.push(node),
        }
    }
    merged
}

fn parse_latex_nodes(source: &str) -> Vec<Node> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let base = || Run {
        text: String::new(),
        scale: 1.,
        shift: 0.,
        bold: false,
        italic: false,
    };
    while i < chars.len() {
        if chars[i] != '$' {
            plain.push(chars[i]);
            i += 1;
            continue;
        }
        let display = i + 1 < chars.len() && chars[i + 1] == '$';
        let delim_len = if display { 2 } else { 1 };
        let math_start = i + delim_len;
        let mut j = math_start;
        let mut found = None;
        while j < chars.len() {
            if chars[j] == '\\' && j + 1 < chars.len() {
                j += 2;
                continue;
            }
            if chars[j] == '$' {
                if display {
                    if j + 1 < chars.len() && chars[j + 1] == '$' {
                        found = Some(j);
                        break;
                    }
                } else {
                    found = Some(j);
                    break;
                }
            }
            j += 1;
        }
        let Some(math_end) = found else {
            plain.extend(&chars[i..]);
            break;
        };
        let mut parsed = Vec::new();
        let mut k = 0;
        let ok = math_end > math_start
            && parse_latex_seq(
                &chars[math_start..math_end],
                &mut k,
                &mut parsed,
                base(),
                false,
                0,
            )
            .is_some();
        if ok {
            if !plain.is_empty() {
                out.push(Node::Run(Run {
                    text: std::mem::take(&mut plain),
                    ..base()
                }));
            }
            out.extend(parsed);
        } else {
            plain.extend(&chars[i..math_end + delim_len]);
        }
        i = math_end + delim_len;
    }
    if !plain.is_empty() || out.is_empty() {
        out.push(Node::Run(Run {
            text: plain,
            ..base()
        }));
    }
    merge_nodes(out)
}

fn flatten_nodes(nodes: &[Node], out: &mut Vec<Run>, depth: usize) {
    if depth >= 32 {
        return;
    }
    for node in nodes {
        match node {
            Node::Run(run) => out.push(run.clone()),
            Node::Fraction {
                num,
                den,
                scale,
                bold,
                italic,
            } => {
                let first = out.len();
                flatten_nodes(num, out, depth + 1);
                out.push(Run {
                    text: "/".into(),
                    scale: scale.factor,
                    shift: 0.,
                    bold: *bold,
                    italic: *italic,
                });
                flatten_nodes(den, out, depth + 1);
                for run in &mut out[first..] {
                    run.shift += scale.shift;
                }
            }
            Node::Sqrt {
                body,
                scale,
                bold,
                italic,
            } => {
                out.push(Run {
                    text: "√(".into(),
                    scale: *scale,
                    shift: 0.,
                    bold: *bold,
                    italic: *italic,
                });
                flatten_nodes(body, out, depth + 1);
                out.push(Run {
                    text: ")".into(),
                    scale: *scale,
                    shift: 0.,
                    bold: *bold,
                    italic: *italic,
                });
            }
        }
    }
}

/// Parses source text into structured markup nodes (including LaTeX fractions and roots).
pub fn parse_nodes(source: &str, interpreter: Interpreter) -> Vec<Node> {
    if interpreter == Interpreter::Latex {
        return parse_latex_nodes(source);
    }
    parse(source, interpreter)
        .into_iter()
        .map(Node::Run)
        .collect()
}

/// Runs retain line breaks; shifts are fractions of the parent font size, positive upward.
pub fn parse(source: &str, interpreter: Interpreter) -> Vec<Run> {
    let mut out = Vec::new();
    if interpreter == Interpreter::Latex {
        flatten_nodes(&parse_latex_nodes(source), &mut out, 0);
        return out;
    }
    if interpreter != Interpreter::Tex {
        out.push(Run {
            text: source.into(),
            scale: 1.,
            shift: 0.,
            bold: false,
            italic: false,
        });
        return out;
    }
    fn sequence(
        chars: &[char],
        i: &mut usize,
        out: &mut Vec<Run>,
        mut style: Run,
        group: bool,
        depth: usize,
    ) {
        while *i < chars.len() {
            let c = chars[*i];
            *i += 1;
            if c == '}' && group {
                return;
            }
            if c == '{' && depth < 64 {
                sequence(chars, i, out, style.clone(), true, depth + 1);
                continue;
            }
            if c == '_' || c == '^' {
                let mut sub = style.clone();
                sub.scale *= 0.7;
                sub.shift += if c == '^' { 0.5 } else { -0.25 };
                if *i < chars.len() && chars[*i] == '{' && depth < 64 {
                    *i += 1;
                    sequence(chars, i, out, sub, true, depth + 1);
                } else if *i < chars.len() && chars[*i] == '\\' {
                    let mut end = (*i + 1).min(chars.len());
                    if end < chars.len() {
                        if chars[end].is_ascii_alphabetic() {
                            while end < chars.len() && chars[end].is_ascii_alphabetic() {
                                end += 1;
                            }
                            let full: String = chars[*i + 1..end].iter().collect();
                            if symbol(&full).is_none()
                                && !matches!(
                                    full.as_str(),
                                    "bf" | "it" | "rm" | "fontsize" | "color" | "fontname"
                                )
                            {
                                for p in (*i + 2..end).rev() {
                                    let prefix: String = chars[*i + 1..p].iter().collect();
                                    if symbol(&prefix).is_some() {
                                        end = p;
                                        break;
                                    }
                                }
                            }
                        } else {
                            end += 1;
                        }
                    }
                    let mut j = 0;
                    sequence(&chars[*i..end], &mut j, out, sub, false, depth + 1);
                    *i = end;
                } else if *i < chars.len() {
                    let end = (*i + 1).min(chars.len());
                    let mut j = 0;
                    sequence(&chars[*i..end], &mut j, out, sub, false, depth + 1);
                    *i = end;
                } else {
                    style.text = c.to_string();
                    out.push(style.clone());
                }
                continue;
            }
            let mut text = c.to_string();
            if c == '\\' && *i < chars.len() {
                let start = *i;
                if chars[*i].is_ascii_alphabetic() {
                    while *i < chars.len() && chars[*i].is_ascii_alphabetic() {
                        *i += 1;
                    }
                } else {
                    *i += 1;
                }
                let command: String = chars[start..*i].iter().collect();
                match command.as_str() {
                    "bf" => {
                        style.bold = true;
                        continue;
                    }
                    "it" => {
                        style.italic = true;
                        continue;
                    }
                    "rm" => {
                        style.bold = false;
                        style.italic = false;
                        continue;
                    }
                    "fontsize" | "color" | "fontname" => {
                        if command == "color" && *i < chars.len() && chars[*i] == '[' {
                            *i += 1;
                            while *i < chars.len() && chars[*i] != ']' {
                                *i += 1;
                            }
                            if *i < chars.len() {
                                *i += 1;
                            }
                        }
                        if *i < chars.len() && chars[*i] == '{' {
                            *i += 1;
                            while *i < chars.len() && chars[*i] != '}' {
                                *i += 1;
                            }
                            if *i < chars.len() {
                                *i += 1;
                            }
                        }
                        continue;
                    }
                    "\\" | "_" | "^" | "{" | "}" => text = command,
                    _ => {
                        text =
                            symbol(&command).map_or_else(|| format!("\\{command}"), str::to_owned)
                    }
                }
            }
            style.text = text;
            out.push(style.clone());
        }
    }
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    sequence(
        &chars,
        &mut i,
        &mut out,
        Run {
            text: String::new(),
            scale: 1.,
            shift: 0.,
            bold: false,
            italic: false,
        },
        false,
        0,
    );
    let mut merged: Vec<Run> = Vec::new();
    for run in out {
        if let Some(last) = merged.last_mut()
            && last.scale == run.scale
            && last.shift == run.shift
            && last.bold == run.bold
            && last.italic == run.italic
        {
            last.text.push_str(&run.text);
        } else {
            merged.push(run);
        }
    }
    merged
}
