//! `help`/`doc` commands: the overview alone, otherwise one function's help.
use super::{Engine, Execution, HELP, Value};
use crate::{Result, syntax::Expr};

impl Engine {
    pub(super) fn help_command(
        &mut self,
        command: &str,
        args: &[Expr],
        out: &mut Execution,
    ) -> Result<()> {
        let text = match args {
            [] => HELP.to_string(),
            [arg] => {
                let value = self.eval(arg, None, out)?;
                let Some(Value::Text(name)) = self.text_arguments(vec![value])?.pop() else {
                    return Err(format!("{command}: the function name must be text"));
                };
                crate::help::help(&name, &self.help_path())?
            }
            _ => {
                return Err(format!(
                    "{command}: more than one name is not supported yet"
                ));
            }
        };
        self.emit(out, text)
    }
}
