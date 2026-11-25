//! Inheritance: `classdef C < A & B`. A subclass's [`ClassInfo`] lists its
//! own members and then each superclass's (left to right); members keep their
//! defining class and program, so inherited methods, defaults and get/set
//! methods run where they were written, with that class's access. The
//! constructor first calls the superclass constructors it does not call
//! itself (no arguments, left to right); `obj@A(args)` calls A's constructor
//! on the object being built and `method@A(obj, ...)` A's version of a
//! method.
use super::super::{Engine, Execution, Value};
use super::{
    ClassInfo, MethodInfo, Program, ResolvedFunction, abstract_call_message, ambiguous_message,
    is_object, object_class,
};
use crate::{
    Array, Result,
    syntax::{Diagnostic, Expr, FunctionDefinition, PathStep, Statement, StatementKind},
};
use std::sync::Arc;

/// Deepest superclass chain followed while loading a class.
const MAX_DEPTH: usize = 16;

/// True when the constructor calls `out@base(...)` (`out` its output), the
/// explicit superclass constructor call that replaces the implicit one. The
/// parser makes that a call named `out@base` (syntax.rs
/// `superclass_reference`), so comments and text in literals never count.
/// Nested function definitions and anonymous function bodies are not the
/// constructor's own statements.
pub(super) fn calls_superclass(constructor: &FunctionDefinition, base: &str) -> bool {
    let Some(output) = constructor.outputs.first() else {
        return false;
    };
    statements_call(&constructor.body, &format!("{output}@{base}"))
}
fn statements_call(body: &[Statement], name: &str) -> bool {
    body.iter().any(|statement| match &statement.kind {
        StatementKind::Evaluate(e) => {
            expr_calls(&e.expr, name)
                || e.indexed_target
                    .as_ref()
                    .is_some_and(|(_, args)| exprs_call(args, name))
                || e.path_target.as_ref().is_some_and(|(_, steps)| {
                    steps.iter().any(|step| match step {
                        PathStep::Paren(args) | PathStep::Brace(args) => exprs_call(args, name),
                        PathStep::Dynamic(e) => expr_calls(e, name),
                        PathStep::Field(_) => false,
                    })
                })
                || e.list_target
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|e| expr_calls(e, name))
        }
        StatementKind::If {
            branches,
            otherwise,
        } => {
            branches
                .iter()
                .any(|b| expr_calls(&b.condition, name) || statements_call(&b.body, name))
                || statements_call(otherwise, name)
        }
        StatementKind::While { condition, body } => {
            expr_calls(condition, name) || statements_call(body, name)
        }
        StatementKind::SerialBlock { arguments, body } => {
            exprs_call(arguments, name) || statements_call(body, name)
        }
        StatementKind::For { values, body, .. } => {
            expr_calls(values, name) || statements_call(body, name)
        }
        StatementKind::Try { body, handler, .. } => {
            statements_call(body, name) || statements_call(handler, name)
        }
        StatementKind::Switch {
            subject,
            cases,
            otherwise,
        } => {
            expr_calls(subject, name)
                || cases
                    .iter()
                    .any(|c| expr_calls(&c.value, name) || statements_call(&c.body, name))
                || statements_call(otherwise, name)
        }
        StatementKind::Declaration { .. }
        | StatementKind::Function(_)
        | StatementKind::Break
        | StatementKind::Continue
        | StatementKind::Return => false,
    })
}
fn exprs_call(exprs: &[Expr], name: &str) -> bool {
    exprs.iter().any(|e| expr_calls(e, name))
}
fn expr_calls(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Call(called, args) => called == name || exprs_call(args, name),
        Expr::Unary(_, e)
        | Expr::Not(e)
        | Expr::Transpose(e)
        | Expr::NonConjugateTranspose(e)
        | Expr::Field(e, _) => expr_calls(e, name),
        Expr::Binary(_, a, b) | Expr::DynamicField(a, b) => {
            expr_calls(a, name) || expr_calls(b, name)
        }
        Expr::Range(start, step, stop) => {
            expr_calls(start, name)
                || step.as_deref().is_some_and(|e| expr_calls(e, name))
                || expr_calls(stop, name)
        }
        Expr::Matrix(rows) | Expr::CellLiteral(rows) => {
            rows.iter().any(|row| exprs_call(row, name))
        }
        Expr::CellContents(_, args) => exprs_call(args, name),
        Expr::Index(e, args) | Expr::Brace(e, args) | Expr::Apply(e, args) => {
            expr_calls(e, name) || exprs_call(args, name)
        }
        Expr::Number(_)
        | Expr::Imaginary(_)
        | Expr::Name(_)
        | Expr::NameValue(_)
        | Expr::String(_)
        | Expr::DoubleQuoted(_)
        | Expr::All
        | Expr::FunctionHandle(_)
        | Expr::Anonymous(_) => false,
    }
}

/// Load the direct superclasses of `class` and add their members (see the
/// module comment). Errors name the class file and the line of the
/// superclass list.
pub(super) fn merge_superclasses(
    engine: &mut Engine,
    class: &mut ClassInfo,
    supers: &[(String, usize)],
    file: &str,
    source: &str,
) -> Result<()> {
    if supers.is_empty() {
        return Ok(());
    }
    if engine.functions.classes.building.len() >= MAX_DEPTH {
        return Err(format!(
            "{file}: superclass chains deeper than {MAX_DEPTH} classes are not supported"
        ));
    }
    engine.functions.classes.building.push(class.name.clone());
    let result = merge(engine, class, supers, file, source);
    engine.functions.classes.building.pop();
    result
}

fn merge(
    engine: &mut Engine,
    class: &mut ClassInfo,
    supers: &[(String, usize)],
    file: &str,
    source: &str,
) -> Result<()> {
    let line = |byte: usize| Diagnostic::new(source, byte, String::new()).line;
    let mut handle_bases = Vec::new();
    let mut value_bases = Vec::new();
    for (base, byte) in supers {
        if base == "handle" || base == super::APPBASE_CLASS {
            handle_bases.push(base.clone());
            continue;
        }
        if *base == class.name || engine.functions.classes.building.contains(base) {
            return Err(format!(
                "{file}: class '{}' cannot inherit from itself through '{base}' (line {})",
                class.name,
                line(*byte)
            ));
        }
        // A dotted name is that package class only (`+pkg/A.m`, `+pkg/@A/A.m`,
        // nested `+a/+b`). A plain name is not looked up in the package that
        // holds this class file, and not through the import list: the
        // classdef line is not a function, so `import pkg.*` does not make
        // `< A` see `pkg.A` (MathWorks "Packages Create Namespaces": the
        // prefix is required on the classdef line; import shortens names
        // inside functions).
        let found = if base.as_bytes().contains(&b'.') {
            engine.pkg_qualified_program(base)?
        } else {
            engine.cls_stored_program(base)?
        };
        let program = found.filter(|p| p.class.is_some()).ok_or_else(|| {
            if base.contains('.') {
                format!(
                    "{file}: the superclass '{base}' of class '{}' cannot be found (line {})",
                    class.name,
                    line(*byte)
                )
            } else {
                format!(
                    "{file}: the superclass '{base}' of class '{}' is not a class file in the function folder (line {})",
                    class.name,
                    line(*byte)
                )
            }
        })?;
        let info = program.class.clone().expect("class program");
        if info.sealed {
            return Err(format!(
                "{file}: class '{base}' is Sealed and cannot be subclassed (line {})",
                line(*byte)
            ));
        }
        if info.is_handle {
            handle_bases.push(base.clone());
        } else {
            value_bases.push(base.clone());
        }
        class.super_programs.push(program);
    }
    if !handle_bases.is_empty() && !value_bases.is_empty() {
        return Err(format!(
            "{file}: class '{}' cannot derive from both handle and value classes (handle: {}; value: {}) (line {})",
            class.name,
            handle_bases.join(", "),
            value_bases.join(", "),
            line(supers[0].1)
        ));
    }
    class.is_handle = !handle_bases.is_empty();
    // Every superclass, depth first, left to right, each once.
    let mut ancestors: Vec<String> = Vec::new();
    let add = |name: &str, ancestors: &mut Vec<String>| {
        if !ancestors.iter().any(|a| a == name) {
            ancestors.push(name.to_owned());
        }
    };
    let mut programs = class.super_programs.iter();
    for (base, _) in supers {
        add(base, &mut ancestors);
        if base == super::APPBASE_CLASS {
            add("handle", &mut ancestors);
        } else if base != "handle"
            && let Some(program) = programs.next()
        {
            for a in &program.class.as_ref().expect("class program").ancestors {
                add(a, &mut ancestors);
            }
        }
    }
    // `handle` closes the list, as the reference lists it last.
    if let Some(at) = ancestors.iter().position(|a| a == "handle") {
        let handle = ancestors.remove(at);
        ancestors.push(handle);
    }
    class.ancestors = ancestors;
    for program in &class.super_programs {
        for event in &program.class.as_ref().expect("class program").events {
            if !class.events.contains(event) {
                class.events.push(event.clone());
            }
        }
    }
    let own_methods: Vec<String> = class.methods.keys().cloned().collect();
    for program in class.super_programs.clone() {
        let info = program.class.as_ref().expect("class program");
        for (j, property) in info.properties.iter().enumerate() {
            if let Some(existing) = class.properties.iter().find(|p| p.name == property.name) {
                if existing.owner == property.owner {
                    continue; // The same property through a common base.
                }
                return Err(if existing.owner == class.name {
                    format!(
                        "{file}: property '{}' of class '{}' is already defined by its superclass '{}'",
                        property.name, class.name, property.owner
                    )
                } else {
                    format!(
                        "{file}: property '{}' of class '{}' is defined by both superclasses '{}' and '{}'",
                        property.name, class.name, existing.owner, property.owner
                    )
                });
            }
            let mut inherited = property.clone();
            inherited.inherited = Some(
                property
                    .inherited
                    .clone()
                    .unwrap_or_else(|| (program.clone(), j)),
            );
            class.properties.push(inherited);
        }
        for (name, method) in &info.methods {
            if name == info.constructor_name() || own_methods.contains(name) {
                continue; // Constructors are not inherited; own methods win.
            }
            let inherited = MethodInfo {
                program: Some(method.program.clone().unwrap_or_else(|| program.clone())),
                ..method.clone()
            };
            if class.ambiguous.contains_key(name) {
                continue;
            }
            match class.methods.get(name) {
                None => {
                    class.methods.insert(name.clone(), inherited);
                }
                Some(existing) if existing.owner == inherited.owner => {}
                Some(existing) if existing.is_abstract && !inherited.is_abstract => {
                    class.methods.insert(name.clone(), inherited);
                }
                Some(_) if inherited.is_abstract => {}
                Some(existing) => {
                    let first = existing.owner.clone();
                    class.methods.remove(name);
                    class
                        .ambiguous
                        .insert(name.clone(), (first, inherited.owner.clone()));
                }
            }
        }
        for (name, pair) in &info.ambiguous {
            if !own_methods.contains(name) {
                class.methods.remove(name);
                class.ambiguous.insert(name.clone(), pair.clone());
            }
        }
    }
    Ok(())
}

impl Engine {
    /// The object a constructor of `program`'s class starts from: the object
    /// a superclass constructor call hands over, else (an outside
    /// construction) a new default object after the access and abstract
    /// checks. Then the superclass constructors the constructor does not
    /// call itself run without arguments, left to right.
    pub(in crate::engine) fn cls_constructor_preset(
        &mut self,
        program: &Arc<Program>,
        out: &mut Execution,
    ) -> Result<Array> {
        let class = program.class.clone().expect("class program");
        let mut object = match self.functions.classes.super_preset.take() {
            Some(object) => object,
            None => {
                self.cls_constructor_access(&class)?;
                let missing = class.abstract_methods();
                if !missing.is_empty() {
                    return Err(format!(
                        "Cannot create an object of the abstract class '{}': it does not implement the abstract methods {}.",
                        class.name,
                        missing.join(", ")
                    ));
                }
                self.cls_default_object(program, out)?
            }
        };
        for super_program in &class.super_programs {
            let info = super_program.class.as_ref().expect("class program");
            if !class.explicit_supers.contains(&info.name) {
                object = self.cls_super_construct(super_program, object, Vec::new(), out)?;
            }
        }
        Ok(object)
    }
    /// Run the constructor of a superclass on `object` (the subclass object
    /// being built) and return the object it produces.
    fn cls_super_construct(
        &mut self,
        super_program: &Arc<Program>,
        object: Array,
        args: Vec<Value>,
        out: &mut Execution,
    ) -> Result<Array> {
        let index = super_program
            .class
            .as_ref()
            .expect("class program")
            .constructor;
        self.functions.classes.super_preset = Some(object);
        let result = self.invoke_function(
            ResolvedFunction::at(super_program.clone(), index),
            args,
            1,
            None,
            out,
        );
        self.functions.classes.super_preset = None;
        let value = result?.remove(0).array(&self.cancel)?;
        if !is_object(&value) {
            return Err(format!(
                "The constructor of '{}' must return the object it was given.",
                super_program.class.as_ref().expect("class program").name
            ));
        }
        Ok(value)
    }
    /// `obj@Super(args)` in a constructor of a direct subclass of Super, and
    /// `method@Super(obj, ...)` in a method of any subclass of Super.
    pub(super) fn cls_super_call(
        &mut self,
        name: &str,
        args: &[Value],
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (left, base) = name.split_once('@').expect("superclass call");
        let frame = self
            .functions
            .cls_frame()
            .and_then(|(program, current)| Some((program.clone(), current?)));
        // A method of the class file, or the method of a method file of the
        // class folder (never the constructor).
        let running = frame.and_then(|(program, current)| {
            if let Some(class) = program.method_of.clone() {
                return (program.primary == Some(current)).then_some((program, class, None));
            }
            let class = program.class.clone()?;
            (current < class.inside).then_some((program, class, Some(current)))
        });
        let Some((program, class, current)) = running else {
            return Err(format!(
                "{name}: superclass calls (name@Superclass) are valid only in methods of a subclass."
            ));
        };
        let constructor_output = program
            .function(class.constructor)
            .and_then(|f| f.outputs.first().cloned());
        if current == Some(class.constructor) && constructor_output.as_deref() == Some(left) {
            if !class.supers.iter().any(|s| s == base) {
                return Err(format!(
                    "{name}: '{base}' is not a direct superclass of '{}'.",
                    class.name
                ));
            }
            let object = self
                .container_variables
                .get(left)
                .filter(|a| is_object(a))
                .cloned()
                .ok_or_else(|| format!("{name}: '{left}' is not the object being constructed."))?;
            let object = match class
                .super_programs
                .iter()
                .find(|p| p.class.as_ref().is_some_and(|c| c.name == base))
            {
                Some(super_program) => {
                    self.cls_super_construct(&super_program.clone(), object, args.to_vec(), out)?
                }
                // `handle` has nothing to construct.
                None if args.is_empty() => object,
                None => return Err("Too many input arguments.".into()),
            };
            self.container_variables
                .insert(left.to_owned(), object.clone());
            return Ok(vec![Value::from(object)]);
        }
        if !class.ancestors.iter().any(|a| a == base)
            || base == "handle"
            || base == super::APPBASE_CLASS
        {
            return Err(format!(
                "{name}: '{base}' is not a superclass of '{}'.",
                class.name
            ));
        }
        let (super_program, info) = self.cls_require(base, out)?;
        if let Some((a, b)) = info.ambiguous.get(left) {
            return Err(ambiguous_message(base, left, a, b));
        }
        let Some(method) = info.methods.get(left).filter(|_| left != base) else {
            return Err(format!(
                "Unrecognized method '{left}' for superclass '{base}'."
            ));
        };
        if method.is_abstract {
            return Err(abstract_call_message(base, left));
        }
        if !self.cls_allowed(&method.owner, method.access) {
            return Err(format!(
                "Cannot access {} method '{left}' of class '{}'.",
                method.access.name(),
                method.owner
            ));
        }
        if !method.is_static
            && !args
                .first()
                .is_some_and(|v| matches!(v, Value::Container(a) if object_class(a).is_some()))
        {
            return Err(format!(
                "{name}: the first input must be an object of class '{}' or a subclass.",
                base
            ));
        }
        let (target, index) = self.cls_target(&super_program, method)?;
        self.cls_invoke(&target, index, args.to_vec(), requested, targets, out)
    }
    /// `superclasses(obj)` / `superclasses('Name')`: every superclass, depth
    /// first, left to right (`handle` last); a listing without an output.
    pub(super) fn cls_superclasses(
        &mut self,
        class: &ClassInfo,
        requested: usize,
        out: &mut Execution,
    ) -> Result<Value> {
        if requested == 0 {
            let mut listing = String::new();
            if !self.display.compact {
                listing.push('\n');
            }
            if class.ancestors.is_empty() {
                listing.push_str(&format!("No superclasses for class {}\n", class.name));
            } else {
                listing.push_str(&format!("Superclasses for class {}:\n", class.name));
                if !self.display.compact {
                    listing.push('\n');
                }
                for n in &class.ancestors {
                    listing.push_str(&format!("    {n}\n"));
                }
            }
            if self.display.compact {
                listing.pop();
            }
            self.emit(out, listing)?;
            return Ok(Value::Void);
        }
        self.cls_names_cell(class.ancestors.clone())
    }
}
