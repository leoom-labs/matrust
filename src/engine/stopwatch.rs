//! `tic` / `toc`. The id returned by `id = tic` is a double counting
//! microseconds on a process-wide monotonic clock, so `toc(id)` never depends
//! on the session's own timer.
use super::{Engine, Execution, Value};
use crate::clock::Instant;
use crate::{Matrix, Result};
use std::sync::OnceLock;

fn micros_now() -> f64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    // One microsecond offset keeps every id nonzero.
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64() * 1e6 + 1.
}

#[derive(Clone, Default, Debug)]
pub(super) struct Stopwatch {
    started: Option<f64>,
}

impl Engine {
    /// `wants_output` is true when the call result is consumed (`t = toc`,
    /// `id = tic`, `disp(toc)`); statement-level calls pass false.
    pub(super) fn stopwatch_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
        wants_output: bool,
    ) -> Result<Value> {
        self.check_cancel()?;
        if name == "tic" {
            Self::arity_check(name, args, &[0])?;
            let now = micros_now();
            return if wants_output {
                Ok(Value::Matrix(Matrix::scalar(now)))
            } else {
                self.stopwatch.started = Some(now);
                Ok(Value::Void)
            };
        }
        Self::arity_check(name, args, &[0, 1])?;
        let start = match args.first() {
            Some(id) => {
                let id = self.real_arg(name, id)?;
                let id = id.as_scalar().map_err(|_| {
                    "toc: the timer id must be a scalar returned by tic".to_string()
                })?;
                if !id.is_finite() || id < 1. {
                    return Err("toc: the timer id must be a scalar returned by tic".into());
                }
                id
            }
            None => self
                .stopwatch
                .started
                .ok_or("toc: call tic before calling toc without a timer id")?,
        };
        let seconds = ((micros_now() - start) / 1e6).max(0.);
        if wants_output {
            Ok(Value::Matrix(Matrix::scalar(seconds)))
        } else {
            self.emit(out, format!("Elapsed time is {seconds:.6} seconds."))?;
            Ok(Value::Void)
        }
    }
}
