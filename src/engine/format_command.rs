//! `format` with its option words (`format long g`, `format('longG')`,
//! `format compact`) and plain `format` (reset).
use super::{Engine, Execution, Value};
use crate::{Result, syntax::Expr};

impl Engine {
    pub(super) fn format_command(&mut self, args: &[Expr], out: &mut Execution) -> Result<()> {
        if args.len() > 2 {
            return Err("format: use at most two option words, such as format long g".into());
        }
        let mut options = Vec::with_capacity(args.len());
        for arg in args {
            let value = self.eval(arg, None, out)?;
            let Some(Value::Text(option)) = self.text_arguments(vec![value])?.pop() else {
                return Err("format: the option must be text".into());
            };
            options.push(option);
        }
        let mut display = self.display;
        display.apply_options(&options.iter().map(String::as_str).collect::<Vec<_>>())?;
        self.display = display;
        Ok(())
    }

    /// Console width in characters used to wrap `Columns 1 through N` blocks
    /// and help prose.
    pub fn set_console_width(&mut self, width: usize) {
        self.display.width = width.max(1);
        self.display.wrap_help = true;
    }
}
