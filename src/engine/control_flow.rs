//! Parse once, borrow loop bodies, and bound execution/output independently of UI.
use super::{Engine, Execution, Value};
use crate::{
    Array, LogicalMatrix, Matrix, Result,
    logical::BooleanOp,
    syntax::{Expr, Statement, StatementKind, Token},
};

pub(super) enum Flow {
    Next,
    Break,
    Continue,
    Return,
}
const MAX_STEPS: usize = 1_000_000;
const MAX_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_ENTRIES: usize = 2000;

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
                Array::Character(_) | Array::Cell(_) | Array::Graphics(_) | Array::Struct(_) | Array::Handle(_),
            ) => Err("Iteration over character, cell, struct, function handle or graphics handle arrays is not implemented".into()),
            Self::Array(a) if a.cols() == 1 => Ok(a.clone().into()),
            Self::Array(Array::Typed(t)) => {
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
        if self.control_steps >= MAX_STEPS {
            return Err("Execution exceeds 1000000 statement/loop steps; split the work or use array operations".into());
        }
        self.control_steps += 1;
        self.evaluated_elements = 0;
        Ok(())
    }
    /// One unit of the step budget for a loop over user data (no statement reset).
    pub(super) fn count_step(&mut self) -> Result<()> {
        self.check_cancel()?;
        if self.control_steps >= MAX_STEPS {
            return Err("Execution exceeds 1000000 statement/loop steps; split the work or use array operations".into());
        }
        self.control_steps += 1;
        Ok(())
    }
    /// Fail before a loop of `steps` iterations that cannot fit the step budget.
    pub(super) fn admit_steps(&self, steps: usize) -> Result<()> {
        if self.control_steps.saturating_add(steps) > MAX_STEPS {
            return Err("Execution exceeds 1000000 statement/loop steps; split the work or use array operations".into());
        }
        Ok(())
    }
    fn charge_output(&mut self, bytes: usize) -> Result<()> {
        if self.emitted_entries >= MAX_OUTPUT_ENTRIES
            || bytes > MAX_OUTPUT_BYTES - self.emitted_bytes
        {
            return Err(
                "Execution output exceeds 2000 entries or 2 MiB; use semicolons to suppress output"
                    .into(),
            );
        }
        self.emitted_bytes += bytes;
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
    fn condition_value(&mut self, expr: &Expr, out: &mut Execution) -> Result<Value> {
        if let Expr::Binary(op @ (Token::And | Token::Or), left, right) = expr {
            let a = self.condition_value(left, out)?;
            if a.view(&self.cancel)?.as_ref().len() == 1 {
                let truth = self.truth(&a)?;
                if (op == &Token::And && !truth) || (op == &Token::Or && truth) {
                    self.charge(1)?;
                    return Ok(Value::Logical(LogicalMatrix::scalar(truth)));
                }
            }
            let b = self.condition_value(right, out)?;
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
                .condition_value(inner, out)?
                .logical(&self.cancel)?
                .not(&self.cancel)?;
            self.charge(value.len())?;
            return Ok(Value::Logical(value));
        }
        self.eval(expr, None, out)
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
            self.control_step(statement.span.start, byte)?;
            self.lng_statement_start();
            match &statement.kind {
                StatementKind::Function(_) => unreachable!(),
                StatementKind::Declaration { persistent, names } => {
                    self.lng_declare(*persistent, names, out)?
                }
                StatementKind::Evaluate(e) => {
                    if !self.lng_statement(e, out)? && !self.lg3_statement(e, out)? {
                        self.execute_evaluation(e, out)?;
                    }
                    self.lng_sweep_cleanup(out)?;
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
                        let condition = self.condition_value(&branch.condition, out)?;
                        if self.truth(&condition)? {
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
                    let value = self.condition_value(condition, out)?;
                    if !self.truth(&value)? {
                        break;
                    }
                    match self.execute_statements(body, byte, out)? {
                        Flow::Break => break,
                        Flow::Return => return Ok(Flow::Return),
                        _ => (),
                    }
                },
                StatementKind::For { name, values, body } => {
                    let values = if let Expr::Range(start, step, stop) = values {
                        let (first, step, last) =
                            self.range_operands(start, step.as_deref(), stop, None, out)?;
                        if super::indexing::typed_range_operands(&first, step.as_ref(), &last) {
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
                        if !matches!(a, Array::Typed(_)) {
                            a.require_numeric_array()?;
                        }
                        IterationValues::Array(a.without_coordinate_facts(&self.cancel)?)
                    };
                    let retained = values.bytes();
                    if self.loop_bytes + retained > 256 * 1024 * 1024 {
                        return Err("Active for-loop inputs exceed 256 MiB".into());
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
        }
        Ok(Flow::Next)
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
        let failure = self.functions.failure.clone();
        let file_diagnostic = self.file_diagnostic.clone();
        let message = match self.execute_statements(body, byte, out) {
            Err(message) if self.is_catchable(&message) => message,
            other => return other,
        };
        // The error is handled: forget the failing-function bookkeeping so a
        // later uncaught error reports its own location.
        self.functions.failure = failure;
        self.file_diagnostic = file_diagnostic;
        let value = self.error_value(&message)?;
        if let Some(name) = error {
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
