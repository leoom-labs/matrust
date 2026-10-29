//! `format short|long|compact|loose` and plain `format` (reset).
use super::{Engine, Execution, Value};
use crate::{Result, syntax::Expr};

impl Engine {
    pub(super) fn format_command(&mut self, args: &[Expr], out: &mut Execution) -> Result<()> {
        let mut display = self.display;
        match args {
            [] => display.apply_option(None)?,
            [arg] => {
                let value = self.eval(arg, None, out)?;
                let Some(Value::Text(option)) = self.text_arguments(vec![value])?.pop() else {
                    return Err("format: the option must be text".into());
                };
                display.apply_option(Some(&option))?;
            }
            _ => return Err("format: one option at a time is supported".into()),
        }
        self.display = display;
        Ok(())
    }

    /// Console width in characters used to wrap `Columns 1 through N` blocks.
    pub fn set_console_width(&mut self, width: usize) {
        self.display.width = width.max(1);
    }
}
