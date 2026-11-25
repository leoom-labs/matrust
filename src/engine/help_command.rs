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
                // `doc` stays on the function/package topic. Method-file forms
                // (`Cls.method`, `Cls/method`) are help only.
                if command == "doc" {
                    crate::help::doc(&name, &self.help_path())?
                } else {
                    crate::help::help(&name, &self.help_path())?
                }
            }
            _ => {
                return Err(format!(
                    "{command}: more than one name is not supported yet"
                ));
            }
        };
        let text = if self.display.wrap_help {
            wrap_prose(&text, self.display.width)
        } else {
            text
        };
        self.emit(out, text)
    }
}

/// Help pages are prose: lines wider than the console wrap at spaces (MATLAB
/// prints help pre-wrapped), so they read without side-scrolling. Indented
/// lines (code, tables) keep their layout.
pub(crate) fn wrap_prose(text: &str, width: usize) -> String {
    let width = width.max(40);
    let mut result = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            result.push('\n');
        }
        if line.chars().count() <= width || line.starts_with(char::is_whitespace) {
            result.push_str(line);
            continue;
        }
        let mut column = 0;
        for word in line.split(' ') {
            let len = word.chars().count();
            if column > 0 && column + 1 + len > width {
                result.push_str("\n    ");
                column = 4;
            } else if column > 0 {
                result.push(' ');
                column += 1;
            }
            result.push_str(word);
            column += len;
        }
    }
    result
}
