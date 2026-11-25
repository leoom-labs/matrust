//! Model specifications and script fragments. All statistics are engine outputs.
use super::*;

pub const MAX_MODELS: usize = 32;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelKind {
    #[default]
    Arima,
    Garch,
    Egarch,
    Gjr,
    ArimaGarch,
    ArimaEgarch,
    ArimaGjr,
}
impl ModelKind {
    pub const ALL: [Self; 7] = [
        Self::Arima,
        Self::Garch,
        Self::Egarch,
        Self::Gjr,
        Self::ArimaGarch,
        Self::ArimaEgarch,
        Self::ArimaGjr,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Arima => "ARIMA",
            Self::Garch => "GARCH",
            Self::Egarch => "EGARCH",
            Self::Gjr => "GJR",
            Self::ArimaGarch => "ARIMA + GARCH",
            Self::ArimaEgarch => "ARIMA + EGARCH",
            Self::ArimaGjr => "ARIMA + GJR",
        }
    }
    pub fn mean(self) -> bool {
        !matches!(self, Self::Garch | Self::Egarch | Self::Gjr)
    }
    fn variance(self) -> Option<&'static str> {
        match self {
            Self::Arima => None,
            Self::Garch | Self::ArimaGarch => Some("garch"),
            Self::Egarch | Self::ArimaEgarch => Some("egarch"),
            Self::Gjr | Self::ArimaGjr => Some("gjr"),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ModelSpec {
    pub name: String,
    pub kind: ModelKind,
    pub series: usize,
    pub p: usize,
    pub d: usize,
    pub q: usize,
    pub sar: Vec<usize>,
    pub sma: Vec<usize>,
    pub period: usize,
    pub constant: bool,
    pub student: bool,
    pub vp: usize,
    pub vq: usize,
    pub diagnostic_lags: usize,
    pub horizon: Option<usize>,
    pub selected: bool,
}
impl Default for ModelSpec {
    fn default() -> Self {
        Self {
            name: "Model1".into(),
            kind: ModelKind::Arima,
            series: 0,
            p: 1,
            d: 0,
            q: 0,
            sar: vec![],
            sma: vec![],
            period: 0,
            constant: true,
            student: false,
            vp: 1,
            vq: 1,
            diagnostic_lags: 10,
            horizon: None,
            selected: true,
        }
    }
}
impl ModelSpec {
    pub fn validate(&self, count: usize) -> Result<(), String> {
        checked_name(&self.name)?;
        if self.series >= count
            || self.p > 16
            || self.q > 16
            || self.d > 2
            || self.vp > 16
            || !(1..=16).contains(&self.vq)
            || self.period > 256
            || !(1..=256).contains(&self.diagnostic_lags)
            || self.horizon.is_some_and(|h| !(1..=1000).contains(&h))
        {
            return Err(
                "Model limits: p/q/P ≤16, D ≤2, Q 1–16, period/lags ≤256, horizon 1–1000".into(),
            );
        }
        for lags in [&self.sar, &self.sma] {
            if lags.len() > 16
                || lags.iter().any(|n| !(1..=256).contains(n))
                || lags.windows(2).any(|w| w[0] >= w[1])
            {
                return Err(
                    "Seasonal lags must be increasing, unique integers in 1–256 (at most 16)"
                        .into(),
                );
            }
        }
        Ok(())
    }
    pub fn constructor(&self) -> String {
        let distribution = if self.student { "t" } else { "Gaussian" };
        let variance = self.kind.variance().map(|v| {
            format!(
                "{v}({},{} ,'Distribution','{distribution}')",
                self.vp, self.vq
            )
        });
        if !self.kind.mean() {
            return variance.unwrap();
        }
        let lags = |v: &[usize]| {
            format!(
                "[{}]",
                v.iter().map(usize::to_string).collect::<Vec<_>>().join(" ")
            )
        };
        let mut code = format!(
            "arima('ARLags',{},'D',{},'MALags',{},'SARLags',{},'SMALags',{},'Seasonality',{},'Constant',{},'Distribution','{distribution}'",
            lags(&(1..=self.p).collect::<Vec<_>>()),
            self.d,
            lags(&(1..=self.q).collect::<Vec<_>>()),
            lags(&self.sar),
            lags(&self.sma),
            self.period,
            if self.constant { "NaN" } else { "0" }
        );
        if let Some(v) = variance {
            code += &format!(",'Variance',{v}");
        }
        code + ")"
    }
}

pub fn parse_lags(text: &str) -> Result<Vec<usize>, String> {
    if text.len() > 128 {
        return Err("Seasonal lags exceed 128 bytes".into());
    }
    let text = text.trim();
    if text.is_empty() {
        return Ok(vec![]);
    }
    super::super::single_expression(text)?;
    let ast = matrust::syntax::parse(&format!("app_lags={text};"))?;
    use matrust::syntax::{Expr, StatementKind};
    let StatementKind::Evaluate(ev) = &ast[0].kind else {
        return Err("Use a literal lag vector".into());
    };
    let items: Vec<&Expr> = match &ev.expr {
        Expr::Number(_) => vec![&ev.expr],
        Expr::Matrix(rows) if rows.len() <= 1 => rows.iter().flatten().collect(),
        _ => return Err("Use a literal row vector of seasonal lags, such as [12 24]".into()),
    };
    let lags = items
        .into_iter()
        .map(|x| match x {
            Expr::Number(x) if x.is_finite() && x.fract() == 0. && (1. ..=256.).contains(x) => {
                Ok(*x as usize)
            }
            _ => Err("Seasonal lags must be integers in 1–256".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if lags.len() > 16 || lags.windows(2).any(|w| w[0] >= w[1]) {
        return Err("Use at most 16 increasing, unique seasonal lags".into());
    }
    Ok(lags)
}

pub(super) fn model_script(s: &ModelSpec, i: usize) -> String {
    let y = format!("app_econ_s{}", s.series);
    let mut c = format!(
        "app_econ_model_total=app_econ_model_total+numel({y});\nif app_econ_model_total>1000000, error('Fitted models exceed 1000000 retained observations'); end\n[app_econ_fit,app_econ_cov,app_econ_ll]=estimate({}, {y},'Display','off');\napp_econ_summary=summarize(app_econ_fit);\n",
        s.constructor()
    );
    if s.kind.mean() {
        c += &format!("[app_econ_res,app_econ_var]=infer(app_econ_fit,{y});\n");
    } else {
        c += &format!(
            "app_econ_var=infer(app_econ_fit,{y});\napp_econ_res=({y}-app_econ_fit.Offset)./sqrt(app_econ_var);\n"
        );
    }
    c += &format!(
        "app_econ_clean=app_econ_res(isfinite(app_econ_res));\nif numel(app_econ_clean)<3, error('Residual diagnostics require at least three finite residuals'); end\napp_econ_lags=min({},numel(app_econ_clean)-1);\n[app_econ_ra,app_econ_rl,app_econ_rb]=autocorr(app_econ_clean,'NumLags',app_econ_lags);\n[app_econ_h,app_econ_p,app_econ_stat,app_econ_cv]=lbqtest(app_econ_clean,'Lags',app_econ_lags);\n[app_econ_counts,app_econ_edges]=histcounts(app_econ_clean,20,'Normalization','pdf');\napp_econ_centers=(app_econ_edges(1:end-1)+app_econ_edges(2:end))/2;\napp_econ_normal=normpdf(app_econ_centers,mean(app_econ_clean),max(std(app_econ_clean),eps));\napp_econ_qqx=norminv(((1:numel(app_econ_clean))'-0.5)/numel(app_econ_clean));\napp_econ_qqy=sort(app_econ_clean);\napp_econ_qqfit=mean(app_econ_clean)+std(app_econ_clean)*app_econ_qqx;\napp_econ_f=[]; app_econ_mse=[]; app_econ_lower=[]; app_econ_upper=[];\n",
        s.diagnostic_lags
    );
    if let Some(h) = s.horizon {
        if s.kind.mean() {
            c += &format!(
                "[app_econ_f,app_econ_mse]=forecast(app_econ_fit,{h},{y});\napp_econ_lower=app_econ_f-1.96*sqrt(app_econ_mse);\napp_econ_upper=app_econ_f+1.96*sqrt(app_econ_mse);\n"
            );
        } else {
            c += &format!("app_econ_f=forecast(app_econ_fit,{h},{y});\n");
        }
    }
    c += &format!(
        "app_econ_result.Models{{{}}}=struct('Name','{}','Kind','{}','Series',{},'DiagnosticLags',{},'Horizon',{},'Model',app_econ_fit,'Summary',app_econ_summary,'Residuals',app_econ_res,'Variance',app_econ_var,'ACF',app_econ_ra,'ACFLags',app_econ_rl,'ACFBounds',app_econ_rb,'LjungBox',[double(app_econ_h) app_econ_p app_econ_stat app_econ_cv],'HistogramX',app_econ_centers,'HistogramY',app_econ_counts,'Normal',app_econ_normal,'QQX',app_econ_qqx,'QQY',app_econ_qqy,'QQFit',app_econ_qqfit,'Forecast',app_econ_f,'YMSE',app_econ_mse,'Lower',app_econ_lower,'Upper',app_econ_upper);\n",
        i + 1,
        s.name,
        s.kind.label(),
        s.series + 1,
        s.diagnostic_lags,
        s.horizon.unwrap_or(0)
    );
    c
}

pub struct Parameter {
    pub name: String,
    pub values: [f64; 4],
}
pub struct ModelView {
    pub parameters: Vec<Parameter>,
    pub log_likelihood: f64,
    pub aic: f64,
    pub bic: f64,
    pub residuals: Vec<f64>,
    pub variance: Vec<f64>,
    pub acf: Vec<f64>,
    pub acf_lags: Vec<f64>,
    pub acf_bounds: Vec<f64>,
    pub ljung_box: Vec<f64>,
    pub histogram_x: Vec<f64>,
    pub histogram_y: Vec<f64>,
    pub normal: Vec<f64>,
    pub qq_x: Vec<f64>,
    pub qq_y: Vec<f64>,
    pub qq_fit: Vec<f64>,
    pub forecast: Vec<f64>,
    pub mse: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
}
pub(super) fn decode_models(a: &Array) -> Result<Vec<ModelView>, String> {
    let Array::Cell(c) = struct_field(a, "Models")? else {
        return Err("Missing model list".into());
    };
    let mut views = vec![];
    for i in 0..c.len() {
        let a = c.get_linear(i)?;
        let n = |key| numbers(struct_field(a, key)?);
        let summary = struct_field(a, "Summary")?;
        let scalar = |key| {
            numbers(struct_field(summary, key)?)
                .and_then(|v| v.first().copied().ok_or("Missing fit statistic".into()))
        };
        let mut parameters = vec![];
        for key in ["Table", "VarianceTable"] {
            if let Ok(Array::Table(t)) = struct_field(summary, key) {
                let cols = t
                    .columns()
                    .iter()
                    .map(numbers)
                    .collect::<Result<Vec<_>, _>>()?;
                for (row, name) in t.row_names().unwrap_or(&[]).iter().enumerate() {
                    parameters.push(Parameter {
                        name: name.clone(),
                        values: std::array::from_fn(|j| cols[j][row]),
                    });
                }
            }
        }
        views.push(ModelView {
            parameters,
            log_likelihood: scalar("LogLikelihood")?,
            aic: scalar("AIC")?,
            bic: scalar("BIC")?,
            residuals: n("Residuals")?,
            variance: n("Variance")?,
            acf: n("ACF")?,
            acf_lags: n("ACFLags")?,
            acf_bounds: n("ACFBounds")?,
            ljung_box: n("LjungBox")?,
            histogram_x: n("HistogramX")?,
            histogram_y: n("HistogramY")?,
            normal: n("Normal")?,
            qq_x: n("QQX")?,
            qq_y: n("QQY")?,
            qq_fit: n("QQFit")?,
            forecast: n("Forecast")?,
            mse: n("YMSE")?,
            lower: n("Lower")?,
            upper: n("Upper")?,
        });
    }
    Ok(views)
}
