//! Bounded interpreted functions with private workspaces and shared input arrays.
pub(super) mod nested;
use super::{Engine, Execution, Value};
use crate::{
    Array, Result,
    syntax::{self, Expr, Statement, StatementKind},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(super) struct Program {
    pub path: Option<PathBuf>,
    pub(super) source: Arc<str>,
    statements: Vec<Statement>,
    names: BTreeMap<String, usize>,
    scopes: BTreeMap<usize, nested::Scope>,
    pub(super) primary: Option<usize>,
    /// Set for a classdef file: its methods, generated constructor and
    /// default-value functions come first in `statements`, then local functions.
    pub(super) class: Option<Arc<super::classdef::ClassInfo>>,
    /// Set for a method file of a class folder (`@Cls/name.m`): the class.
    /// Its first function is the method and runs with the class's access.
    pub(super) method_of: Option<Arc<super::classdef::ClassInfo>>,
    /// The private folder the code of this file sees (`prv_folder`).
    private: Option<PathBuf>,
}
/// The class folder `@name` when `path` is the file `@name/name.*` in it.
pub(super) fn cls_folder<'a>(path: &'a Path, name: &str) -> Option<&'a Path> {
    let folder = path.parent()?;
    let class = folder.file_name()?.to_str()?.strip_prefix('@')?;
    (class == name).then_some(folder)
}
/// The private folder visible to the code of the file at `path`: the file's
/// own folder when that is named `private` (private functions call their
/// siblings), else its `private` subfolder. The folder need not exist.
fn prv_folder(path: &Path) -> Option<PathBuf> {
    let folder = path.parent()?;
    Some(if folder.file_name().is_some_and(|n| n == "private") {
        folder.to_path_buf()
    } else {
        folder.join("private")
    })
}
/// Lookups in one private folder by name, hit or miss.
#[derive(Clone, Default)]
struct PrivateFolder {
    /// The folder existed when first used in this run. If not, every name
    /// misses without file access.
    exists: bool,
    names: BTreeMap<String, Option<Arc<Program>>>,
}
/// One package (`+a`, or `+b` inside it) as looked up in this run.
#[derive(Clone, Default)]
struct Package {
    /// Its folders in lookup order: `+name` in each current or search path
    /// folder (for a sub-package: in each folder of its parent) that has one.
    /// A package spread over several folders is one package; per name the
    /// first folder wins. Empty: there is no such package.
    folders: Vec<PathBuf>,
    /// Files looked up by name, hit or miss.
    names: BTreeMap<String, Option<Arc<Program>>>,
    /// Sub-packages looked up by name, existing or not.
    packages: BTreeMap<String, Package>,
}
impl Package {
    fn forget(&mut self, name: &str) {
        self.names.remove(name);
        for package in self.packages.values_mut() {
            package.forget(name);
        }
    }
    fn clear_classes(&mut self, keep: &BTreeSet<String>) {
        self.names
            .retain(|_, program| class_program_stays(program, keep));
        for package in self.packages.values_mut() {
            package.clear_classes(keep);
        }
    }
}
/// A cached lookup stays unless it is a class file outside `keep`.
fn class_program_stays(program: &Option<Arc<Program>>, keep: &BTreeSet<String>) -> bool {
    match program {
        Some(program) => program
            .class
            .as_ref()
            .is_none_or(|class| keep.contains(&class.name)),
        None => true,
    }
}
/// One entry of an import list: `a.b.*` (every name of package `a.b`) or
/// `a.b.name`.
#[derive(Clone, PartialEq)]
struct Import {
    package: Vec<String>,
    /// None for a wildcard.
    name: Option<String>,
}
impl Import {
    fn text(&self) -> String {
        format!(
            "{}.{}",
            self.package.join("."),
            self.name.as_deref().unwrap_or("*")
        )
    }
}
/// The parts of a package-qualified name `a.b.name`: two to eight valid
/// names joined by dots.
fn pkg_parts(name: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = name.split('.').collect();
    ((2..=8).contains(&parts.len()) && parts.iter().all(|part| crate::files::valid_name(part)))
        .then_some(parts)
}
/// `name` is spelled like a package-qualified name (`a.b.name`).
pub(super) fn pkg_name(name: &str) -> bool {
    name.as_bytes().contains(&b'.') && pkg_parts(name).is_some()
}
/// The package folder `+a/+b` of `parts` inside `folder`.
fn pkg_folder<S: AsRef<str>>(folder: &Path, parts: &[S]) -> PathBuf {
    let mut path = folder.to_path_buf();
    for part in parts {
        path.push(format!("+{}", part.as_ref()));
    }
    path
}
/// What a call of `name` runs when the lookup found the file `program`.
fn program_function(name: &str, program: Arc<Program>) -> Result<Option<ResolvedFunction>> {
    // A classdef file: `Name(args)` calls its constructor.
    if let Some(class) = &program.class {
        let index = class.constructor;
        return Ok(Some(ResolvedFunction::at(program, index)));
    }
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
    Ok(Some(ResolvedFunction::at(program, index)))
}
/// Open `name`.mr, else `name`.m, in `folder`. With `root` (an added folder),
/// a file whose real path leaves `root` is a miss, as for `which`/`exist`.
fn spath_open_in(
    folder: &Path,
    name: &str,
    root: Option<&Path>,
) -> Result<Option<(PathBuf, File)>> {
    for extension in ["mr", "m"] {
        let path = folder.join(format!("{name}.{extension}"));
        match File::open(&path) {
            Ok(_) if root.is_some_and(|r| !path.canonicalize().is_ok_and(|p| p.starts_with(r))) => {
            }
            Ok(file) => return Ok(Some((path, file))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // A search path folder removed or replaced since addpath.
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(None)
}
/// Open the file a lookup of `name` in `folders` finds: folder by folder in
/// lookup order, the class file `@name/name` of a class folder, else
/// `name`.mr, else `name`.m; the first folder with either wins (reference
/// precedence since R2026a: path position decides, and within one folder
/// the @folder class comes before a plain file of that name).
fn spath_open(
    folders: &[PathBuf],
    name: &str,
    current: Option<&Path>,
) -> Result<Option<(PathBuf, File)>> {
    let class_folder = format!("@{name}");
    for directory in folders {
        let root = (current != Some(directory.as_path())).then_some(directory.as_path());
        let folder = directory.join(&class_folder);
        // One stat per folder; a folder `@name` without `name.m` (methods
        // for a builtin class) is passed over.
        if folder.is_dir()
            && let Some(found) = spath_open_in(&folder, name, root)?
        {
            return Ok(Some(found));
        }
        if let Some(found) = spath_open_in(directory, name, root)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}
/// Existing program files in one lookup root, in class/extension precedence order.
pub(super) fn spath_existing<'a>(
    folder: crate::help::LookupFolder<'a>,
    package: &'a Path,
    name: &'a str,
) -> impl Iterator<Item = PathBuf> + 'a {
    [package.join(format!("@{name}")), package.to_owned()]
        .into_iter()
        .flat_map(move |directory| {
            ["mr", "m"].into_iter().filter_map(move |ext| {
                folder.existing_file(&directory.join(format!("{name}.{ext}")))
            })
        })
}
impl Program {
    /// A classdef file: only the local functions are callable by plain name.
    pub(super) fn class_file(
        source: Arc<str>,
        statements: Vec<Statement>,
        path: PathBuf,
        names: BTreeMap<String, usize>,
        class: super::classdef::ClassInfo,
    ) -> Self {
        Self {
            private: prv_folder(&path),
            path: Some(path),
            source,
            statements,
            names,
            scopes: BTreeMap::new(),
            primary: None,
            class: Some(Arc::new(class)),
            method_of: None,
        }
    }
    pub(super) fn function(&self, index: usize) -> Option<&syntax::FunctionDefinition> {
        match &self.statements.get(index)?.kind {
            StatementKind::Function(f) => Some(f),
            _ => None,
        }
    }
    fn new(source: Arc<str>, mut statements: Vec<Statement>, path: Option<PathBuf>) -> Self {
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
            scopes: nested::index(&mut statements),
            private: path.as_deref().and_then(prv_folder),
            path,
            source,
            statements,
            names,
            primary,
            class: None,
            method_of: None,
        }
    }
}
#[derive(Clone, Default)]
pub(super) struct FunctionState {
    pub(super) lng: super::lang_more::LngState,
    directory: Option<PathBuf>,
    /// User search path folders after the current folder, in lookup order:
    /// absolute, canonical, no duplicates. Kept across runs.
    search_path: Vec<PathBuf>,
    /// Hosts opt in to loading preferences once; default engines do no I/O.
    pub(super) spath_loaded: bool,
    /// A session's selected user folder, keyed by preference directory. Another
    /// session writing the same file must not change this session's live path.
    pub(super) spath_userpath: Option<(PathBuf, Option<PathBuf>)>,
    scope: Option<Arc<Program>>,
    /// Lexical function lookup while evaluating an anonymous body. Its
    /// workspace variables still come from its own argument/snapshot maps.
    anonymous_parent: Option<Arc<nested::Frame>>,
    /// Files whose workspace the running scripts share, innermost last: a
    /// script falls back to their local functions after its own (MATLAB).
    hosts: Vec<Arc<Program>>,
    files: BTreeMap<String, Option<Arc<Program>>>,
    /// Name caches of folders left by run('folder/script'), reused on return.
    folders: BTreeMap<PathBuf, BTreeMap<String, Option<Arc<Program>>>>,
    /// Private-folder lookups keyed by (private folder, name), apart from
    /// the name-only `files`: a private function of one folder never answers
    /// a call from another folder or from the Command Window.
    private: BTreeMap<OsString, PrivateFolder>,
    /// The private folder of the document being run, found once per run.
    document_private: Option<Option<PathBuf>>,
    /// Package lookups keyed by (package path, name): the top-level packages
    /// by name, each holding its files and sub-packages. Apart from `files`:
    /// a package function never answers an unqualified call.
    packages: BTreeMap<String, Package>,
    package_count: usize,
    /// The import list of the running workspace. A function starts with an
    /// empty list and drops it on return; a script uses the list of the
    /// workspace it runs in; the base list is kept across runs.
    imports: Arc<Vec<Import>>,
    /// Method files of class folders loaded in this run, keyed by
    /// `@Cls/name` (no extension). Loaded when a method is first called.
    class_methods: BTreeMap<PathBuf, Arc<Program>>,
    source_bytes: usize,
    file_count: usize,
    depth: usize,
    pub(super) retained_bytes: usize,
    pub(super) current: Option<usize>,
    pub arguments: Option<(usize, usize)>,
    pub failure: Option<(Arc<Program>, usize)>,
    pub(super) classes: super::classdef::ClassRunState,
}
impl FunctionState {
    /// `rehash`: forget every cached lookup, including parked folder caches,
    /// so created, edited or deleted files are reflected in the next lookup.
    pub(super) fn lng_rehash(&mut self) {
        self.spath_forget_lookups();
    }
    /// Drop every cached lookup, hit or miss: the lookup order changed.
    /// Running functions keep their own program. Re-reads are not charged
    /// against the per-run file totals (as for a change of folder).
    fn spath_forget_lookups(&mut self) {
        self.files.clear();
        self.folders.clear();
        self.private.clear();
        self.class_methods.clear();
        self.pkg_forget();
        self.file_count = 0;
        self.source_bytes = 0;
    }
    /// `clear functions` and `clear all`: drop cached file lookups. The
    /// running program, the search path and the current folder stay, so a
    /// function that is on the stack can finish.
    pub(super) fn lng_clear_file_caches(&mut self) {
        self.spath_forget_lookups();
    }
    /// `clear classes`: drop cached class files and class-method files.
    /// Classes in `keep` stay, because an instance of them is still reachable.
    pub(super) fn lng_clear_class_caches(&mut self, keep: &BTreeSet<String>) {
        self.files
            .retain(|_, program| class_program_stays(program, keep));
        for map in self.folders.values_mut() {
            map.retain(|_, program| class_program_stays(program, keep));
        }
        for folder in self.private.values_mut() {
            folder
                .names
                .retain(|_, program| class_program_stays(program, keep));
        }
        for package in self.packages.values_mut() {
            package.clear_classes(keep);
        }
        self.class_methods.retain(|_, program| {
            program
                .method_of
                .as_ref()
                .is_some_and(|class| keep.contains(&class.name))
        });
    }
    pub(super) fn lng_forget(&mut self, name: &str) {
        self.files.remove(name);
        // A method file of that name, and the method files of that class.
        self.class_methods.retain(|key, program| {
            key.file_name().is_none_or(|method| method != name)
                && program
                    .method_of
                    .as_ref()
                    .is_none_or(|class| class.constructor_name() != name)
        });
        for folder in self.private.values_mut() {
            folder.names.remove(name);
        }
        for package in self.packages.values_mut() {
            package.forget(name);
        }
    }
    fn pkg_forget(&mut self) {
        self.packages.clear();
        self.package_count = 0;
    }
    /// The cached package `parts` (`a`, then `b` inside it, ...).
    fn pkg_node<S: AsRef<str>>(&self, parts: &[S]) -> Option<&Package> {
        let (first, rest) = parts.split_first()?;
        let mut node = self.packages.get(first.as_ref())?;
        for part in rest {
            node = node.packages.get(part.as_ref())?;
        }
        Some(node)
    }
    fn pkg_node_mut<S: AsRef<str>>(&mut self, parts: &[S]) -> Option<&mut Package> {
        let (first, rest) = parts.split_first()?;
        let mut node = self.packages.get_mut(first.as_ref())?;
        for part in rest {
            node = node.packages.get_mut(part.as_ref())?;
        }
        Some(node)
    }
    /// The import list of the running workspace as text (`a.*`, `a.b.name`).
    pub(super) fn imp_list(&self) -> Vec<String> {
        self.imports.iter().map(Import::text).collect()
    }
    /// `clear import`: empty the import list of the running workspace.
    pub(super) fn imp_clear(&mut self) {
        self.imports = Arc::default();
    }
    /// Enter a function: it starts with an empty import list. Returns the
    /// caller's list for `imp_leave`.
    fn imp_enter(&mut self) -> Arc<Vec<Import>> {
        std::mem::take(&mut self.imports)
    }
    fn imp_leave(&mut self, caller: Arc<Vec<Import>>) {
        self.imports = caller;
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
                .scope
                .as_ref()
                .and_then(|p| p.private.as_ref())
                .and_then(|folder| self.private.get(folder.as_os_str()))
                .and_then(|folder| folder.names.get(name))
                .filter(|private| private.is_some())
                .or_else(|| self.files.get(name))
                .is_some_and(|p| p.as_ref().is_some_and(|p| p.primary.is_some()))
    }
    /// A local function of the running file, or inside a script of a host file.
    fn local_function(&self, name: &str) -> Option<ResolvedFunction> {
        nested::resolve(self, name).or_else(|| self.file_local_function(name))
    }

    fn file_local_function(&self, name: &str) -> Option<ResolvedFunction> {
        self.scope
            .iter()
            .chain(self.hosts.iter().rev())
            .find_map(|program| {
                let index = *program.names.get(name)?;
                Some(ResolvedFunction::at(program.clone(), index))
            })
    }
    /// The running program and the index of the running definition in it.
    pub(super) fn cls_frame(&self) -> Option<(&Arc<Program>, Option<usize>)> {
        self.scope.as_ref().map(|program| (program, self.current))
    }
    /// A class file already loaded in this run (no file access).
    pub(super) fn cls_cached(&self, name: &str) -> Option<&Arc<Program>> {
        self.files
            .get(name)?
            .as_ref()
            .filter(|program| program.class.is_some())
    }
    pub(super) fn active_path(&self) -> Option<&Path> {
        self.scope.as_ref().and_then(|p| p.path.as_deref())
    }
    pub(super) fn active_program(&self) -> Option<&Arc<Program>> {
        self.scope.as_ref()
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
                    && program.scopes.get(&i).is_none_or(|s| s.parent.is_none())
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
        let search_path = std::mem::take(&mut self.search_path);
        let spath_loaded = self.spath_loaded;
        let spath_userpath = self.spath_userpath.take();
        let mut lng = std::mem::take(&mut self.lng);
        lng.thrown_stack = None;
        lng.thrown_object = None;
        let classes = self.classes.after_run();
        // The base workspace's import list lasts until `clear import`.
        let imports = std::mem::take(&mut self.imports);
        *self = Self {
            imports,
            directory,
            search_path,
            spath_loaded,
            spath_userpath,
            lng,
            classes,
            ..Self::default()
        };
    }
}
#[derive(Clone)]
pub(super) struct ResolvedFunction {
    parent: Option<Arc<nested::Frame>>,
    program: Arc<Program>,
    index: usize,
}
impl ResolvedFunction {
    pub(super) fn at(program: Arc<Program>, index: usize) -> Self {
        Self {
            program,
            index,
            parent: None,
        }
    }
    pub(super) fn file(&self) -> Option<&Path> {
        self.program.path.as_deref()
    }
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
    pub(super) fn variadic_output(&self) -> bool {
        match &self.program.statements[self.index].kind {
            StatementKind::Function(function) => {
                function.outputs.last().is_some_and(|n| n == "varargout")
            }
            _ => false,
        }
    }
    /// The last declared input is `varargin`: any number of inputs fits.
    pub(super) fn variadic(&self) -> bool {
        match &self.program.statements[self.index].kind {
            StatementKind::Function(function) => function
                .inputs
                .last()
                .is_some_and(|n| n.as_deref() == Some("varargin")),
            _ => false,
        }
    }
    /// True if the function has an `arguments` block with name-value or repeating inputs.
    pub(super) fn has_nv_or_repeating(&self) -> bool {
        match &self.program.statements[self.index].kind {
            StatementKind::Function(function) => function.arguments.iter().any(|b| {
                b.kind == crate::syntax::ArgumentsKind::Repeating
                    || b.specs.iter().any(|s| s.group.is_some())
            }),
            _ => false,
        }
    }
}
/// The function-lookup context of one workspace: the source whose code runs
/// in it (with the files hosting a running script), the definition running
/// and the import list. `evalin` swaps it together with the variables, so
/// every parked workspace keeps its own (`LngState`).
#[derive(Clone, Default)]
pub(super) struct LookupContext {
    scope: Option<Arc<Program>>,
    /// Lexical function lookup while evaluating an anonymous body. Its
    /// workspace variables still come from its own argument/snapshot maps.
    anonymous_parent: Option<Arc<nested::Frame>>,
    hosts: Vec<Arc<Program>>,
    imports: Arc<Vec<Import>>,
    current: Option<usize>,
}
impl LookupContext {
    /// The Command Window's context for the base workspace this one is: no
    /// file runs there, so a script running in it gives no private folder
    /// and no local functions. The functions of text submitted without a
    /// file and the base import list stay.
    pub(super) fn ctx_base(&self) -> Self {
        let outermost = self.hosts.first().or(self.scope.as_ref());
        Self {
            anonymous_parent: None,
            scope: outermost.filter(|program| program.path.is_none()).cloned(),
            hosts: Vec::new(),
            imports: self.imports.clone(),
            current: None,
        }
    }
    /// The function `name` of the source `file` (None: text run without a
    /// file) when the code of this workspace belongs to that source.
    fn ctx_function(&self, name: &str, file: Option<&Path>) -> Option<ResolvedFunction> {
        self.scope
            .iter()
            .chain(self.hosts.iter().rev())
            .find_map(|program| {
                if program.path.as_deref() != file {
                    return None;
                }
                let index = *program.names.get(name)?;
                Some(ResolvedFunction::at(program.clone(), index))
            })
    }
}
impl FunctionState {
    /// The lookup context of the running workspace (shared, no copies).
    pub(super) fn ctx_snapshot(&self) -> LookupContext {
        LookupContext {
            anonymous_parent: self.anonymous_parent.clone(),
            scope: self.scope.clone(),
            hosts: self.hosts.clone(),
            imports: self.imports.clone(),
            current: self.current,
        }
    }
    /// Make `context` the lookup context of the running code; returns the
    /// one it replaces, to be put back the same way.
    pub(super) fn ctx_replace(&mut self, context: LookupContext) -> LookupContext {
        LookupContext {
            anonymous_parent: std::mem::replace(
                &mut self.anonymous_parent,
                context.anonymous_parent,
            ),
            scope: std::mem::replace(&mut self.scope, context.scope),
            hosts: std::mem::replace(&mut self.hosts, context.hosts),
            imports: std::mem::replace(&mut self.imports, context.imports),
            current: std::mem::replace(&mut self.current, context.current),
        }
    }
}
impl Engine {
    /// `name` is a function of the source the running code belongs to (a
    /// local function, or the main function of the file): the file of that
    /// source, None for text run without a file. A handle made here is
    /// bound to that source.
    pub(super) fn ctx_own_source(&self, name: &str) -> Option<Option<PathBuf>> {
        let function = self.functions.file_local_function(name)?;
        Some(function.program.path.clone())
    }
    /// The function `name` of the source `file` a handle was bound to, when
    /// that source is running: in the running workspace or in one of the
    /// workspaces that wait for it, innermost first. No file access.
    pub(super) fn ctx_source_function(
        &self,
        name: &str,
        file: Option<&Path>,
    ) -> Option<ResolvedFunction> {
        let functions = &self.functions;
        functions
            .ctx_snapshot()
            .ctx_function(name, file)
            .or_else(|| {
                functions
                    .lng
                    .ctx_parked()
                    .find_map(|context| context.ctx_function(name, file))
            })
    }
    /// The file the lookup every caller shares finds for `name`: the file of
    /// a package-qualified name, else the current folder and the search path.
    fn ctx_path_program(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        if name.as_bytes().contains(&b'.') {
            self.pkg_qualified_program(name)
        } else {
            self.spath_folder_program(name, true)
        }
    }
    /// What a call of `name` runs when no function of the calling code (an
    /// imported, local or private one) is a candidate: `ctx_path_program`.
    pub(super) fn ctx_path_function(&mut self, name: &str) -> Result<Option<ResolvedFunction>> {
        if self.lg3_builtin_only(name) {
            return Ok(None);
        }
        match self.ctx_path_program(name)? {
            Some(program) => program_function(name, program),
            None => Ok(None),
        }
    }
    /// What a call of `name` by the running code runs when that is one of
    /// its own functions (imported, local or private): what
    /// `resolve_function` finds, unless that is the file `ctx_path_program`
    /// finds for every caller. Reference precedence puts a method of an
    /// object argument between the two.
    pub(super) fn ctx_scoped_function(&mut self, name: &str) -> Result<Option<ResolvedFunction>> {
        let Some(resolved) = self.resolve_file_function(name)? else {
            return Ok(None);
        };
        let shared = self.ctx_path_program(name)?;
        Ok(Some(resolved).filter(|function| {
            shared
                .as_ref()
                .is_none_or(|program| !Arc::ptr_eq(&function.program, program))
        }))
    }
}
/// What a handle made by running code keeps of it (the handle's opaque
/// `Scope`), so the names its body uses, or the local function it names,
/// are looked up as there wherever it is called: the file and the files
/// hosting a running script (local functions, private folder) and the import
/// list, all shared, no copies. `document`: the document being run, which
/// gives the private folder and `mfilename` when no file of `lookup` does.
pub(super) struct HandleScope {
    lookup: LookupContext,
    document: Option<PathBuf>,
}
/// What `ctx_enter_handle` replaced; `ctx_leave_handle` puts it back.
pub(super) struct HandleCaller {
    lookup: LookupContext,
    document: Option<(Option<PathBuf>, Option<Option<PathBuf>>)>,
}
impl HandleScope {
    /// The function `name` of the file `file` in this scope: what a handle
    /// to a local function made there calls after its file returned.
    pub(super) fn ctx_function(&self, name: &str, file: &Path) -> Option<ResolvedFunction> {
        self.lookup.ctx_function(name, Some(file))
    }
    /// A file of the scope runs (a function or script file): it gives the
    /// private folder and `mfilename`, not the document.
    fn has_file(&self) -> bool {
        self.lookup
            .scope
            .as_ref()
            .is_some_and(|program| program.path.is_some())
    }
}
impl Engine {
    /// The scope a handle made now keeps (`HandleScope`). `file`: the
    /// functions of the running source (local, private) are part of it;
    /// otherwise only the import list, as at the Command Window (text of an
    /// anonymous function given to `str2func`).
    pub(super) fn ctx_handle_scope(
        &self,
        file: bool,
        anonymous_parent: Option<Arc<nested::Frame>>,
    ) -> crate::function_handle::Scope {
        let functions = &self.functions;
        let lookup = if file {
            LookupContext {
                current: None,
                anonymous_parent,
                ..functions.ctx_snapshot()
            }
        } else {
            LookupContext {
                imports: functions.imports.clone(),
                ..LookupContext::default()
            }
        };
        let document = if file {
            self.document_path.clone()
        } else {
            None
        };
        Arc::new(HandleScope { lookup, document })
    }
    /// Make the scope a handle keeps the lookup context of the running code
    /// (its body runs as if inside the code that made it). Returns the
    /// replaced context for `ctx_leave_handle`; restore it on every path.
    pub(super) fn ctx_enter_handle(&mut self, scope: &HandleScope) -> HandleCaller {
        let lookup = self.functions.ctx_replace(scope.lookup.clone());
        // The document matters only when no file of the scope runs; it
        // comes with its private folder (found on first use).
        let document = (!scope.has_file() && scope.document != self.document_path).then(|| {
            (
                std::mem::replace(&mut self.document_path, scope.document.clone()),
                self.functions.document_private.take(),
            )
        });
        HandleCaller { lookup, document }
    }
    pub(super) fn ctx_leave_handle(&mut self, caller: HandleCaller) {
        self.functions.ctx_replace(caller.lookup);
        if let Some((document, private)) = caller.document {
            self.document_path = document;
            self.functions.document_private = private;
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
            // The dropped caches are re-read on demand; those re-reads do not
            // count against the per-run file and byte totals.
            self.functions.spath_forget_lookups();
        }
        self.functions.directory = Some(path);
        Ok(())
    }
    /// The folders `help` searches for user functions, in the order calls do.
    pub fn help_path(&self) -> crate::help::PathContext {
        let mut folders = self.spath_folders().into_iter();
        let directory = self.spath_current().map(Path::to_path_buf);
        if directory.is_some() {
            folders.next();
        }
        crate::help::PathContext {
            directory,
            path: folders.collect(),
        }
    }
    /// The folder searched first: the function directory, else the current folder.
    fn spath_current(&self) -> Option<&Path> {
        self.functions
            .directory
            .as_deref()
            .or(self.file_directory.as_deref())
    }
    /// The ordered folders searched for a .mr/.m file by name: the current
    /// folder, then the user search path. The ONE lookup order shared by
    /// calls, feval, handles, run, which, exist and help.
    pub(super) fn spath_folders(&self) -> Vec<PathBuf> {
        let current = self.spath_current();
        current
            .map(Path::to_path_buf)
            .into_iter()
            .chain(
                self.functions
                    .search_path
                    .iter()
                    .filter(|p| Some(p.as_path()) != current)
                    .cloned(),
            )
            .collect()
    }
    /// Every existing .mr/.m file for `name` in lookup order (class folders
    /// `@name` first, then plain files; .mr before .m within a folder); the
    /// first one is what a call runs.
    pub(super) fn spath_files(&self, name: &str, all: bool) -> Vec<PathBuf> {
        let (name, package) = if crate::files::valid_name(name) {
            (name, PathBuf::new())
        } else if let Some(parts) = pkg_parts(name) {
            let (name, packages) = parts.split_last().expect("two or more parts");
            (*name, pkg_folder(Path::new(""), packages))
        } else {
            return Vec::new();
        };
        let context = self.help_path();
        let mut hits = context
            .folders()
            .flat_map(|folder| spath_existing(folder, &package, name));
        if all {
            hits.collect()
        } else {
            hits.next().into_iter().collect()
        }
    }
    /// The user search path folders (current folder excluded).
    pub(super) fn spath_list(&self) -> &[PathBuf] {
        &self.functions.search_path
    }
    /// Replace the user search path; cached lookups are dropped so the new
    /// order applies from the next call.
    pub(super) fn spath_set(&mut self, folders: Vec<PathBuf>) {
        self.functions.search_path = folders;
        self.functions.spath_forget_lookups();
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
            // Script text of a named document. A function file keeps only its
            // function frame (the branch below).
            self.lg3_document_script(Arc::from(source), |engine| {
                engine.execute_statements(&statements, byte, out)
            })?;
            return Ok(());
        }
        let program = Arc::new(Program::new(Arc::from(source), statements, None));
        self.functions.source_bytes = source.len();
        self.functions.scope = Some(program.clone());
        if let Some(index) = program.primary {
            *byte = program.statements[index].span.start;
            let result = self.invoke_function(
                ResolvedFunction::at(program, index),
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
            // Script with local functions: the document frame is the script,
            // not an extra wrapper around a primary function.
            self.lg3_document_script(program.source.clone(), |engine| {
                engine.execute_statements(&program.statements, byte, out)
            })?;
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
        if !self.lg3_builtin_only(name)
            && let Some(function) = nested::resolve(&self.functions, name)
        {
            self.check_function_input(name)?;
            return Ok(Some(function));
        }
        self.resolve_file_function(name)
    }

    /// Resolution available to string-based calls, which cannot bind a
    /// nested definition to a lexical activation.
    fn resolve_file_function(&mut self, name: &str) -> Result<Option<ResolvedFunction>> {
        self.check_function_input(name)?;
        if self.lg3_builtin_only(name) {
            return Ok(None);
        }
        // An explicitly imported name comes before file-local functions.
        if !self.functions.imports.is_empty()
            && let Some(program) = self.imp_program(name, false)?
        {
            return program_function(name, program);
        }
        if let Some(function) = self.functions.file_local_function(name) {
            return Ok(Some(function));
        }
        match self.folder_program(name)? {
            Some(program) => program_function(name, program),
            None => Ok(None),
        }
    }
    /// File of `name` when it is a local (not the primary) function of the
    /// running program: a handle made here is bound to that file.
    pub(super) fn lg3_local_file(&self, name: &str) -> Option<PathBuf> {
        let function = self.functions.file_local_function(name)?;
        if function.program.primary == Some(function.index) {
            return None;
        }
        function.program.path.clone()
    }
    /// A statement `name` or `name()`: a folder script unless a visible local
    /// function takes precedence. Variables are checked by the caller.
    pub(super) fn resolve_script(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        self.check_function_input(name)?;
        if nested::resolve(&self.functions, name).is_some() {
            return Ok(None);
        }
        if !self.functions.imports.is_empty()
            && let Some(program) = self.imp_program(name, false)?
        {
            return Ok(Some(program).filter(|p| p.primary.is_none() && p.class.is_none()));
        }
        if self.functions.file_local_function(name).is_some() {
            return Ok(None);
        }
        Ok(self
            .folder_program(name)?
            .filter(|p| p.primary.is_none() && p.class.is_none()))
    }
    pub(super) fn function_directory(&self) -> Option<&Path> {
        self.functions.directory.as_deref()
    }
    /// The .mr/.m file a name used by the running code resolves to, function
    /// or script, cached per run. A package-qualified name (`a.b.name`) is
    /// the file of that package and nothing else. For a plain name the steps
    /// that depend on the running code come first (the import list of its
    /// workspace, explicit entries before wildcards, then the private folder
    /// of its file), then the lookup every caller shares: the current folder
    /// and the search path.
    pub(super) fn folder_program(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        if name.as_bytes().contains(&b'.') {
            return self.pkg_qualified_program(name);
        }
        if !self.functions.imports.is_empty() {
            if let Some(program) = self.imp_program(name, false)? {
                return Ok(Some(program));
            }
            if let Some(program) = self.imp_program(name, true)? {
                return Ok(Some(program));
            }
        }
        self.cls_stored_program(name)
    }
    /// The class file of the class name stored in an existing object (or the
    /// file of a plain name after the import steps): the import list never
    /// applies, so `import a.C` does not change what an object of `C` is.
    pub(super) fn cls_stored_program(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        if name.as_bytes().contains(&b'.') {
            return self.pkg_qualified_program(name);
        }
        if let Some(program) = self.prv_program(name)? {
            return Ok(Some(program));
        }
        self.spath_folder_program(name, true)
    }
    /// True when a plain call `name(obj, ...)` reaches a function before a
    /// method of the object's class (reference precedence: explicit import,
    /// local function, wildcard import, private function, then the methods
    /// of the arguments' class; the current folder, the path and builtins
    /// come after the method). The method file that is running does not
    /// shadow itself.
    pub(super) fn cls_shadowed(&mut self, name: &str) -> Result<bool> {
        if !crate::files::valid_name(name) || self.lg3_builtin_only(name) {
            return Ok(false);
        }
        let imports = !self.functions.imports.is_empty();
        if imports && self.imp_program(name, false)?.is_some() {
            return Ok(true);
        }
        // A file's own primary function is not a local function: inside a plain
        // `area.m`, `area(obj)` still dispatches to the object's method.
        if let Some(local) = self.functions.local_function(name)
            && local.program.primary != Some(local.index)
        {
            return Ok(true);
        }
        if imports && self.imp_program(name, true)?.is_some() {
            return Ok(true);
        }
        Ok(self.prv_program(name)?.is_some())
    }
    /// True when `parts` (`a`, then `b` inside it, ...) is a package: a
    /// `+a` folder directly inside the current folder or a search path
    /// folder, `+b` inside one of those. Looked up once per run.
    fn pkg_ensure<S: AsRef<str>>(&mut self, parts: &[S]) -> Result<bool> {
        for depth in 1..=parts.len() {
            match self.functions.pkg_node(&parts[..depth]) {
                Some(package) if package.folders.is_empty() => return Ok(false),
                Some(_) => continue,
                None => {}
            }
            let name = parts[depth - 1].as_ref();
            if !crate::files::valid_name(name) {
                return Ok(false);
            }
            if self.functions.package_count >= 256 {
                return Err("Function resolution exceeds 256 packages per run".into());
            }
            self.check_cancel()?;
            let parents = match self.functions.pkg_node(&parts[..depth - 1]) {
                Some(parent) => parent.folders.clone(),
                None => self.spath_folders(),
            };
            let folder = format!("+{name}");
            let package = Package {
                folders: parents
                    .iter()
                    .map(|parent| parent.join(&folder))
                    .filter(|path| path.is_dir())
                    .collect(),
                ..Package::default()
            };
            let exists = !package.folders.is_empty();
            let siblings = match self.functions.pkg_node_mut(&parts[..depth - 1]) {
                Some(parent) => &mut parent.packages,
                None => &mut self.functions.packages,
            };
            siblings.insert(name.to_owned(), package);
            self.functions.package_count += 1;
            if !exists {
                return Ok(false);
            }
        }
        Ok(!parts.is_empty())
    }
    /// True when the plain name `name` is a top-level package.
    pub(super) fn pkg_is_package(&mut self, name: &str) -> Result<bool> {
        // Per use: one map lookup, no allocation and no file access.
        if let Some(package) = self.functions.packages.get(name) {
            return Ok(!package.folders.is_empty());
        }
        self.pkg_ensure(&[name])
    }
    /// The file `name` of package `package`, cached by (package, name): the
    /// first of the package's folders that has `name.mr`, else `name.m`.
    fn pkg_file<S: AsRef<str>>(
        &mut self,
        package: &[S],
        name: &str,
    ) -> Result<Option<Arc<Program>>> {
        if !self.pkg_ensure(package)? {
            return Ok(None);
        }
        let Some(node) = self.functions.pkg_node(package) else {
            return Ok(None);
        };
        if let Some(cached) = node.names.get(name) {
            return Ok(cached.clone());
        }
        if !crate::files::valid_name(name) {
            return Ok(None);
        }
        if node.names.len() >= 256 {
            return Err("Function resolution exceeds 256 names per run".into());
        }
        let folders = node.folders.clone();
        self.check_cancel()?;
        let program = match spath_open(&folders, name, None)? {
            Some((path, file)) => {
                let qualifier = package
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join(".");
                Some(self.spath_load(name, &qualifier, path, file)?)
            }
            None => None,
        };
        if let Some(node) = self.functions.pkg_node_mut(package) {
            node.names.insert(name.into(), program.clone());
        }
        Ok(program)
    }
    /// What the dotted names `parts` reach when `parts[0]` is a package:
    /// sub-packages are entered while one matches and a name follows it,
    /// then one file of the package reached. Returns that file and how many
    /// parts name it; any parts left index into what the file returns.
    pub(super) fn pkg_resolve(&mut self, parts: &[&str]) -> Result<Option<(Arc<Program>, usize)>> {
        if parts.len() < 2 || !self.pkg_is_package(parts[0])? {
            return Ok(None);
        }
        let mut depth = 1;
        while depth + 1 < parts.len() && self.pkg_ensure(&parts[..depth + 1])? {
            depth += 1;
        }
        Ok(self
            .pkg_file(&parts[..depth], parts[depth])?
            .map(|program| (program, depth + 1)))
    }
    /// The file the package-qualified name `a.b.name` is, every part used.
    pub(super) fn pkg_qualified_program(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        let Some(parts) = pkg_parts(name) else {
            return Ok(None);
        };
        Ok(match self.pkg_resolve(&parts)? {
            Some((program, used)) if used == parts.len() => Some(program),
            _ => None,
        })
    }
    /// What a call of the file `program`, found as `name`, runs.
    pub(super) fn pkg_function(
        &self,
        name: &str,
        program: Arc<Program>,
    ) -> Result<Option<ResolvedFunction>> {
        program_function(name, program)
    }
    /// The package file the plain name `name` is imported as in the running
    /// workspace: by an explicit entry (`import a.name`), or by the first
    /// wildcard entry (`import a.*`) whose package has it.
    fn imp_program(&mut self, name: &str, wildcard: bool) -> Result<Option<Arc<Program>>> {
        let imports = self.functions.imports.clone();
        for import in imports.iter() {
            let listed = match &import.name {
                Some(imported) => !wildcard && imported == name,
                None => wildcard,
            };
            if listed && let Some(program) = self.pkg_file(&import.package, name)? {
                return Ok(Some(program));
            }
        }
        Ok(None)
    }
    /// The qualified name a handle to the plain name `name` is bound to when
    /// the running workspace imports it: an explicit entry first, a wildcard
    /// entry unless a local function has the name.
    pub(super) fn imp_qualified(&mut self, name: &str) -> Result<Option<String>> {
        if self.functions.imports.is_empty() || !crate::files::valid_name(name) {
            return Ok(None);
        }
        let imports = self.functions.imports.clone();
        for wildcard in [false, true] {
            if wildcard && self.functions.file_local_function(name).is_some() {
                break;
            }
            for import in imports.iter() {
                let listed = match &import.name {
                    Some(imported) => !wildcard && imported == name,
                    None => wildcard,
                };
                if listed && self.pkg_file(&import.package, name)?.is_some() {
                    return Ok(Some(format!("{}.{name}", import.package.join("."))));
                }
            }
        }
        Ok(None)
    }
    /// Existing files the plain name `name` is imported as, explicit entries
    /// first (uncached: `which`, `exist`).
    pub(super) fn imp_files(&self, name: &str, all: bool) -> Vec<PathBuf> {
        let mut found = Vec::new();
        if self.functions.imports.is_empty() || !crate::files::valid_name(name) {
            return found;
        }
        for wildcard in [false, true] {
            for import in self.functions.imports.iter() {
                let listed = match &import.name {
                    Some(imported) => !wildcard && imported == name,
                    None => wildcard,
                };
                if listed {
                    let qualified = format!("{}.{name}", import.package.join("."));
                    found.extend(self.spath_files(&qualified, all));
                    if !all && !found.is_empty() {
                        return found;
                    }
                }
            }
        }
        found
    }
    /// `import a.*`, `import a.b.name`: add the entries to the import list of
    /// the running workspace, all or none. A wildcard entry needs its
    /// package, a named entry its file.
    pub(super) fn imp_add(&mut self, entries: &[String]) -> Result<()> {
        let unknown = |entry: &str| {
            format!("Import argument '{entry}' cannot be found or cannot be imported.")
        };
        let mut added = Vec::new();
        for entry in entries {
            let import = match entry.strip_suffix(".*") {
                Some(package) => {
                    let parts: Vec<&str> = package.split('.').collect();
                    if parts.len() > 8
                        || parts.iter().any(|part| !crate::files::valid_name(part))
                        || !self.pkg_ensure(&parts)?
                    {
                        return Err(unknown(entry));
                    }
                    Import {
                        package: parts.into_iter().map(str::to_owned).collect(),
                        name: None,
                    }
                }
                None => {
                    let Some(parts) = pkg_parts(entry) else {
                        return Err(unknown(entry));
                    };
                    let (name, package) = parts.split_last().expect("two or more parts");
                    if self.pkg_file(package, name)?.is_none() {
                        return Err(unknown(entry));
                    }
                    Import {
                        package: package.iter().map(|part| (*part).to_owned()).collect(),
                        name: Some((*name).to_owned()),
                    }
                }
            };
            added.push(import);
        }
        if self.functions.imports.len() + added.len() > 256 {
            return Err("import: at most 256 entries per workspace".into());
        }
        let imports = Arc::make_mut(&mut self.functions.imports);
        for import in added {
            if !imports.contains(&import) {
                imports.push(import);
            }
        }
        Ok(())
    }
    /// The private-folder file `name` the running code sees (reference
    /// precedence: after local functions, before the current folder and the
    /// search path): the private folder of the running file, else of the
    /// document being run. None at the Command Window, where no file runs.
    fn prv_program(&mut self, name: &str) -> Result<Option<Arc<Program>>> {
        let functions = &mut self.functions;
        let folder = match &functions.scope {
            Some(program) if program.path.is_some() => program.private.as_deref(),
            _ => {
                let document = self.document_path.as_deref();
                functions
                    .document_private
                    .get_or_insert_with(|| document.and_then(prv_folder))
                    .as_deref()
            }
        };
        let Some(folder) = folder else {
            return Ok(None);
        };
        // Per call: two map lookups, no allocation and no file access.
        if let Some(cache) = functions.private.get(folder.as_os_str()) {
            if !cache.exists {
                return Ok(None);
            }
            if let Some(cached) = cache.names.get(name) {
                return Ok(cached.clone());
            }
        }
        let folder = folder.to_path_buf();
        self.prv_folder_program(&folder, name)
    }
    /// `name` in the private folder `folder`, cached by (folder, name).
    fn prv_folder_program(&mut self, folder: &Path, name: &str) -> Result<Option<Arc<Program>>> {
        let key = folder.as_os_str();
        match self.functions.private.get(key) {
            Some(cache) if !cache.exists => return Ok(None),
            Some(cache) => {
                if let Some(cached) = cache.names.get(name) {
                    return Ok(cached.clone());
                }
                if cache.names.len() >= 256 {
                    return Err("Function resolution exceeds 256 names per run".into());
                }
            }
            None => {
                if self.functions.private.len() >= 256 {
                    return Err("Function resolution exceeds 256 private folders per run".into());
                }
                let exists = folder.is_dir();
                let cache = PrivateFolder {
                    exists,
                    names: BTreeMap::new(),
                };
                self.functions.private.insert(key.to_owned(), cache);
                if !exists {
                    return Ok(None);
                }
            }
        }
        if !crate::files::valid_name(name) {
            return Ok(None);
        }
        self.check_cancel()?;
        // Plain files only: a class folder inside a private folder is not looked up.
        let program = match spath_open_in(folder, name, None)? {
            Some((path, file)) => Some(self.spath_load(name, "", path, file)?),
            None => None,
        };
        if let Some(cache) = self.functions.private.get_mut(key) {
            cache.names.insert(name.into(), program.clone());
        }
        Ok(program)
    }
    /// Existing private files for `name` that the running code sees, .mr
    /// before .m (uncached: `which` and `exist`).
    pub(super) fn prv_files(&self, name: &str) -> Vec<PathBuf> {
        if !crate::files::valid_name(name) {
            return Vec::new();
        }
        let folder = match &self.functions.scope {
            Some(program) if program.path.is_some() => program.private.clone(),
            _ => self.document_path.as_deref().and_then(prv_folder),
        };
        let Some(folder) = folder else {
            return Vec::new();
        };
        // Known missing in this run: no file access.
        if self
            .functions
            .private
            .get(folder.as_os_str())
            .is_some_and(|cache| !cache.exists)
        {
            return Vec::new();
        }
        ["mr", "m"]
            .into_iter()
            .map(|extension| folder.join(format!("{name}.{extension}")))
            .filter(|path| path.is_file())
            .collect()
    }
    /// `@name` or str2func inside a file: a handle bound to the private file
    /// when `name` is not a local function and the private folder has it.
    pub(super) fn prv_handle(&mut self, name: &str) -> Result<Option<crate::FunctionHandle>> {
        if self.functions.local_function(name).is_some() {
            return Ok(None);
        }
        Ok(self.prv_program(name)?.and_then(|program| {
            program
                .path
                .as_deref()
                .map(|path| crate::FunctionHandle::private(name, path))
        }))
    }
    /// The function a handle bound to the private file `file` calls, from
    /// any folder and from the Command Window. None when the file is gone.
    pub(super) fn prv_bound_function(
        &mut self,
        name: &str,
        file: &Path,
    ) -> Result<Option<ResolvedFunction>> {
        let Some(folder) = file.parent() else {
            return Ok(None);
        };
        match self.prv_folder_program(folder, name)? {
            Some(program) => program_function(name, program),
            None => Ok(None),
        }
    }
    /// An explicit run target searches only its folder, including cached hits.
    fn spath_folder_program(
        &mut self,
        name: &str,
        include_path: bool,
    ) -> Result<Option<Arc<Program>>> {
        if self.spath_current().is_none()
            && (!include_path || self.functions.search_path.is_empty())
        {
            return Ok(None);
        }
        if let Some(cached) = self.functions.files.get(name)
            && (include_path
                || cached.as_ref().is_some_and(|program| {
                    program.path.as_deref().and_then(Path::parent) == self.spath_current()
                }))
        {
            return Ok(cached.clone());
        }
        if !crate::files::valid_name(name) {
            return Ok(None);
        }
        if self.functions.files.len() >= 256 {
            return Err("Function resolution exceeds 256 names per run".into());
        }
        self.check_cancel()?;
        // Current folder, then search path for an unqualified lookup; native
        // .mr wins over .m within a folder. The per-run cache reset also makes a
        // newly added .mr visible on the next run.
        let folders = if include_path {
            self.spath_folders()
        } else {
            self.spath_current()
                .map(Path::to_path_buf)
                .into_iter()
                .collect()
        };
        let current = self.spath_current().map(Path::to_path_buf);
        let Some((path, file)) = spath_open(&folders, name, current.as_deref())? else {
            // A folder-only miss must not hide a subsequent search-path lookup.
            if include_path {
                self.functions.files.insert(name.into(), None);
            }
            return Ok(None);
        };
        let program = self.spath_load(name, "", path, file)?;
        self.functions
            .files
            .insert(name.into(), Some(program.clone()));
        Ok(Some(program))
    }
    /// The text of the opened .mr/.m file `path`, checked against the
    /// per-run file and byte totals (the caller adds it to them).
    fn spath_read(&mut self, path: &Path, file: File) -> Result<String> {
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
        let mut bytes = Vec::new();
        file.take(1_000_001)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.len() > 1_000_000 {
            return Err("Function source exceeds 1 MB".into());
        }
        let source = crate::files::decode_script_source(path, bytes)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if source.len() > 1_000_000 {
            return Err("Function source exceeds 1 MB".into());
        }
        if self.functions.source_bytes + source.len() > 8 * 1024 * 1024 {
            return Err("Function sources exceed 8 MiB per run".into());
        }
        self.check_cancel()?;
        Ok(source)
    }
    /// The statements of the function or script file `path`.
    fn spath_parse(&mut self, path: &Path, source: &str) -> Result<Vec<Statement>> {
        syntax::parse_detailed(source).map_err(|e| {
            self.file_diagnostic = Some(super::FileDiagnostic {
                path: path.to_owned(),
                source: Arc::from(source),
                diagnostic: e.clone(),
            });
            format!("{}: {e}", path.display())
        })
    }
    /// The method `name` of `class` kept in its own file of the class folder
    /// `folder` (`@Cls/name.mr`, else `.m`): its program, whose first
    /// function is the method. Loaded when first called in a run, charged
    /// like any function file, then two map lookups per call. None: the
    /// folder has no such file.
    pub(super) fn cls_method_program(
        &mut self,
        class: &Arc<super::classdef::ClassInfo>,
        folder: &Path,
        name: &str,
    ) -> Result<Option<Arc<Program>>> {
        let key = folder.join(name);
        if let Some(program) = self.functions.class_methods.get(&key) {
            return Ok(Some(program.clone()));
        }
        if self.functions.class_methods.len() >= 256 {
            return Err("Function resolution exceeds 256 method files per run".into());
        }
        self.check_cancel()?;
        let Some((path, file)) = spath_open_in(folder, name, None)? else {
            return Ok(None);
        };
        let source = self.spath_read(&path, file)?;
        let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
        if syntax::is_class_file(source) {
            return Err(format!(
                "{}: a file of the class folder of '{}' other than its class file cannot be a classdef file",
                path.display(),
                class.name
            ));
        }
        let statements = self.spath_parse(&path, source)?;
        let mut program = Program::new(Arc::from(source), statements, Some(path.clone()));
        let named = program
            .primary
            .and_then(|index| program.function(index))
            .is_some_and(|function| function.name == name);
        if !named {
            return Err(format!(
                "{}: a method file of class '{}' must start with the function '{name}'",
                path.display(),
                class.name
            ));
        }
        program.method_of = Some(class.clone());
        let program = Arc::new(program);
        self.functions.source_bytes += source.len();
        self.functions.file_count += 1;
        self.functions.class_methods.insert(key, program.clone());
        Ok(Some(program))
    }
    /// Read and parse the opened .mr/.m file `path` as the definition of
    /// `name`, charged against the per-run file and byte totals. The caller
    /// caches the result. `package` is the package the file belongs to
    /// (`a.b`), empty for none: a class of a package is named `a.b.Name`.
    fn spath_load(
        &mut self,
        name: &str,
        package: &str,
        path: PathBuf,
        file: File,
    ) -> Result<Arc<Program>> {
        let source = self.spath_read(&path, file)?;
        let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
        if syntax::is_class_file(source) {
            let program = super::classdef::class_program(self, name, package, &path, source)?;
            self.functions.source_bytes += source.len();
            self.functions.file_count += 1;
            return Ok(program);
        }
        // `@name/name.m` that is a function or a script: a class of the
        // old kind, whose constructor calls class().
        if cls_folder(&path, name).is_some() {
            return Err(format!(
                "Old-style @{name} class folders (class() constructors) are not supported; use a classdef file."
            ));
        }
        let statements = self.spath_parse(&path, source)?;
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
        Ok(program)
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
        if self.profiler.enabled {
            return super::profiler::program_call(
                self,
                &program.clone(),
                name,
                "M-script",
                |engine| engine.profiler_run_script(program, name, out),
            );
        }
        self.profiler_run_script(program, name, out)
    }
    fn profiler_run_script(
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
        // The script shares the caller workspace (not an `lng_enter` frame) but
        // it is still a dbstack frame. Leave on error so neither the frame nor
        // the call-site byte remains.
        let label = program.path.as_deref().and_then(Self::lg3_script_label);
        let result = match label {
            Some(label) => self.lg3_scoped(&label, program.source.clone(), |engine| {
                engine.execute_statements(&program.statements, &mut byte, out)
            }),
            None => self.execute_statements(&program.statements, &mut byte, out),
        };
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
        let include_path = folder.is_none();
        let Some(folder) = folder.filter(|f| self.functions.directory.as_ref() != Some(f)) else {
            return self.run_folder_file(name, text, include_path, out);
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
        // Packages are looked up from the current folder: start over there
        // and again on return.
        self.functions.pkg_forget();
        let caller_file_directory = self.file_directory.replace(folder.clone());
        let result = self.run_folder_file(name, text, include_path, out);
        self.functions.pkg_forget();
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
    fn run_folder_file(
        &mut self,
        name: &str,
        text: &str,
        include_path: bool,
        out: &mut Execution,
    ) -> Result<()> {
        let program = self
            .spath_folder_program(name, include_path)?
            .ok_or_else(|| format!("{text} not found."))?;
        if program.class.is_some() {
            return Err(format!(
                "{text} is a class definition; call {name}(...) to construct an object"
            ));
        }
        let Some(index) = program.primary else {
            return self.run_script(program, name, out);
        };
        // A function file runs as the quiet statement `name;` would.
        let results = self.invoke_function(
            ResolvedFunction::at(program, index),
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
            let nv_start = args.iter().position(|a| matches!(a, Expr::NameValue(_)));
            let values = args
                .iter()
                .map(|arg| self.eval(arg, end, out))
                .collect::<Result<Vec<_>>>()?;
            self.functions.lng.pending_nv_start = nv_start;
            return Ok(values);
        }
        let mut values = Vec::new();
        let mut nv_start = None;
        for arg in args {
            if nv_start.is_none() && matches!(arg, Expr::NameValue(_)) {
                nv_start = Some(values.len());
            }
            let list = self.eval_list(arg, end, out)?;
            if values.len().saturating_add(list.len()) > crate::script_containers::MAX_CELL_SLOTS {
                return Err("Argument expansion exceeds 65536 values".into());
            }
            values.extend(list);
        }
        self.functions.lng.pending_nv_start = nv_start;
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
        if self.profiler.enabled {
            let program = resolved.program.clone();
            let function = program.function(resolved.index).unwrap();
            let local = program.primary != Some(resolved.index);
            let name = if local {
                format!(
                    "{}>{}",
                    program
                        .path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .and_then(|s| s.to_str())
                        .unwrap_or("prompt"),
                    function.name
                )
            } else {
                function.name.clone()
            };
            return super::profiler::program_call(
                self,
                &program,
                &name,
                if local { "M-subfunction" } else { "M-function" },
                |engine| engine.profiler_invoke_function(resolved, args, requested, targets, out),
            );
        }
        self.profiler_invoke_function(resolved, args, requested, targets, out)
    }
    fn profiler_invoke_function(
        &mut self,
        resolved: ResolvedFunction,
        args: Vec<Value>,
        requested: usize,
        targets: Option<&[Option<String>]>,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        // Taken on entry so a failing call never leaves names for the next one.
        let input_names = std::mem::take(&mut self.functions.lng.pending_names);
        let nv_start = self.functions.lng.pending_nv_start.take();
        // A class constructor starts from the default object (defaults are
        // evaluated once per run, before this frame is entered), or from the
        // subclass object a superclass constructor call hands over; a private
        // constructor runs only for the class's own methods.
        let preset = match &resolved.program.class {
            Some(class) if class.constructor == resolved.index => {
                Some(self.cls_constructor_preset(&resolved.program, out)?)
            }
            _ => None,
        };
        let nested_name = resolved
            .parent
            .as_ref()
            .map(|_| nested::qualified_name(&resolved));
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
        let has_nv_or_repeating = function.arguments.iter().any(|b| {
            b.kind == crate::syntax::ArgumentsKind::Repeating
                || b.specs.iter().any(|s| s.group.is_some())
        });
        if args.len() > function.inputs.len() && !varargs && !has_nv_or_repeating {
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
            > super::limits::memory_bytes(&self.limits)
        {
            return Err(format!(
                "Retained function caller workspaces/evaluation estimates exceed {} MiB",
                super::limits::memory_bytes(&self.limits) / (1024 * 1024)
            ));
        }
        let input_count = args.len();
        let raw_args_for_validation: Vec<Value> = if !function.arguments.is_empty() {
            args.iter()
                .map(|v| {
                    Ok(Value::from(
                        stripping.strip(&v.clone().array(&self.cancel)?)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
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
        let has_input_or_repeating_block = function.arguments.iter().any(|b| {
            matches!(
                b.kind,
                crate::syntax::ArgumentsKind::Input | crate::syntax::ArgumentsKind::Repeating
            )
        });
        let supplied_names = function.inputs.iter().take(if varargs {
            input_count.min(function.inputs.len() - 1)
        } else {
            input_count
        });
        let variadic_name = varargs.then(|| function.inputs.last().unwrap());
        if has_input_or_repeating_block {
            for value in args {
                let array = stripping.strip(&value.array(&self.cancel)?)?;
                input_bytes += array.retained_bytes();
            }
        } else {
            for (name, value) in supplied_names.chain(variadic_name).zip(args) {
                let array = stripping.strip(&value.array(&self.cancel)?)?;
                if let Some(name) = name {
                    input_bytes += array.retained_bytes();
                    match array {
                        a @ (Array::Str(_)
                        | Array::Character(_)
                        | Array::Cell(_)
                        | Array::Graphics(_)
                        | Array::Struct(_)
                        | Array::Handle(_)
                        | Array::Typed(_)
                        | Array::Categorical(_)
                        | Array::Datetime(_)
                        | Array::Duration(_)
                        | Array::Table(_)
                        | Array::Sparse(_)) => {
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
        }
        drop(stripping);
        if let (Some(object), Some(name)) = (preset, function.outputs.first())
            && !function.inputs.iter().flatten().any(|input| input == name)
        {
            containers.insert(name.clone(), object);
        }
        if input_bytes > super::limits::memory_bytes(&self.limits) {
            return Err(format!(
                "Function inputs exceed the {} MiB workspace limit",
                super::limits::memory_bytes(&self.limits) / (1024 * 1024)
            ));
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
                    a @ (Array::Str(_)
                    | Array::Character(_)
                    | Array::Cell(_)
                    | Array::Graphics(_)
                    | Array::Struct(_)
                    | Array::Handle(_)
                    | Array::Typed(_)
                    | Array::Categorical(_)
                    | Array::Datetime(_)
                    | Array::Duration(_)
                    | Array::Table(_)
                    | Array::Sparse(_)) => {
                        caller_containers.insert(name, a);
                    }
                    Array::Logical(_) | Array::Complex(_) => (),
                }
            }
        }
        let before_park = self.nested.bytes;
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
                nested_name.as_deref().unwrap_or(&function.name)
            ),
            input_names,
        );
        self.lg3_frame_source(program.source.clone());
        // Shared values now have their own storage charge, whether parked or active.
        let retained = retained - (self.nested.bytes - before_park);
        let caller_scope = self.functions.scope.replace(program.clone());
        let caller_anonymous_parent = self.functions.anonymous_parent.take();
        // A function sees only its own file's local functions.
        let caller_hosts = std::mem::take(&mut self.functions.hosts);
        let caller_imports = self.functions.imp_enter();
        let caller_arguments = self.functions.arguments.replace((input_count, requested));
        let caller_handle_frame = self.cls_enter_function();
        let caller_current = self.functions.current.replace(resolved.index);
        nested::resume(
            self,
            nested::frame(&program, resolved.index, resolved.parent),
        );
        let caller_evaluated = self.evaluated_elements;
        self.functions.depth += 1;
        self.functions.retained_bytes += retained;
        let mut byte = statement.span.start;
        let result = (|| {
            if self.functions.lng.nested_frame().is_some() {
                if self.workspace_bytes() > super::limits::memory_bytes(&self.limits) {
                    return Err(super::limits::workspace_error(&self.limits));
                }
                if self.workspace_count() > 10000 {
                    return Err("Workspace exceeds 10000 variables; use clear".into());
                }
            }
            if !function.arguments.is_empty() {
                super::arguments::validate_inputs(
                    self,
                    &function.arguments,
                    &function.inputs,
                    &raw_args_for_validation,
                    nv_start,
                    &mut byte,
                    out,
                )?;
            }
            self.execute_statements(&function.body, &mut byte, out)?;
            self.check_cancel()?;
            if !function.arguments.is_empty() {
                super::arguments::validate_outputs(
                    self,
                    &function.arguments,
                    requested,
                    &mut byte,
                    out,
                )?;
            }
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
        self.functions.anonymous_parent = caller_anonymous_parent;
        self.functions.imp_leave(caller_imports);
        self.functions.arguments = caller_arguments;
        self.cls_leave_function(caller_handle_frame);
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
