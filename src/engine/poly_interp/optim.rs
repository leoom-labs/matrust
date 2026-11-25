//! fzero, fminsearch, fminbnd, optimplotx and optimplotfval.
//!
//! fzero is Brent's zero finder (R. P. Brent, Algorithms for Minimization without
//! Derivatives, 1973, ch. 4; Forsythe, Malcolm & Moler, Computer Methods for
//! Mathematical Computations, 1977, sec. 7.2 "zeroin"): secant and inverse quadratic
//! interpolation steps, safeguarded by bisection. From a scalar start it first widens
//! a symmetric search until the sign changes (see `polyopt_fzero`).
//! fminbnd is Brent's one-dimensional minimiser (Brent 1973, ch. 5; Forsythe, Malcolm
//! & Moler 1977, sec. 8.2 "fmin"): golden-section search with parabolic interpolation.
//! fminsearch is the Nelder-Mead simplex method (Nelder & Mead, "A simplex method for
//! function minimization", Computer J. 7, 1965) in the form analysed by Lagarias,
//! Reeds, Wright & Wright ("Convergence properties of the Nelder-Mead simplex method
//! in low dimensions", SIAM J. Optim. 9, 1998): reflection 1, expansion 2, contraction
//! 1/2, shrink 1/2, with their ordering and acceptance rules.
use super::{BUDGET, Engine, Execution, Value, matrix_value, real_matrix, scalar_of};
use crate::engine::optim_nl::{
    canonical, is_empty_value, onl_scalar, onl_text, polyopt_fun_val_check, polyopt_option_name,
};
use crate::engine::text_format::{mat2str_complex, mat2str_real};
use crate::{
    Array, CharacterRow, Complex64, FunctionHandle, LogicalMatrix, Matrix, Result, StructArray,
};
use std::cmp::Ordering;
use std::sync::Arc;

/// Most halvings towards the far bracket end that fzero tries after a NaN value.
const FZERO_NAN_PROBES: usize = 60;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OptDisplay {
    Off,
    Iter,
    Final,
    Notify,
}

struct PolyOptOptions {
    tol_x: f64,
    tol_fun: f64,
    max_iter: Option<usize>,
    max_fun_evals: Option<usize>,
    display: OptDisplay,
    output_fcns: Vec<FunctionHandle>,
    plot_fcns: Vec<FunctionHandle>,
    fun_val_check: bool,
}

impl PolyOptOptions {
    fn defaults_for(who: &str, numel: usize) -> Self {
        match who {
            "fzero" => Self {
                tol_x: f64::EPSILON,
                tol_fun: 1e-4,
                max_iter: None,
                max_fun_evals: None,
                display: OptDisplay::Notify,
                output_fcns: Vec::new(),
                plot_fcns: Vec::new(),
                fun_val_check: false,
            },
            "fminbnd" => Self {
                tol_x: 1e-4,
                tol_fun: 1e-4,
                max_iter: Some(500),
                max_fun_evals: Some(500),
                display: OptDisplay::Notify,
                output_fcns: Vec::new(),
                plot_fcns: Vec::new(),
                fun_val_check: false,
            },
            _ => {
                let limit = 200usize.saturating_mul(numel.max(1));
                Self {
                    tol_x: 1e-4,
                    tol_fun: 1e-4,
                    max_iter: Some(limit),
                    max_fun_evals: Some(limit),
                    display: OptDisplay::Notify,
                    output_fcns: Vec::new(),
                    plot_fcns: Vec::new(),
                    fun_val_check: false,
                }
            }
        }
    }

    fn evaluations_exhausted(&self, evaluations: usize) -> bool {
        self.max_fun_evals.is_some_and(|limit| evaluations >= limit)
    }

    fn iterations_exhausted(&self, iterations: usize) -> bool {
        self.max_iter.is_some_and(|limit| iterations >= limit)
    }

    fn limit_message(&self, iterations: usize, evaluations: usize) -> Option<&'static str> {
        if self.iterations_exhausted(iterations) {
            Some(
                "Exiting: Maximum number of iterations has been exceeded\n         - increase MaxIter option.",
            )
        } else if self.evaluations_exhausted(evaluations) {
            Some(
                "Exiting: Maximum number of function evaluations has been exceeded\n         - increase MaxFunEvals option.",
            )
        } else {
            None
        }
    }
}

const CALLBACK_STOP: &str = "Optimization terminated by the output or plot function.";

struct OptCallbacks<'a> {
    options: &'a PolyOptOptions,
    plot_axes: Vec<Value>,
}

struct OptIteration {
    evaluations: usize,
    fval: f64,
    index: usize,
    procedure: &'static str,
    bracket: Option<(f64, f64, f64, f64)>,
}

impl<'a> OptCallbacks<'a> {
    fn new(options: &'a PolyOptOptions) -> Self {
        Self {
            options,
            plot_axes: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.options.output_fcns.is_empty() && self.options.plot_fcns.is_empty()
    }

    fn call(
        &mut self,
        engine: &mut Engine,
        x: &Matrix,
        iteration: OptIteration,
        state: &str,
        out: &mut Execution,
    ) -> Result<bool> {
        if self.is_empty() {
            return Ok(false);
        }
        let options = self.options;
        let values = build_optim_values(
            engine,
            iteration.evaluations,
            iteration.fval,
            iteration.index,
            iteration.procedure,
            iteration.bracket,
        )?;
        let initializing = state == "init";
        let state = Value::from(char_array(engine, state)?);
        let mut stop = false;
        for fcn in &options.output_fcns {
            stop |= engine.polyopt_call_callback(fcn, x, &values, &state, out)?;
        }
        let count = options.plot_fcns.len();
        if initializing {
            for index in 0..count {
                let axes = if count == 1 {
                    engine.call("gca", vec![], out)?
                } else {
                    let rows = count.div_ceil(2);
                    engine.call(
                        "subplot",
                        [rows, 2, index + 1]
                            .map(|value| Value::Matrix(Matrix::scalar(value as f64)))
                            .to_vec(),
                        out,
                    )?
                };
                self.plot_axes.push(axes);
            }
        }
        for (axes, fcn) in self.plot_axes.iter().zip(&options.plot_fcns) {
            engine.call("axes", vec![axes.clone()], out)?;
            stop |= engine.polyopt_call_callback(fcn, x, &values, &state, out)?;
        }
        Ok(stop)
    }
}

struct ZeroProgress {
    point: (f64, f64),
    bracket: [(f64, f64); 2],
    interval_iters: usize,
    iterations: usize,
    evaluations: usize,
    procedure: &'static str,
}

impl ZeroProgress {
    fn new(x: f64) -> Self {
        Self {
            point: (x, f64::NAN),
            bracket: [(x, f64::NAN); 2],
            interval_iters: 0,
            iterations: 0,
            evaluations: 0,
            procedure: "initial",
        }
    }

    fn total_iterations(&self) -> usize {
        self.interval_iters + self.iterations
    }

    fn limit(&self, options: &PolyOptOptions) -> Option<&'static str> {
        options.limit_message(self.total_iterations(), self.evaluations)
    }

    fn callback(
        &self,
        engine: &mut Engine,
        callbacks: &mut OptCallbacks<'_>,
        state: &str,
        out: &mut Execution,
    ) -> Result<bool> {
        if callbacks.is_empty() {
            return Ok(false);
        }
        let [(a, fa), (b, fb)] = self.bracket;
        callbacks.call(
            engine,
            &Matrix::scalar(self.point.0),
            OptIteration {
                evaluations: self.evaluations,
                fval: self.point.1,
                index: self.total_iterations(),
                procedure: self.procedure,
                bracket: Some((a, b, fa, fb)),
            },
            state,
            out,
        )
    }
}

struct OptOutput {
    iterations: usize,
    func_count: usize,
    algorithm: &'static str,
    message: String,
    fzero_extra: Option<(usize, [f64; 2], [f64; 2])>,
}

fn ascending(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// a < b with NaN above every number (the simplex order of `ascending`).
fn below(a: f64, b: f64) -> bool {
    ascending(a, b) == Ordering::Less
}

fn fmt_num(v: f64) -> String {
    format!("{:>13}", mat2str_real(v, 6))
}

fn char_array(engine: &Engine, s: &str) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_units(
        s.encode_utf16().collect(),
        &engine.cancel,
    )?))
}

fn make_struct(engine: &Engine, fields: Vec<(&str, Array)>) -> Result<Value> {
    let mut names = Vec::with_capacity(fields.len());
    let mut slots = Vec::with_capacity(fields.len());
    for (name, arr) in fields {
        names.push(name.to_string());
        slots.push(Arc::new(arr));
    }
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1,
        1,
        names,
        slots,
        &engine.cancel,
    )?)))
}

fn build_output_struct(engine: &Engine, info: OptOutput) -> Result<Value> {
    let mut fields = vec![
        (
            "iterations",
            Array::Numeric(Matrix::scalar(info.iterations as f64)),
        ),
        (
            "funcCount",
            Array::Numeric(Matrix::scalar(info.func_count as f64)),
        ),
        ("algorithm", char_array(engine, info.algorithm)?),
        ("message", char_array(engine, &info.message)?),
    ];
    if let Some((interval_iters, bx, by)) = info.fzero_extra {
        fields.push((
            "intervaliterations",
            Array::Numeric(Matrix::scalar(interval_iters as f64)),
        ));
        fields.push((
            "bracketx",
            Array::Numeric(Matrix::from_column_slice(1, 2, &bx)?),
        ));
        fields.push((
            "brackety",
            Array::Numeric(Matrix::from_column_slice(1, 2, &by)?),
        ));
    }
    make_struct(engine, fields)
}

fn build_optim_values(
    engine: &Engine,
    func_count: usize,
    fval: f64,
    iteration: usize,
    procedure: &str,
    fzero_bracket: Option<(f64, f64, f64, f64)>,
) -> Result<Value> {
    let mut fields = vec![
        (
            "funccount",
            Array::Numeric(Matrix::scalar(func_count as f64)),
        ),
        ("fval", Array::Numeric(Matrix::scalar(fval))),
        (
            "iteration",
            Array::Numeric(Matrix::scalar(iteration as f64)),
        ),
        ("procedure", char_array(engine, procedure)?),
    ];
    if let Some((ia, ib, fia, fib)) = fzero_bracket {
        fields.push(("intervala", Array::Numeric(Matrix::scalar(ia))));
        fields.push(("intervalb", Array::Numeric(Matrix::scalar(ib))));
        fields.push(("fintervala", Array::Numeric(Matrix::scalar(fia))));
        fields.push(("fintervalb", Array::Numeric(Matrix::scalar(fib))));
    }
    make_struct(engine, fields)
}

fn option_exponential(value: f64) -> String {
    let scientific = format!("{value:.6e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("finite tolerance");
    let exponent: i32 = exponent.parse().expect("valid exponent");
    format!("{mantissa}e{exponent:+03}")
}

fn real_objective_value(f: &FunctionHandle, value: Complex64) -> Result<f64> {
    if value.im != 0. {
        return Err(format!(
            "User function '{}' must return a real scalar value",
            f.func2str()
        ));
    }
    Ok(value.re)
}

fn zero_interval_message(bracket: [(f64, f64); 2]) -> String {
    format!(
        "Zero found in the interval [{}, {}]",
        mat2str_real(bracket[0].0, 6),
        mat2str_real(bracket[1].0, 6)
    )
}

fn zero_search_failure(x: f64, fx: Complex64) -> (f64, String) {
    let (flag, reason) = if fx.im != 0. {
        (-4., "complex function value encountered")
    } else if !fx.re.is_finite() {
        (-3., "NaN or Inf function value encountered")
    } else {
        (-6., "no sign change was found before x reached")
    };
    let message = if flag == -6. {
        format!(
            "Exiting fzero: aborting search for an interval containing a sign change\n    because {reason} {}.\nThe function may have no real root.",
            mat2str_real(x, 6)
        )
    } else {
        let value = if fx.im != 0. {
            mat2str_complex(fx.re, fx.im, 6)
        } else {
            mat2str_real(fx.re, 6)
        };
        format!(
            "Exiting fzero: aborting search for an interval containing a sign change\n    because {reason} at x = {}: f(x) = {value}.\nCheck the function, or start from another point or interval.",
            mat2str_real(x, 6)
        )
    };
    (flag, message)
}

impl Engine {
    /// The function as a handle: a handle as given (anonymous handles carry their
    /// captured variables), or the name of a function given as text.
    fn polyopt_function(&self, who: &str, value: &Value) -> Result<FunctionHandle> {
        match value {
            Value::Container(Array::Handle(handle)) => Ok(handle.clone()),
            Value::Text(name) | Value::QuotedString(name) if !name.trim().is_empty() => {
                match self.named_handle(name.trim())? {
                    Value::Container(Array::Handle(handle)) => Ok(handle),
                    _ => unreachable!("named_handle returns a handle"),
                }
            }
            Value::Container(Array::Character(c)) => {
                let name = c.to_utf8(&self.cancel)?;
                if name.trim().is_empty() {
                    return Err(format!(
                        "{who}: the function must be a function handle, e.g. {who}(@(x) x.^2 - 2, ...), or a function name as text"
                    ));
                }
                match self.named_handle(name.trim())? {
                    Value::Container(Array::Handle(handle)) => Ok(handle),
                    _ => unreachable!("named_handle returns a handle"),
                }
            }
            _ => Err(format!(
                "{who}: the function must be a function handle, e.g. {who}(@(x) x.^2 - 2, ...), or a function name as text"
            )),
        }
    }

    pub(in crate::engine) fn polyopt_parse_fcns(
        &self,
        who: &str,
        what: &str,
        value: &Value,
        dest: &mut Vec<FunctionHandle>,
    ) -> Result<()> {
        if is_empty_value(value) {
            return Ok(());
        }
        match value {
            Value::Container(Array::Handle(h)) => dest.push(h.clone()),
            Value::Container(Array::Cell(cell)) => {
                for idx in 0..cell.len() {
                    let elem = Value::from(cell.get_linear(idx)?.as_ref().clone());
                    if is_empty_value(&elem) {
                        continue;
                    }
                    dest.push(self.polyopt_function(who, &elem)?);
                }
            }
            _ => {
                if let Some(t) = onl_text(self, value) {
                    if !t.trim().is_empty() {
                        dest.push(self.polyopt_function(who, value)?);
                    }
                } else {
                    return Err(format!(
                        "{who}: {what} must be a function handle, function name, or cell array of handles"
                    ));
                }
            }
        }
        Ok(())
    }

    fn polyopt_read_options(
        &self,
        who: &str,
        numel: usize,
        value: Option<&Value>,
    ) -> Result<PolyOptOptions> {
        let mut opts = PolyOptOptions::defaults_for(who, numel);
        let Some(val) = value else {
            return Ok(opts);
        };
        if is_empty_value(val) {
            return Ok(opts);
        }
        let Value::Container(Array::Struct(s)) = val else {
            return Err(format!(
                "{who}: options must be a scalar structure from optimset or optimoptions"
            ));
        };
        if s.len() != 1 {
            return Err(format!(
                "{who}: options must be a scalar structure from optimset or optimoptions"
            ));
        }
        for (i, name) in s.field_names().iter().enumerate() {
            let field = Value::from(s.get(i, 0)?.as_ref().clone());
            if is_empty_value(&field) {
                continue;
            }
            let parse_count = |what: &str| -> Result<Option<usize>> {
                match onl_scalar(&field) {
                    Some(v) if v == f64::INFINITY => Ok(None),
                    Some(v) if v >= 0. && v.fract() == 0. => {
                        Ok(Some(v.min(usize::MAX as f64) as usize))
                    }
                    _ => Err(format!(
                        "{who}: {what} must be a nonnegative integer or Inf"
                    )),
                }
            };
            let parse_tol = |what: &str| -> Result<f64> {
                match onl_scalar(&field) {
                    Some(v) if v > 0. && v.is_finite() => Ok(v),
                    _ => Err(format!("{who}: {what} must be a finite positive scalar")),
                }
            };
            let Some(key) = canonical(name).or_else(|| polyopt_option_name(name)) else {
                continue;
            };
            match key {
                "Display" => {
                    let text = onl_text(self, &field)
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    opts.display = match text.as_str() {
                        "off" | "none" => OptDisplay::Off,
                        "final" | "final-detailed" => OptDisplay::Final,
                        "notify" | "notify-detailed" => OptDisplay::Notify,
                        "iter" | "iter-detailed" => OptDisplay::Iter,
                        _ => {
                            return Err(format!(
                                "{who}: Display must be 'off', 'final', 'notify' or 'iter'"
                            ));
                        }
                    };
                }
                "MaxIterations" => opts.max_iter = parse_count("MaxIterations")?,
                "MaxFunctionEvaluations" => {
                    opts.max_fun_evals = parse_count("MaxFunctionEvaluations")?;
                }
                "StepTolerance" => opts.tol_x = parse_tol("TolX")?,
                "FunctionTolerance" | "TolFun" => opts.tol_fun = parse_tol("TolFun")?,
                "OutputFcn" => {
                    self.polyopt_parse_fcns(who, "OutputFcn", &field, &mut opts.output_fcns)?
                }
                "PlotFcns" => {
                    self.polyopt_parse_fcns(who, "PlotFcns", &field, &mut opts.plot_fcns)?
                }
                "FunValCheck" => {
                    opts.fun_val_check = polyopt_fun_val_check(self, who, &field)?;
                }
                _ => {}
            }
        }
        Ok(opts)
    }

    /// One function value as a complex scalar (real results have im == 0).
    fn polyopt_value(
        &mut self,
        f: &FunctionHandle,
        x: Matrix,
        extra: &[Value],
        fun_val_check: bool,
        out: &mut Execution,
    ) -> Result<Complex64> {
        self.check_cancel()?;
        let mut call_args = Vec::with_capacity(1 + extra.len());
        call_args.push(Value::Matrix(x));
        call_args.extend(extra.iter().cloned());
        let value = self
            .call_handle(f, call_args, 1, None, out)?
            .into_iter()
            .next()
            .unwrap_or(Value::Void);
        let z = match value {
            Value::Matrix(m) if m.len() == 1 => Complex64::new(m.data()[0], 0.),
            Value::Logical(m) if m.len() == 1 => Complex64::new(f64::from(m.data()[0]), 0.),
            Value::Complex(m) if m.len() == 1 => m.data()[0],
            _ => {
                return Err(format!(
                    "User function '{}' must return a real scalar value",
                    f.func2str()
                ));
            }
        };
        if fun_val_check && (z.im != 0. || !z.re.is_finite()) {
            return Err(format!(
                "User function '{}' returned a non-finite or complex value",
                f.func2str()
            ));
        }
        Ok(z)
    }

    fn polyopt_f(
        &mut self,
        f: &FunctionHandle,
        x: Matrix,
        extra: &[Value],
        fun_val_check: bool,
        out: &mut Execution,
    ) -> Result<f64> {
        let v = self.polyopt_value(f, x, extra, false, out)?;
        if fun_val_check && (v.im != 0. || v.re.is_nan()) {
            return Err(format!(
                "User function '{}' returned a NaN or complex value",
                f.func2str()
            ));
        }
        real_objective_value(f, v)
    }

    fn polyopt_call_callback(
        &mut self,
        fcn: &FunctionHandle,
        x: &Matrix,
        optim_values: &Value,
        state: &Value,
        out: &mut Execution,
    ) -> Result<bool> {
        self.check_cancel()?;
        let res = self
            .call_handle(
                fcn,
                vec![
                    Value::Matrix(x.clone()),
                    optim_values.clone(),
                    state.clone(),
                ],
                1,
                None,
                out,
            )?
            .into_iter()
            .next()
            .unwrap_or(Value::Void);
        Ok(match res {
            Value::Logical(m) if !m.is_empty() => m.data()[0] != 0,
            Value::Matrix(m) if !m.is_empty() => m.data()[0] != 0.,
            _ => false,
        })
    }

    fn polyopt_emit_final(
        &mut self,
        display: OptDisplay,
        flag: f64,
        message: &str,
        out: &mut Execution,
    ) -> Result<()> {
        let show = match display {
            OptDisplay::Off => false,
            OptDisplay::Notify => flag <= 0.,
            OptDisplay::Final | OptDisplay::Iter => true,
        };
        if show && !message.is_empty() {
            for line in message.lines() {
                self.emit(out, line.to_string())?;
            }
        }
        Ok(())
    }

    pub(super) fn polyopt_optimize(
        &mut self,
        name: &str,
        args: &[Value],
        nout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let min = if name == "fminbnd" { 3 } else { 2 };
        if args.len() < min {
            return Err("Not enough input arguments.".into());
        }
        let f = self.polyopt_function(name, &args[0])?;
        let (x, fval, flag, info) = match name {
            "fzero" => {
                let opts = self.polyopt_read_options(name, 1, args.get(2))?;
                let extra = if args.len() > 3 { &args[3..] } else { &[] };
                self.polyopt_fzero(&f, &args[1], &opts, extra, out)?
            }
            "fminsearch" => {
                let start = real_matrix(&args[1], "fminsearch")?;
                if start.is_empty() {
                    return Err("fminsearch: the starting point x0 must not be empty".into());
                }
                let opts = self.polyopt_read_options(name, start.len(), args.get(2))?;
                let extra = if args.len() > 3 { &args[3..] } else { &[] };
                self.polyopt_fminsearch(&f, &start, &opts, extra, out)?
            }
            _ => {
                let ax = scalar_of(&args[1], name, "the lower bound")?;
                let bx = scalar_of(&args[2], name, "the upper bound")?;
                let opts = self.polyopt_read_options(name, 1, args.get(3))?;
                let extra = if args.len() > 4 { &args[4..] } else { &[] };
                if ax > bx {
                    let message =
                        "Exiting due to infeasibility:  lower bound exceeds the corresponding upper bound."
                            .to_string();
                    self.polyopt_emit_final(opts.display, -2., &message, out)?;
                    let empty = matrix_value(0, 0, Vec::new())?;
                    let mut outputs = vec![empty.clone()];
                    if nout > 1 {
                        outputs.push(empty);
                    }
                    if nout > 2 {
                        outputs.push(matrix_value(1, 1, vec![-2.])?);
                    }
                    if nout > 3 {
                        outputs.push(build_output_struct(
                            self,
                            OptOutput {
                                iterations: 0,
                                func_count: 0,
                                algorithm: "golden section search, parabolic interpolation",
                                message,
                                fzero_extra: None,
                            },
                        )?);
                    }
                    return Ok(outputs);
                }
                self.polyopt_fminbnd(&f, ax, bx, &opts, extra, out)?
            }
        };
        let mut outputs = vec![Value::Matrix(x)];
        if nout > 1 {
            outputs.push(matrix_value(1, 1, vec![fval])?);
        }
        if nout > 2 {
            outputs.push(matrix_value(1, 1, vec![flag])?);
        }
        if nout > 3 {
            outputs.push(build_output_struct(self, info)?);
        }
        Ok(outputs)
    }

    /// PlotFcns helpers: a bar chart of x and a single function-value history line.
    pub(super) fn polyopt_optimplot(
        &mut self,
        name: &str,
        args: &[Value],
        nout: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if args.len() < 3 {
            return Err("Not enough input arguments.".into());
        }
        let state = onl_text(self, &args[2])
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(state.as_str(), "init" | "iter" | "done") {
            let text = |s: &str| Value::Text(s.to_owned());
            if name == "optimplotx" {
                self.call("hold", vec![text("off")], out)?;
                let x = real_matrix(&args[0], name)?;
                self.call(
                    "bar",
                    vec![Value::Matrix(Matrix::from_column_slice(
                        1,
                        x.len(),
                        x.data(),
                    )?)],
                    out,
                )?;
                self.call("title", vec![text("Current Point")], out)?;
                self.call("xlabel", vec![text("Variable number")], out)?;
                self.call("ylabel", vec![text("Current point")], out)?;
            } else {
                let get_field = |name: &str| -> Option<f64> {
                    let Value::Container(Array::Struct(values)) = &args[1] else {
                        return None;
                    };
                    let index = values
                        .field_names()
                        .iter()
                        .position(|field| field.eq_ignore_ascii_case(name))?;
                    onl_scalar(&Value::from(values.get(index, 0).ok()?.as_ref().clone()))
                };
                let iter = get_field("iteration").unwrap_or(0.);
                let fval = get_field("fval").unwrap_or(0.);
                if state != "init"
                    && let Some(figure) = &self.figure
                    && let [series] = figure.series.as_slice()
                    && series.kind == crate::plot::PlotKind::Line
                {
                    let last = series
                        .points
                        .len()
                        .checked_sub(1)
                        .and_then(|i| series.points.get(i));
                    let same_iteration = last.is_some_and(|[x, _]| x == iter);
                    let update = if same_iteration {
                        last.is_some_and(|[_, y]| y.to_bits() != fval.to_bits())
                    } else {
                        state == "iter" && figure.point_count() < crate::plot::MAX_POINTS
                    };
                    if update {
                        let handle = self.graphics_series_result(figure, 0)?;
                        let (mut x, mut y): (Vec<_>, Vec<_>) =
                            series.points.iter().map(|[x, y]| (x, y)).unzip();
                        if same_iteration {
                            *y.last_mut().expect("history has a last point") = fval;
                        } else {
                            x.push(iter);
                            y.push(fval);
                        }
                        self.call(
                            "set",
                            vec![
                                handle,
                                text("XData"),
                                matrix_value(1, x.len(), x)?,
                                text("YData"),
                                matrix_value(1, y.len(), y)?,
                            ],
                            out,
                        )?;
                    }
                } else if state != "done" {
                    self.call("hold", vec![text("off")], out)?;
                    self.call(
                        "plot",
                        vec![
                            Value::Matrix(Matrix::scalar(iter)),
                            Value::Matrix(Matrix::scalar(fval)),
                            text("-kd"),
                        ],
                        out,
                    )?;
                    self.call("xlabel", vec![text("Iteration")], out)?;
                    self.call("ylabel", vec![text("Function value")], out)?;
                }
                self.call(
                    "title",
                    vec![Value::Text(format!(
                        "Current Function Value: {}",
                        mat2str_real(fval, 6)
                    ))],
                    out,
                )?;
            }
        }
        let stop = Value::Logical(LogicalMatrix::scalar(false));
        Ok(if nout == 0 { Vec::new() } else { vec![stop] })
    }

    /// fzero on a supplied bracket or on a bracket found from a scalar start.
    fn polyopt_fzero(
        &mut self,
        f: &FunctionHandle,
        x0: &Value,
        opts: &PolyOptOptions,
        extra: &[Value],
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64, OptOutput)> {
        let start = real_matrix(x0, "fzero")?;
        let coords = start.data();
        if !(1..=2).contains(&coords.len()) {
            return Err(
                "fzero: x0 must be a starting point or a two-element interval [a b]".into(),
            );
        }
        let scalar_start = coords.len() == 1;
        let endpoint_error = if scalar_start {
            "fzero: the function value at the starting point must be finite and real"
        } else {
            "fzero: the function values at the interval endpoints must be finite and real"
        };
        if coords.iter().any(|x| !x.is_finite()) {
            return Err(if scalar_start {
                "fzero: the starting point must be finite"
            } else {
                "fzero: the interval endpoints must be finite"
            }
            .into());
        }
        let mut progress = ZeroProgress::new(coords[0]);
        let mut endpoints_evaluated = 0;
        for (index, &x) in coords.iter().enumerate() {
            if opts.evaluations_exhausted(progress.evaluations) {
                break;
            }
            let fx = self.polyopt_value(f, Matrix::scalar(x), extra, opts.fun_val_check, out)?;
            progress.evaluations += 1;
            endpoints_evaluated += 1;
            if fx.im != 0. || !fx.re.is_finite() {
                return Err(endpoint_error.into());
            }
            let point = (x, fx.re);
            if index == 0 {
                progress.bracket = [point; 2];
            } else {
                progress.bracket[index] = point;
            }
            if progress.point.1 != 0. {
                progress.point = point;
            }
        }
        let [(_, fa), (_, fb)] = progress.bracket;
        let limit = progress.limit(opts);
        // Both endpoints evaluated: the sign check wins over a limit (Octave 10.3).
        let both = endpoints_evaluated == coords.len();
        if !scalar_start
            && (both || limit.is_none())
            && fa != 0.
            && fb != 0.
            && (fa > 0.) == (fb > 0.)
        {
            return Err(
                "fzero: the function values at the interval endpoints must differ in sign".into(),
            );
        }
        if scalar_start && opts.display == OptDisplay::Iter {
            self.emit(
                out,
                format!(
                    "Search for an interval around {} containing a sign change:",
                    mat2str_real(coords[0], 6)
                ),
            )?;
            self.emit(
                out,
                " Func-count    a          f(a)             b          f(b)        Procedure"
                    .to_owned(),
            )?;
            self.polyopt_zero_interval_row(&progress, "initial interval", out)?;
        }
        let mut callbacks = OptCallbacks::new(opts);
        let stopped = progress.callback(self, &mut callbacks, "init", out)?;
        let outcome = if let Some(message) = limit {
            Some((0., message.to_owned()))
        } else if stopped {
            Some((-1., CALLBACK_STOP.to_owned()))
        } else if progress.point.1 == 0. {
            let message = if scalar_start {
                format!("Zero found at x = {}", mat2str_real(progress.point.0, 6))
            } else {
                zero_interval_message(progress.bracket)
            };
            progress.bracket = [progress.point; 2];
            Some((1., message))
        } else if scalar_start {
            self.polyopt_fzero_search(f, extra, &mut progress, &mut callbacks, out)?
        } else {
            None
        };
        let (flag, message) = match outcome {
            Some(outcome) => outcome,
            None => self.polyopt_fzero_brent(f, extra, &mut progress, &mut callbacks, out)?,
        };
        progress.callback(self, &mut callbacks, "done", out)?;
        self.polyopt_emit_final(opts.display, flag, &message, out)?;
        let (x, fx) = progress.point;
        let [(a, fa), (b, fb)] = progress.bracket;
        let (bracketx, brackety) = if a <= b {
            ([a, b], [fa, fb])
        } else {
            ([b, a], [fb, fa])
        };
        Ok((
            Matrix::scalar(x),
            fx,
            flag,
            OptOutput {
                iterations: progress.iterations,
                func_count: progress.evaluations,
                algorithm: "bisection, interpolation",
                message,
                fzero_extra: Some((progress.interval_iters, bracketx, brackety)),
            },
        ))
    }

    /// Search symmetrically, keeping the last same-sign sample on each side.
    fn polyopt_fzero_search(
        &mut self,
        f: &FunctionHandle,
        extra: &[Value],
        progress: &mut ZeroProgress,
        callbacks: &mut OptCallbacks<'_>,
        out: &mut Execution,
    ) -> Result<Option<(f64, String)>> {
        let opts = callbacks.options;
        let (x, fx) = progress.point;
        let mut dx = x.abs() / 50.;
        if dx < f64::MIN_POSITIVE {
            dx = 1. / 50.;
        }
        loop {
            progress.interval_iters += 1;
            progress.procedure = "search";
            let mut bracketed = false;
            let mut failure = None;
            for (side, probe) in [x - dx, x + dx].into_iter().enumerate() {
                if opts.evaluations_exhausted(progress.evaluations) {
                    break;
                }
                let last = progress.bracket[side];
                if !probe.is_finite() {
                    failure = Some((probe, Complex64::new(last.1, 0.)));
                    break;
                }
                let fp =
                    self.polyopt_value(f, Matrix::scalar(probe), extra, opts.fun_val_check, out)?;
                progress.evaluations += 1;
                if fp.im != 0. || !fp.re.is_finite() {
                    failure = Some((probe, fp));
                    break;
                }
                progress.point = (probe, fp.re);
                if fp.re == 0. || (fp.re > 0.) != (fx > 0.) {
                    progress.bracket = [last, progress.point];
                    if side == 0 {
                        progress.bracket.swap(0, 1);
                    }
                    bracketed = true;
                    break;
                }
                progress.bracket[side] = progress.point;
            }
            if opts.display == OptDisplay::Iter {
                self.polyopt_zero_interval_row(progress, "search", out)?;
            }
            let stopped = progress.callback(self, callbacks, "iter", out)?;
            if let Some(message) = progress.limit(opts) {
                return Ok(Some((0., message.to_owned())));
            }
            if stopped {
                return Ok(Some((-1., CALLBACK_STOP.to_owned())));
            }
            if let Some((probe, value)) = failure {
                progress.point = (f64::NAN, f64::NAN);
                return Ok(Some(zero_search_failure(probe, value)));
            }
            if bracketed {
                if progress.point.1 == 0. {
                    progress.bracket = [progress.point; 2];
                    return Ok(Some((
                        1.,
                        format!("Zero found at x = {}", mat2str_real(progress.point.0, 6)),
                    )));
                }
                return Ok(None);
            }
            dx *= std::f64::consts::SQRT_2;
        }
    }

    fn polyopt_zero_interval_row(
        &mut self,
        progress: &ZeroProgress,
        procedure: &str,
        out: &mut Execution,
    ) -> Result<()> {
        let [(a, fa), (b, fb)] = progress.bracket;
        self.emit(
            out,
            format!(
                " {:5}   {}   {}   {}   {}   {procedure}",
                progress.evaluations,
                fmt_num(a),
                fmt_num(fa),
                fmt_num(b),
                fmt_num(fb)
            ),
        )
    }

    fn polyopt_fzero_brent(
        &mut self,
        f: &FunctionHandle,
        extra: &[Value],
        progress: &mut ZeroProgress,
        callbacks: &mut OptCallbacks<'_>,
        out: &mut Execution,
    ) -> Result<(f64, String)> {
        let opts = callbacks.options;
        let initial_bracket = progress.bracket;
        let [mut left, mut right] = initial_bracket;
        if right != progress.point {
            std::mem::swap(&mut left, &mut right);
        }
        let (mut a, mut fa) = left;
        let (mut b, mut fb) = right;
        let (mut c, mut fc) = left;
        let mut d = b - a;
        let mut e = d;

        if opts.display == OptDisplay::Iter {
            self.emit(
                out,
                format!(
                    "Search for a zero in the interval [{}, {}]:",
                    mat2str_real(a, 6),
                    mat2str_real(b, 6)
                ),
            )?;
            self.emit(
                out,
                " Func-count    x          f(x)             Procedure".to_owned(),
            )?;
            self.emit(
                out,
                format!(
                    " {:5}   {}   {}   initial",
                    progress.evaluations,
                    fmt_num(b),
                    fmt_num(fb)
                ),
            )?;
        }

        loop {
            if fc.abs() < fb.abs() {
                (a, fa) = (b, fb);
                (b, fb) = (c, fc);
                (c, fc) = (a, fa);
            }
            progress.point = (b, fb);
            progress.bracket = [(b, fb), (c, fc)];
            if let Some(message) = progress.limit(opts) {
                return Ok((0., message.to_owned()));
            }
            let tol = 2. * f64::EPSILON * b.abs() + 0.5 * opts.tol_x;
            let width = c - b;
            // Halve the width before taking the midpoint step, with a fallback
            // for wide brackets. The full width and adjacent-float checks also
            // cover subnormal brackets where half of TolX can round to zero.
            let m = if width.is_finite() {
                0.5 * width
            } else {
                0.5 * c - 0.5 * b
            };
            if fb == 0.
                || width.abs() <= opts.tol_x
                || m.abs() <= tol
                || b.min(c).next_up() >= b.max(c)
            {
                let initial_residual = initial_bracket[0].1.abs().max(initial_bracket[1].1.abs());
                if fb.abs() > initial_residual {
                    return Ok((
                        -5.,
                        format!(
                            "fzero: x may be near a singular point: the interval around the sign change\n    shrank to the tolerance, but |f(x)| grew from {} to {} as it shrank.",
                            mat2str_real(initial_residual, 6),
                            mat2str_real(fb.abs(), 6)
                        ),
                    ));
                }
                if fb == 0. {
                    progress.bracket = [progress.point; 2];
                }
                return Ok((1., zero_interval_message(initial_bracket)));
            }
            let mut proc_name = "bisection";
            if e.abs() < tol || fa.abs() <= fb.abs() {
                d = m;
                e = m;
            } else {
                let s = fb / fa;
                let (mut p, mut q) = if a == c {
                    (2. * m * s, 1. - s)
                } else {
                    let (q, r) = (fa / fc, fb / fc);
                    (
                        s * (2. * m * q * (q - r) - (b - a) * (r - 1.)),
                        (q - 1.) * (r - 1.) * (s - 1.),
                    )
                };
                if p > 0. {
                    q = -q;
                } else {
                    p = -p;
                }
                let before_last = e;
                e = d;
                if 2. * p < 3. * m * q - (tol * q).abs() && p < (0.5 * before_last * q).abs() {
                    d = p / q;
                    proc_name = "interpolation";
                } else {
                    d = m;
                    e = m;
                }
            }
            (a, fa) = (b, fb);
            b += if d.abs() > tol { d } else { tol.copysign(m) };
            if b == a {
                b = if c > a { a.next_up() } else { a.next_down() };
            }
            let mut probes = 0;
            loop {
                let value =
                    self.polyopt_value(f, Matrix::scalar(b), extra, opts.fun_val_check, out)?;
                progress.evaluations += 1;
                fb = real_objective_value(f, value)?;
                if !fb.is_nan() {
                    break;
                }
                if let Some(message) = progress.limit(opts) {
                    return Ok((0., message.to_owned()));
                }
                let next = b.midpoint(c);
                probes += 1;
                if probes > FZERO_NAN_PROBES || next == b || next == c {
                    progress.point = (f64::NAN, f64::NAN);
                    return Ok((
                        -3.,
                        format!(
                            "Exiting fzero: the function returned NaN inside the interval\n    around x = {} and no finite value was found next to it.",
                            mat2str_real(b, 6)
                        ),
                    ));
                }
                b = next;
            }
            if (fb > 0.) != (fa > 0.) {
                (c, fc) = (a, fa);
                d = b - a;
                e = d;
            } else if probes > 0 {
                d = c - b;
                e = d;
            }
            progress.iterations += 1;
            progress.point = (b, fb);
            progress.bracket = [(b, fb), (c, fc)];
            progress.procedure = proc_name;
            if opts.display == OptDisplay::Iter {
                self.emit(
                    out,
                    format!(
                        " {:5}   {}   {}   {proc_name}",
                        progress.evaluations,
                        fmt_num(b),
                        fmt_num(fb)
                    ),
                )?;
            }
            if progress.callback(self, callbacks, "iter", out)? && progress.limit(opts).is_none() {
                return Ok((-1., CALLBACK_STOP.to_owned()));
            }
        }
    }

    /// Nelder-Mead on the vertices of a simplex (Lagarias et al. 1998, sec. 2).
    fn polyopt_fminsearch(
        &mut self,
        f: &FunctionHandle,
        start: &Matrix,
        opts: &PolyOptOptions,
        extra: &[Value],
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64, OptOutput)> {
        let (rows, cols, n) = (start.rows(), start.cols(), start.len());
        if (n + 1).saturating_mul(n).saturating_mul(8) > BUDGET {
            return Err(format!(
                "fminsearch: x0 has {n} elements; the simplex of {} points would exceed the memory budget",
                n + 1
            ));
        }
        let mut callbacks = OptCallbacks::new(opts);
        let mut evaluations = 0;
        let initial_fval = if opts.evaluations_exhausted(evaluations) {
            f64::NAN
        } else {
            let value = self.polyopt_f(f, start.clone(), extra, opts.fun_val_check, out)?;
            evaluations += 1;
            value
        };
        if opts.display == OptDisplay::Iter {
            self.emit(
                out,
                " Iteration   Func-count     min f(x)         Procedure".to_owned(),
            )?;
            self.polyopt_simplex_row(0, evaluations, initial_fval, "", out)?;
        }
        let stopped = callbacks.call(
            self,
            start,
            OptIteration {
                evaluations,
                fval: initial_fval,
                index: 0,
                procedure: "initial",
                bracket: None,
            },
            "init",
            out,
        )?;
        let mut simplex = Vec::new();
        simplex.push((initial_fval, start.data().to_vec()));
        let mut steps = 0;
        let mut last_proc = "initial";
        let mut flag = if stopped && opts.limit_message(steps, evaluations).is_none() {
            -1.
        } else {
            0.
        };
        let evaluate_vertex =
            |engine: &mut Self, v: &[f64], evaluations: &mut usize, out: &mut Execution| {
                *evaluations += 1;
                engine.polyopt_f(
                    f,
                    Matrix::from_column_slice(rows, cols, v)?,
                    extra,
                    opts.fun_val_check,
                    out,
                )
            };
        if !stopped && opts.limit_message(steps, evaluations).is_none() {
            simplex.reserve_exact(n);
            for (k, &value) in start.data().iter().enumerate() {
                if opts.evaluations_exhausted(evaluations) {
                    break;
                }
                let mut v = start.data().to_vec();
                v[k] = if value == 0. { 0.00025 } else { 1.05 * value };
                let fv = evaluate_vertex(self, &v, &mut evaluations, out)?;
                simplex.push((fv, v));
            }
            simplex.sort_by(|p, q| ascending(p.0, q.0));
            if simplex.len() == n + 1 {
                steps = 1;
                last_proc = "initial simplex";
                if self.polyopt_simplex_iteration(
                    &mut callbacks,
                    rows,
                    cols,
                    &simplex[0],
                    steps,
                    evaluations,
                    last_proc,
                    out,
                )? {
                    flag = -1.;
                }
            }
        }

        while flag == 0. && opts.limit_message(steps, evaluations).is_none() {
            let (f_best, best) = (simplex[0].0, &simplex[0].1);
            if simplex[1..].iter().all(|(fv, v)| {
                (fv - f_best).abs() <= opts.tol_fun
                    && v.iter().zip(best).all(|(x, y)| (x - y).abs() <= opts.tol_x)
            }) {
                flag = 1.;
                break;
            }
            let mut m = vec![0.; n];
            for (_, v) in &simplex[..n] {
                for (s, x) in m.iter_mut().zip(v) {
                    *s += x;
                }
            }
            for s in &mut m {
                *s /= n as f64;
            }
            let (f_worst, worst) = &simplex[n];
            let (f_second_worst, f_worst) = (simplex[n - 1].0, *f_worst);
            let line = |t: f64| -> Vec<f64> {
                m.iter()
                    .zip(worst)
                    .map(|(c, w)| (1. + t) * c - t * w)
                    .collect()
            };
            let reflected = line(1.);
            let f_reflected = evaluate_vertex(self, &reflected, &mut evaluations, out)?;
            let mut proc_name = "reflect";
            let accepted = if below(f_reflected, f_best) {
                // Keep an improving reflection if the budget does not permit an expansion.
                if opts.evaluations_exhausted(evaluations) {
                    Some((f_reflected, reflected))
                } else {
                    let expanded = line(2.);
                    let f_expanded = evaluate_vertex(self, &expanded, &mut evaluations, out)?;
                    if below(f_expanded, f_reflected) {
                        proc_name = "expand";
                        Some((f_expanded, expanded))
                    } else {
                        Some((f_reflected, reflected))
                    }
                }
            } else if below(f_reflected, f_second_worst) {
                Some((f_reflected, reflected))
            } else {
                if opts.evaluations_exhausted(evaluations) {
                    break;
                }
                let outside = below(f_reflected, f_worst);
                let contracted = line(if outside { 0.5 } else { -0.5 });
                let f_contracted = evaluate_vertex(self, &contracted, &mut evaluations, out)?;
                if outside && !below(f_reflected, f_contracted) {
                    proc_name = "contract outside";
                    Some((f_contracted, contracted))
                } else if !outside && below(f_contracted, f_worst) {
                    proc_name = "contract inside";
                    Some((f_contracted, contracted))
                } else {
                    None
                }
            };
            let mut completed = true;
            if let Some(vertex) = accepted {
                simplex[n] = vertex;
            } else {
                proc_name = "shrink";
                let best = simplex[0].1.clone();
                for (fv, v) in &mut simplex[1..] {
                    if opts.evaluations_exhausted(evaluations) {
                        completed = false;
                        break;
                    }
                    for (x, y) in v.iter_mut().zip(&best) {
                        *x = y + 0.5 * (*x - y);
                    }
                    *fv = evaluate_vertex(self, v, &mut evaluations, out)?;
                }
            }
            simplex.sort_by(|p, q| ascending(p.0, q.0));
            if !completed {
                break;
            }
            steps += 1;
            last_proc = proc_name;
            if self.polyopt_simplex_iteration(
                &mut callbacks,
                rows,
                cols,
                &simplex[0],
                steps,
                evaluations,
                last_proc,
                out,
            )? {
                flag = -1.;
            }
        }

        let (fval, x_vec) = simplex.swap_remove(0);
        let x_mat = Matrix::from_column_vec(rows, cols, x_vec)?;
        let message = if flag == 1. {
            format!(
                "Optimization terminated:\n the current x satisfies the termination criteria using OPTIONS.TolX of {}\n and F(X) satisfies the convergence criteria using OPTIONS.TolFun of {}",
                option_exponential(opts.tol_x),
                option_exponential(opts.tol_fun)
            )
        } else if flag == -1. {
            CALLBACK_STOP.to_owned()
        } else {
            let limit = opts
                .limit_message(steps, evaluations)
                .expect("a limit was reached");
            format!("{limit}\n         Current function value: {fval:.6}")
        };
        callbacks.call(
            self,
            &x_mat,
            OptIteration {
                evaluations,
                fval,
                index: steps,
                procedure: last_proc,
                bracket: None,
            },
            "done",
            out,
        )?;
        self.polyopt_emit_final(opts.display, flag, &message, out)?;
        Ok((
            x_mat,
            fval,
            flag,
            OptOutput {
                iterations: steps,
                func_count: evaluations,
                algorithm: "Nelder-Mead simplex direct search",
                message,
                fzero_extra: None,
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn polyopt_simplex_iteration(
        &mut self,
        callbacks: &mut OptCallbacks<'_>,
        rows: usize,
        cols: usize,
        best: &(f64, Vec<f64>),
        steps: usize,
        evaluations: usize,
        procedure: &'static str,
        out: &mut Execution,
    ) -> Result<bool> {
        if callbacks.options.display == OptDisplay::Iter {
            self.polyopt_simplex_row(steps, evaluations, best.0, procedure, out)?;
        }
        if callbacks.is_empty() {
            return Ok(false);
        }
        let stopped = callbacks.call(
            self,
            &Matrix::from_column_slice(rows, cols, &best.1)?,
            OptIteration {
                evaluations,
                fval: best.0,
                index: steps,
                procedure,
                bracket: None,
            },
            "iter",
            out,
        )?;
        Ok(stopped
            && callbacks
                .options
                .limit_message(steps, evaluations)
                .is_none())
    }

    fn polyopt_simplex_row(
        &mut self,
        steps: usize,
        evaluations: usize,
        fval: f64,
        procedure: &str,
        out: &mut Execution,
    ) -> Result<()> {
        let suffix = if procedure.is_empty() {
            String::new()
        } else {
            format!("         {procedure}")
        };
        self.emit(
            out,
            format!(
                " {steps:5}        {evaluations:5}     {}{suffix}",
                fmt_num(fval)
            ),
        )
    }

    /// Brent's minimiser on [ax, bx] (Brent 1973, ch. 5 procedure localmin;
    /// Forsythe, Malcolm & Moler 1977, sec. 8.2).
    fn polyopt_fminbnd(
        &mut self,
        f: &FunctionHandle,
        ax: f64,
        bx: f64,
        opts: &PolyOptOptions,
        extra: &[Value],
        out: &mut Execution,
    ) -> Result<(Matrix, f64, f64, OptOutput)> {
        if !(bx - ax).is_finite() {
            return Err("fminbnd: the bounds must be finite".into());
        }
        let golden = 0.5 * (3. - 5f64.sqrt());
        let root_eps = f64::EPSILON.sqrt();
        let (mut a, mut b) = (ax, bx);
        let mut x = a + golden * (b - a);
        let mut func_count = 0;
        let mut fx = if opts.evaluations_exhausted(func_count) {
            f64::NAN
        } else {
            func_count += 1;
            self.polyopt_f(f, Matrix::scalar(x), extra, opts.fun_val_check, out)?
        };
        let (mut w, mut fw, mut v, mut fv) = (x, fx, x, fx);
        let (mut d, mut e) = (0f64, 0f64);
        let mut iterations = 0usize;

        if opts.display == OptDisplay::Iter {
            self.emit(
                out,
                " Func-count     x          f(x)         Procedure".to_string(),
            )?;
            self.emit(
                out,
                format!(
                    " {func_count:5}     {}  {}        initial",
                    fmt_num(x),
                    fmt_num(fx)
                ),
            )?;
        }
        let mut callbacks = OptCallbacks::new(opts);
        let mut stopped = callbacks.call(
            self,
            &Matrix::scalar(x),
            OptIteration {
                evaluations: func_count,
                fval: fx,
                index: 0,
                procedure: "initial",
                bracket: None,
            },
            "init",
            out,
        )?;
        let mut last_proc = "initial";
        let (flag, message) = loop {
            if let Some(message) = opts.limit_message(iterations, func_count) {
                break (0., message.to_owned());
            }
            if stopped {
                break (-1., CALLBACK_STOP.to_owned());
            }
            let mid = 0.5 * (a + b);
            let tol = root_eps * x.abs() + opts.tol_x / 3.;
            let tol2 = 2. * tol;
            if (x - mid).abs() <= tol2 - 0.5 * (b - a) {
                let msg = format!(
                    "Optimization terminated:\n the current x satisfies the termination criteria using OPTIONS.TolX of {}",
                    option_exponential(opts.tol_x)
                );
                break (1., msg);
            }
            let mut parabolic = false;
            if e.abs() > tol {
                let r = (x - w) * (fx - fv);
                let mut q = (x - v) * (fx - fw);
                let mut p = (x - v) * q - (x - w) * r;
                q = 2. * (q - r);
                if q > 0. {
                    p = -p;
                } else {
                    q = -q;
                }
                let before_last = e;
                e = d;
                if p.abs() < (0.5 * q * before_last).abs() && p > q * (a - x) && p < q * (b - x) {
                    d = p / q;
                    let u = x + d;
                    if u - a < tol2 || b - u < tol2 {
                        d = if x < mid { tol } else { -tol };
                    }
                    parabolic = true;
                }
            }
            if !parabolic {
                e = if x < mid { b - x } else { a - x };
                d = golden * e;
            }
            let u = x + if d.abs() >= tol {
                d
            } else if d > 0. {
                tol
            } else {
                -tol
            };
            let fu = self.polyopt_f(f, Matrix::scalar(u), extra, opts.fun_val_check, out)?;
            func_count += 1;
            iterations += 1;
            if fu <= fx {
                if u < x {
                    b = x;
                } else {
                    a = x;
                }
                (v, fv) = (w, fw);
                (w, fw) = (x, fx);
                (x, fx) = (u, fu);
            } else {
                if u < x {
                    a = u;
                } else {
                    b = u;
                }
                if fu <= fw || w == x {
                    (v, fv) = (w, fw);
                    (w, fw) = (u, fu);
                } else if fu <= fv || v == x || v == w {
                    (v, fv) = (u, fu);
                }
            }
            let proc_name = if parabolic { "parabolic" } else { "golden" };
            last_proc = proc_name;
            if opts.display == OptDisplay::Iter {
                self.emit(
                    out,
                    format!(
                        " {func_count:5}     {}  {}        {proc_name}",
                        fmt_num(u),
                        fmt_num(fu)
                    ),
                )?;
            }
            stopped = callbacks.call(
                self,
                &Matrix::scalar(x),
                OptIteration {
                    evaluations: func_count,
                    fval: fx,
                    index: iterations,
                    procedure: proc_name,
                    bracket: None,
                },
                "iter",
                out,
            )?;
        };

        let x_mat = Matrix::scalar(x);
        callbacks.call(
            self,
            &x_mat,
            OptIteration {
                evaluations: func_count,
                fval: fx,
                index: iterations,
                procedure: last_proc,
                bracket: None,
            },
            "done",
            out,
        )?;
        self.polyopt_emit_final(opts.display, flag, &message, out)?;
        Ok((
            x_mat,
            fx,
            flag,
            OptOutput {
                iterations,
                func_count,
                algorithm: "golden section search, parabolic interpolation",
                message,
                fzero_extra: None,
            },
        ))
    }
}
