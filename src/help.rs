//! Function help: embedded pages, user-file help comments and honest messages
//! for names MatRust does not support. The engine and the editor share this API.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const PAGES: &str = include_str!("../assets/help/functions.md");
const MATLAB_NAMES: &str = include_str!("../assets/help/matlab-names.txt");
/// Help reads at most this many bytes of a user file; help text sits at the top.
const MAX_USER_SOURCE: u64 = 1_000_000;
/// Directory entries one path folder is scanned for `@Class` method files.
/// The class loader uses the same bound.
const MAX_DIR_ENTRIES: usize = 4096;

/// Where help looks for user functions and scripts: the Current Folder,
/// then the user search path folders, in the order calls resolve names.
#[derive(Clone, Debug, Default)]
pub struct PathContext {
    pub directory: Option<PathBuf>,
    pub path: Vec<PathBuf>,
}

/// The root of a lookup, kept even when descending into packages and classes.
#[derive(Clone, Copy)]
pub(crate) struct LookupFolder<'a> {
    pub root: &'a Path,
    pub added: bool,
}
impl LookupFolder<'_> {
    pub(crate) fn existing_file(self, relative: &Path) -> Option<PathBuf> {
        if self.added {
            crate::files::read_fallback::contained_file(self.root, relative)
        } else {
            let file = self.root.join(relative);
            file.is_file().then_some(file)
        }
    }
}
impl PathContext {
    pub(crate) fn folders(&self) -> impl Iterator<Item = LookupFolder<'_>> {
        self.directory
            .iter()
            .map(|root| LookupFolder { root, added: false })
            .chain(
                self.path
                    .iter()
                    .map(|root| LookupFolder { root, added: true }),
            )
    }
}

/// Split help markdown into `name -> page` without copying. A page runs from
/// its `## name` heading to the next `## ` heading; text before the first
/// heading is ignored, surrounding blank lines are dropped and the first
/// heading of a repeated name wins.
pub fn parse_pages(text: &str) -> BTreeMap<&str, &str> {
    let mut pages = BTreeMap::new();
    let mut current: Option<(&str, usize)> = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some((name, start)) = current.take() {
                pages
                    .entry(name)
                    .or_insert_with(|| page_body(&text[start..at]));
            }
            current = heading
                .split_whitespace()
                .next()
                .map(|name| (name, at + line.len()));
        }
        at += line.len();
    }
    if let Some((name, start)) = current {
        pages
            .entry(name)
            .or_insert_with(|| page_body(&text[start..]));
    }
    pages
}

fn page_body(mut body: &str) -> &str {
    while let Some((line, rest)) = body.split_once('\n')
        && line.trim().is_empty()
    {
        body = rest;
    }
    body.trim_end()
}

fn pages() -> &'static BTreeMap<&'static str, &'static str> {
    static PARSED: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    PARSED.get_or_init(|| parse_pages(PAGES))
}

fn matlab_names() -> &'static BTreeSet<&'static str> {
    static NAMES: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        MATLAB_NAMES
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect()
    })
}

/// The help page of a built-in function, as written (line endings normalized).
pub fn page(name: &str) -> Option<String> {
    pages().get(name).map(|page| page.replace('\r', ""))
}

/// `help PDEModel`: the page of the one built-in package class `pkg.PDEModel`
/// (lowercase package, class name starting uppercase); none when ambiguous.
fn package_class_page(name: &str) -> Option<String> {
    if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    let mut found = pages().keys().filter(|key| {
        key.split_once('.').is_some_and(|(pkg, class)| {
            class == name && pkg.chars().all(|c| c.is_ascii_lowercase())
        })
    });
    match (found.next(), found.next()) {
        (Some(key), None) => page(key),
        _ => None,
    }
}

// Owners of Type.member pages in functions.md. A page for the prefix alone
// is insufficient: parallel is both a function and a package.
const MEMBER_PAGE_CLASSES: &[&str] = &[
    "ClassificationKNN",
    "ClassificationTree",
    "Figure",
    "GeneralizedLinearModel",
    "Image",
    "LinearModel",
    "Patch",
    "Rectangle",
    "RegressionTree",
    "Root",
    "Text",
    "TiledChartLayout",
    "VideoReader",
    "VideoWriter",
    "affinetform2d",
    "arima",
    "audioplayer",
    "audiorecorder",
    "cameraCalibrationErrors",
    "cameraIntrinsics",
    "cameraParameters",
    "cfit",
    "cvpartition",
    "cwtfilterbank",
    "datetime",
    "decomposition",
    "delaunayTriangulation",
    "digitalFilter",
    "duration",
    "garch",
    "imref2d",
    "matlab.apps.AppBase",
    "mpc",
    "parallel.FevalFuture",
    "pde.AnalyticGeometry",
    "pde.EigenResults",
    "pde.FEMesh",
    "pde.PDEModel",
    "pde.StationaryResults",
    "pde.TimeDependentResults",
    "prob.NormalDistribution",
    "ss",
    "table",
    "tf",
];

// Canonical App Designer classes whose help is stored under the short name.
fn ui_page_name(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some(("matlab.ui", class @ "Figure"))
        | Some((
            "matlab.ui.container",
            class @ ("GridLayout" | "Panel" | "TabGroup" | "Tab" | "ButtonGroup"),
        ))
        | Some((
            "matlab.ui.control",
            class @ ("Button" | "StateButton" | "Label" | "EditField" | "NumericEditField"
            | "TextArea" | "DropDown" | "ListBox" | "CheckBox" | "RadioButton" | "Slider"
            | "Spinner" | "Switch" | "Table" | "UIAxes"),
        )) => class,
        _ => name,
    }
}

/// The first line of a built-in function's help page.
pub fn summary(name: &str) -> Option<&'static str> {
    pages().get(name).and_then(|page| page.lines().next())
}

/// MatRust implements `name` (case-sensitive).
pub fn is_supported(name: &str) -> bool {
    crate::session::FUNCTIONS.contains(&name)
}

/// Whether `name` is in the MATLAB function-name list (case-sensitive).
pub fn is_matlab_name(name: &str) -> bool {
    matlab_names().contains(name)
}

/// Error text for a call that no user function or built-in handles.
pub fn unknown_function(name: &str) -> String {
    if is_supported(name) {
        format!("{name}: this call form is not supported yet")
    } else if is_matlab_name(name) {
        format!("'{name}' is a MATLAB function that MatRust does not support yet.")
    } else {
        format!("Undefined function or variable '{name}'.")
    }
}

/// The help comment block of MATLAB source: consecutive `%` lines at the top
/// of the file or after the `function` line (blank lines before the block
/// skipped). As in MATLAB, each line prints as one space plus the text after `%`.
pub fn h1_text(source: &str) -> Option<String> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut lines = source
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .peekable();
    let top = comment_block(&mut lines);
    if !top.is_empty() {
        return Some(top);
    }
    let mut line = lines.next()?.trim_start();
    if !line
        .strip_prefix("function")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '[']))
    {
        return None;
    }
    // A signature continued with ... ends on its first line without one.
    while continues(line) {
        line = lines.next()?;
    }
    while lines.next_if(|l| l.trim().is_empty()).is_some() {}
    let after = comment_block(&mut lines);
    (!after.is_empty()).then_some(after)
}

/// Whether the line ends in a `...` continuation: one before any `%` comment
/// and outside quotes.
fn continues(line: &str) -> bool {
    let mut quoted = None;
    let mut dots = 0;
    for c in line.chars() {
        match (quoted, c) {
            (Some(q), c) if c == q => quoted = None,
            (Some(_), _) => {}
            (None, '"') => quoted = Some('"'),
            (None, '\'') => quoted = Some('\''),
            (None, '%') => return false,
            (None, '.') => {
                dots += 1;
                if dots == 3 {
                    return true;
                }
                continue;
            }
            _ => {}
        }
        dots = 0;
    }
    false
}

fn comment_block<'a>(lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>) -> String {
    let mut block = String::new();
    while let Some(text) = lines
        .peek()
        .and_then(|line| line.trim_start().strip_prefix('%'))
        // A %{ block comment is not help text.
        .filter(|text| text.trim_end() != "{")
    {
        if !block.is_empty() {
            block.push('\n');
        }
        block.push(' ');
        block.push_str(text.trim_end());
        lines.next();
    }
    block
}

fn is_identifier(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The text `help name` prints. User files (.mr, then .m) in the path context
/// come first, as for calls; then built-in pages and MATLAB name knowledge.
/// Unsupported name forms and unreadable user files become the returned text.
pub fn help_text(name: &str, path: &PathContext) -> String {
    help(name, path).unwrap_or_else(|error| error)
}

/// Like [`help_text`], but unsupported name forms and unreadable user files
/// are errors. `Cls.method` and `Cls/method` (and `pkg.Cls.method`) print the
/// leading comment of that method file in an `@` folder. `doc` does not.
pub fn help(name: &str, path: &PathContext) -> crate::Result<String> {
    topic(name, path, true)
}

/// `doc name`: the same text as [`help`] for a function, script or package
/// file. Class-folder method forms are left unresolved.
pub fn doc(name: &str, path: &PathContext) -> crate::Result<String> {
    topic(name, path, false)
}

fn topic(name: &str, path: &PathContext, search_methods: bool) -> crate::Result<String> {
    if let Some(text) = user_help(name, path)? {
        return Ok(text);
    }
    if !is_identifier(name) {
        let key = ui_page_name(name);
        if search_methods && let Some((class, method)) = help_method(name) {
            let member_class = MEMBER_PAGE_CLASSES.contains(&class.as_str());
            // Package functions and constructors take precedence over @ files.
            if (member_class || !pages().contains_key(key))
                && let Some(text) = method_help(name, path, &class, &method)?
            {
                return Ok(text);
            }
            if member_class
                && name.contains(['/', '\\'])
                && let Some(text) = page(&format!("{class}.{method}"))
            {
                return Ok(text);
            }
        }
        if let Some(page) = page(key) {
            return Ok(page);
        }
        return Err(format!(
            "help: '{name}' is not a function name; help for folders, packages, methods and operators is not supported yet"
        ));
    }
    if let Some(page) = page(name) {
        return Ok(page);
    }
    if is_supported(name) {
        return Ok(format!("{name} is supported. No help page yet."));
    }
    if let Some(page) = package_class_page(name) {
        return Ok(page);
    }
    if is_matlab_name(name) {
        return Ok(format!(
            "{name} is a MATLAB function that MatRust does not support yet."
        ));
    }
    // The final line points to MatRust help; no help browser is available.
    let mut text = format!("\n{name} not found.\n\nType help to list the supported functions.");
    if let Some(similar) = crate::session::FUNCTIONS
        .iter()
        .find(|f| f.eq_ignore_ascii_case(name))
    {
        text.push_str(&format!("\nDid you mean {similar}?"));
    }
    Ok(text)
}

fn user_help(name: &str, path: &PathContext) -> crate::Result<Option<String>> {
    // `a.b.name`: the file `name` in the package folders `+a/+b` of a folder.
    let mut packages: Vec<&str> = name.split('.').collect();
    let file_name = packages.pop().unwrap_or(name);
    if packages.len() > 8
        || !crate::files::valid_name(file_name)
        || packages.iter().any(|part| !crate::files::valid_name(part))
    {
        return Ok(None);
    }
    let mut package_dir = PathBuf::new();
    for package in &packages {
        package_dir.push(format!("+{package}"));
    }
    for directory in path.folders() {
        for extension in ["mr", "m"] {
            let name_in_root = package_dir.join(format!("{file_name}.{extension}"));
            let file = if !directory.added {
                directory.root.join(name_in_root)
            } else if let Some(file) = directory.existing_file(&name_in_root) {
                file
            } else {
                continue;
            };
            if let Some(source) = read_user_source(&file)? {
                return Ok(Some(help_of_source(name, extension, &source)));
            }
        }
    }
    Ok(None)
}

/// One method file in an `@Class` folder on the path.
pub(crate) struct MethodHit {
    pub path: PathBuf,
    /// Class name as `which` prints it (`Mk`, or `box.Bk` for `+box/@Bk`).
    pub class: String,
}

/// Method files named `name` in `@Class` folders that sit directly in `folders`.
///
/// Package folders are not walked: MATLAB's `which`, when the class is not
/// loaded, finds a method only as a separate file in an `@` folder that is
/// not inside a namespace. The class file `@Class/Class.m` is not a method.
/// `.mr` wins over `.m`; `all` keeps both. Class folders in one directory are
/// taken in name order. A directory that cannot be read is skipped, and a
/// directory is not read past [`MAX_DIR_ENTRIES`] entries.
pub(crate) fn method_files(folders: &PathContext, name: &str, all: bool) -> Vec<MethodHit> {
    if !crate::files::valid_name(name) {
        return Vec::new();
    }
    let mut found = Vec::new();
    for folder in folders.folders() {
        let mut classes = Vec::new();
        let Ok(entries) = std::fs::read_dir(folder.root) else {
            continue;
        };
        for (count, entry) in entries.enumerate() {
            if count >= MAX_DIR_ENTRIES {
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(class) = file_name.to_str().and_then(|n| n.strip_prefix('@')) else {
                continue;
            };
            if class == name || !crate::files::valid_name(class) {
                continue;
            }
            classes.push(class.to_owned());
        }
        classes.sort();
        for class in classes {
            let dir = PathBuf::from(format!("@{class}"));
            push_method_file(&mut found, folder, &dir, &class, name, all);
            if !all && !found.is_empty() {
                return found;
            }
        }
    }
    found
}

/// Method files of the class `class` (`Mk` or `pkg.Sub.Cls`) named `method`:
/// `folder/@Mk/method.m`, or `folder/+pkg/+Sub/@Cls/method.m`. The class file
/// itself (`method` equal to the last part of `class`) is not a method.
pub(crate) fn class_method_files(
    folders: &PathContext,
    class: &str,
    method: &str,
    all: bool,
) -> Vec<MethodHit> {
    if !crate::files::valid_name(method) {
        return Vec::new();
    }
    let parts: Vec<&str> = class.split('.').collect();
    if parts.is_empty()
        || parts.len() > 8
        || parts.iter().any(|part| !crate::files::valid_name(part))
    {
        return Vec::new();
    }
    let simple = parts[parts.len() - 1];
    if method == simple {
        return Vec::new();
    }
    let mut found = Vec::new();
    for folder in folders.folders() {
        let mut dir = PathBuf::new();
        for part in &parts[..parts.len() - 1] {
            dir.push(format!("+{part}"));
        }
        dir.push(format!("@{simple}"));
        push_method_file(&mut found, folder, &dir, class, method, all);
        if !all && !found.is_empty() {
            return found;
        }
    }
    found
}

/// `Cls.method` / `pkg.Cls.method`, or a slash form `Cls/method`, `@Cls/method`,
/// `+pkg/@Cls/method` (and `\` the same way). The method part may end in `.m`
/// or `.mr` only in a slash form: `Cls.m` is the method `m` of class `Cls`.
pub(crate) fn qualified_method(name: &str) -> Option<(String, String)> {
    if name.contains(['/', '\\']) {
        return slash_method(name);
    }
    let parts: Vec<&str> = name.split('.').collect();
    if !(2..=8).contains(&parts.len()) || parts.iter().any(|part| !crate::files::valid_name(part)) {
        return None;
    }
    let method = parts[parts.len() - 1];
    let class = parts[..parts.len() - 1].join(".");
    Some((class, method.to_owned()))
}

fn slash_method(name: &str) -> Option<(String, String)> {
    // A partial path (`toolbox/sin`, `C:\...`) is left to the caller.
    if name.starts_with(['/', '\\']) || name.contains(':') {
        return None;
    }
    let parts: Vec<&str> = name
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 2 || parts.len() > 9 {
        return None;
    }
    let last = parts[parts.len() - 1];
    let method = match last.rsplit_once('.') {
        Some((stem, "m" | "mr")) => stem,
        Some(_) => return None,
        None => last,
    };
    if !crate::files::valid_name(method) {
        return None;
    }
    let folders = &parts[..parts.len() - 1];
    let class_seg = folders[folders.len() - 1];
    let class_name = class_seg.strip_prefix('@').unwrap_or(class_seg);
    if !crate::files::valid_name(class_name) {
        return None;
    }
    let mut class_parts = Vec::with_capacity(folders.len());
    for pkg in &folders[..folders.len() - 1] {
        let pkg = pkg.strip_prefix('+')?;
        if !crate::files::valid_name(pkg) {
            return None;
        }
        class_parts.push(pkg);
    }
    class_parts.push(class_name);
    if class_parts.len() > 8 {
        return None;
    }
    Some((class_parts.join("."), method.to_owned()))
}

/// Help also accepts a documented package class followed by a slash and member.
fn help_method(name: &str) -> Option<(String, String)> {
    if let Some((class, member)) = name.split_once(['/', '\\'])
        && class.contains('.')
        && MEMBER_PAGE_CLASSES.contains(&class)
    {
        let method = member
            .strip_suffix(".mr")
            .or_else(|| member.strip_suffix(".m"))
            .unwrap_or(member);
        return crate::files::valid_name(method).then(|| (class.to_owned(), method.to_owned()));
    }
    qualified_method(name)
}

fn push_method_file(
    found: &mut Vec<MethodHit>,
    root: LookupFolder<'_>,
    folder: &Path,
    class: &str,
    method: &str,
    all: bool,
) {
    for extension in ["mr", "m"] {
        let name = folder.join(format!("{method}.{extension}"));
        if let Some(path) = root.existing_file(&name) {
            found.push(MethodHit {
                path,
                class: class.to_owned(),
            });
            if !all {
                return;
            }
        }
    }
}

/// Help for `Cls.method`, `pkg.Cls.method`, `Cls/method` and `@Cls/method`
/// when that method file exists. Anything else is `None` (the caller's own
/// answer stands, including `help name` for a bare method-file name).
fn method_help(
    name: &str,
    path: &PathContext,
    class: &str,
    method: &str,
) -> crate::Result<Option<String>> {
    let Some(hit) = class_method_files(path, class, method, false)
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let Some(source) = read_user_source(&hit.path)? else {
        return Ok(None);
    };
    let extension = hit
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("m");
    Ok(Some(help_of_source(name, extension, &source)))
}

fn read_user_source(file: &Path) -> crate::Result<Option<Vec<u8>>> {
    let mut source = Vec::new();
    let read = File::open(file).and_then(|f| {
        if !f.metadata()?.is_file() {
            return Ok(false);
        }
        f.take(MAX_USER_SOURCE).read_to_end(&mut source)?;
        Ok(true)
    });
    match read {
        Ok(true) => Ok(Some(source)),
        Ok(false) => Ok(None),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

fn help_of_source(name: &str, extension: &str, source: &[u8]) -> String {
    h1_text(&String::from_utf8_lossy(source))
        .unwrap_or_else(|| format!("\nNo help found for {name}.{extension}."))
}
