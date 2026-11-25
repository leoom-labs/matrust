//! Parse once, borrow loop bodies, and bound execution/output independently of UI.
use super::{Engine, Execution, Value, limits, scalar_loops, user_functions::nested};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
    logical::BooleanOp,
    syntax::{Diagnostic, Evaluation, Expr, Statement, StatementKind, Token},
};

pub(super) enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

enum IterationValues {
    Range {
        start: f64,
        step: f64,
        count: usize,
        character: bool,
    },
    Array(Array),
}
impl IterationValues {
    fn count(&self) -> usize {
        match self {
            Self::Range { count, .. } => *count,
            Self::Array(a) => {
                if a.is_empty() {
                    0
                } else {
                    a.cols()
                }
            }
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Range { .. } => 0,
            Self::Array(a) => a.retained_bytes(),
        }
    }
    fn column(&self, i: usize, engine: &Engine) -> Result<Value> {
        match self {
            Self::Range { start, step, character, .. } => {
                let value = start + *step * i as f64;
                if *character { engine.character_range(value, 1., 1) }
                else { Ok(Value::Matrix(Matrix::scalar(value))) }
            }
            Self::Array(
                Array::Character(_) | Array::Cell(_) | Array::Graphics(_) | Array::Struct(_) | Array::Handle(_) | Array::Table(_),
            ) => Err("Iteration over character, cell, struct, function handle, graphics handle or table arrays is not implemented".into()),
            Self::Array(a) if a.cols() == 1 => Ok(a.clone().into()),
            Self::Array(Array::Str(s)) => {
                let mut data = Vec::with_capacity(s.rows());
                for item in &s.data()[i * s.rows()..(i + 1) * s.rows()] {
                    engine.check_cancel()?;
                    data.push(item.clone());
                }
                Ok(super::string_class::value(crate::StringArray::new(s.rows(), 1, data)?))
            }
            Self::Array(Array::Sparse(s)) => Ok(Value::Container(Array::Sparse(s.select(
                crate::sparse::Rows::All,
                &[i],
                &engine.cancel,
            )?))),
            Self::Array(Array::Typed(t)) => {
                let positions: Vec<usize> = (i * t.rows()..(i + 1) * t.rows()).collect();
                Ok(Value::Container(t.select(t.rows(), 1, &positions)?.into()))
            }
            Self::Array(Array::Categorical(t)) => {
                let positions: Vec<usize> = (i * t.rows()..(i + 1) * t.rows()).collect();
                Ok(Value::Container(t.select(t.rows(), 1, &positions)?.into()))
            }
            Self::Array(Array::Datetime(t)) => {
                let positions: Vec<usize> = (i * t.rows()..(i + 1) * t.rows()).collect();
                Ok(Value::Container(t.select(t.rows(), 1, &positions)?.into()))
            }
            Self::Array(Array::Duration(t)) => {
                let positions: Vec<usize> = (i * t.rows()..(i + 1) * t.rows()).collect();
                Ok(Value::Container(t.select(t.rows(), 1, &positions)?.into()))
            }
            Self::Array(Array::Complex(m)) => {
                Ok(Value::Complex(crate::ComplexMatrix::from_column_slice(
                    m.rows(),
                    1,
                    &m.data()[i * m.rows()..(i + 1) * m.rows()],
                    &engine.cancel,
                )?))
            }
            Self::Array(Array::Numeric(m)) => {
                let mut values = Vec::with_capacity(m.rows());
                for chunk in m.data()[i * m.rows()..(i + 1) * m.rows()].chunks(8192) {
                    engine.check_cancel()?;
                    values.extend_from_slice(chunk);
                }
                Ok(Value::Matrix(Matrix::from_column_vec(m.rows(), 1, values)?))
            }
            Self::Array(Array::Logical(m)) => {
                let mut values = Vec::with_capacity(m.rows());
                for chunk in m.data()[i * m.rows()..(i + 1) * m.rows()].chunks(8192) {
                    engine.check_cancel()?;
                    values.extend_from_slice(chunk);
                }
                Ok(Value::Logical(LogicalMatrix::from_canonical_vec(
                    m.rows(),
                    1,
                    values,
                )?))
            }
        }
    }
    fn empty(&self, engine: &Engine) -> Result<Value> {
        match self {
            Self::Range {
                character: true, ..
            } => engine.character_range(0., 1., 0),
            Self::Range { .. } => Ok(Value::Matrix(Matrix::from_column_vec(1, 0, Vec::new())?)),
            Self::Array(a) => Ok(a.clone().into()),
        }
    }
}

impl Engine {
    pub(super) fn control_step(&mut self, at: usize, byte: &mut usize) -> Result<()> {
        *byte = at;
        self.lg3_step(at);
        self.check_cancel()?;
        self.limits_count_step()?;
        self.evaluated_elements = 0;
        Ok(())
    }
    /// One unit of the step budget for a loop over user data (no statement reset).
    pub(super) fn count_step(&mut self) -> Result<()> {
        self.check_cancel()?;
        self.limits_count_step()?;
        Ok(())
    }
    /// Fail before a loop of `steps` iterations that cannot fit the step budget.
    pub(super) fn admit_steps(&self, steps: usize) -> Result<()> {
        if let Some(max) = self.limits.steps
            && self.control_steps.saturating_add(steps) > max
        {
            return Err(limits::steps_error(max));
        }
        Ok(())
    }
    /// Count one step against the active step limit (none on desktop and CLI).
    fn limits_count_step(&mut self) -> Result<()> {
        if let Some(max) = self.limits.steps
            && self.control_steps >= max
        {
            return Err(limits::steps_error(max));
        }
        self.control_steps = self.control_steps.saturating_add(1);
        Ok(())
    }
    fn charge_output(&mut self, bytes: usize) -> Result<()> {
        let entries_full = self
            .limits
            .output_entries
            .is_some_and(|max| self.emitted_entries >= max);
        let bytes_full = self
            .limits
            .output_bytes
            .is_some_and(|max| bytes > max.saturating_sub(self.emitted_bytes));
        if entries_full || bytes_full {
            return Err(limits::output_error(&self.limits));
        }
        self.emitted_bytes = self.emitted_bytes.saturating_add(bytes);
        Ok(())
    }
    pub(super) fn emit(&mut self, out: &mut Execution, mut text: String) -> Result<()> {
        // Held `fprintf` text was charged when it was held.
        self.charge_output(text.len())?;
        self.emitted_entries += 1;
        // Held text reached the diary when it was held.
        let diary = self.lg3_diary_write(&text, true);
        text.insert_str(0, &std::mem::take(&mut self.pending_line));
        out.output.push(text);
        if out.stream {
            self.hostio_show_output(out)?;
        }
        diary
    }
    /// Hold `fprintf` text that has no newline yet; the next output continues it.
    pub(super) fn hold_output(&mut self, text: &str) -> Result<()> {
        if !text.is_empty() {
            self.charge_output(text.len())?;
            self.pending_line.push_str(text);
            self.lg3_diary_write(text, false)?;
        }
        Ok(())
    }
    fn truth(&self, value: &Value) -> Result<bool> {
        let view = value.view(&self.cancel)?;
        let a = view.as_ref();
        if a.is_empty() {
            return Ok(false);
        }
        match a {
            crate::logical::ArrayRef::Numeric(m) => {
                for chunk in m.data().chunks(8192) {
                    self.check_cancel()?;
                    if chunk.contains(&0.) {
                        return Ok(false);
                    }
                }
            }
            crate::logical::ArrayRef::Logical(m) => {
                for chunk in m.data().chunks(8192) {
                    self.check_cancel()?;
                    if chunk.contains(&0) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
    fn condition_value(
        &mut self,
        expr: &Expr,
        keyword: &str,
        out: &mut Execution,
    ) -> Result<Value> {
        if let Expr::Binary(op @ (Token::And | Token::Or), left, right) = expr {
            let a = self.condition_value(left, keyword, out)?;
            if a.view(&self.cancel)?.as_ref().len() == 1 {
                let truth = self.truth(&a)?;
                if (op == &Token::And && !truth) || (op == &Token::Or && truth) {
                    self.charge(1)?;
                    return Ok(Value::Logical(LogicalMatrix::scalar(truth)));
                }
            }
            let b = self.condition_value(right, keyword, out)?;
            let a = a.logical(&self.cancel)?;
            let b = b.logical(&self.cancel)?;
            let value = a.combine(
                &b,
                if op == &Token::And {
                    BooleanOp::And
                } else {
                    BooleanOp::Or
                },
                &self.cancel,
            )?;
            self.charge(value.len())?;
            return Ok(Value::Logical(value));
        }
        if let Expr::Not(inner) = expr {
            let value = self
                .condition_value(inner, keyword, out)?
                .logical(&self.cancel)?
                .not(&self.cancel)?;
            self.charge(value.len())?;
            return Ok(Value::Logical(value));
        }
        let value = self.eval(expr, None, out)?;
        // A sparse condition becomes full under the same limit as any other conversion.
        super::sparse_fns::densify(self, keyword, value)
    }
    pub(super) fn execute_statements(
        &mut self,
        statements: &[Statement],
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<Flow> {
        if statements.is_empty() {
            return Ok(Flow::Next);
        }
        if self.statement_depth >= 128 {
            return Err("Combined block/function execution exceeds 128 levels".into());
        }
        self.statement_depth += 1;
        let result = self.execute_statement_block(statements, byte, out);
        self.statement_depth -= 1;
        if let Err(message) = &result {
            // Temporary cleanup objects from a failed expression have no
            // workspace owner. Keep the original failure if cleanup fails too.
            let _ = self.lng_sweep_cleanup(out);
            if self.is_catchable(message) {
                self.lng_note_failure(message);
            }
        }
        result
    }
    fn execute_statement_block(
        &mut self,
        statements: &[Statement],
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<Flow> {
        for statement in statements {
            if matches!(statement.kind, StatementKind::Function(_)) {
                continue;
            }
            if self.profiler.enabled {
                self.profiler.line(statement.span.start)?;
            }
            self.control_step(statement.span.start, byte)?;
            self.lng_statement_start();
            let result = (|| -> Result<Flow> {
                match &statement.kind {
                    StatementKind::Function(_) => unreachable!(),
                    StatementKind::Declaration { persistent, names } => {
                        if self.lg3_eval_active() {
                            for name in names {
                                nested::check_write(self, name)?;
                            }
                        }
                        self.lng_declare(*persistent, names, out)?
                    }
                    StatementKind::Evaluate(e) => {
                        if !self.probe_mode {
                            self.execute_evaluate_statement(e, out)?;
                        } else if let Err(message) = self.execute_evaluate_statement(e, out) {
                            self.handle_probe_error(statement.span.start, message, out)?;
                        }
                    }
                    StatementKind::Return => return Ok(Flow::Return),
                    StatementKind::Break => return Ok(Flow::Break),
                    StatementKind::Continue => return Ok(Flow::Continue),
                    StatementKind::If {
                        branches,
                        otherwise,
                    } => {
                        let mut selected = otherwise.as_slice();
                        for branch in branches {
                            self.control_step(branch.byte, byte)?;
                            let truth = match scalar_loops::condition(self, &branch.condition)? {
                                Some(truth) => truth,
                                None => {
                                    let condition =
                                        self.condition_value(&branch.condition, "if", out)?;
                                    self.truth(&condition)?
                                }
                            };
                            if truth {
                                selected = &branch.body;
                                break;
                            }
                        }
                        match self.execute_statements(selected, byte, out)? {
                            Flow::Next => (),
                            other => return Ok(other),
                        }
                    }
                    StatementKind::Try {
                        body,
                        error,
                        handler,
                    } => match self.execute_try(body, error.as_deref(), handler, byte, out)? {
                        Flow::Next => (),
                        other => return Ok(other),
                    },
                    StatementKind::Switch {
                        subject,
                        cases,
                        otherwise,
                    } => {
                        let selected = self.select_switch(subject, cases, otherwise, byte, out)?;
                        match self.execute_statements(selected, byte, out)? {
                            Flow::Next => (),
                            other => return Ok(other),
                        }
                    }
                    StatementKind::While { condition, body } => loop {
                        self.control_step(statement.span.start, byte)?;
                        let truth = match scalar_loops::condition(self, condition)? {
                            Some(truth) => truth,
                            None => {
                                let value = self.condition_value(condition, "while", out)?;
                                self.truth(&value)?
                            }
                        };
                        if !truth {
                            break;
                        }
                        match self.execute_statements(body, byte, out)? {
                            Flow::Break => break,
                            Flow::Return => return Ok(Flow::Return),
                            _ => (),
                        }
                    },
                    StatementKind::SerialBlock { arguments, body } => {
                        for expr in arguments {
                            // Evaluate for side effects/errors, without publishing ans.
                            self.eval(expr, None, out)?;
                        }
                        match self.execute_statements(body, byte, out)? {
                            Flow::Next => (),
                            flow => return Ok(flow),
                        }
                    }
                    StatementKind::For { name, values, body } => {
                        let values = if let Expr::Range(start, step, stop) = values {
                            let (first, step, last) =
                                self.range_operands(start, step.as_deref(), stop, None, out)?;
                            if super::datetime_fns::is_dt(&first)
                                || super::datetime_fns::is_dt(&last)
                                || step.as_ref().is_some_and(super::datetime_fns::is_dt)
                            {
                                // A datetime/duration range: iterate its columns.
                                IterationValues::Array(
                                    self.dtm_range(first, step, last)?.array(&self.cancel)?,
                                )
                            } else if super::indexing::typed_range_operands(
                                &first,
                                step.as_ref(),
                                &last,
                            ) {
                                IterationValues::Array(
                                    self.typed_range(first, step, last)?.array(&self.cancel)?,
                                )
                            } else {
                                let (start, step, count, character) =
                                    self.range_scalars(first, step, last)?;
                                self.charge(count)?;
                                IterationValues::Range {
                                    start,
                                    step,
                                    count,
                                    character,
                                }
                            }
                        } else {
                            let a = self.eval(values, None, out)?.array(&self.cancel)?;
                            if !matches!(
                                a,
                                Array::Typed(_)
                                    | Array::Str(_)
                                    | Array::Categorical(_)
                                    | Array::Datetime(_)
                                    | Array::Duration(_)
                                    | Array::Sparse(_)
                            ) {
                                a.require_numeric_array()?;
                            }
                            IterationValues::Array(a.without_coordinate_facts(&self.cancel)?)
                        };
                        let retained = values.bytes();
                        if let Some(max) = self.limits.loop_bytes
                            && self.loop_bytes.saturating_add(retained) > max
                        {
                            return Err(limits::loop_bytes_error(max));
                        }
                        self.loop_bytes += retained;
                        let result =
                            self.execute_for(name, &values, body, statement.span.start, byte, out);
                        self.loop_bytes -= retained;
                        if let Flow::Return = result? {
                            return Ok(Flow::Return);
                        }
                    }
                }
                Ok(Flow::Next)
            })();
            if result.is_err()
                && self.probe_mode
                && matches!(
                    statement.kind,
                    StatementKind::If { .. }
                        | StatementKind::While { .. }
                        | StatementKind::For { .. }
                        | StatementKind::Switch { .. }
                )
            {
                self.probe_control_error = true;
            }
            match result? {
                Flow::Next => (),
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }
    #[inline]
    fn execute_evaluate_statement(&mut self, e: &Evaluation, out: &mut Execution) -> Result<()> {
        if self.lg3_eval_active() && self.functions.lng.nested_frame().is_some() {
            nested::check_eval_targets(self, e)?;
        }
        if !self.lng_statement(e, out)? && !self.lg3_statement(e, out)? {
            self.execute_evaluation(e, out)?;
        }
        self.lng_sweep_cleanup(out)
    }
    fn handle_probe_error(
        &mut self,
        byte: usize,
        message: String,
        out: &mut Execution,
    ) -> Result<()> {
        if self.try_depth > 0
            || self.probe_control_error
            || self.probe_count >= 200
            || !self.is_catchable(&message)
        {
            return Err(message);
        }
        // Temporaries from the failed statement have no workspace owner.
        // Sweep before recording recovery. Cancellation and execution limits
        // from that sweep are the run's result: they are not probe lines and
        // this statement is not a pass.
        self.lng_sweep_cleanup(out)?;
        self.probe_count += 1;
        let (file_name, line) = self.probe_location(byte);
        let first_line = message.split(['\r', '\n']).next().unwrap_or("");
        eprintln!("MATRUST-PROBE: {file_name}:{line}: {first_line}");
        self.raised = None;
        self.functions.failure = None;
        self.file_diagnostic = None;
        Ok(())
    }
    fn probe_location(&self, byte: usize) -> (String, usize) {
        if let Some(program) = self.functions.active_program() {
            let file_name = program
                .path
                .as_ref()
                .or(self.document_path.as_ref())
                .map_or_else(|| "script".into(), |p| p.display().to_string());
            let line = Diagnostic::new(&program.source, byte, String::new()).line;
            return (file_name, line);
        }
        let file_name = self
            .document_path
            .as_ref()
            .map_or_else(|| "script".into(), |p| p.display().to_string());
        let line = self.probe_source.as_deref().map_or(1, |source| {
            Diagnostic::new(source, byte, String::new()).line
        });
        (file_name, line)
    }
    /// Run `body`; an ordinary error jumps to `handler` with the error bound to
    /// `error`. Cancellation and run limits propagate untouched.
    fn execute_try(
        &mut self,
        body: &[Statement],
        error: Option<&str>,
        handler: &[Statement],
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<Flow> {
        let control_error = self.probe_control_error;
        let failure = self.functions.failure.clone();
        let file_diagnostic = self.file_diagnostic.clone();
        self.try_depth += 1;
        let result = self.execute_statements(body, byte, out);
        self.try_depth -= 1;
        let message = match result {
            Err(message) if self.is_catchable(&message) => message,
            other => return other,
        };
        // The error is handled: forget the failing-function bookkeeping so a
        // later uncaught error reports its own location.
        self.functions.failure = failure;
        self.file_diagnostic = file_diagnostic;
        self.probe_control_error = control_error;
        let value = self.error_value(&message)?;
        if let Some(name) = error {
            if self.lg3_eval_active() {
                nested::check_write(self, name)?;
            }
            self.assign_workspace_array(name, value, true, out)?;
        }
        self.execute_statements(handler, byte, out)
    }
    fn execute_for(
        &mut self,
        name: &str,
        values: &IterationValues,
        body: &[Statement],
        at: usize,
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<Flow> {
        if self.lg3_eval_active() {
            nested::check_write(self, name)?;
        }
        if values.count() == 0 {
            self.assign_workspace_array(name, values.empty(self)?.array(&self.cancel)?, true, out)?;
        }
        for i in 0..values.count() {
            self.control_step(at, byte)?;
            if let IterationValues::Range {
                start,
                step,
                character: false,
                ..
            } = values
            {
                let value = start + step * i as f64;
                // Reuse only an exclusively owned scalar. A captured alias or
                // desktop snapshot forces ordinary copy-on-write assignment.
                let reused = self
                    .variables
                    .get_mut(name)
                    .is_some_and(|m| m.len() == 1 && m.write_unique(&[(0, value)]));
                if !reused {
                    self.assign_workspace_array(name, Matrix::scalar(value).into(), true, out)?;
                }
            } else {
                let value = values.column(i, self)?;
                self.assign_workspace_array(name, value.array(&self.cancel)?, true, out)?;
            }
            match self.execute_statements(body, byte, out)? {
                Flow::Break => break,
                Flow::Return => return Ok(Flow::Return),
                _ => (),
            }
        }
        Ok(Flow::Next)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Engine, Limits, limits};

    fn limited(change: impl FnOnce(&mut Limits)) -> Engine {
        let mut limits = Limits::browser();
        change(&mut limits);
        Engine::with_limits(limits)
    }

    #[test]
    fn browser_limits_stop_at_one_million_steps() {
        assert_eq!(Engine::default().limits_active(), Limits::browser());
        for mut engine in [Engine::default(), Engine::with_limits(Limits::browser())] {
            let error = engine.execute("for i=1:1500000, end").unwrap_err();
            assert!(
                error.contains("Execution exceeds 1000000 statement/loop steps"),
                "{error}"
            );
            let error = engine.execute("n=0; while true, n=n+1; end").unwrap_err();
            assert!(error.contains("1000000 statement/loop steps"), "{error}");
        }
    }

    #[test]
    fn local_limits_run_past_one_million_steps_and_output_entries() {
        let mut engine = Engine::with_limits(Limits::local());
        engine.execute("s=0; for i=1:1500000, s=s+i; end").unwrap();
        assert_eq!(
            engine.variables["s"].as_scalar().unwrap(),
            1_125_000_750_000.
        );
        let n = engine.execute("n=0; while n<1200000, n=n+1; end").unwrap();
        assert!(n.output.is_empty());
        let output = engine.execute("for i=1:2500, disp(i), end").unwrap();
        assert_eq!(output.output.len(), 2500);
    }

    #[test]
    fn limit_errors_name_the_active_value() {
        let mut engine = limited(|l| l.steps = Some(1000));
        let error = engine.execute("for i=1:2000, end").unwrap_err();
        assert!(
            error.contains("Execution exceeds 1000 statement/loop steps"),
            "{error}"
        );
        // A script cannot catch a run limit, whatever its value.
        let error = engine
            .execute("try, n=0; while true, n=n+1; end, catch, caught=1; end")
            .unwrap_err();
        assert!(error.contains("1000 statement/loop steps"), "{error}");
        assert!(!engine.variables.contains_key("caught"));

        let mut engine = limited(|l| l.output_entries = Some(3));
        let error = engine.execute("for i=1:5, disp(i), end").unwrap_err();
        assert!(
            error.contains("Execution output exceeds 3 entries or 2 MiB"),
            "{error}"
        );
        let mut engine = limited(|l| {
            l.output_entries = None;
            l.output_bytes = Some(1024 * 1024);
        });
        let error = engine
            .execute("x = repmat('x', 1, 60000); for i=1:40, disp(x), end")
            .unwrap_err();
        assert!(error.contains("Execution output exceeds 1 MiB;"), "{error}");

        let mut engine = limited(|l| l.evaluation_units = Some(256 * 1024));
        let error = engine.execute("x = zeros(1, 300000);").unwrap_err();
        assert!(
            error.contains("Statement exceeds its 2 MiB evaluation budget"),
            "{error}"
        );
        engine.execute("x = zeros(1, 200000);").unwrap();

        let mut engine = limited(|l| l.loop_bytes = Some(1024 * 1024));
        let error = engine
            .execute("A = zeros(4, 200000); for c = A, end")
            .unwrap_err();
        assert!(
            error.ends_with("Active for-loop inputs exceed 1 MiB"),
            "{error}"
        );
    }

    #[test]
    fn local_memory_guards_follow_physical_ram() {
        let gib = 1024 * 1024 * 1024;
        let local = limits::local_for_memory(Some(16 * gib));
        assert_eq!(local.steps, None);
        assert_eq!(local.output_entries, None);
        assert_eq!(local.output_bytes, None);
        assert_eq!(local.evaluation_units, Some(gib as usize));
        assert_eq!(local.loop_bytes, Some(8 * gib as usize));
        // The memory budget is half the RAM, capped at 4 GiB.
        assert_eq!(limits::memory_bytes(&local), 4 * gib as usize);
        let small = limits::local_for_memory(Some(4 * gib));
        assert_eq!(limits::memory_bytes(&small), 2 * gib as usize);
        // Unknown or small RAM keeps today's guards.
        let browser = Limits::browser();
        for memory in [None, Some(64 * 1024 * 1024)] {
            let local = limits::local_for_memory(memory);
            assert_eq!(local.evaluation_units, browser.evaluation_units);
            assert_eq!(local.loop_bytes, browser.loop_bytes);
            assert_eq!(limits::memory_bytes(&local), 256 * 1024 * 1024);
        }
        assert_eq!(
            limits::meminfo_total("MemTotal:       16303364 kB\nMemFree: 1 kB\n"),
            Some(16_303_364 * 1024)
        );
        assert_eq!(limits::meminfo_total("MemFree: 1 kB\n"), None);
        if cfg!(any(target_os = "linux", windows, target_os = "macos")) {
            assert!(limits::physical_memory().is_some());
        }
    }
}
