//! Canonical, bounded script construction. No econometric arithmetic lives here.
use super::*;
use matrust::syntax::{Expr, StatementKind};

pub const MAX_SERIES: usize = 32;
pub const MAX_TESTS: usize = 64;
pub const MAX_POINTS: usize = 100_000;
pub const MAX_TOTAL: usize = 1_000_000;

pub(super) fn numeric_source(a: &Array) -> bool {
    matches!(a, Array::Numeric(_) | Array::Typed(_))
        || matches!(a, Array::Sparse(s) if !s.is_logical())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub(super) root: String,
    pub(super) expression: String,
}

fn identifier(name: &str) -> Result<(), String> {
    if !matrust::files::valid_name(name) || name.starts_with("app_econ_") {
        return Err("Use a variable name outside app_econ_".into());
    }
    super::super::single_expression(name)?;
    matrust::syntax::parse(&format!("{name}=0;"))?;
    Ok(())
}

pub fn checked_name(name: &str) -> Result<(), String> {
    identifier(name)?;
    super::super::check_export_name(name)
}

/// Parse and whitelist workspace selectors, never arbitrary function calls.
/// Reconstructing the accepted selector also removes comments and whitespace.
impl Source {
    pub fn parse(text: &str, ws: &Snapshot) -> Result<Self, String> {
        if text.len() > 256 {
            return Err("Source expression exceeds 256 bytes".into());
        }
        super::super::single_expression(text)?;
        let ast = matrust::syntax::parse(&format!("app_selector=({text});"))?;
        let StatementKind::Evaluate(ev) = &ast[0].kind else {
            return Err("Choose a workspace vector, matrix column or table variable".into());
        };
        fn column(args: &[Expr]) -> Result<usize, String> {
            if let [Expr::All, Expr::Number(n)] = args
                && n.is_finite()
                && *n >= 1.
                && *n <= MAX_POINTS as f64
                && n.fract() == 0.
            {
                return Ok(*n as usize);
            }
            Err("Select one column with (:, positive integer)".into())
        }
        fn resolve<'a>(
            expr: &Expr,
            ws: &'a Snapshot,
        ) -> Result<(String, String, &'a Array, Option<usize>), String> {
            match expr {
                Expr::Name(root) => {
                    identifier(root)?;
                    let a = ws
                        .get(root)
                        .ok_or_else(|| format!("Workspace variable {root} does not exist"))?;
                    Ok((root.clone(), root.clone(), a, None))
                }
                Expr::Field(base, name) => {
                    identifier(name)?;
                    let (root, code, a, selected) = resolve(base, ws)?;
                    if selected.is_some() {
                        return Err("Choose a table variable before a matrix column".into());
                    }
                    let Array::Table(table) = a else {
                        return Err("Dot selection requires a table or timetable".into());
                    };
                    let i = table
                        .names()
                        .iter()
                        .position(|n| n == name)
                        .ok_or("Table variable does not exist")?;
                    Ok((root, format!("{code}.{name}"), table.column(i), None))
                }
                Expr::Call(root, args) => {
                    let (root, code, a, _) = resolve(&Expr::Name(root.clone()), ws)?;
                    let col = column(args)?;
                    Ok((root, format!("{code}(:,{col})"), a, Some(col)))
                }
                Expr::Index(base, args) => {
                    let (root, code, a, selected) = resolve(base, ws)?;
                    if selected.is_some() {
                        return Err("Select only one matrix column".into());
                    }
                    let col = column(args)?;
                    Ok((root, format!("{code}(:,{col})"), a, Some(col)))
                }
                _ => Err("Use x, X(:,2), T.variable or T.variable(:,2)".into()),
            }
        }
        let (root, expression, a, column) = resolve(&ev.expr, ws)?;
        if !numeric_source(a) {
            return Err("Select a real numeric vector or a numeric table variable".into());
        }
        if a.dims().len() != 2 || a.is_empty() || column.is_some_and(|c| c > a.cols()) {
            return Err("Select a nonempty two-dimensional numeric column".into());
        }
        let len = if column.is_some() { a.rows() } else { a.len() };
        if (column.is_none() && a.rows() > 1 && a.cols() > 1) || len > MAX_POINTS {
            return Err(format!(
                "Select a vector or matrix column with at most {MAX_POINTS} observations"
            ));
        }
        if ws.get(&root).is_some_and(|a| a.bytes() > 64 * 1024 * 1024) {
            return Err("Source workspace variable exceeds 64 MiB".into());
        }
        Ok(Self { root, expression })
    }
}

/// Text fields must survive the actual script parser, then be bounded literals.
pub fn integer(text: &str, min: usize, max: usize, what: &str) -> Result<usize, String> {
    if text.len() > 32 {
        return Err(format!("{what}: use an integer in {min}…{max}"));
    }
    super::super::single_expression(text).map_err(|e| format!("{what}: {e}"))?;
    let n = text
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("{what}: use an integer in {min}…{max}"))?;
    if !(min..=max).contains(&n) {
        return Err(format!("{what}: use an integer in {min}…{max}"));
    }
    Ok(n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Log,
    Difference,
    Seasonal(usize),
    Detrend,
}
impl Transform {
    pub fn label(self) -> String {
        match self {
            Self::Log => "log".into(),
            Self::Difference => "difference".into(),
            Self::Seasonal(p) => format!("seasonal({p})"),
            Self::Detrend => "linear detrend".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct SeriesSpec {
    pub source: Source,
    pub transforms: Vec<Transform>,
}
impl SeriesSpec {
    pub fn label(&self) -> String {
        std::iter::once(self.source.expression.clone())
            .chain(self.transforms.iter().map(|t| t.label()))
            .collect::<Vec<_>>()
            .join(" → ")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestKind {
    Adf,
    Kpss,
    Pp,
    LjungBox,
    Arch,
}
impl TestKind {
    pub const ALL: [Self; 5] = [Self::Adf, Self::Kpss, Self::Pp, Self::LjungBox, Self::Arch];
    pub fn label(self) -> &'static str {
        match self {
            Self::Adf => "ADF",
            Self::Kpss => "KPSS",
            Self::Pp => "PP",
            Self::LjungBox => "Ljung-Box",
            Self::Arch => "ARCH",
        }
    }
    pub fn function(self) -> &'static str {
        match self {
            Self::Adf => "adftest",
            Self::Kpss => "kpsstest",
            Self::Pp => "pptest",
            Self::LjungBox => "lbqtest",
            Self::Arch => "archtest",
        }
    }
}
#[derive(Clone, Debug)]
pub struct TestSpec {
    pub kind: TestKind,
    pub series: usize,
    pub lags: usize,
    pub alpha: f64,
    pub model: String,
    pub statistic: String,
    pub trend: bool,
    pub dof: Option<usize>,
}
impl TestSpec {
    pub(super) fn validate(&self, count: usize) -> Result<(), String> {
        let min_lag = usize::from(matches!(self.kind, TestKind::LjungBox | TestKind::Arch));
        let alpha_range = match self.kind {
            TestKind::Kpss => 0.01..=0.1,
            TestKind::Adf | TestKind::Pp => 0.001..=0.999,
            _ => f64::MIN_POSITIVE..=1. - f64::EPSILON,
        };
        if self.series >= count
            || !(min_lag..=256).contains(&self.lags)
            || !alpha_range.contains(&self.alpha)
        {
            return Err("Choose compatible lags, series and significance level".into());
        }
        if !["AR", "ARD", "TS"].contains(&self.model.as_str())
            || !["t1", "t2", "F"].contains(&self.statistic.as_str())
            || (self.statistic == "F" && (self.kind != TestKind::Adf || self.model == "AR"))
            || self
                .dof
                .is_some_and(|d| self.kind != TestKind::LjungBox || d == 0 || d > self.lags)
        {
            return Err("Choose supported test options".into());
        }
        Ok(())
    }
    pub fn call(&self, data: &str) -> String {
        let mut code = format!(
            "{}({data},'Lags',{},'Alpha',{}",
            self.kind.function(),
            self.lags,
            super::super::number(self.alpha)
        );
        match self.kind {
            TestKind::Adf | TestKind::Pp => {
                code += &format!(",'Model','{}','Test','{}'", self.model, self.statistic)
            }
            TestKind::Kpss => code += &format!(",'Trend',{}", u8::from(self.trend)),
            TestKind::LjungBox => {
                if let Some(d) = self.dof {
                    code += &format!(",'DoF',{d}");
                }
            }
            TestKind::Arch => {}
        }
        code + ")"
    }
    pub fn options_label(&self) -> String {
        let extra = match self.kind {
            TestKind::Adf | TestKind::Pp => format!(", {}, {}", self.model, self.statistic),
            TestKind::Kpss => format!(", trend={}", self.trend),
            TestKind::LjungBox => format!(", DoF={}", self.dof.unwrap_or(self.lags)),
            TestKind::Arch => String::new(),
        };
        format!(
            "lags={}, α={}{}",
            self.lags,
            super::super::short(self.alpha),
            extra
        )
    }
}
#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub series: Vec<SeriesSpec>,
    pub tests: Vec<TestSpec>,
    pub diagnostics: Option<(usize, usize)>,
    pub models: Vec<ModelSpec>,
}
impl Plan {
    pub fn roots(&self) -> Vec<String> {
        self.series
            .iter()
            .map(|s| s.source.root.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

/// This is the only builder used by the Runner, preview, Export and saved script.
/// All temporaries are local; the only workspace assignment is the checked name.
pub fn build_script(plan: &Plan, name: &str) -> Result<String, String> {
    checked_name(name)?;
    if plan.series.is_empty()
        || plan.series.len() > MAX_SERIES
        || plan.tests.len() > MAX_TESTS
        || plan.models.len() > models::MAX_MODELS
    {
        return Err("Load 1–32 series and keep at most 64 tests and 32 models".into());
    }
    let inputs = plan.roots();
    let roots = inputs.join(",");
    let args = (0..inputs.len())
        .map(|i| format!("app_econ_input{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut code = format!(
        "{name}=app_econ_build({roots});\nfunction app_econ_result=app_econ_build({args})\napp_econ_result=struct();\napp_econ_result.Series={{}};\napp_econ_result.Tests={{}};\napp_econ_result.Models={{}};\napp_econ_result.Diagnostics=struct();\napp_econ_total=0;\napp_econ_model_total=0;\n"
    );
    for (i, s) in plan.series.iter().enumerate() {
        identifier(&s.source.root)?;
        super::super::single_expression(&s.source.expression)?;
        if s.transforms.len() > 32 || s.source.expression.len() > 256 {
            return Err("Series expression exceeds limits".into());
        }
        let y = format!("app_econ_s{i}");
        let input = inputs
            .iter()
            .position(|root| root == &s.source.root)
            .ok_or("Missing source argument")?;
        let suffix = s
            .source
            .expression
            .strip_prefix(&s.source.root)
            .ok_or("Invalid source selector")?;
        code += &format!(
            "{y}=app_econ_input{input}{suffix};\nif ~isnumeric({y}) || ~isreal({y}) || ~isvector({y}) || isempty({y}) || numel({y})>{MAX_POINTS}, error('Select a nonempty real numeric vector with at most {MAX_POINTS} observations'); end\n{y}=full(double({y}(:)));\n"
        );
        for t in &s.transforms {
            let expr = match t {
                Transform::Log => {
                    code += &format!(
                        "if any({y}(~isnan({y}))<=0), error('Log requires positive observations'); end\n"
                    );
                    format!("log({y})")
                }
                Transform::Difference => format!("diff({y})"),
                Transform::Seasonal(p) => {
                    if !(1..=256).contains(p) {
                        return Err("Seasonal period must be 1–256".into());
                    }
                    format!("{y}({}+1:end)-{y}(1:end-{})", p, p)
                }
                Transform::Detrend => format!("detrend({y},1)"),
            };
            code += &format!("{y}={expr};\n");
        }
        code += &format!(
            "if ~isreal({y}) || isempty({y}) || any(isinf({y})), error('Series must be nonempty and real, with no Inf values'); end\napp_econ_total=app_econ_total+numel({y});\nif app_econ_total>{MAX_TOTAL}, error('Retained series exceed {MAX_TOTAL} observations'); end\napp_econ_result.Series{{{}}}={y};\n",
            i + 1
        );
    }
    if let Some((series, lags)) = plan.diagnostics {
        if series >= plan.series.len() || !(1..=256).contains(&lags) {
            return Err("Select a series and 1–256 diagnostic lags".into());
        }
        code += &format!(
            "[app_econ_acf,app_econ_al,app_econ_ab]=autocorr(app_econ_s{series},'NumLags',{lags});\n[app_econ_pacf,app_econ_pl,app_econ_pb]=parcorr(app_econ_s{series},'NumLags',{lags});\napp_econ_result.Diagnostics=struct('Series',{},'ACF',app_econ_acf,'ACFLags',app_econ_al,'ACFBounds',app_econ_ab,'PACF',app_econ_pacf,'PACFLags',app_econ_pl,'PACFBounds',app_econ_pb);\n",
            series + 1
        );
    }
    for (i, t) in plan.tests.iter().enumerate() {
        t.validate(plan.series.len())?;
        code += &format!(
            "[app_econ_h,app_econ_p,app_econ_stat,app_econ_cv]={};\napp_econ_result.Tests{{{}}}=struct('Kind','{}','Series',{},'Lags',{},'Alpha',{},'Model','{}','StatisticType','{}','Trend',{},'DoF',{},'H',double(app_econ_h),'PValue',app_econ_p,'Statistic',app_econ_stat,'CriticalValue',app_econ_cv);\n",
            t.call(&format!("app_econ_s{}", t.series)),
            i + 1,
            t.kind.function(),
            t.series + 1,
            t.lags,
            super::super::number(t.alpha),
            t.model,
            t.statistic,
            u8::from(t.trend),
            t.dof.map_or("NaN".into(), |d| d.to_string())
        );
    }
    for (i, model) in plan.models.iter().enumerate() {
        model.validate(plan.series.len())?;
        code += &models::model_script(model, i);
    }
    code += "end\n";
    if code.len() > 256 * 1024 {
        return Err("Generated script exceeds 256 KiB".into());
    }
    Ok(code)
}
