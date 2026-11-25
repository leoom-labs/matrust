//! Shared data/network/train/results stages for the shallow neural-network apps.
//! Clean-room contracts: MathWorks public nftool/nprtool/nctool/ntstool documentation. Examples
//! are synthetic constructions, not copies of vendor datasets or layouts.
use super::{App, AppKind, Host, Outputs, Request, Runner, Snapshot, field, name_field, short};
use crate::{theme::ThemedScrollArea, toolkits::Action};
use eframe::egui;
use matrust::{
    Array,
    engine::{neural_app_hooks, nnet::train},
};
use std::{
    ops::ControlFlow,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Fitting,
    PatternRecognition,
    Clustering,
    TimeSeries,
}
impl Mode {
    pub fn kind(self) -> AppKind {
        match self {
            Self::Fitting => AppKind::NeuralNetFitting,
            Self::PatternRecognition => AppKind::NeuralNetPatternRecognition,
            Self::Clustering => AppKind::NeuralNetClustering,
            Self::TimeSeries => AppKind::NeuralNetTimeSeries,
        }
    }
    fn constructor(self) -> &'static str {
        match self {
            Self::Fitting => "fitnet",
            Self::PatternRecognition => "patternnet",
            Self::Clustering => "selforgmap",
            Self::TimeSeries => "narxnet",
        }
    }
    fn views(self) -> &'static [&'static str] {
        match self {
            Self::Clustering => &[
                "Neighbour distances",
                "Sample hits",
                "Weight positions",
                "Weight planes",
                "Topology",
            ],
            Self::TimeSeries => &[
                "Performance",
                "Training state",
                "Error histogram",
                "Response",
                "Error autocorrelation",
                "Input-error correlation",
                "Prediction",
            ],
            Self::Fitting => &[
                "Performance",
                "Training state",
                "Error histogram",
                "Regression",
            ],
            Self::PatternRecognition => &[
                "Performance",
                "Training state",
                "Error histogram",
                "Confusion",
                "ROC",
            ],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    Narx,
    Nar,
    InputOutput,
}
#[derive(Clone, Debug)]
pub struct Spec {
    pub mode: Mode,
    pub example: bool,
    pub predictors: String,
    pub responses: String,
    pub rows: bool,
    pub classes: bool,
    pub validation: u32,
    pub test: u32,
    pub seed: u32,
    pub hidden: u32,
    pub algorithm: String,
    pub epochs: u32,
    pub max_fail: u32,
    pub map_rows: u32,
    pub map_cols: u32,
    pub hex: bool,
    pub problem: Problem,
    pub input_delay: u32,
    pub feedback_delay: u32,
    pub horizon: u32,
}
impl Spec {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            example: true,
            predictors: String::new(),
            responses: String::new(),
            rows: false,
            classes: false,
            validation: 15,
            test: 15,
            seed: 0,
            hidden: 10,
            algorithm: if mode != Mode::PatternRecognition {
                "trainlm"
            } else {
                "trainscg"
            }
            .into(),
            epochs: if mode == Mode::Clustering { 200 } else { 1000 },
            max_fail: 6,
            map_rows: 10,
            map_cols: 10,
            hex: true,
            problem: Problem::Narx,
            input_delay: 2,
            feedback_delay: 2,
            horizon: 20,
        }
    }
    fn inputs(&self) -> Vec<String> {
        if self.example {
            vec![]
        } else if self.mode == Mode::Clustering {
            vec![self.predictors.clone()]
        } else if self.mode == Mode::TimeSeries && self.problem == Problem::Nar {
            vec![self.responses.clone()]
        } else if self.predictors == self.responses {
            vec![self.predictors.clone()]
        } else {
            vec![self.predictors.clone(), self.responses.clone()]
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.validation.saturating_add(self.test) >= 100
            || self.hidden == 0
            || self.hidden > 256
            || self.epochs > 10000
            || self.max_fail > 10000
        {
            return Err("Neural network: use a positive training split, 1–256 hidden units and at most 10000 epochs/checks".into());
        }
        if self.mode == Mode::Clustering
            && (self.map_rows == 0
                || self.map_cols == 0
                || self.map_rows.saturating_mul(self.map_cols) > 1024)
        {
            return Err("Neural network: map size must contain 1–1024 neurons".into());
        }
        if self.mode == Mode::TimeSeries
            && (!(1..=128).contains(&self.input_delay)
                || !(1..=128).contains(&self.feedback_delay)
                || !(1..=1000).contains(&self.horizon))
        {
            return Err("Neural network: delays must be 1–128 and horizon 1–1000".into());
        }
        if !algorithms().contains(&self.algorithm.as_str()) {
            return Err("Neural network: choose a registered training algorithm".into());
        }
        for name in self.inputs() {
            checked_name(&name)?;
        }
        Ok(())
    }
}
pub fn algorithms() -> Vec<&'static str> {
    [
        "trainlm", "trainscg", "trainbr", "trainbfg", "trainrp", "traingdx", "traingd", "traingdm",
    ]
    .into_iter()
    .filter(|n| train::algorithm(n).is_ok())
    .collect()
}
fn checked_name(name: &str) -> Result<(), String> {
    if !matrust::files::valid_name(name) || name.starts_with("app_nn_") {
        return Err("Neural network: choose a valid variable name outside app_nn_".into());
    }
    super::single_expression(name)?;
    super::check_export_name(name)
}
const EXTRA: [&str; 3] = ["app_nn_metrics", "app_nn_confusion", "app_nn_targets"];

/// The sole script builder for training, preview, export and script save.
/// A local function isolates temporary names and workspace function shadowing.
pub fn build_script(
    spec: &Spec,
    names: &[String; 4],
    checkpoint: &str,
    include_results: bool,
) -> Result<String, String> {
    spec.validate()?;
    for (i, name) in names.iter().enumerate() {
        checked_name(name)?;
        if names[..i].contains(name) {
            return Err("Neural network: export names must be distinct".into());
        }
    }
    if matches!(spec.mode, Mode::Clustering | Mode::TimeSeries) {
        return extended_script(spec, names, checkpoint, include_results);
    }
    let args = if spec.example {
        String::new()
    } else {
        format!("{}, {}", spec.predictors, spec.responses)
    };
    let mut destinations = names.to_vec();
    if include_results {
        destinations.extend(EXTRA.map(str::to_owned));
    }
    let mut code = format!(
        "[{}] = app_nn_run({args});\nfunction [net,tr,y,e,metrics,cm,t] = app_nn_run({})\nrng({});\n",
        destinations.join(", "),
        if spec.example { "" } else { "x,t" },
        spec.seed
    );
    if spec.example {
        code += match spec.mode {
            Mode::Fitting => "x = linspace(-pi,pi,101);\nt = sin(x);\n",
            Mode::Clustering | Mode::TimeSeries => unreachable!(),
            Mode::PatternRecognition => {
                "x = [randn(2,100)-1.5 randn(2,100)+1.5];\nt = [ones(1,100) zeros(1,100); zeros(1,100) ones(1,100)];\n"
            }
        };
    } else {
        if spec.rows {
            code += "x = x.';\n";
            if !spec.classes {
                code += "t = t.';\n";
            }
        }
        if spec.mode == Mode::PatternRecognition && spec.classes {
            code += "if ~isvector(t), error('Class targets must be a vector'); end\nif isnumeric(t) && (any(~isfinite(t(:))) || any(t(:) ~= fix(t(:)))), error('Class labels must be finite integers'); end\nt = full(ind2vec(double(categorical(t(:))).'));\n";
        }
    }
    code += "if ~isnumeric(x) || ~isreal(x) || ~isnumeric(t) || ~isreal(t) || isempty(x) || isempty(t) || ndims(x)>2 || ndims(t)>2, error('Predictors and targets must be nonempty real matrices'); end\nif size(x,2) ~= size(t,2), error('Predictors and targets must have the same sample count'); end\nif size(x,2)>50000 || size(x,1)>256 || size(t,1)>256 || numel(x)+numel(t)>2000000, error('Neural app data limit: 50000 samples, 256 rows, 2000000 elements'); end\n";
    code += &format!(
        "net = {}({}, '{}');\nnet.divideFcn = 'dividerand';\nnet.divideParam.trainRatio = {};\nnet.divideParam.valRatio = {};\nnet.divideParam.testRatio = {};\nnet.trainParam.epochs = {};\nnet.trainParam.max_fail = {};\nnet.trainParam.showWindow = false;\n[net,tr] = train(net,x,t);\n",
        spec.mode.constructor(),
        spec.hidden,
        spec.algorithm,
        (100 - spec.validation - spec.test) as f64 / 100.,
        spec.validation as f64 / 100.,
        spec.test as f64 / 100.,
        if checkpoint.is_empty() {
            spec.epochs
        } else {
            0
        },
        spec.max_fail
    );
    code += checkpoint;
    if !checkpoint.is_empty() {
        code += &format!("net.trainParam.epochs = {};\n", spec.epochs);
    }
    code += "y = net(x);\ne = t-y;\nmetrics = NaN(3,2);\nparts = {tr.trainInd,tr.valInd,tr.testInd};\nfor k=1:3\nii=parts{k};\nif ~isempty(ii)\ntt=t(:,ii); yy=y(:,ii);\nmetrics(k,1)=perform(net,tt,yy);\n";
    code += match spec.mode {
        Mode::Clustering | Mode::TimeSeries => unreachable!(),
        Mode::Fitting => "if numel(tt)>1\nr=corrcoef(tt(:),yy(:));\nmetrics(k,2)=r(1,2);\nend\n",
        Mode::PatternRecognition => "metrics(k,2)=100*confusion(tt,yy);\n",
    };
    code += "end\nend\ncm=[];\nplotperform(tr);\nplottrainstate(tr);\nploterrhist(e);\n";
    code += match spec.mode {
        Mode::Clustering | Mode::TimeSeries => unreachable!(),
        Mode::Fitting => "if numel(t)>1 && max(t(:))>min(t(:)), plotregression(t,y); end\n",
        Mode::PatternRecognition => {
            // At ten classes plotconfusion's extra percentages and totals require
            // 262 text objects. Use the counts heatmap for this figure instead.
            "[fraction,cm]=confusion(t,y);\n\
             if size(cm,1)==10\nfigure; heatmap(cm.','Title','Confusion','XLabel','Target class','YLabel','Predicted class');\n\
             else\nplotconfusion(t,y);\nend\nplotroc(t,y);\n"
        }
    };
    code += "end\n";
    if code.len() > 1024 * 1024 {
        return Err("Neural network: generated script exceeds 1 MiB".into());
    }
    Ok(code)
}
/// Matrix and one-stream cell inputs share the same bounded training contract.
const SERIES_MATRIX: &str = "function v=app_nn_matrix(v,rows)\nif iscell(v)\nif ~isvector(v) || numel(v)>50000, error('Use one cell per time step, at most 50000 steps'); end\ntotal=0;\nfor j=1:numel(v)\na=v{j};\nif ~isnumeric(a) || ~isreal(a) || isempty(a) || size(a,2)~=1 || size(a,1)>256, error('Each time step must be a real column vector with at most 256 channels'); end\ntotal=total+numel(a);\nif total>1000000, error('Time series element limit: 1000000'); end\nend\nv=cell2mat(v(:).');\nelseif rows\nv=v.';\nend\nif ~isnumeric(v) || ~isreal(v) || isempty(v) || ndims(v)>2 || size(v,1)>256 || size(v,2)>50000 || numel(v)>1000000 || any(~isfinite(v(:))), error('Use a finite real matrix: at most 256 channels, 50000 steps, 1000000 elements'); end\nend\n";

fn extended_script(
    spec: &Spec,
    names: &[String; 4],
    checkpoint: &str,
    results: bool,
) -> Result<String, String> {
    let som = spec.mode == Mode::Clustering;
    let nar = spec.problem == Problem::Nar;
    let args = if spec.example {
        String::new()
    } else if som {
        spec.predictors.clone()
    } else if nar {
        spec.responses.clone()
    } else {
        format!("{},{}", spec.predictors, spec.responses)
    };
    let params = if spec.example {
        ""
    } else if som {
        "x"
    } else if nar {
        "t"
    } else {
        "x,t"
    };
    let mut destinations = names.to_vec();
    if results {
        destinations.extend(EXTRA.map(str::to_owned));
        if !som {
            destinations.push("app_nn_prediction".into());
        }
    }
    let mut code = format!(
        "[{}]=app_nn_run({args});\nfunction [net,tr,y,e,metrics,cm,t,prediction]=app_nn_run({params})\nrng({});\n",
        destinations.join(","),
        spec.seed
    );
    if spec.example {
        code += if som {
            "x=[randn(2,100)*0.3+repmat([-2;0],1,100) randn(2,100)*0.3+repmat([2;0],1,100) randn(2,100)*0.3+repmat([0;3],1,100)];\n"
        } else {
            // Discrete damped oscillator, driven by two steps. Initial state is zero.
            "x=zeros(1,160); x(31:100)=1; x(101:end)=-0.5;\nt=zeros(1,160);\nfor k=3:160\nt(k)=1.7*t(k-1)-0.75*t(k-2)+0.05*x(k-1);\nend\n"
        };
    }
    let rows = u8::from(spec.rows && !spec.example);
    if som || !nar {
        code += &format!("x=app_nn_matrix(x,{rows});\n");
    }
    if !som {
        code += &format!("t=app_nn_matrix(t,{rows});\n");
    }
    let epochs = if checkpoint.is_empty() {
        spec.epochs
    } else {
        0
    };
    if som {
        code += &format!(
            "if size(x,1)>16, error('SOM views support at most 16 input channels'); end\nif size(x,1)*{}*4>10000, error('SOM weight-plane limit: 10000 faces'); end\nif size(x,2)*{}>8000000, error('SOM output limit: 8000000 neuron-samples'); end\nnet=selforgmap([{} {}],100,3,'{}');\nnet.trainParam.epochs={epochs};\nnet.trainParam.showWindow=false;\n[net,tr]=train(net,x);\n",
            spec.map_rows * spec.map_cols,
            spec.map_rows * spec.map_cols,
            spec.map_rows,
            spec.map_cols,
            if spec.hex { "hextop" } else { "gridtop" }
        );
        code += checkpoint;
        code += &format!("net.trainParam.epochs={};\n", spec.epochs);
        code += "a=net(x); y=vec2ind(a); e=sum(a,2); cm=e; metrics=NaN(3,2); t=x; prediction=[];\nplotsomnd(net);\nplotsomhits(net,x);\n";
        code += "if size(x,1)==2\nplotsompos(net,x);\nelse\nfigure; title('Weight positions require two input channels');\nend\nplotsomplanes(net);\nplotsomtop(net);\nend\n";
    } else {
        if !nar {
            code += "if size(x,2)~=size(t,2), error('Inputs and targets must have the same sample count'); end\n";
        }
        let taps = match spec.problem {
            Problem::Narx => format!(
                "size(x,1)*{}+size(t,1)*{}",
                spec.input_delay, spec.feedback_delay
            ),
            Problem::Nar => format!("size(t,1)*{}", spec.feedback_delay),
            Problem::InputOutput => format!("size(x,1)*{}", spec.input_delay),
        };
        code += &format!(
            "if ({taps})*size(t,2)>2000000, error('Time series delayed workspace limit: 2000000 elements'); end\n"
        );
        code += &match spec.problem {
            Problem::Narx => format!(
                "net=narxnet(1:{},1:{},{},'open','{}');\n",
                spec.input_delay, spec.feedback_delay, spec.hidden, spec.algorithm
            ),
            Problem::Nar => format!(
                "net=narnet(1:{},{},'open','{}');\n",
                spec.feedback_delay, spec.hidden, spec.algorithm
            ),
            Problem::InputOutput => format!(
                "net=timedelaynet(1:{},{},'{}');\n",
                spec.input_delay, spec.hidden, spec.algorithm
            ),
        };
        code += "tc=con2seq(t);\n";
        if nar {
            code += "xc={};\n";
        } else {
            code += "xc=con2seq(x);\n";
        }
        code += if spec.problem == Problem::InputOutput {
            "[xs,xi,ai,ts]=preparets(net,xc,tc);\n"
        } else {
            "[xs,xi,ai,ts]=preparets(net,xc,{},tc);\n"
        };
        code += &format!(
            "net.divideFcn='divideblock';\nnet.divideMode='time';\nnet.divideParam.trainRatio={};\nnet.divideParam.valRatio={};\nnet.divideParam.testRatio={};\nnet.trainParam.epochs={epochs};\nnet.trainParam.max_fail={};\nnet.trainParam.showWindow=false;\n[net,tr]=train(net,xs,ts,xi,ai);\n",
            (100 - spec.validation - spec.test) as f64 / 100.,
            spec.validation as f64 / 100.,
            spec.test as f64 / 100.,
            spec.max_fail
        );
        code += checkpoint;
        code += &format!("net.trainParam.epochs={};\n", spec.epochs);
        code += "[yc,xf,af]=net(xs,xi,ai);\ny=cell2mat(yc); t=cell2mat(ts); err=t-y;\nmetrics=NaN(3,2); parts={tr.trainInd,tr.valInd,tr.testInd};\nfor k=1:3\nii=parts{k};\nif ~isempty(ii)\ntt=t(:,ii); yy=y(:,ii); metrics(k,1)=perform(net,tt,yy);\nif numel(tt)>1\nr=corrcoef(tt(:),yy(:)); metrics(k,2)=r(1,2);\nend\nend\nend\ncm=[];\n[e,xic,aic]=closeloop(net,xf,af);\n";
        code += &if nar {
            format!("future=cell(0,{});\n", spec.horizon)
        } else {
            format!("future=con2seq(repmat(x(:,end),1,{}));\n", spec.horizon)
        };
        code += "prediction=cell2mat(e(future,xic,aic));\nplotperform(tr);\nplottrainstate(tr);\nploterrhist(err);\nfigure;\nsubplot(2,1,1); shown=1:min(8,size(t,1)); plot(t(shown,:).','DisplayName','Targets'); hold on; plot(y(shown,:).','DisplayName','Open-loop outputs'); title('Targets and open-loop responses');\nsubplot(2,1,2); plot(err(shown,:).'); title('Response errors'); xlabel('Prepared time step');\nfigure;\n";
        code += "for k=1:min(8,size(err,1))\nsubplot(min(8,size(err,1)),1,k); [c,lags]=xcorr(err(k,:),min(20,size(err,2)-1),'biased'); stem(lags,c); title('Error autocorrelation (biased)'); xlabel('Lag');\nend\nfigure;\n";
        if nar {
            code += "title('NAR has no external input');\n";
        } else {
            code += "aligned=x(:,end-size(err,2)+1:end);\nfor k=1:min(8,size(err,1))\nsubplot(min(8,size(err,1)),1,k); hold on;\nfor j=1:min(8,size(aligned,1))\n[c,lags]=xcorr(aligned(j,:),err(k,:),min(20,size(err,2)-1),'biased'); stem(lags,c);\nend\ntitle('Input-error cross-correlation (biased)'); xlabel('Lag');\nend\n";
        }
        code += "figure; plot(prediction(shown,:).'); title('Multi-step prediction'); xlabel('Future step (external inputs held at last value)');\nend\n";
    }
    code += SERIES_MATRIX;
    if code.len() > 1024 * 1024 {
        return Err("Neural network: generated script exceeds 1 MiB".into());
    }
    Ok(code)
}

#[derive(Clone)]
pub struct Curve {
    label: String,
    points: Vec<[f64; 2]>,
    scatter: bool,
    bars: bool,
    patch: bool,
    corners: usize,
    intensity: f32,
}
#[derive(Clone)]
pub struct Chart {
    title: String,
    x: String,
    y: String,
    curves: Vec<Curve>,
}
#[derive(Clone)]
pub struct ResultView {
    pub outputs: Outputs,
    pub metrics: Vec<f64>,
    pub confusion: Vec<f64>,
    pub stop: String,
    pub charts: Vec<Vec<Chart>>,
    checkpoint: Result<String, String>,
}
fn values(o: &Outputs, name: &str) -> Result<Vec<f64>, String> {
    match o.get(name) {
        Some(Array::Numeric(m)) => Ok(m.data().to_vec()),
        _ => Err(format!("Neural network: missing numeric {name}")),
    }
}
fn struct_field<'a>(a: &'a Array, name: &str) -> Result<&'a Array, String> {
    let Array::Struct(s) = a else {
        return Err("Expected training record".into());
    };
    Ok(
        s.get(s.field_index(name).ok_or("Missing training state")?, 0)?
            .as_ref(),
    )
}
fn text(a: &Array) -> Result<String, String> {
    match a {
        Array::Character(c) => Ok(String::from_utf16_lossy(c.units())),
        _ => Err("Expected text".into()),
    }
}
fn snapshot(code: &mut String, name: &str, a: &Array) -> Result<(), String> {
    match a {
        Array::Numeric(m) => {
            code.push_str(&format!("{name}=zeros({},{});\n", m.rows(), m.cols()));
            for (i, chunk) in m.data().chunks(64).enumerate() {
                let nums = chunk
                    .iter()
                    .map(|v| {
                        if v.is_nan() {
                            "NaN".into()
                        } else if *v == f64::INFINITY {
                            "Inf".into()
                        } else if *v == f64::NEG_INFINITY {
                            "-Inf".into()
                        } else {
                            v.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                code.push_str(&format!(
                    "{name}({}:{})=[{nums}];\n",
                    i * 64 + 1,
                    i * 64 + chunk.len()
                ));
                if code.len() > 900 * 1024 {
                    return Err("Neural network: checkpoint exceeds script limit".into());
                }
            }
        }
        Array::Logical(m) => {
            let numeric = matrust::Matrix::from_column_vec(
                m.rows(),
                m.cols(),
                m.data().iter().map(|&v| f64::from(v)).collect(),
            )?;
            snapshot(code, name, &Array::Numeric(numeric))?;
            code.push_str(&format!("{name}=logical({name});\n"));
        }
        Array::Character(c) => code.push_str(&format!(
            "{name}='{}';\n",
            String::from_utf16_lossy(c.units()).replace('\'', "''")
        )),
        Array::Struct(s) => {
            code.push_str(&format!("{name}=struct();\n"));
            for (i, k) in s.field_names().iter().enumerate() {
                snapshot(code, &format!("{name}.{k}"), s.get(i, 0)?.as_ref())?;
            }
        }
        _ => return Err("Neural network: unsupported checkpoint field".into()),
    }
    if code.len() > 900 * 1024 {
        return Err("Neural network: checkpoint exceeds script limit".into());
    }
    Ok(())
}

fn axis_label(label: &str, scale: matrust::plot::AxisScale) -> String {
    if scale == matrust::plot::AxisScale::Linear {
        label.into()
    } else {
        format!("log10({label})")
    }
}
struct Prepared {
    charts: Vec<Vec<Chart>>,
    checkpoint: Result<String, String>,
}

fn prepare_checkpoint(o: &Outputs) -> Result<String, String> {
    let mut code = String::new();
    snapshot(&mut code, "app_nn_wb", &o["app_nn_weights"])?;
    code += "net=setwb(net,app_nn_wb);\n";
    snapshot(&mut code, "tr", &o["app_nn_record"])?;
    Ok(code)
}

pub struct NeuralNet {
    pub spec: Spec,
    pub names: [String; 4],
    pub current: Option<ResultView>,
    active: Option<Spec>,
    frozen: Vec<(String, Array)>,
    versions: Vec<(String, Option<u64>)>,
    serial: u64,
    pub running: bool,
    pub progress: Arc<Mutex<Option<train::EpochReport>>>,
    pub stop: Arc<AtomicBool>,
    pending: Arc<Mutex<Option<Prepared>>>,
    checkpoint: Result<String, String>,
    pub view: usize,
    train_requested: bool,
}
// Time-series has the longest form. Put its short numeric editors beside their labels.
fn neural_number_field(
    ui: &mut egui::Ui,
    compact: bool,
    label: &str,
    add: impl FnOnce(&mut egui::Ui),
) {
    if compact {
        ui.horizontal(|ui| {
            ui.label(label);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), add);
        });
    } else {
        field(ui, label, add);
    }
}

impl NeuralNet {
    pub fn new(mode: Mode) -> Self {
        Self {
            spec: Spec::new(mode),
            names: match mode {
                Mode::Clustering => ["net", "tr", "clusterIndex", "hits"],
                Mode::TimeSeries => ["net", "tr", "y", "closedNet"],
                _ => ["net", "tr", "y", "e"],
            }
            .map(str::to_owned),
            current: None,
            active: None,
            frozen: vec![],
            versions: vec![],
            serial: 0,
            running: false,
            progress: Arc::default(),
            stop: Arc::default(),
            pending: Arc::default(),
            checkpoint: Ok(String::new()),
            view: 0,
            train_requested: false,
        }
    }
    pub fn start(&mut self, ws: &Snapshot) -> Result<(), String> {
        build_script(&self.spec, &self.names, "", true)?;
        let frozen = self
            .spec
            .inputs()
            .into_iter()
            .map(|n| {
                ws.get(&n)
                    .cloned()
                    .map(|a| (n.clone(), a))
                    .ok_or_else(|| format!("Workspace variable {n} does not exist"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.versions = self
            .spec
            .inputs()
            .into_iter()
            .map(|n| {
                let v = ws.version(&n);
                (n, v)
            })
            .collect();
        self.frozen = frozen;
        self.active = Some(self.spec.clone());
        self.serial += 1;
        self.checkpoint = Ok(String::new());
        self.stop = Arc::default();
        self.progress = Arc::default();
        self.pending = Arc::default();
        Ok(())
    }
}
/// Prefer the exact integer. If even 9 pt is too wide, the app's limit of
/// 50,000 samples lets a count fit in thousands.
fn confusion_label(
    painter: &egui::Painter,
    count: f64,
    width: f32,
    ink: egui::Color32,
) -> Arc<egui::Galley> {
    let fit = |text: String| {
        let layout =
            |size| painter.layout_no_wrap(text.clone(), egui::FontId::proportional(size), ink);
        let full = layout(crate::tokens::HEADING);
        if full.size().x <= width {
            return full;
        }
        let size = (crate::tokens::HEADING * width / full.size().x)
            .floor()
            .max(9.);
        layout(size)
    };
    let exact = fit(format!("{count:.0}"));
    if exact.size().x <= width || count < 1000. {
        return exact;
    }
    let thousands = count / 1000.;
    let compact = fit(format!("{thousands:.1}k"));
    if compact.size().x <= width {
        compact
    } else {
        fit(format!("{thousands:.0}k"))
    }
}

impl App for NeuralNet {
    fn kind(&self) -> AppKind {
        self.spec.mode.kind()
    }
    fn script(&self, ws: &Snapshot) -> Result<String, String> {
        for (name, version) in &self.versions {
            if ws.version(name) != *version {
                return Err(format!(
                    "Neural network: {name} changed; retrain before exporting"
                ));
            }
        }
        build_script(
            self.active.as_ref().unwrap_or(&self.spec),
            &self.names,
            self.checkpoint.as_ref().map_err(Clone::clone)?,
            true,
        )
    }
    fn export_code(&self, ws: &Snapshot) -> Result<String, String> {
        self.script(ws)?;
        let current = self
            .current
            .as_ref()
            .ok_or("Train a network before exporting")?;
        build_script(
            self.active.as_ref().unwrap_or(&self.spec),
            &self.names,
            current.checkpoint.as_ref().map_err(Clone::clone)?,
            false,
        )
    }
    fn inputs(&self) -> Vec<String> {
        vec![]
    }
    fn probe(&self, _: &Snapshot) -> (String, Vec<String>) {
        (
            format!(
                "app_nn_record={};\napp_nn_weights=getwb({});\n",
                self.names[1], self.names[0]
            ),
            EXTRA
                .into_iter()
                .chain((self.spec.mode == Mode::TimeSeries).then_some("app_nn_prediction"))
                .chain(["app_nn_record", "app_nn_weights"])
                .map(str::to_owned)
                .chain(self.names.clone())
                .collect(),
        )
    }
    fn submit(&self, runner: &mut Runner, ctx: &egui::Context, mut request: Request) {
        if self.active.is_none() {
            runner.poll(ctx);
            return;
        }
        if runner.key() == Some(self.serial) {
            runner.poll(ctx);
            return;
        }
        request.inputs = self.frozen.clone();
        let progress = self.progress.clone();
        let stop = self.stop.clone();
        let pending = self.pending.clone();
        let context = ctx.clone();
        let mode = self.active.as_ref().unwrap().mode;
        runner.submit_task(
            ctx,
            self.serial,
            Box::new(move |engine| {
                let _hook = neural_app_hooks::progress(move |report| {
                    *progress.lock().unwrap() = Some(report);
                    context.request_repaint();
                    if stop.load(Ordering::Relaxed) {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                });
                let outputs = super::run(engine, request)?;
                let mut figures = engine.figures().collect::<Vec<_>>();
                figures.sort_by_key(|(id, _)| *id);
                let charts = figures
                    .into_iter()
                    .map(|(_, f)| {
                        f.all_axes()
                            .map(|(_, a)| Chart {
                                title: a.title.clone(),
                                x: axis_label(&a.xlabel, a.xscale),
                                y: axis_label(
                                    if mode == Mode::PatternRecognition
                                        && a.ylabel == "Mean Squared Error (mse)"
                                    {
                                        "Cross-entropy"
                                    } else {
                                        &a.ylabel
                                    },
                                    a.yscale,
                                ),
                                curves: a
                                    .series
                                    .iter()
                                    .map(|s| Curve {
                                        label: s.label.clone(),
                                        points: s.display_points_axes(
                                            2000,
                                            [
                                                matrust::plot::AxisTransform {
                                                    scale: a.xscale,
                                                    negative: false,
                                                },
                                                matrust::plot::AxisTransform {
                                                    scale: a.yscale,
                                                    negative: false,
                                                },
                                            ],
                                        ),
                                        scatter: s.pattern == matrust::plot::LinePattern::None
                                            || s.kind == matrust::plot::PlotKind::Scatter,
                                        bars: s.kind == matrust::plot::PlotKind::Bar,
                                        patch: s.kind == matrust::plot::PlotKind::Patch,
                                        corners: s.patch.map_or(4, |p| p.vertices_per_face()),
                                        // SOM scalar bins encode distance as [a,a,0] and
                                        // weight as [min(2a,1),max(2a-1,0),0]. Their
                                        // average recovers a for a single-accent scale.
                                        intensity: if mode == Mode::Clustering && s.color[2] == 0 {
                                            0.15 + 0.8
                                                * (f32::from(s.color[0]) + f32::from(s.color[1]))
                                                / 510.
                                        } else {
                                            0.2
                                        },
                                    })
                                    .collect(),
                            })
                            .collect()
                    })
                    .collect();
                let checkpoint = prepare_checkpoint(&outputs);
                *pending.lock().unwrap() = Some(Prepared { charts, checkpoint });
                Ok(outputs)
            }),
        );
    }
    fn receive(&mut self, o: &Outputs) -> Result<(), String> {
        let metrics = values(o, EXTRA[0])?;
        if metrics.len() != 6 {
            return Err("Invalid results table".into());
        }
        let stop = text(struct_field(&o["app_nn_record"], "stop")?)?;
        let prepared = self
            .pending
            .lock()
            .unwrap()
            .take()
            .ok_or("Neural network: missing prepared views")?;
        let checkpoint = if stop == "User stop." {
            prepared.checkpoint.clone()
        } else {
            Ok(String::new())
        };
        let result = ResultView {
            outputs: o.clone(),
            metrics,
            confusion: values(o, EXTRA[1])?,
            stop,
            charts: prepared.charts,
            checkpoint: prepared.checkpoint,
        };
        self.checkpoint = checkpoint;
        self.current = Some(result);
        Ok(())
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let time_series = self.spec.mode == Mode::TimeSeries;
        ui.add_enabled_ui(!self.running, |ui| {
            ui.heading("Data");
            if self.spec.mode == Mode::TimeSeries {
                field(ui, "Problem", |ui| {
                    egui::ComboBox::from_id_salt("nn_problem")
                        .selected_text(match self.spec.problem {
                            Problem::Narx => "NARX",
                            Problem::Nar => "NAR",
                            Problem::InputOutput => "Input-output",
                        })
                        .show_ui(ui, |ui| {
                            for (p, label) in [
                                (Problem::Narx, "NARX: input and feedback"),
                                (Problem::Nar, "NAR: feedback only"),
                                (Problem::InputOutput, "Input-output: no feedback"),
                            ] {
                                ui.selectable_value(&mut self.spec.problem, p, label);
                            }
                        });
                });
            }
            ui.checkbox(&mut self.spec.example, "Use synthetic example");
            if self.spec.example {
                ui.label(match self.spec.mode {
                    Mode::Fitting => "101 samples of a sine curve",
                    Mode::PatternRecognition => "Two Gaussian classes, 200 samples",
                    Mode::Clustering => "Three 2-D Gaussian blobs, 300 samples",
                    Mode::TimeSeries => "Damped oscillator driven by steps, 160 time steps",
                });
            } else {
                for (label, name, target) in [
                    ("Predictors", &mut self.spec.predictors, false),
                    ("Responses", &mut self.spec.responses, true),
                ] {
                    if (self.spec.mode == Mode::Clustering && target)
                        || (self.spec.mode == Mode::TimeSeries
                            && self.spec.problem == Problem::Nar
                            && !target)
                    {
                        continue;
                    }
                    field(ui, label, |ui| {
                        egui::ComboBox::from_id_salt(label)
                            .selected_text(if name.is_empty() {
                                "Choose variable"
                            } else {
                                name.as_str()
                            })
                            .show_ui(ui, |ui| {
                                for (n, a) in ws.variables() {
                                    if matches!(a, Array::Numeric(_) | Array::Typed(_))
                                        || (self.spec.mode == Mode::TimeSeries
                                            && matches!(a, Array::Cell(_)))
                                        || (target && matches!(a, Array::Categorical(_)))
                                    {
                                        ui.selectable_value(name, n.to_owned(), n);
                                    }
                                }
                            });
                    });
                }
                field(ui, "Samples are", |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.spec.rows, false, "Columns");
                        ui.selectable_value(&mut self.spec.rows, true, "Rows");
                    });
                });
                if self.spec.mode == Mode::PatternRecognition {
                    ui.checkbox(&mut self.spec.classes, "Targets are class labels");
                }
            }
            ui.add_space(8.);
            ui.heading(if self.spec.mode == Mode::Clustering {
                "Randomness"
            } else {
                "Split"
            });
            if self.spec.mode == Mode::TimeSeries {
                ui.label("Consecutive training, validation and test blocks after delay removal.");
                ui.label("Cell input: one column vector per time step.");
                ui.label(
                    "Series plots show the first 8 channels; metrics and exports use all channels.",
                );
            }
            for (label, value, range) in [
                ("Validation %", &mut self.spec.validation, 0..=99),
                ("Test %", &mut self.spec.test, 0..=99),
                ("Seed", &mut self.spec.seed, 0..=u32::MAX),
            ] {
                if self.spec.mode == Mode::Clustering && label != "Seed" {
                    continue;
                }
                neural_number_field(ui, time_series, label, |ui| {
                    ui.add(egui::DragValue::new(value).range(range));
                });
            }
            ui.heading("Network");
            if self.spec.mode == Mode::Clustering {
                for (label, value) in [
                    ("Map rows", &mut self.spec.map_rows),
                    ("Map columns", &mut self.spec.map_cols),
                ] {
                    field(ui, label, |ui| {
                        ui.add(egui::DragValue::new(value).range(1..=1024));
                    });
                }
                field(ui, "Topology", |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.spec.hex, true, "Hex");
                        ui.selectable_value(&mut self.spec.hex, false, "Grid");
                    });
                });
            }
            if self.spec.mode == Mode::TimeSeries {
                for (label, value) in [
                    ("Input delays (1 to N)", &mut self.spec.input_delay),
                    ("Feedback delays (1 to N)", &mut self.spec.feedback_delay),
                ] {
                    neural_number_field(ui, true, label, |ui| {
                        ui.add(egui::DragValue::new(value).range(1..=128));
                    });
                }
                neural_number_field(ui, time_series, "Prediction horizon", |ui| {
                    ui.add(egui::DragValue::new(&mut self.spec.horizon).range(1..=1000));
                });
                ui.label("Future external inputs stay at their last observed value.");
                ui.label("Retrain to apply a new horizon.");
            }
            if self.spec.mode != Mode::Clustering {
                neural_number_field(ui, time_series, "Hidden layer size", |ui| {
                    ui.add(egui::DragValue::new(&mut self.spec.hidden).range(1..=256));
                });
                field(ui, "Training algorithm", |ui| {
                    egui::ComboBox::from_id_salt("algorithm")
                        .selected_text(&self.spec.algorithm)
                        .show_ui(ui, |ui| {
                            for n in algorithms() {
                                ui.selectable_value(&mut self.spec.algorithm, n.into(), n);
                            }
                        });
                });
            }
            neural_number_field(ui, time_series, "Max epochs", |ui| {
                ui.add(egui::DragValue::new(&mut self.spec.epochs).range(0..=10000));
            });
            if self.spec.mode != Mode::Clustering {
                neural_number_field(ui, time_series, "Max validation checks", |ui| {
                    ui.add(egui::DragValue::new(&mut self.spec.max_fail).range(0..=10000));
                });
            }
            if ui
                .button(if self.current.is_some() {
                    "Retrain"
                } else {
                    "Train"
                })
                .clicked()
            {
                self.train_requested = true;
            }
        });
        if ui
            .add_enabled(self.running, egui::Button::new("Stop"))
            .clicked()
        {
            self.stop.store(true, Ordering::Relaxed);
        }
        if let Some(p) = self.progress.lock().unwrap().as_ref() {
            ui.label(format!(
                "Epoch {} · {} s\nPerformance {}\nGradient {} · validation checks {}",
                p.epoch,
                short(p.time),
                short(p.perf),
                short(p.gradient),
                p.val_fail
            ));
        }
    }
    fn main_caption(&self) -> &'static str {
        "Training results"
    }
    fn main_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (i, label) in self.spec.mode.views().iter().enumerate() {
                ui.selectable_value(&mut self.view, i, *label);
            }
        });
        let Some(result) = &self.current else {
            ui.label("Choose data and train a network to view results.");
            return;
        };
        if self.spec.mode == Mode::PatternRecognition && self.view == 3 {
            let n = (result.confusion.len() as f64).sqrt() as usize;
            let palette = crate::tokens::palette(ui.ctx());
            let max = result.confusion.iter().copied().fold(1., f64::max);
            let classes = |_: egui_plot::GridInput| {
                (1..=n)
                    .map(|class| egui_plot::GridMark {
                        value: class as f64,
                        step_size: 1.,
                    })
                    .collect()
            };
            // Class marks are one unit apart. Let egui_plot check each integer's
            // actual glyph width/height instead of fading ticks below 60 points.
            let class_axis = |axis, title| {
                egui_plot::AxisHints::new(axis)
                    .label(title)
                    .label_spacing(0.0..=1.0)
            };
            let mut labels = Vec::with_capacity(n * n);
            let response = egui_plot::Plot::new("nn_confusion")
                .custom_x_axes(vec![class_axis(egui_plot::Axis::X, "Target class")])
                .custom_y_axes(vec![class_axis(egui_plot::Axis::Y, "Predicted class")])
                .x_grid_spacer(classes)
                .y_grid_spacer(classes)
                .show_grid(false)
                .data_aspect(1.)
                .show(ui, |plot| {
                    for predicted in 0..n {
                        for target in 0..n {
                            // confusion stores target rows and output columns; the plot
                            // places output classes vertically, matching plotconfusion.
                            let count = result.confusion[target + predicted * n];
                            let (x, y) = ((target + 1) as f64, (predicted + 1) as f64);
                            let corners = vec![
                                [x - 0.5, y - 0.5],
                                [x + 0.5, y - 0.5],
                                [x + 0.5, y + 0.5],
                                [x - 0.5, y + 0.5],
                            ];
                            let fill = palette
                                .surface
                                .lerp_to_gamma(palette.accent, (0.1 + 0.9 * count / max) as f32);
                            let rgb = egui::Rgba::from(fill);
                            let luminance = 0.2126 * rgb.r() + 0.7152 * rgb.g() + 0.0722 * rgb.b();
                            // Choose the greater WCAG contrast ratio against the opaque cell.
                            let ink = if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::WHITE
                            };
                            plot.polygon(
                                egui_plot::Polygon::new(
                                    format!(
                                        "Target {}, predicted {}: {count:.0}",
                                        target + 1,
                                        predicted + 1
                                    ),
                                    corners,
                                )
                                .fill_color(fill)
                                .stroke(egui::Stroke::new(1., palette.accent)),
                            );
                            labels.push((egui_plot::PlotPoint::new(x, y), count, ink));
                        }
                    }
                });
            // The final transform includes this frame's axes, aspect ratio and zoom.
            let transform = response.transform;
            let painter = ui.painter().with_clip_rect(*transform.frame());
            let width = transform.dpos_dvalue_x().abs() as f32 - crate::tokens::XS;
            for (point, count, ink) in labels {
                let galley = confusion_label(&painter, count, width, ink);
                let center = transform.position_from_point(&point);
                painter.galley(center - galley.size() * 0.5, galley, ink);
            }
            return;
        }
        let Some(charts) = result.charts.get(self.view) else {
            ui.label("This plot is unavailable for constant targets.");
            return;
        };
        egui::ScrollArea::vertical().show_themed(ui, |ui| {
            for (i, chart) in charts.iter().enumerate() {
                ui.label(&chart.title);
                let accent = crate::tokens::palette(ui.ctx()).accent;
                egui_plot::Plot::new(("neural_plot", self.view, i))
                    .height((ui.available_height() / charts.len() as f32).max(160.))
                    .x_axis_label(&chart.x)
                    .y_axis_label(&chart.y)
                    .legend(egui_plot::Legend::default())
                    .show(ui, |plot| {
                        for (series_index, c) in chart.curves.iter().enumerate() {
                            if c.patch {
                                for (k, face) in c.points.chunks_exact(c.corners).enumerate() {
                                    plot.polygon(
                                        egui_plot::Polygon::new(
                                            if k == 0 { c.label.as_str() } else { "" },
                                            face.to_vec(),
                                        )
                                        .fill_color(accent.gamma_multiply(c.intensity))
                                        .stroke(egui::Stroke::new(1., accent)),
                                    );
                                }
                            } else if c.bars {
                                plot.bar_chart(
                                    egui_plot::BarChart::new(
                                        &c.label,
                                        c.points
                                            .iter()
                                            .map(|p| egui_plot::Bar::new(p[0], p[1]))
                                            .collect(),
                                    )
                                    .color(accent),
                                );
                            } else if c.scatter {
                                plot.points(
                                    egui_plot::Points::new(&c.label, c.points.clone())
                                        .color(accent)
                                        .radius(2.),
                                );
                            } else {
                                plot.line(
                                    egui_plot::Line::new(&c.label, c.points.clone())
                                        .color(accent)
                                        .style(if series_index % 2 == 0 {
                                            egui_plot::LineStyle::Solid
                                        } else {
                                            egui_plot::LineStyle::dashed_dense()
                                        }),
                                );
                            }
                        }
                    });
            }
        });
    }
    fn results(&self) -> Vec<(&'static str, String)> {
        let Some(r) = &self.current else {
            return vec![];
        };
        if self.spec.mode == Mode::Clustering {
            return vec![
                ("Samples", short(r.confusion.iter().sum())),
                (
                    "Occupied neurons",
                    r.confusion.iter().filter(|v| **v > 0.).count().to_string(),
                ),
                (
                    "Hits by neuron",
                    r.confusion
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("{}: {}", i + 1, short(*v)))
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            ];
        }
        let labels = if self.spec.mode != Mode::PatternRecognition {
            ["Training MSE / R", "Validation MSE / R", "Test MSE / R"]
        } else {
            [
                "Training CE / % error",
                "Validation CE / % error",
                "Test CE / % error",
            ]
        };
        labels
            .into_iter()
            .enumerate()
            .map(|(i, l)| {
                (
                    l,
                    format!("{} / {}", short(r.metrics[i]), short(r.metrics[i + 3])),
                )
            })
            .collect()
    }
    fn names_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        ui.add_enabled_ui(!self.running, |ui| {
            let labels = match self.spec.mode {
                Mode::Clustering => [
                    "Network",
                    "Training record",
                    "Cluster indices",
                    "Hits per neuron",
                ],
                Mode::TimeSeries => [
                    "Network",
                    "Training record",
                    "Open-loop outputs",
                    "Closed-loop network",
                ],
                _ => ["Network", "Training record", "Outputs", "Errors"],
            };
            for (label, name) in labels.into_iter().zip(&mut self.names) {
                name_field(ui, label, name, ws);
            }
        });
    }
    fn exported(&self, _: &Snapshot) -> Vec<String> {
        self.names.to_vec()
    }
    fn summary(&self) -> Option<String> {
        self.current.as_ref().map(|r| {
            format!(
                "{}: {} ({} samples)",
                self.kind().label(),
                r.stop,
                r.outputs.get("app_nn_targets").map_or(0, Array::cols)
            )
        })
    }
}
pub(crate) fn show(
    host: &mut Host<NeuralNet>,
    ui: &mut egui::Ui,
    ws: &Snapshot,
    busy: bool,
    action: &mut Option<Action>,
) {
    host.set_folder(ws.folder());
    host.app.running = host.running();
    host.show(ui, ws, busy, action);
    if std::mem::take(&mut host.app.train_requested) {
        start_training(host, ui.ctx(), ws);
    }
}
pub(crate) fn start_training(host: &mut Host<NeuralNet>, ctx: &egui::Context, ws: &Snapshot) {
    match host.app.start(ws) {
        Ok(()) => {
            host.run_error = None;
            host.delivered = None;
            host.refresh(ctx, ws);
        }
        Err(e) => host.run_error = Some(e),
    }
}
