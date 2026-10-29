//! Bounded current-workspace deletion; no function/cache/graphics reset.
use super::{Engine, Expr};
use crate::Result;

impl Engine {
    pub(super) fn clear_variables_command(&mut self, args: &[Expr]) -> Result<()> {
        if args.len() > 255 {
            return Err("clear supports at most 255 literal variable names".into());
        }
        let mut all_variables = args.is_empty();
        // Classify every item against the ORIGINAL workspace. In particular,
        // duplicate variables named all/variables must never become modes after
        // an earlier occurrence is removed. No array/string payload is cloned.
        for arg in args {
            self.check_cancel()?;
            let (Expr::String(name) | Expr::DoubleQuoted(name)) = arg else {
                return Err("clear requires literal variable names; use clear A B or clear('A','B'). Dynamic arguments and expressions are unsupported".into());
            };
            if self.contains_array(name) {
                Self::validate_workspace_name(name)?;
                continue;
            }
            match name.as_str() {
                "variables" if args.len() == 1 => all_variables = true,
                "variables" => return Err(
                    "clear variables must be used alone unless variables is a workspace variable"
                        .into(),
                ),
                "all" | "functions" | "classes" | "global" | "import" | "java" | "mex" => {
                    return Err(format!(
                        "clear {name} is unsupported: function/cache/global/class clearing is not implemented; use bare clear for current-workspace variables"
                    ));
                }
                _ => {
                    Self::validate_workspace_name(name)?;
                    if self.functions.contains_retained_function(name) {
                        return Err(format!(
                            "clear cannot unload retained function '{name}'; only variable bindings are supported"
                        ));
                    }
                }
            }
        }
        self.check_cancel()?;
        // Deliberately noncancellable commit: <=255 removals or three bounded
        // workspace maps, with no fallible validation after the first mutation.
        // Dropping the last shared owner can deallocate its existing buffer.
        if all_variables {
            self.clear_arrays();
        } else {
            for arg in args {
                if let Expr::String(name) | Expr::DoubleQuoted(name) = arg {
                    self.remove_array(name);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn clear_preflight_cancellation_and_argument_limit_leave_bindings_shared() {
        let mut engine = Engine::default();
        engine
            .execute("A=[1 2 3];B=A;mask=logical([0 1]);")
            .unwrap();
        let original = engine.workspace();
        let pointer = engine.variables["A"].data().as_ptr();
        engine.cancel.store(true, Ordering::Relaxed);
        for args in [vec![], vec![Expr::String("A".into())]] {
            assert!(engine.clear_variables_command(&args).is_err());
        }
        engine.cancel.store(false, Ordering::Relaxed);
        let names = vec![Expr::String("A".into()); 256];
        assert!(engine.clear_variables_command(&names).is_err());
        assert_eq!(engine.variables["A"].data().as_ptr(), pointer);
        for (name, value) in original {
            assert!(value.same_snapshot(&engine.array(&name).unwrap()));
        }
    }
}
