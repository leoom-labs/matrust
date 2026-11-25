//! Package folders in expressions and statements: `a.fn(x)`, `a.b.fn`,
//! `a.Cls(x)`, `a.Cls.staticMethod(x)`, a package script as a statement, and
//! the `import` list. The lookup itself (package folders, caches, imports)
//! is in user_functions.rs; a name whose first part is a variable never
//! reaches a package.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, Result,
    syntax::{Evaluation, Expr},
};
use std::sync::Arc;

/// The first name of a pure dotted chain `a.b.c`.
fn chain_root(chain: &Expr) -> Option<&str> {
    let mut expr = chain;
    loop {
        match expr {
            Expr::Field(base, _) => expr = base,
            Expr::Name(name) => return Some(name),
            _ => return None,
        }
    }
}
/// The names of a pure dotted chain, first name first.
fn chain_parts(chain: &Expr) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut expr = chain;
    loop {
        match expr {
            Expr::Field(base, name) => {
                parts.push(name.as_str());
                expr = base;
            }
            Expr::Name(name) => {
                parts.push(name);
                break;
            }
            _ => break,
        }
    }
    parts.reverse();
    parts
}
/// `a.b.c` and `a.b.c(args)` split into the chain and the arguments.
fn chain_call(expr: &Expr) -> Option<(&Expr, Option<&[Expr]>)> {
    match expr {
        Expr::Field(..) => Some((expr, None)),
        Expr::Index(base, args) if matches!(base.as_ref(), Expr::Field(..)) => {
            Some((base, Some(args)))
        }
        _ => None,
    }
}

impl Engine {
    /// The names of the dotted chain `chain` when its first name is a
    /// package and not a variable (variables always win).
    fn pkg_chain<'a>(&mut self, chain: &'a Expr) -> Result<Option<Vec<&'a str>>> {
        let Some(root) = chain_root(chain) else {
            return Ok(None);
        };
        if root == "end" || self.contains_array(root) || !self.pkg_is_package(root)? {
            return Ok(None);
        }
        Ok(Some(chain_parts(chain)))
    }
    /// A dotted name whose first part is a package and that names nothing in it.
    fn pkg_unresolved(&mut self, parts: &[&str]) -> String {
        self.raise(
            format!("Unable to resolve the name '{}'.", parts.join(".")),
            "MATLAB:undefinedVarOrClass".into(),
        )
    }
    /// One value of `value.name`.
    fn pkg_field_of(&mut self, value: Value, name: &str, out: &mut Execution) -> Result<Value> {
        let mut values = self.obj_field_values_resolved(value, name, out)?;
        if values.len() != 1 {
            return Err(format!(
                "Expected one output from a curly brace or dot indexing expression, but there were {} results",
                values.len()
            ));
        }
        Ok(values.remove(0))
    }
    /// `a.b.c` / `a.b.c(args)` as one value when `a` is a package: a call of
    /// the package function or class constructor the names reach, a static
    /// member of a package class, then any names left as fields of that.
    pub(super) fn pkg_value(
        &mut self,
        chain: &Expr,
        args: Option<&[Expr]>,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let Some(parts) = self.pkg_chain(chain)? else {
            return Ok(None);
        };
        let Some((program, used)) = self.pkg_resolve(&parts)? else {
            return Err(self.pkg_unresolved(&parts));
        };
        let name = parts[..used].join(".");
        let rest = &parts[used..];
        // The arguments belong to the last name.
        let (first_args, last_args) = if rest.is_empty() {
            (args, None)
        } else {
            (None, args)
        };
        let (mut value, rest) = match rest.split_first() {
            // `a.Cls.member`: a static method, Constant property or
            // enumeration member of the class, under its qualified name.
            Some((member, rest)) if program.class.is_some() => {
                let member_args = if rest.is_empty() { last_args } else { None };
                let class = Expr::Name(name);
                let Some(value) = self.cls_static(&class, member, member_args, out)? else {
                    return Err(self.pkg_unresolved(&parts));
                };
                if rest.is_empty() {
                    return Ok(Some(value));
                }
                (value, rest)
            }
            _ => {
                let Some(function) = self.pkg_function(&name, program)? else {
                    return Err(self.pkg_unresolved(&parts));
                };
                let values = match first_args {
                    Some(args) => {
                        let values = self.function_arguments(args, None, out)?;
                        self.lng_note_inputs(args, values.len());
                        values
                    }
                    None => Vec::new(),
                };
                let mut results = self.invoke_function(function, values, 1, None, out)?;
                if results.is_empty() {
                    return Err(format!("{name}: no value was returned"));
                }
                (results.remove(0), rest)
            }
        };
        for field in rest {
            value = self.pkg_field_of(value, field, out)?;
        }
        match last_args {
            Some(args) if !rest.is_empty() => self.index_value(value, args, out).map(Some),
            _ => Ok(Some(value)),
        }
    }
    /// Statements with nargout 0: `a.fn(x)`, `a.fn`, a package script
    /// `a.script`, `a.Cls(x)`, `a.Cls.staticMethod(x)`, and `import ...`.
    /// True if handled.
    pub(super) fn pkg_statement(&mut self, st: &Evaluation, out: &mut Execution) -> Result<bool> {
        if !st.targets.is_empty()
            || st.indexed_target.is_some()
            || st.path_target.is_some()
            || st.list_target.is_some()
        {
            return Ok(false);
        }
        let Some((chain, args)) = chain_call(&st.expr) else {
            return match &st.expr {
                Expr::Name(name) if name == "import" => self.imp_statement(&[], st.quiet, out),
                Expr::Call(name, args) if name == "import" => {
                    self.imp_statement(args, st.quiet, out)
                }
                _ => Ok(false),
            };
        };
        let Some(parts) = self.pkg_chain(chain)? else {
            return Ok(false);
        };
        // Anything else (unknown names, static members, fields of a result)
        // is an expression: `pkg_value` evaluates or reports it.
        let Some((program, used)) = self.pkg_resolve(&parts)? else {
            return Ok(false);
        };
        // `a.Cls.staticMethod(x);`: nargout 0, as `Cls.staticMethod(x);`.
        if program.class.is_some() && used + 1 == parts.len() {
            let class = parts[..used].join(".");
            let Some(value) =
                self.cls_static_statement(&class, parts[used], args.unwrap_or(&[]), out)?
            else {
                return Ok(false);
            };
            if !matches!(value, Value::Void) {
                self.assign_workspace_array("ans", value.array(&self.cancel)?, st.quiet, out)?;
            }
            return Ok(true);
        }
        if used != parts.len() {
            return Ok(false);
        }
        let name = parts.join(".");
        let args = args.unwrap_or(&[]);
        if program.primary.is_none() && program.class.is_none() {
            if !args.is_empty() {
                return Ok(false);
            }
            self.run_script(program, &name, out)?;
            return Ok(true);
        }
        let Some(function) = self.pkg_function(&name, program)? else {
            return Ok(false);
        };
        let values = self.function_arguments(args, None, out)?;
        self.lng_note_inputs(args, values.len());
        let mut results = self.invoke_function(function, values, 0, None, out)?;
        self.charge_function_outputs(&results)?;
        if let Some(value) = results.pop() {
            self.assign_workspace_array("ans", value.array(&self.cancel)?, st.quiet, out)?;
        }
        Ok(true)
    }
    /// `[p, q] = a.fn(x)` and `[p, q] = a.Cls.staticMethod(x)`.
    pub(super) fn pkg_outputs(
        &mut self,
        expression: &Expr,
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        let Some((chain, args)) = chain_call(expression) else {
            return Ok(None);
        };
        let Some(parts) = self.pkg_chain(chain)? else {
            return Ok(None);
        };
        let Some((program, used)) = self.pkg_resolve(&parts)? else {
            return Err(self.pkg_unresolved(&parts));
        };
        let name = parts[..used].join(".");
        let args = args.unwrap_or(&[]);
        if used == parts.len() {
            let Some(function) = self.pkg_function(&name, program)? else {
                return Err(self.pkg_unresolved(&parts));
            };
            let values = self.function_arguments(args, None, out)?;
            self.lng_note_inputs(args, values.len());
            let values =
                self.invoke_function(function, values, targets.len(), Some(targets), out)?;
            self.charge_function_outputs(&values)?;
            return Ok(Some(values));
        }
        if program.class.is_some() && used + 1 == parts.len() {
            // The class's own multi-output dispatch, under its qualified name.
            let member = Expr::Field(Box::new(Expr::Name(name)), parts[used].to_owned());
            let call = Expr::Index(Box::new(member), args.to_vec());
            // No such member: report the qualified name, never fall back to the constructor.
            return match self.cls_outputs(&call, targets, out)? {
                Some(values) => Ok(Some(values)),
                None => Err(self.pkg_unresolved(&parts)),
            };
        }
        Ok(None)
    }
    /// The entries `import` was given: text such as `a.*` or `a.b.name`.
    fn imp_entries(&self, values: Vec<Value>) -> Result<Vec<String>> {
        self.text_arguments(values)?
            .into_iter()
            .map(|value| match value {
                Value::Text(text) => Ok(text),
                _ => Err("import: each argument must be text such as 'pkg.*' or 'pkg.name'".into()),
            })
            .collect()
    }
    /// The statement `import a.* b.name`: nothing is shown. A bare `import`
    /// shows the import list of the running workspace.
    fn imp_statement(&mut self, args: &[Expr], quiet: bool, out: &mut Execution) -> Result<bool> {
        if self.contains_array("import") || self.resolve_function("import")?.is_some() {
            return Ok(false);
        }
        let values = self.function_arguments(args, None, out)?;
        let entries = self.imp_entries(values)?;
        self.imp_add(&entries)?;
        if entries.is_empty() && !quiet {
            let list = self.functions.imp_list();
            if !list.is_empty() {
                let lines: Vec<String> = list.iter().map(|entry| format!("    {entry}")).collect();
                self.emit(out, lines.join("\n"))?;
            }
        }
        Ok(true)
    }
    /// `L = import` and `L = import('a.*', ...)`: add the entries, then the
    /// import list of the running workspace as a cell column of text.
    pub(super) fn imp_call(&mut self, args: &[Value]) -> Result<Value> {
        let entries = self.imp_entries(args.to_vec())?;
        self.imp_add(&entries)?;
        let slots = self
            .functions
            .imp_list()
            .into_iter()
            .map(|entry| Value::Text(entry).array(&self.cancel).map(Arc::new))
            .collect::<Result<Vec<Arc<Array>>>>()?;
        Ok(Value::Container(
            CellArray::from_shared(slots.len(), 1, slots, &self.cancel)?.into(),
        ))
    }
    /// `clear import`: at the Command Window (base workspace) only.
    pub(super) fn imp_clear_command(&mut self) -> Result<()> {
        if self.functions.arguments.is_some() {
            return Err(
                "clear import is only allowed at the Command Window, not inside a function".into(),
            );
        }
        self.functions.imp_clear();
        Ok(())
    }
}
