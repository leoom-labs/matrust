//! Bounded interpreted functions with private workspaces and shared input arrays.
use super::{Engine, Execution, Value};
use crate::{
    Array, Result,
    syntax::{self, Expr, Statement, StatementKind},
};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct Program {
    pub path: Option<PathBuf>,
    source: Arc<str>,
    statements: Vec<Statement>,
    names: BTreeMap<String, usize>,
    primary: Option<usize>,
}
impl Program {
    fn new(source: Arc<str>, statements: Vec<Statement>, path: Option<PathBuf>) -> Self {
        let primary = statements
            .first()
            .filter(|s| matches!(s.kind, StatementKind::Function(_)))
            .map(|_| 0);
        let names = statements
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if let StatementKind::Function(f) = &s.kind {
                    Some((f.name.clone(), i))
                } else {
                    None
                }
            })
            .collect();
        Self {
            path,
            source,
            statements,
            names,
            primary,
        }
    }
}
#[derive(Clone, Default)]
pub(super) struct FunctionState {
    pub(super) lng: super::lang_more::LngState,
    directory: Option<PathBuf>,
    scope: Option<Arc<Program>>,
    /// Files whose workspace the running scripts share, innermost last: a
    /// script falls back to their local functions after its own (MATLAB).
    hosts: Vec<Arc<Program>>,
    files: BTreeMap<String, Option<Arc<Program>>>,
    /// Name caches of folders left by run('folder/script'), reused on return.
    folders: BTreeMap<PathBuf, BTreeMap<String, Option<Arc<Program>>>>,
    source_bytes: usize,
    file_count: usize,
    depth: usize,
    pub(super) retained_bytes: usize,
    pub(super) current: Option<usize>,
    pub arguments: Option<(usize, usize)>,
    pub failure: Option<(Arc<Program>, usize)>,
}
impl FunctionState {
    pub(super) fn lng_rehash(&mut self) {
        self.files.retain(|_, p| p.is_some());
    }
    pub(super) fn lng_forget(&mut self, name: &str) {
        self.files.remove(name);
    }
    /// Source text of the running function's definition.
    pub(super) fn lng_definition(&self) -> Option<&str> {
        let program = self.scope.as_ref()?;
        program
            .source
            .get(program.statements.get(self.current?)?.span.clone())
    }
    // Read only already retained metadata: targeted variable clear must not
    // open a new .mr/.m file or mutate active function/cache state to classify names.
    pub(super) fn contains_retained_function(&self, name: &str) -> bool {
        self.local_function(name).is_some()
            || self
                .files
                .get(name)
                .is_some_and(|p| p.as_ref().is_some_and(|p| p.primary.is_some()))
    }
    /// A local function of the running file, or inside a script of a host file.
    fn local_function(&self, name: &str) -> Option<ResolvedFunction> {
        self.scope
            .iter()
            .chain(self.hosts.iter().rev())
            .find_map(|program| {
                let index = *program.names.get(name)?;
                Some(ResolvedFunction {
                    program: program.clone(),
                    index,
                })
            })
    }
    pub(super) fn active_path(&self) -> Option<&Path> {
        self.scope.as_ref().and_then(|p| p.path.as_deref())
    }
    /// Local definitions in source order, from the running program (which may
    /// be unsaved editor text), excluding its primary function.
    pub(super) fn lg3_local_names(&self) -> Vec<String> {
        let Some(program) = self.scope.as_ref().filter(|_| self.arguments.is_some()) else {
            return Vec::new();
        };
        program
            .statements
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if Some(i) != program.primary
                    && let StatementKind::Function(f) = &s.kind
                {
                    Some(f.name.clone())
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn reset_run(&mut self) {
        let directory = self.directory.take();
        let mut lng = std::mem::take(&mut self.lng);
        lng.thrown_stack = None;
        lng.thrown_object = None;
        *self = Self {
            directory,
            lng,
            ..Self::default()
        };
    }
}
pub(super) struct ResolvedFunction {
    program: Arc<Program>,
    index: usize,
}
impl ResolvedFunction {
    pub(super) fn input_count(&self) -> usize {
        match &self.program.statements[self.index].kind {
            StatementKind::Function(function) => function.inputs.len(),
            _ => 0,
        }
    }
    pub(super) fn output_count(&self) -> usize {
        match &self.program.statements[self.index].kind {
            StatementKind::Function(function) => function.outputs.len(),
            _ => 0,
        }
    }
}
impl Engine {
    /// Enable .mr/.m function resolution in this directory. Files refresh each run.
    /// Without an explicit search directory, Current Folder is searched.
    pub fn set_function_directory(&mut self, path: &Path) -> Result<()> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Function directory: {e}"))?;
        if !path.is_dir() {
            return Err("Function search path must be a directory".into());
        }
        if self.functions.directory.as_ref() != Some(&path) {
            self.functions.files.clear();
            self.functions.folders.clear();
            // The dropped caches are re-read on demand; those re-reads must not
            // count against the per-run file and byte totals.
            self.functions.file_count = 0;
            self.functions.source_bytes = 0;
        }
        self.functions.directory = Some(path);
        Ok(())
    }
    /// The folder `help` searches for user functions, as calls do.
    pub fn help_path(&self) -> crate::help::PathContext {
        crate::help::PathContext {
            directory: self
                .functions
                .directory
                .clone()
                .or_else(|| self.file_directory.clone()),
        }
    }
    pub(super) fn execute_program(
        &mut self,
        source: &str,
        statements: Vec<Statement>,
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<()> {
        if !statements
            .iter()
            .any(|s| matches!(s.kind, StatementKind::Function(_)))
        {
            self.execute_statements(&statements, byte, out)?;
            return Ok(());
        }
        let program = Arc::new(Program::new(Arc::from(source), statements, None));
        self.functions.source_bytes = source.len();
        self.functions.scope = Some(program.clone());
        if let Some(index) = program.primary {
            *byte = program.statements[index].span.start;
            let result = self.invoke_function(
                ResolvedFunction { program, index },
                Vec::new(),
                0,
                None,
                out,
            )?;
            self.charge_function_outputs(&result)?;
            if let Some(value) = result.into_iter().next() {
                self.assign_workspace_array("ans", value.array(&self.cancel)?, false, out)?;
            }
        } else {
            self.execute_statements(&program.statements, byte, out)?;
        }
        Ok(())
    }
    pub(super) fn execute_selected_program(
        &mut self,
        source: &str,
        context: Vec<Statement>,
        selected: &[Statement],
        byte: &mut usize,
        out: &mut Execution,
    ) -> Result<()> {
        let definitions: Vec<_> = context
            .into_iter()
            .filter(|s| matches!(s.kind, StatementKind::Function(_)))
            .collect();
        if !definitions.is_empty() {
            self.functions.source_bytes = source.len();
            self.functions.scope =
                Some(Arc::new(Program::new(Arc::from(source), definitions, None)));
        }
        // Register definitions without implicitly invoking a primary function.
        self.execute_statements(selected, byte, out)?;
        Ok(())
    }
    pub(super) fn resolve_function(&mut self, name: &str) -> Result<Option<ResolvedFunction>> {
        self.check_function_input(name)?;
        if self.lg3_builtin_only(name) {
            return Ok(None);
        }
        if let Some(function) = self.functions.local_function(name) {
            return Ok(Some(function));
        }
        let Some(program) = self.folder_program(name)? else {
            return Ok(None);
        };
        // Reference behavior: only a statement runs a folder script; a call with arguments
        // or an output reaches the builtin, or else reports the script.
        let Some(index) = program.primary else {
            if crate::help::is_supported(name) {
                return Ok(None);
            }
            let path = program.path.as_deref().unwrap_or(Path::new(name));
            return Err(format!(
                "Attempt to execute SCRIPT {name} as a function:\n{}",
                path.display()
            ));
        };
        Ok(Some(ResolvedFunction { program, index }))
    }
    /// File of `name` when it is a local (not the primary) function of the
    /// running program: a handle made here is bound to that file.
    pub(super) fn lg3_local_file(&self, name: &str) -> Option<PathBuf> {
        let function = self.functions.local_function(name)?;
        if function.program.primary == Some(function.index) {
            return None;
        }
        function.program.path.clone()
    }
    /// A statement `name` or `name()`: a folder script unless a visible local
    /// function takes precedence. Variables are checked by the caller.
    pub(super) fn resolve_script(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        self.check_function_input(name)?;
        if self.functions.local_function(name).is_some() {
            return Ok(None);
        }
        Ok(self.folder_program(name)?.filter(|p| p.primary.is_none()))
    }
    pub(super) fn function_directory(&self) -> Option<&Path> {
        self.functions.directory.as_deref()
    }
    /// The .mr/.m file `name` of the lookup folder, function or script, cached per run.
    fn folder_program(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        let Some(directory) = self
            .functions
            .directory
            .as_ref()
            .or(self.file_directory.as_ref())
        else {
            return Ok(None);
        };
        if let Some(cached) = self.functions.files.get(name) {
            return Ok(cached.clone());
        }
        if !crate::files::valid_name(name) {
            return Ok(None);
        }
        if self.functions.files.len() >= 256 {
            return Err("Function resolution exceeds 256 names per run".into());
        }
        self.check_cancel()?;
        // Native .mr wins over .m within the folder; the per-run cache reset
        // also makes a newly added .mr visible on the next run.
        let mut found = None;
        for extension in ["mr", "m"] {
            let path = directory.join(format!("{name}.{extension}"));
            match File::open(&path) {
                Ok(file) => {
                    found = Some((path, file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        let Some((path, file)) = found else {
            self.functions.files.insert(name.into(), None);
            return Ok(None);
        };
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > 1_000_000 {
            return Err(format!(
                "{}: function source must be a regular file of at most 1 MB",
                path.display()
            ));
        }
        if self.functions.file_count >= 64 {
            return Err("At most 64 function files can be loaded per run".into());
        }
        let mut source = String::new();
        file.take(1_000_001)
            .read_to_string(&mut source)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if source.len() > 1_000_000 {
            return Err("Function source exceeds 1 MB".into());
        }
        if self.functions.source_bytes + source.len() > 8 * 1024 * 1024 {
            return Err("Function sources exceed 8 MiB per run".into());
        }
        self.check_cancel()?;
        let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
        let statements = syntax::parse_detailed(source).map_err(|e| {
            self.file_diagnostic = Some(super::FileDiagnostic {
                path: path.clone(),
                source: Arc::from(source),
                diagnostic: e.clone(),
            });
            format!("{}: {e}", path.display())
        })?;
        let program = Arc::new(Program::new(
            Arc::from(source),
            statements,
            Some(path.clone()),
        ));
        if let Some(index) = program.primary {
            let StatementKind::Function(first) = &program.statements[index].kind else {
                unreachable!()
            };
            if first.name != name {
                return Err(format!(
                    "{}: first function must be named '{name}'",
                    path.display()
                ));
            }
        }
        self.functions.source_bytes += source.len();
        self.functions.file_count += 1;
        self.functions
            .files
            .insert(name.into(), Some(program.clone()));
        Ok(Some(program))
    }
    /// Run a script in the caller's workspace (a function's when called from
    /// one). It shares the call depth limit, the step budget and cancellation;
    /// its own file identifies mfilename and its local functions come first,
    /// then the caller's (reference: a script sees its host function's helpers).
    pub(super) fn run_script(
        &mut self,
        program: Arc<Program>,
        name: &str,
        out: &mut Execution,
    ) -> Result<()> {
        if self.functions.depth >= 32 {
            return Err("Script/function call depth exceeds 32".into());
        }
        self.check_cancel()?;
        let caller_scope = self.functions.scope.replace(program.clone());
        let caller_hosts = self.functions.hosts.len();
        self.functions.hosts.extend(caller_scope.clone());
        // Input checks name the caller function's definition in its own scope.
        let caller_current = self.functions.current.take();
        self.functions.depth += 1;
        let mut byte = 0;
        let result = self.execute_statements(&program.statements, &mut byte, out);
        self.functions.hosts.truncate(caller_hosts);
        self.functions.scope = caller_scope;
        self.functions.current = caller_current;
        self.functions.depth -= 1;
        result
            .map(drop)
            .map_err(|message| self.located_failure(&program, name, byte, message))
    }
    /// run('name') and run('folder/name.m'): the folder, when given, is the
    /// lookup and file folder while the file runs (MATLAB cd's into it).
    pub(super) fn run_file(
        &mut self,
        folder: Option<PathBuf>,
        name: &str,
        text: &str,
        out: &mut Execution,
    ) -> Result<()> {
        let Some(folder) = folder.filter(|f| self.functions.directory.as_ref() != Some(f)) else {
            return self.run_folder_file(name, text, out);
        };
        // One cache per folder, shared by every frame: the active map belongs
        // to `functions.directory`, all others wait in `folders` (top folder
        // included), so a nested run back into a folder reuses its files.
        let caller_directory = self.functions.directory.clone();
        let mut parked = std::mem::take(&mut self.functions.files);
        if let Some(directory) = &caller_directory {
            self.functions
                .folders
                .insert(directory.clone(), std::mem::take(&mut parked));
        }
        let files = match self.functions.folders.remove(&folder) {
            Some(files) => files,
            None if self.functions.folders.len() >= 16 => {
                if let Some(directory) = &caller_directory {
                    self.functions.files = self.functions.folders.remove(directory).unwrap();
                }
                return Err("run supports at most 16 other folders per run".into());
            }
            None => BTreeMap::new(),
        };
        self.functions.files = files;
        self.functions.directory = Some(folder.clone());
        let caller_file_directory = self.file_directory.replace(folder.clone());
        let result = self.run_folder_file(name, text, out);
        let files = std::mem::take(&mut self.functions.files);
        // cd inside the script may have moved the active name cache to a
        // different folder; never label those functions with the run folder.
        let cache_folder = self.functions.directory.clone().unwrap_or(folder);
        self.functions.folders.insert(cache_folder, files);
        self.functions.files = match &caller_directory {
            Some(directory) => self.functions.folders.remove(directory).unwrap_or_default(),
            None => parked,
        };
        self.functions.directory = caller_directory;
        self.file_directory = caller_file_directory;
        result
    }
    fn run_folder_file(&mut self, name: &str, text: &str, out: &mut Execution) -> Result<()> {
        let program = self
            .folder_program(name)?
            .ok_or_else(|| format!("{text} not found."))?;
        let Some(index) = program.primary else {
            return self.run_script(program, name, out);
        };
        // A function file runs as the quiet statement `name;` would.
        let results = self.invoke_function(
            ResolvedFunction { program, index },
            Vec::new(),
            0,
            None,
            out,
        )?;
        self.charge_function_outputs(&results)?;
        if let Some(value) = results.into_iter().next() {
            self.assign_workspace_array("ans", value.array(&self.cancel)?, true, out)?;
        }
        Ok(())
    }
    pub(super) fn function_arguments(
        &mut self,
        args: &[Expr],
        end: Option<usize>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        if !args.iter().any(Expr::may_expand) {
            return args.iter().map(|arg| self.eval(arg, end, out)).collect();
        }
        let mut values = Vec::new();
        for arg in args {
            let list = self.eval_list(arg, end, out)?;
            if values.len().saturating_add(list.len()) > crate::script_containers::MAX_CELL_SLOTS {
                return Err("Argument expansion exceeds 65536 values".into());
            }
            values.extend(list);
        }
        Ok(values)
    }
    pub(super) fn check_function_input(&self, name: &str) -> Result<()> {
        if let (Some(program), Some(index), Some((supplied, _))) = (
            &self.functions.scope,
            self.functions.current,
            self.functions.arguments,
        ) {
            let StatementKind::Function(function) = &program.statements[index].kind else {
                unreachable!()
            };
            if function
                .inputs
                .iter()
                .skip(supplied)
                .flatten()
                .any(|input| input == name)
            {
                return Err(format!(
                    "Input argument '{name}' was not supplied to {}; pass it in the call or assign a default",
                    function.name
                ));
            }
        }
        Ok(())
    }
    pub(super) fn charge_function_outputs(&mut self, values: &[Value]) -> Result<()> {
        for value in values {
            if let Some(units) = value.allocation_units() {
                self.charge(units)?;
            }
        }
        Ok(())
    }
    pub(super) fn invoke_function(
        &mut self,
        resolved: ResolvedFunction,
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        // Taken on entry so a failing call never leaves names for the next one.
        let input_names = std::mem::take(&mut self.functions.lng.pending_names);
        let program = resolved.program;
        let statement = &program.statements[resolved.index];
        let StatementKind::Function(function) = &statement.kind else {
            unreachable!()
        };
        let varargs = function
            .inputs
            .last()
            .is_some_and(|n| n.as_deref() == Some("varargin"));
        let varout = function.outputs.last().is_some_and(|n| n == "varargout");
        if args.len() > function.inputs.len() && !varargs {
            return Err(format!(
                "{} accepts at most {} inputs",
                function.name,
                function.inputs.len()
            ));
        }
        if requested > function.outputs.len() && !varout {
            return Err(format!(
                "{} provides at most {} outputs",
                function.name,
                function.outputs.len()
            ));
        }
        if self.functions.depth >= 32 {
            return Err("Function call depth exceeds 32".into());
        }
        // Park caller frames without optional coordinate facts. Otherwise a
        // later nested call could exceed retention solely because an older Rust
        // stack frame cannot be revisited to discard those hints.
        let cancel = self.cancel.clone();
        let mut stripping = crate::array::coordinate_retention::FactStripper::new(&cancel);
        let mut caller_factless = if self
            .variables
            .values()
            .any(|a| a.retained_bytes() > a.len() * 8)
            || self
                .container_variables
                .values()
                .any(|a| a.retained_bytes() > a.bytes())
        {
            let mut candidate = self.workspace();
            for value in candidate.values_mut() {
                *value = stripping.strip(value)?;
            }
            Some(candidate)
        } else {
            None
        };
        let parked_bytes = caller_factless.as_ref().map_or_else(
            || self.obj_workspace_frame_bytes(),
            |values| values.values().map(Array::retained_bytes).sum(),
        );
        let retained = parked_bytes.saturating_add(self.evaluated_elements.saturating_mul(8));
        if self
            .functions
            .retained_bytes
            .saturating_add(retained)
            .saturating_add(self.obj_workspace_bytes())
            > 256 * 1024 * 1024
        {
            return Err(
                "Retained function caller workspaces/evaluation estimates exceed 256 MiB".into(),
            );
        }
        let input_count = args.len();
        let args = if varargs {
            let fixed = function.inputs.len() - 1;
            let mut args = args;
            let rest = args.split_off(fixed.min(args.len()));
            let slots = rest
                .into_iter()
                .map(|v| v.array(&self.cancel).map(Arc::new))
                .collect::<Result<Vec<_>>>()?;
            // Reference behavior: no extra inputs leave a 0x0 varargin, not 1x0.
            let rows = usize::from(!slots.is_empty());
            args.push(Value::Container(
                crate::CellArray::from_shared(rows, slots.len(), slots, &self.cancel)?.into(),
            ));
            args
        } else {
            args
        };
        let mut numeric = BTreeMap::new();
        let mut logical = BTreeMap::new();
        let mut complex = BTreeMap::new();
        let mut containers = BTreeMap::new();
        let mut input_bytes = 0usize;
        let supplied_names = function.inputs.iter().take(if varargs {
            input_count.min(function.inputs.len() - 1)
        } else {
            input_count
        });
        let variadic_name = varargs.then(|| function.inputs.last().unwrap());
        for (name, value) in supplied_names.chain(variadic_name).zip(args) {
            let array = stripping.strip(&value.array(&self.cancel)?)?;
            if let Some(name) = name {
                input_bytes += array.retained_bytes();
                match array {
                    a @ (Array::Character(_)
                    | Array::Cell(_)
                    | Array::Graphics(_)
                    | Array::Struct(_)
                    | Array::Handle(_)
                    | Array::Typed(_)) => {
                        containers.insert(name.clone(), a);
                    }
                    Array::Numeric(m) => {
                        numeric.insert(name.clone(), m);
                    }
                    Array::Logical(m) => {
                        logical.insert(name.clone(), m);
                    }
                    Array::Complex(m) => {
                        complex.insert(name.clone(), m);
                    }
                }
            }
        }
        drop(stripping);
        if input_bytes > 256 * 1024 * 1024 {
            return Err("Function inputs exceed the 256 MiB workspace limit".into());
        }
        self.check_cancel()?;
        self.lng_sync()?;
        let mut caller_numeric = std::mem::replace(&mut self.variables, numeric);
        let caller_logical = std::mem::replace(&mut self.logical_variables, logical);
        let caller_complex = std::mem::replace(&mut self.complex_variables, complex);
        let mut caller_containers = std::mem::replace(&mut self.container_variables, containers);
        if let Some(values) = caller_factless.take() {
            caller_numeric.clear();
            caller_containers.clear();
            for (name, array) in values {
                match array {
                    Array::Numeric(matrix) => {
                        caller_numeric.insert(name, matrix);
                    }
                    a @ (Array::Character(_)
                    | Array::Cell(_)
                    | Array::Graphics(_)
                    | Array::Struct(_)
                    | Array::Handle(_)
                    | Array::Typed(_)) => {
                        caller_containers.insert(name, a);
                    }
                    Array::Logical(_) | Array::Complex(_) => (),
                }
            }
        }
        self.lng_enter(
            super::lang_more::LngWorkspace {
                numeric: caller_numeric,
                logical: caller_logical,
                complex: caller_complex,
                containers: caller_containers,
            },
            format!(
                "{}::{}",
                program
                    .path
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string()),
                function.name
            ),
            input_names,
        );
        let caller_scope = self.functions.scope.replace(program.clone());
        // A function sees only its own file's local functions.
        let caller_hosts = std::mem::take(&mut self.functions.hosts);
        let caller_arguments = self.functions.arguments.replace((input_count, requested));
        let caller_current = self.functions.current.replace(resolved.index);
        let caller_evaluated = self.evaluated_elements;
        self.functions.depth += 1;
        self.functions.retained_bytes += retained;
        let mut byte = statement.span.start;
        let result = (|| {
            self.execute_statements(&function.body, &mut byte, out)?;
            self.check_cancel()?;
            if varout {
                return self.lng_varargout(&function.outputs, requested, targets);
            }
            if requested == 0 {
                return Ok(function
                    .outputs
                    .first()
                    .and_then(|n| self.array(n))
                    .map(Value::from)
                    .into_iter()
                    .collect());
            }
            function
                .outputs
                .iter()
                .take(requested)
                .enumerate()
                .map(|(i, n)| {
                    if targets.is_some_and(|t| t[i].is_none()) {
                        return Ok(Value::Void);
                    }
                    self.array(n).map(Value::from).ok_or_else(|| {
                        format!("Output '{n}' was not assigned by {}", function.name)
                    })
                })
                .collect()
        })();
        // Always restore callers, including cancellation and missing-output errors.
        let (caller, cleanup) = self.lng_leave(out, result.as_ref().map_or(&[], Vec::as_slice));
        self.variables = caller.numeric;
        self.logical_variables = caller.logical;
        self.complex_variables = caller.complex;
        self.container_variables = caller.containers;
        self.functions.scope = caller_scope;
        self.functions.hosts = caller_hosts;
        self.functions.arguments = caller_arguments;
        self.functions.current = caller_current;
        self.evaluated_elements = caller_evaluated;
        self.functions.depth -= 1;
        self.functions.retained_bytes -= retained;
        result
            .and_then(|values| cleanup.map(|()| values))
            .map_err(|message| self.located_failure(&program, &function.name, byte, message))
    }
    /// Keep the innermost failing file and byte; prefix the caller chain.
    fn located_failure(
        &mut self,
        program: &Arc<Program>,
        name: &str,
        byte: usize,
        message: String,
    ) -> String {
        if self.functions.failure.is_none() {
            self.functions.failure = Some((program.clone(), byte));
            if self.file_diagnostic.is_none()
                && let Some(path) = &program.path
            {
                self.file_diagnostic = Some(super::FileDiagnostic {
                    path: path.clone(),
                    source: program.source.clone(),
                    diagnostic: syntax::Diagnostic::new(&program.source, byte, message.clone()),
                });
            }
        }
        let raw = message.clone();
        let diagnostic = syntax::Diagnostic::new(&program.source, byte, message);
        let location = program
            .path
            .as_ref()
            .map_or_else(|| "submitted source".into(), |p| p.display().to_string());
        let wrapped = format!("In {name} ({location}): {diagnostic}");
        self.note_wrapped(&raw);
        wrapped
    }
}
