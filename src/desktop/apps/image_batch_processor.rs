//! Image Batch Processor: one image function over every matching file of a
//! folder. `Plan` is the only script builder: its local functions list the
//! folder and run the loop (try/catch per file, imwrite, table building). The
//! private runs call the same functions, the listing once and the loop for one
//! file at a time, so Stop cancels the current file and the list shows each
//! file's status. Generate script saves the same functions with one call over
//! the whole listing. Export publishes the table the app collected.
use super::{AppKind, Outputs, Request, Runner, Snapshot, field, name_field, number, short};
use crate::toolkits::{Action, AppStatus};
use eframe::egui;
use matrust::Array;
use matrust::typed_array::TypedData;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

/// Most files one listing may hold.
pub const MAX_FILES: usize = 10_000;
/// Longest side of a thumbnail, in pixels.
pub const THUMB_SIDE: usize = 160;
pub const DEFAULT_PATTERN: &str = "*.png; *.jpg; *.jpeg; *.tif; *.tiff; *.bmp; *.gif";
const MAX_PATTERNS: usize = 16;
const MAX_PATH: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    /// A function on the path, in the current folder or a workspace handle.
    Named,
    Gray,
    Resize,
    Binarize,
    Contrast,
}
impl Function {
    pub const ALL: [Self; 5] = [
        Self::Named,
        Self::Gray,
        Self::Resize,
        Self::Binarize,
        Self::Contrast,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Named => "Function by name",
            Self::Gray => "Template: convert to gray",
            Self::Resize => "Template: resize to N %",
            Self::Binarize => "Template: binarize",
            Self::Contrast => "Template: adjust contrast",
        }
    }
    /// The template's function (and .m file) name.
    pub fn template(self) -> Option<&'static str> {
        match self {
            Self::Named => None,
            Self::Gray => Some("batch_to_gray"),
            Self::Resize => Some("batch_resize"),
            Self::Binarize => Some("batch_binarize"),
            Self::Contrast => Some("batch_contrast"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Returns {
    /// One output: an image (written) or a struct (a table row).
    One,
    /// `[image, row] = f(image)`; either may be empty.
    Both,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Pending,
    Ok,
    /// Stop cancelled this file before it finished. A later run tries it again.
    Stopped,
    Error(String),
}

#[derive(Clone, Debug)]
pub struct FileRow {
    /// Path relative to the input folder.
    pub name: String,
    pub status: Status,
    pub seconds: Option<f64>,
    /// The struct's scalar results, in field order.
    pub values: Vec<(String, f64)>,
}
impl FileRow {
    fn new(name: String) -> Self {
        Self {
            name,
            status: Status::Pending,
            seconds: None,
            values: Vec::new(),
        }
    }
}

/// The results table the app collected: one row per file that ran ok.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub files: Vec<String>,
    pub names: Vec<String>,
    /// Column-wise values, NaN where a row lacks the field.
    pub columns: Vec<Vec<f64>>,
}

/// A thumbnail: RGB pixels row-major.
#[derive(Clone)]
pub struct Thumb {
    pub size: [usize; 2],
    pub rgb: Vec<u8>,
}

pub struct Plan {
    pub input: String,
    pub output: String,
    patterns: Vec<String>,
    recursive: bool,
    list_fn: String,
    process_fn: String,
    /// `@name`, a workspace handle variable or a template handle.
    function: String,
    /// Workspace variables the private runs need.
    pub inputs: Vec<String>,
    /// Both local functions plus the template, in script order.
    functions: String,
}
impl Plan {
    fn listing(&self) -> String {
        format!(
            "{}({}, {{{}}}, {})",
            self.list_fn,
            quote(&self.input),
            self.patterns
                .iter()
                .map(|p| quote(p))
                .collect::<Vec<_>>()
                .join(", "),
            if self.recursive { "true" } else { "false" }
        )
    }
    fn call(&self, files: &str) -> String {
        format!(
            "{}({}, {}, {files}, {})",
            self.process_fn,
            quote(&self.input),
            quote(&self.output),
            self.function
        )
    }
    /// The generated script with `table` as the results name.
    pub fn script(&self, table: &str) -> String {
        format!(
            "% Image Batch Processor: every file of {} matching {} is read, passed to the\n\
             % function and its image written under the same name to {}; struct\n\
             % results become one table row per file (file name first). A file that\n\
             % fails is reported and skipped.\n\
             {table} = {};\n{}",
            self.input.replace('\n', " "),
            self.patterns.join(" "),
            self.output.replace('\n', " "),
            self.call(&self.listing()),
            self.functions
        )
    }
    /// Private listing run: the relative file names.
    pub fn list_code(&self) -> String {
        format!("ibp_probe_files = {};\n{}", self.listing(), self.functions)
    }
    /// Private run of one file: its status, message, time and table row.
    pub fn file_code(&self, file: &str) -> String {
        format!(
            "[ibp_probe_table, ibp_probe_report] = {};\n\
             ibp_probe_status = ibp_probe_report.status{{1}};\n\
             ibp_probe_message = ibp_probe_report.message{{1}};\n\
             ibp_probe_seconds = ibp_probe_report.seconds(1);\n{}",
            self.call(&format!("{{{}}}", quote(file))),
            self.functions
        )
    }
    /// Private thumbnail run: the input and (when written) the output file.
    pub fn thumb_code(&self, file: &str) -> String {
        format!(
            "ibp_probe_input = ibp_thumb(imread(fullfile({in_folder}, {f})));\n\
             ibp_probe_output = uint8([]);\n\
             ibp_probe_target = fullfile({out_folder}, {f});\n\
             if exist(ibp_probe_target, 'file') == 2, ibp_probe_output = ibp_thumb(imread(ibp_probe_target)); end\n\
             function small = ibp_thumb(image)\n\
             if ndims(image) > 3 || isempty(image), error('Image Batch Processor: not a 2-D image'); end\n\
             if size(image, 3) == 2, image = image(:, :, 1); elseif size(image, 3) > 3, image = image(:, :, 1:3); end\n\
             step = max(1, ceil(max(size(image, 1), size(image, 2)) / {THUMB_SIDE}));\n\
             small = im2uint8(image(1:step:end, 1:step:end, :));\n\
             end\n",
            in_folder = quote(&self.input),
            out_folder = quote(&self.output),
            f = quote(file),
        )
    }
}

/// A MATLAB char literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
/// A number the engine reads back as the same double, including NaN and Inf.
fn literal(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else {
        number(v)
    }
}

fn template_text(function: Function, percent: &str) -> Vec<String> {
    let Some(name) = function.template() else {
        return Vec::new();
    };
    let body: &[&str] = match function {
        Function::Gray => &[
            "% Convert RGB images to grayscale; grayscale images pass through.",
            "if size(image, 3) == 3",
            "    out = rgb2gray(image);",
            "else",
            "    out = image;",
            "end",
        ],
        Function::Resize => &["% Resize by a percentage of the original size."],
        Function::Binarize => &[
            "% Global (Otsu) threshold of the grayscale image.",
            "if size(image, 3) == 3",
            "    image = rgb2gray(image);",
            "end",
            "out = imbinarize(image);",
        ],
        Function::Contrast => &[
            "% Stretch the intensities to the full range (1 % saturated at each end).",
            "if size(image, 3) == 3",
            "    out = imadjust(image, stretchlim(image));",
            "else",
            "    out = imadjust(image);",
            "end",
        ],
        Function::Named => &[],
    };
    let mut lines = vec![format!("function out = {name}(image)")];
    lines.extend(body.iter().map(|s| s.to_string()));
    if function == Function::Resize {
        lines.push(format!("out = imresize(image, {percent} / 100);"));
    }
    lines.push("end".into());
    lines
}

const LIST: &str = "
function files = IBP_LIST(folder, patterns, recursive)
% Matching files as paths relative to folder, sorted; subfolders when recursive.
if ~exist(folder, 'dir'), error('Image Batch Processor: input folder %s not found', folder); end
files = cell(0, 1);
pending = {''};
while ~isempty(pending)
    sub = pending{1};
    pending(1) = [];
    for p = 1:numel(patterns)
        found = dir(fullfile(folder, sub, patterns{p}));
        for k = 1:numel(found)
            if ~found(k).isdir
                if isempty(sub), files{end+1, 1} = found(k).name; else, files{end+1, 1} = fullfile(sub, found(k).name); end
            end
        end
    end
    if recursive
        found = dir(fullfile(folder, sub));
        for k = 1:numel(found)
            if found(k).isdir && ~any(strcmp(found(k).name, {'.', '..'}))
                if isempty(sub), pending{end+1} = found(k).name; else, pending{end+1} = fullfile(sub, found(k).name); end
            end
        end
    end
    if numel(files) > MAX_FILES, error('Image Batch Processor: more than MAX_FILES files; narrow the pattern'); end
end
if ~isempty(files), files = unique(files); end
end
";

/// Helpers of the process function, filled per plan.
struct Process<'a> {
    name: &'a str,
    returns: Returns,
    template: Option<(&'a str, Vec<String>)>,
}
impl Process<'_> {
    fn text(&self) -> String {
        let mut s = format!(
            "\nfunction [results, report] = {}(in_folder, out_folder, files, func)\n\
             % Runs func on each file: images are written to out_folder under the same\n\
             % relative name and format, struct results become table rows.\n\
             n = numel(files);\n\
             status = repmat({{'pending'}}, n, 1);\n\
             message = repmat({{''}}, n, 1);\n\
             seconds = nan(n, 1);\n\
             rows = cell(n, 1);\n\
             if ~exist(out_folder, 'dir'), mkdir(out_folder); end\n",
            self.name
        );
        if let Some((name, lines)) = &self.template {
            s += &format!(
                "% The template function, saved for editing next to the results.\n\
                 fid = fopen(fullfile(out_folder, '{name}.m'), 'w');\n\
                 if fid < 0, error('Image Batch Processor: cannot write {name}.m in the output folder'); end\n\
                 fprintf(fid, '%s\\n', ...\n"
            );
            for line in lines {
                s += &format!("    {}, ...\n", quote(line));
            }
            s += "    '');\nfclose(fid);\n";
        }
        s += "for k = 1:n\n    started = tic;\n    try\n        image = imread(fullfile(in_folder, files{k}));\n";
        match self.returns {
            Returns::One => {
                s += "        out = func(image);\n        row = struct();\n\
                      \x20       if isa(out, 'struct')\n            row = out;\n            out = [];\n\
                      \x20       elseif ~(isa(out, 'numeric') || isa(out, 'logical')) || isempty(out)\n\
                      \x20           error('Image Batch Processor: the function must return an image or a struct');\n\
                      \x20       end\n";
            }
            Returns::Both => {
                s += "        [out, row] = func(image);\n\
                      \x20       if isempty(row), row = struct(); end\n\
                      \x20       if ~isempty(out) && ~(isa(out, 'numeric') || isa(out, 'logical'))\n\
                      \x20           error('Image Batch Processor: the first output must be an image or []');\n\
                      \x20       end\n\
                      \x20       if ~isa(row, 'struct')\n\
                      \x20           error('Image Batch Processor: the second output must be a struct or []');\n\
                      \x20       end\n";
            }
        }
        s += "        if ~isscalar(row) || any(structfun(@(v) ~((isnumeric(v) || islogical(v)) && isscalar(v) && isreal(v)), row))\n\
              \x20           error('Image Batch Processor: struct results must be real numeric scalars');\n\
              \x20       end\n\
              \x20       if isfield(row, 'File'), error('Image Batch Processor: File is the file name column; rename that field'); end\n\
              \x20       if ~isempty(out)\n\
              \x20           target = fullfile(out_folder, files{k});\n\
              \x20           [where, ~, ~] = fileparts(target);\n\
              \x20           if ~exist(where, 'dir'), mkdir(where); end\n\
              \x20           imwrite(out, target);\n\
              \x20       end\n\
              \x20       rows{k} = row;\n\
              \x20       status{k} = 'ok';\n\
              \x20   catch problem\n\
              \x20       status{k} = 'error';\n\
              \x20       message{k} = problem.message;\n\
              \x20       fprintf('%s: %s\\n', files{k}, problem.message);\n\
              \x20   end\n\
              \x20   seconds(k) = toc(started);\n\
              end\n\
              % One row per file that ran: the file name, then the struct's fields\n\
              % (from the first file that returned any; NaN where a file lacks one).\n\
              done = find(strcmp(status, 'ok'));\n\
              results = table(files(done), 'VariableNames', {'File'});\n\
              names = cell(0, 1);\n\
              for i = 1:numel(done)\n\
              \x20   if ~isempty(fieldnames(rows{done(i)})), names = fieldnames(rows{done(i)}); break; end\n\
              end\n\
              for j = 1:numel(names)\n\
              \x20   values = nan(numel(done), 1);\n\
              \x20   for i = 1:numel(done)\n\
              \x20       if isfield(rows{done(i)}, names{j}), values(i) = double(rows{done(i)}.(names{j})); end\n\
              \x20   end\n\
              \x20   results.(names{j}) = values;\n\
              end\n\
              report = struct('status', {status}, 'message', {message}, 'seconds', seconds);\n\
              fprintf('Image Batch Processor: %d ok, %d errors\\n', numel(done), n - numel(done));\n\
              end\n";
        s
    }
}

pub struct ImageBatchProcessor {
    pub input: String,
    pub subfolders: bool,
    pub pattern: String,
    pub output: String,
    /// Output may be the input folder (confirmed once).
    pub overwrite: bool,
    overwrite_confirmed: bool,
    confirm_overwrite: bool,
    pub function: Function,
    pub name: String,
    pub percent: String,
    pub returns: Returns,
    /// Re-run files that are already ok.
    pub force: bool,
    pub table_name: String,
    pub files: Vec<FileRow>,
    pub selected: Option<usize>,
    /// Files skipped as already ok by the last run.
    pub skipped: usize,
    pub message: String,
    pub(super) saving: bool,
    pub stopped: bool,
    list_runner: Runner,
    file_runner: Runner,
    thumb_runner: Runner,
    /// The listing whose files are shown.
    listed: Option<(u64, String)>,
    list_serial: u64,
    list_error: Option<String>,
    /// The process functions the shown statuses came from.
    processed: Option<u64>,
    queue: VecDeque<usize>,
    active: Option<(usize, u64)>,
    serial: u64,
    run_error: Option<String>,
    thumb_serial: u64,
    thumbs: Option<(u64, Thumb, Option<Thumb>)>,
    textures: Option<(u64, egui::TextureHandle, Option<egui::TextureHandle>)>,
    thumb_error: Option<String>,
    plan_error: Option<String>,
    /// Canonical paths (and, on Unix, device/inode) of the listed inputs.
    input_ids: Option<(u64, String, InputIds)>,
}
impl Default for ImageBatchProcessor {
    fn default() -> Self {
        Self {
            input: String::new(),
            subfolders: false,
            pattern: DEFAULT_PATTERN.into(),
            output: String::new(),
            overwrite: false,
            overwrite_confirmed: false,
            confirm_overwrite: false,
            function: Function::Gray,
            name: String::new(),
            percent: "50".into(),
            returns: Returns::One,
            force: false,
            table_name: "batchResults".into(),
            files: Vec::new(),
            selected: None,
            skipped: 0,
            message: String::new(),
            saving: false,
            stopped: false,
            list_runner: Runner::default(),
            file_runner: Runner::default(),
            thumb_runner: Runner::default(),
            listed: None,
            list_serial: 0,
            list_error: None,
            processed: None,
            queue: VecDeque::new(),
            active: None,
            serial: 0,
            run_error: None,
            thumb_serial: 0,
            thumbs: None,
            textures: None,
            thumb_error: None,
            plan_error: None,
            input_ids: None,
        }
    }
}

fn key(parts: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

/// `text` as an absolute folder: relative paths start at the desktop folder.
fn folder(text: &str, what: &str, ws: &Snapshot) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("Image Batch Processor: choose the {what} folder"));
    }
    if text.len() > MAX_PATH || text.chars().any(char::is_control) {
        return Err(format!(
            "Image Batch Processor: the {what} folder must be one line of at most {MAX_PATH} characters"
        ));
    }
    let path = Path::new(text);
    let path = match ws.folder() {
        Some(base) if path.is_relative() => base.join(path),
        _ => path.to_path_buf(),
    };
    let text = path.to_string_lossy().into_owned();
    super::single_expression(&quote(&text))
        .map_err(|e| format!("Image Batch Processor: {what} folder: {e}"))?;
    Ok(text)
}
/// The same folder, comparing resolved paths where they exist.
fn normal(path: &str) -> PathBuf {
    // A folder that does not exist yet resolves through its deepest existing
    // ancestor, so on Windows it shares the input's verbatim `\\?\` prefix.
    let path: PathBuf = Path::new(path).components().collect();
    let mut rest = Vec::new();
    let mut base = path.as_path();
    loop {
        if let Ok(real) = base.canonicalize() {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (base.parent(), base.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                base = parent;
            }
            _ => return path,
        }
    }
}

/// Listed inputs, so a write can be refused when it is the same file as one of them.
#[derive(Default)]
struct InputIds {
    paths: HashSet<PathBuf>,
    #[cfg(unix)]
    inodes: HashSet<(u64, u64)>,
}
impl InputIds {
    #[cfg(test)]
    fn from_paths(inputs: &[PathBuf]) -> Self {
        let mut ids = Self::default();
        for path in inputs {
            ids.insert(path);
        }
        ids
    }
    fn collect(root: &Path, files: &[FileRow]) -> Self {
        let mut ids = Self::default();
        for file in files {
            ids.insert(&root.join(&file.name));
        }
        ids
    }
    fn insert(&mut self, path: &Path) {
        let Ok(canon) = path.canonicalize() else {
            return;
        };
        #[cfg(unix)]
        if let Ok(meta) = std::fs::metadata(&canon) {
            use std::os::unix::fs::MetadataExt;
            self.inodes.insert((meta.dev(), meta.ino()));
        }
        self.paths.insert(canon);
    }
    /// Canonical path, and on Unix also device and inode (hard links).
    fn conflicts(&self, target: &Path) -> bool {
        let Ok(canon) = target.canonicalize() else {
            return false;
        };
        if self.paths.contains(&canon) {
            return true;
        }
        #[cfg(unix)]
        if let Ok(meta) = std::fs::metadata(&canon) {
            use std::os::unix::fs::MetadataExt;
            if self.inodes.contains(&(meta.dev(), meta.ino())) {
                return true;
            }
        }
        false
    }
}

fn refuse(relative: &Path, why: &str) -> String {
    format!(
        "Image Batch Processor: refuses to write {}: {why}",
        relative.display()
    )
}
fn is_within(root: &Path, path: &Path) -> bool {
    path == root || path.starts_with(root)
}
fn is_relative_write(path: &Path) -> bool {
    !path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
}

/// Canonicalize `path`, creating missing directories one level at a time.
/// Each created level is resolved again and must stay under its parent.
fn establish_root(path: &Path) -> Result<PathBuf, String> {
    match path.canonicalize() {
        Ok(canon) if canon.is_dir() => Ok(canon),
        Ok(_) => Err("Image Batch Processor: the output path is not a folder".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| {
                    "Image Batch Processor: the output folder has no parent".to_owned()
                })?;
            let parent = establish_root(parent)?;
            let name = path
                .file_name()
                .filter(|n| *n != "." && *n != "..")
                .ok_or_else(|| {
                    "Image Batch Processor: the output folder is not a directory".to_owned()
                })?;
            let next = parent.join(name);
            if std::fs::symlink_metadata(&next).is_err() {
                std::fs::create_dir(&next).map_err(|e| {
                    format!("Image Batch Processor: cannot create the output folder: {e}")
                })?;
            }
            let canon = std::fs::canonicalize(&next).map_err(|e| {
                format!("Image Batch Processor: cannot resolve the output folder: {e}")
            })?;
            if !canon.is_dir() {
                return Err("Image Batch Processor: the output path is not a folder".into());
            }
            if !is_within(&parent, &canon) {
                return Err("Image Batch Processor: the output folder leaves its parent".into());
            }
            Ok(canon)
        }
        Err(error) => Err(format!(
            "Image Batch Processor: cannot resolve the output folder: {error}"
        )),
    }
}

/// Where `relative` may be written under `output_root`.
///
/// The deepest existing ancestor is canonicalized, which follows symlinks and,
/// on Windows, junctions (`canonicalize` resolves reparse points). A missing
/// tail is appended lexically. `..` may not leave the output root. Missing
/// directories are created one level at a time and checked again. An existing
/// target that is the same file as an input is refused unless `allow_same_input`.
#[cfg(test)]
pub(crate) fn contained_target(
    output_root: &Path,
    relative: &Path,
    inputs: &[PathBuf],
    allow_same_input: bool,
) -> Result<PathBuf, String> {
    contained_with(
        output_root,
        relative,
        &InputIds::from_paths(inputs),
        allow_same_input,
    )
}

fn contained_with(
    output_root: &Path,
    relative: &Path,
    inputs: &InputIds,
    allow_same_input: bool,
) -> Result<PathBuf, String> {
    if !is_relative_write(relative) {
        return Err(refuse(relative, "it is not under the output folder"));
    }
    let root = establish_root(output_root)?;
    let mut current = root.clone();
    let comps: Vec<Component> = relative.components().collect();
    for (i, comp) in comps.iter().enumerate() {
        current = apply_component(
            &root,
            current,
            *comp,
            i + 1 == comps.len(),
            relative,
            inputs,
            allow_same_input,
        )?;
    }
    if !is_within(&root, &current) {
        return Err(refuse(relative, "it leaves the output folder"));
    }
    Ok(current)
}

fn apply_component(
    root: &Path,
    mut current: PathBuf,
    comp: Component<'_>,
    last: bool,
    relative: &Path,
    inputs: &InputIds,
    allow_same_input: bool,
) -> Result<PathBuf, String> {
    match comp {
        Component::CurDir => Ok(current),
        Component::ParentDir => {
            if current == root {
                return Err(refuse(relative, "it leaves the output folder"));
            }
            current.pop();
            if current.as_os_str().is_empty() || !is_within(root, &current) {
                return Err(refuse(relative, "it leaves the output folder"));
            }
            if current.exists() {
                current = std::fs::canonicalize(&current).map_err(|e| {
                    format!(
                        "Image Batch Processor: cannot resolve {}: {e}",
                        relative.display()
                    )
                })?;
            }
            if !is_within(root, &current) {
                return Err(refuse(relative, "it leaves the output folder"));
            }
            Ok(current)
        }
        Component::Normal(name) => {
            let next = current.join(name);
            match std::fs::symlink_metadata(&next) {
                Ok(_) => accept_existing(root, &next, last, relative, inputs, allow_same_input),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if last && !is_within(root, &next) {
                        return Err(refuse(relative, "it leaves the output folder"));
                    }
                    if last {
                        return Ok(next);
                    }
                    std::fs::create_dir(&next).map_err(|e| {
                        format!(
                            "Image Batch Processor: cannot create {}: {e}",
                            next.display()
                        )
                    })?;
                    let canon = std::fs::canonicalize(&next).map_err(|e| {
                        format!(
                            "Image Batch Processor: cannot resolve {}: {e}",
                            relative.display()
                        )
                    })?;
                    if !canon.is_dir() || !is_within(root, &canon) {
                        return Err(refuse(relative, "it leaves the output folder"));
                    }
                    Ok(canon)
                }
                Err(error) => Err(format!(
                    "Image Batch Processor: cannot prepare {}: {error}",
                    relative.display()
                )),
            }
        }
        Component::Prefix(_) | Component::RootDir => {
            Err(refuse(relative, "it is not under the output folder"))
        }
    }
}

fn accept_existing(
    root: &Path,
    next: &Path,
    last: bool,
    relative: &Path,
    inputs: &InputIds,
    allow_same_input: bool,
) -> Result<PathBuf, String> {
    let canon = std::fs::canonicalize(next).map_err(|e| {
        format!(
            "Image Batch Processor: cannot resolve {}: {e}",
            relative.display()
        )
    })?;
    if !is_within(root, &canon) {
        return Err(refuse(relative, "it leaves the output folder"));
    }
    if !last && !canon.is_dir() {
        return Err(refuse(relative, "a parent is not a folder"));
    }
    if !last {
        return Ok(canon);
    }
    if canon.is_dir() {
        return Err(refuse(relative, "it is a folder"));
    }
    if !allow_same_input && inputs.conflicts(&canon) {
        return Err(refuse(relative, "it is the same file as an input"));
    }
    Ok(canon)
}

fn mark_stopped(files: &mut [FileRow], index: usize) {
    if let Some(row) = files.get_mut(index)
        && matches!(row.status, Status::Pending)
    {
        row.status = Status::Stopped;
    }
}

impl ImageBatchProcessor {
    fn patterns(&self) -> Result<Vec<String>, String> {
        let patterns: Vec<String> = self
            .pattern
            .split([';', ','])
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect();
        if patterns.is_empty() || patterns.len() > MAX_PATTERNS {
            return Err(format!(
                "Image Batch Processor: enter 1 to {MAX_PATTERNS} file patterns separated by ;"
            ));
        }
        for p in &patterns {
            if p.len() > 64 || p.chars().any(|c| c.is_control() || c == '/' || c == '\\') {
                return Err(format!(
                    "Image Batch Processor: {p} is not a file name pattern (no folders, at most 64 characters)"
                ));
            }
            super::single_expression(&quote(p))
                .map_err(|e| format!("Image Batch Processor: pattern {p}: {e}"))?;
        }
        Ok(patterns)
    }
    /// The listing part of the plan: input folder, patterns, subfolders.
    fn list_plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let input = folder(&self.input, "input", ws)?;
        let patterns = self.patterns()?;
        let list_fn = super::unused_name("ibp_list", |n| ws.contains(n));
        let process_fn = super::unused_name("ibp_process", |n| ws.contains(n));
        let functions = LIST
            .replace("IBP_LIST", &list_fn)
            .replace("MAX_FILES", &MAX_FILES.to_string());
        Ok(Plan {
            input: input.clone(),
            output: input,
            patterns,
            recursive: self.subfolders,
            list_fn,
            process_fn,
            function: String::new(),
            inputs: Vec::new(),
            functions,
        })
    }
    /// The whole plan; the error says what to fix.
    pub fn plan(&self, ws: &Snapshot) -> Result<Plan, String> {
        let mut plan = self.list_plan(ws)?;
        plan.output = folder(&self.output, "output", ws)?;
        let (input, output) = (normal(&plan.input), normal(&plan.output));
        if input == output && !self.overwrite {
            return Err(
                "Image Batch Processor: the output folder is the input folder; choose another folder or tick Overwrite originals".into(),
            );
        }
        if self.subfolders && output != input && output.starts_with(&input) {
            return Err(
                "Image Batch Processor: with Subfolders on, the output folder must not be inside the input folder".into(),
            );
        }
        let mut template = None;
        match self.function.template() {
            Some(name) => {
                let mut percent = String::new();
                if self.function == Function::Resize {
                    let p = self.percent.trim();
                    super::single_expression(p)
                        .map_err(|e| format!("Image Batch Processor: percent: {e}"))?;
                    let v = p
                        .parse::<f64>()
                        .ok()
                        .filter(|v| (1. ..=1000.).contains(v))
                        .ok_or("Image Batch Processor: percent must be a number from 1 to 1000")?;
                    percent = number(v);
                }
                plan.function = format!("@{name}");
                template = Some((name, template_text(self.function, &percent)));
            }
            None => {
                let name = self.name.trim();
                if name.is_empty() {
                    return Err("Image Batch Processor: enter a function name".into());
                }
                super::single_expression(name)
                    .map_err(|e| format!("Image Batch Processor: function: {e}"))?;
                if name.len() > 63 || !matrust::files::valid_name(name) {
                    return Err(
                        "Image Batch Processor: the function must be a function name or a workspace function handle".into(),
                    );
                }
                match ws.get(name) {
                    Some(Array::Handle(_)) => {
                        plan.function = name.to_owned();
                        plan.inputs.push(name.to_owned());
                    }
                    Some(_) => {
                        return Err(format!(
                            "Image Batch Processor: workspace variable {name} is not a function handle"
                        ));
                    }
                    None => plan.function = format!("@{name}"),
                }
                if [&plan.list_fn, &plan.process_fn].contains(&&name.to_owned()) {
                    return Err(format!(
                        "Image Batch Processor: {name} is used by the generated script; rename the function"
                    ));
                }
            }
        }
        let process = Process {
            name: &plan.process_fn,
            returns: self.returns,
            template: template.clone(),
        };
        plan.functions = format!("{}{}", plan.functions, process.text());
        if let Some((_, lines)) = template {
            plan.functions = format!("{}\n{}\n", plan.functions, lines.join("\n"));
        }
        Ok(plan)
    }
    fn check_table_name(&self, ws: &Snapshot, plan: &Plan) -> Result<String, String> {
        let name = self.table_name.trim();
        super::single_expression(name)
            .map_err(|e| format!("Image Batch Processor: table name: {e}"))?;
        if name.len() > 63 || !matrust::files::valid_name(name) {
            return Err(
                "Image Batch Processor: the table name must be a valid variable name".into(),
            );
        }
        super::check_export_name(name)?;
        if [&plan.list_fn, &plan.process_fn].contains(&&name.to_owned())
            || Some(name) == self.function.template()
            || plan.inputs.iter().any(|n| n == name)
        {
            return Err(format!(
                "Image Batch Processor: {name} is used by the script; choose another table name"
            ));
        }
        let _ = ws;
        Ok(name.to_owned())
    }
    /// The generated script: listing, loop, imwrite and table building.
    pub fn script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.plan(ws)?;
        let name = self.check_table_name(ws, &plan)?;
        Ok(plan.script(&name))
    }
    /// The collected results: ok files in list order; columns from the first
    /// file with struct fields, as the script's table building does.
    pub fn table(&self) -> Table {
        let ok: Vec<&FileRow> = self
            .files
            .iter()
            .filter(|f| f.status == Status::Ok)
            .collect();
        let names: Vec<String> = ok
            .iter()
            .find(|f| !f.values.is_empty())
            .map(|f| f.values.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        let columns = names
            .iter()
            .map(|n| {
                ok.iter()
                    .map(|f| {
                        f.values
                            .iter()
                            .find(|(m, _)| m == n)
                            .map_or(f64::NAN, |(_, v)| *v)
                    })
                    .collect()
            })
            .collect();
        Table {
            files: ok.iter().map(|f| f.name.clone()).collect(),
            names,
            columns,
        }
    }
    /// Export: the collected table as one assignment (files are not
    /// processed again, so Overwrite originals never applies twice).
    pub fn export_script(&self, ws: &Snapshot) -> Result<String, String> {
        let plan = self.plan(ws)?;
        let name = self.check_table_name(ws, &plan)?;
        let t = self.table();
        if t.files.is_empty() {
            return Err("Image Batch Processor: run the batch first; no file ran ok".into());
        }
        let mut columns = vec![format!(
            "{{{}}}",
            t.files
                .iter()
                .map(|f| quote(f))
                .collect::<Vec<_>>()
                .join("; ")
        )];
        for c in &t.columns {
            columns.push(format!(
                "[{}]",
                c.iter().map(|v| literal(*v)).collect::<Vec<_>>().join("; ")
            ));
        }
        let mut names = vec![quote("File")];
        names.extend(t.names.iter().map(|n| quote(n)));
        Ok(format!(
            "% Image Batch Processor results: one row per file that ran ok.\n\
             {name} = table({}, 'VariableNames', {{{}}});\n",
            columns.join(", "),
            names.join(", ")
        ))
    }
    pub fn running(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty()
    }
    pub fn counts(&self) -> (usize, usize, usize) {
        let ok = self.files.iter().filter(|f| f.status == Status::Ok).count();
        let error = self
            .files
            .iter()
            .filter(|f| matches!(f.status, Status::Error(_)))
            .count();
        (ok, error, self.files.len() - ok - error)
    }
    /// Queue every file (already ok ones only when forced).
    pub fn run_all(&mut self) {
        if self.running() {
            return;
        }
        let force = self.force;
        self.queue = (0..self.files.len())
            .filter(|k| force || self.files[*k].status != Status::Ok)
            .collect();
        self.skipped = self.files.len() - self.queue.len();
        self.start_run();
    }
    /// Queue the selected file, even when it is already ok.
    pub fn run_selected(&mut self) {
        if self.running() {
            return;
        }
        if let Some(k) = self.selected.filter(|k| *k < self.files.len()) {
            self.queue = VecDeque::from([k]);
            self.skipped = 0;
            self.start_run();
        }
    }
    fn start_run(&mut self) {
        self.stopped = false;
        self.run_error = None;
        self.message.clear();
    }
    /// Cancel the file that is running and drop the rest of the queue.
    /// Files that already finished keep their rows. The engine's cancellation
    /// flag stops the current computation, including a loop that would not end.
    pub fn stop(&mut self) {
        if !self.running() {
            return;
        }
        self.queue.clear();
        if let Some((index, _)) = self.active.take() {
            mark_stopped(&mut self.files, index);
        }
        self.stopped = true;
        self.file_runner.stop();
    }
    fn ensure_input_ids(&mut self, plan: &Plan) {
        let stamp = self.listed.as_ref().map(|(k, _)| *k).unwrap_or(0);
        if let Some((have, input, _)) = &self.input_ids
            && *have == stamp
            && input == &plan.input
        {
            return;
        }
        let ids = InputIds::collect(Path::new(&plan.input), &self.files);
        self.input_ids = Some((stamp, plan.input.clone(), ids));
    }
    /// Refuse an image or template write that leaves the output folder or
    /// replaces an input. Missing parents are created one level at a time.
    fn guard_writes(&mut self, plan: &Plan, relative: &str) -> Result<(), String> {
        self.ensure_input_ids(plan);
        let allow = self.overwrite && normal(&plan.input) == normal(&plan.output);
        let Some((_, _, ids)) = &self.input_ids else {
            return Err("Image Batch Processor: cannot check the output path".into());
        };
        if let Some(name) = self.function.template() {
            contained_with(
                Path::new(&plan.output),
                &PathBuf::from(format!("{name}.m")),
                ids,
                allow,
            )?;
        }
        contained_with(Path::new(&plan.output), Path::new(relative), ids, allow)?;
        Ok(())
    }
    /// The app closed: cancel everything now.
    pub fn close(&mut self) {
        self.queue.clear();
        if self.active.take().is_some() {
            self.stopped = true;
        }
        self.list_runner.stop();
        self.file_runner.stop();
        self.thumb_runner.stop();
    }
    /// One frame of work: listing, the next file, thumbnails.
    pub fn poll(&mut self, ctx: &egui::Context, ws: &Snapshot) {
        for runner in [
            &mut self.list_runner,
            &mut self.file_runner,
            &mut self.thumb_runner,
        ] {
            runner.set_folder(ws.folder());
        }
        if !self.running() {
            self.poll_listing(ctx, ws);
        }
        let plan = self.plan(ws);
        self.plan_error = plan.as_ref().err().cloned();
        if let Ok(plan) = &plan {
            // Statuses belong to one input folder, output folder and function.
            let versions: Vec<_> = plan.inputs.iter().map(|n| ws.version(n)).collect();
            let current = key((
                &plan.functions,
                &plan.input,
                &plan.output,
                &plan.function,
                versions,
            ));
            if self.processed != Some(current) && !self.running() {
                if self.processed.is_some() {
                    for f in &mut self.files {
                        *f = FileRow::new(std::mem::take(&mut f.name));
                    }
                }
                self.processed = Some(current);
            }
        }
        self.poll_files(ctx, ws, plan.as_ref().ok());
        self.poll_thumbs(ctx, plan.as_ref().ok());
    }
    fn poll_listing(&mut self, ctx: &egui::Context, ws: &Snapshot) {
        let plan = match self.list_plan(ws) {
            Ok(plan) => plan,
            Err(error) => {
                self.list_runner.stop();
                self.list_error = Some(error);
                self.listed = None;
                return;
            }
        };
        let code = plan.list_code();
        let k = key((&code, self.list_serial));
        self.list_runner.submit(
            ctx,
            Request {
                key: k,
                inputs: Vec::new(),
                code,
                outputs: vec!["ibp_probe_files".into()],
            },
        );
        if self.listed.as_ref().is_some_and(|(l, _)| *l == k) {
            return;
        }
        let Some(result) = self.list_runner.result() else {
            return;
        };
        match result
            .clone()
            .and_then(|out| names(&out, "ibp_probe_files"))
        {
            Ok(list) => {
                let same_folder = self
                    .listed
                    .as_ref()
                    .is_some_and(|(_, input)| *input == plan.input);
                let old: BTreeMap<String, FileRow> = if same_folder {
                    self.files.drain(..).map(|f| (f.name.clone(), f)).collect()
                } else {
                    BTreeMap::new()
                };
                let selected = self
                    .selected
                    .and_then(|k| self.files.get(k))
                    .map(|f| f.name.clone());
                let mut old = old;
                self.files = list
                    .into_iter()
                    .map(|n| old.remove(&n).unwrap_or_else(|| FileRow::new(n)))
                    .collect();
                self.selected = selected
                    .and_then(|s| self.files.iter().position(|f| f.name == s))
                    .or((!self.files.is_empty()).then_some(0));
                self.list_error = None;
            }
            Err(error) => self.list_error = Some(error),
        }
        self.listed = Some((k, plan.input));
    }
    fn poll_files(&mut self, ctx: &egui::Context, ws: &Snapshot, plan: Option<&Plan>) {
        self.file_runner.poll(ctx);
        // A changed desktop folder cancels the runner's job. That file is
        // marked stopped and the rest of the queue is dropped.
        if let Some((index, k)) = self.active
            && self.file_runner.key() != Some(k)
        {
            self.active = None;
            self.queue.clear();
            self.stopped = true;
            mark_stopped(&mut self.files, index);
        }
        if let Some((index, k)) = self.active
            && self.file_runner.key() == Some(k)
            && let Some(result) = self.file_runner.result()
        {
            let result = result.clone();
            self.active = None;
            self.thumb_serial += 1;
            if let Some(row) = self.files.get_mut(index) {
                match result.and_then(|out| file_result(&out)) {
                    Ok((status, seconds, values)) => {
                        row.status = status;
                        row.seconds = Some(seconds);
                        row.values = values;
                    }
                    Err(error) => {
                        row.status = Status::Error(error.clone());
                        row.seconds = None;
                        row.values.clear();
                        self.run_error = Some(error);
                    }
                }
            }
        }
        if self.active.is_some() {
            return;
        }
        let Some(index) = self.queue.pop_front() else {
            return;
        };
        let Some(plan) = plan else {
            // The inputs became invalid: the status line says why.
            self.queue.clear();
            return;
        };
        let Some(name) = self.files.get(index).map(|f| f.name.clone()) else {
            return;
        };
        if let Err(error) = self.guard_writes(plan, &name) {
            if let Some(row) = self.files.get_mut(index) {
                row.status = Status::Error(error.clone());
                row.seconds = None;
                row.values.clear();
            }
            self.run_error = Some(error);
            return;
        }
        let inputs = match plan
            .inputs
            .iter()
            .map(|n| {
                ws.get(n)
                    .map(|a| (n.clone(), a.clone()))
                    .ok_or_else(|| format!("Workspace variable {n} does not exist"))
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(inputs) => inputs,
            Err(error) => {
                self.queue.clear();
                self.run_error = Some(error);
                return;
            }
        };
        let code = plan.file_code(&name);
        self.serial += 1;
        let k = key((&code, self.serial));
        self.active = Some((index, k));
        self.file_runner.submit(
            ctx,
            Request {
                key: k,
                inputs,
                code,
                outputs: ["table", "status", "message", "seconds"]
                    .map(|p| format!("ibp_probe_{p}"))
                    .to_vec(),
            },
        );
    }
    fn poll_thumbs(&mut self, ctx: &egui::Context, plan: Option<&Plan>) {
        let (Some(plan), Some(file)) = (plan, self.selected.and_then(|k| self.files.get(k))) else {
            self.thumb_runner.stop();
            return;
        };
        let code = plan.thumb_code(&file.name);
        let k = key((&code, self.thumb_serial));
        self.thumb_runner.submit(
            ctx,
            Request {
                key: k,
                inputs: Vec::new(),
                code,
                outputs: vec!["ibp_probe_input".into(), "ibp_probe_output".into()],
            },
        );
        if self.thumbs.as_ref().is_some_and(|(t, _, _)| *t == k) {
            return;
        }
        if let Some(result) = self.thumb_runner.result() {
            match result.clone().and_then(|out| {
                let input = thumb(&out, "ibp_probe_input")?
                    .ok_or("Image Batch Processor: empty input image")?;
                Ok((input, thumb(&out, "ibp_probe_output")?))
            }) {
                Ok((input, output)) => {
                    self.thumbs = Some((k, input, output));
                    self.thumb_error = None;
                }
                Err(error) => {
                    // Keep the last thumbnails only for the same file.
                    self.thumbs = None;
                    self.thumb_error = Some(error);
                }
            }
        }
    }
    pub fn status(&self) -> (String, AppStatus) {
        let label = AppKind::ImageBatchProcessor.label();
        if let Some((index, _)) = self.active {
            let (ok, error, _) = self.counts();
            return (
                format!(
                    "{label}: processing {} · {ok} ok · {error} errors · {} queued",
                    self.files.get(index).map_or("", |f| f.name.as_str()),
                    self.queue.len()
                ),
                AppStatus::Plain,
            );
        }
        if let Some(error) = self.list_error.as_ref().or(self.plan_error.as_ref()) {
            return (error.clone(), AppStatus::Error);
        }
        if let Some(error) = &self.run_error {
            return (format!("{label}: {error}"), AppStatus::Error);
        }
        if !self.message.is_empty() {
            return (self.message.clone(), AppStatus::Plain);
        }
        let (ok, error, pending) = self.counts();
        if self.stopped {
            return (
                format!("{label}: stopped · {ok} ok · {error} errors · {pending} pending"),
                AppStatus::Plain,
            );
        }
        if ok + error == 0 {
            return (
                format!("{label}: {} files", self.files.len()),
                AppStatus::Plain,
            );
        }
        let text = format!(
            "{label}: {ok} ok · {error} errors · {} skipped · {pending} pending",
            self.skipped
        );
        if error > 0 {
            (text, AppStatus::Error)
        } else {
            (text, AppStatus::Success)
        }
    }
    pub fn export(&mut self, ws: &Snapshot) -> Option<Action> {
        let script = self.export_script(ws).map_err(|e| self.message = e).ok()?;
        let name = self.table_name.trim().to_owned();
        self.message = format!("Exported {name} to the workspace");
        Some(Action::ExportScript(super::all_or_nothing(
            &script,
            &[name],
        )))
    }
    pub fn generate(&mut self, ws: &Snapshot, time: u64) -> Option<Action> {
        let script = self.script(ws).map_err(|e| self.message = e).ok()?;
        Some(Action::SaveAppScript(
            AppKind::ImageBatchProcessor,
            super::saved_script(AppKind::ImageBatchProcessor, time, &script),
        ))
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        ws: &Snapshot,
        busy: bool,
        action: &mut Option<Action>,
    ) {
        self.poll(ui.ctx(), ws);
        let idle = !busy && !self.saving && !self.running();
        let script_ready = idle && self.script(ws).is_ok();
        let export_ready = script_ready && self.files.iter().any(|f| f.status == Status::Ok);
        let mut click = None;
        super::three_areas(
            ui,
            AppKind::ImageBatchProcessor,
            "FILES",
            |ui, area| match area {
                super::Area::Inputs => self.inputs_ui(ui, ws),
                super::Area::Main => self.main_ui(ui, ws),
                super::Area::Output => {
                    super::result_block(ui, &self.results());
                    ui.add_space(crate::tokens::S);
                    self.table_ui(ui);
                    ui.add_space(crate::tokens::M);
                    name_field(ui, "Results table name", &mut self.table_name, ws);
                    ui.add_space(crate::tokens::S);
                    click = output_buttons(ui, export_ready, script_ready);
                    if !self.message.is_empty() {
                        ui.add_space(crate::tokens::S);
                        crate::tokens::helper(ui, &self.message);
                    }
                }
            },
        );
        match click {
            Some(super::OutputClick::Export) => *action = self.export(ws),
            Some(super::OutputClick::Script) => *action = self.generate(ws, super::now()),
            None => {}
        }
    }
    pub fn results(&self) -> Vec<(&'static str, String)> {
        let (ok, error, pending) = self.counts();
        let times: Vec<f64> = self.files.iter().filter_map(|f| f.seconds).collect();
        let mut rows = vec![
            ("Files", self.files.len().to_string()),
            ("OK", ok.to_string()),
            ("Errors", error.to_string()),
            ("Skipped (already ok)", self.skipped.to_string()),
            ("Pending", pending.to_string()),
        ];
        if !times.is_empty() {
            let total: f64 = times.iter().sum();
            rows.push(("Time per file (s)", short(total / times.len() as f64)));
            rows.push(("Total time (s)", short(total)));
        }
        rows
    }
    fn inputs_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let running = self.running();
        ui.add_enabled_ui(!running, |ui| {
            field(ui, "Input folder", |ui| {
                ui.horizontal(|ui| {
                    let browse = ui.button("Browse…");
                    crate::focus_areas::register(
                        ui,
                        &browse,
                        crate::focus_areas::Area::App(AppKind::ImageBatchProcessor),
                    );
                    if browse.clicked()
                        && let Some(path) = pick_folder(ws)
                    {
                        self.input = path;
                    }
                    path_edit(ui, &mut self.input);
                });
            });
            ui.checkbox(&mut self.subfolders, "Include subfolders");
            ui.add_space(crate::tokens::S);
            field(ui, "File pattern (; separated)", |ui| {
                ui.add(
                    crate::tokens::input(
                        egui::TextEdit::singleline(&mut self.pattern).char_limit(256),
                    )
                    .desired_width(ui.available_width()),
                );
            });
            if ui.button("Refresh list").clicked() {
                self.list_serial += 1;
            }
            ui.add_space(crate::tokens::M);
            field(ui, "Output folder", |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Browse…").clicked()
                        && let Some(path) = pick_folder(ws)
                    {
                        self.output = path;
                    }
                    path_edit(ui, &mut self.output);
                });
            });
            let mut overwrite = self.overwrite;
            ui.checkbox(&mut overwrite, "Overwrite originals (output = input allowed)");
            if overwrite && !self.overwrite {
                self.request_overwrite();
            } else if !overwrite {
                self.overwrite = false;
            }
            if self.asking_overwrite() {
                let p = crate::tokens::palette(ui.ctx());
                ui.label(
                    egui::RichText::new(
                        "Results replace the original files when the output folder is the input folder.",
                    )
                    .color(p.warning),
                );
                ui.horizontal(|ui| {
                    if ui.button("Allow overwrite").clicked() {
                        self.confirm_overwrite_now();
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_overwrite = false;
                    }
                });
            }
            ui.add_space(crate::tokens::M);
            let mut function = self.function;
            field(ui, "Function", |ui| {
                egui::ComboBox::from_id_salt("ibp_function")
                    .width(ui.available_width())
                    .selected_text(function.label())
                    .show_ui(ui, |ui| {
                        for f in Function::ALL {
                            ui.selectable_value(&mut function, f, f.label());
                        }
                    });
            });
            self.function = function;
            match self.function {
                Function::Named => {
                    field(ui, "Function name or workspace handle", |ui| {
                        ui.add(
                            crate::tokens::input(
                                egui::TextEdit::singleline(&mut self.name).char_limit(63),
                            )
                            .desired_width(ui.available_width()),
                        );
                    });
                    field(ui, "The function returns", |ui| {
                        egui::ComboBox::from_id_salt("ibp_returns")
                            .width(ui.available_width())
                            .selected_text(returns_label(self.returns))
                            .show_ui(ui, |ui| {
                                for r in [Returns::One, Returns::Both] {
                                    ui.selectable_value(&mut self.returns, r, returns_label(r));
                                }
                            });
                    });
                }
                Function::Resize => {
                    field(ui, "Percent of the original size (1–1000)", |ui| {
                        ui.add(
                            crate::tokens::input(
                                egui::TextEdit::singleline(&mut self.percent).char_limit(16),
                            )
                            .desired_width(ui.available_width()),
                        );
                    });
                }
                _ => {}
            }
            if let Some(name) = self.function.template() {
                crate::tokens::helper(
                    ui,
                    format!("The template is saved as {name}.m in the output folder."),
                );
            }
        });
        ui.add_space(crate::tokens::S);
        super::expression_message(ui, self.plan(ws).map(|_| ()));
        crate::tokens::helper(
            ui,
            "Images are written under the same name and format; struct results of real scalars become table rows.",
        );
    }
    /// The answer to the overwrite question: asked once per session.
    pub fn confirm_overwrite_now(&mut self) {
        self.overwrite = true;
        self.overwrite_confirmed = true;
        self.confirm_overwrite = false;
    }
    /// Tick Overwrite originals: asks first unless already confirmed.
    pub fn request_overwrite(&mut self) {
        if self.overwrite_confirmed {
            self.overwrite = true;
        } else {
            self.confirm_overwrite = true;
        }
    }
    pub fn asking_overwrite(&self) -> bool {
        self.confirm_overwrite
    }
    fn main_ui(&mut self, ui: &mut egui::Ui, ws: &Snapshot) {
        let running = self.running();
        let plan_ok = self.plan(ws).is_ok();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !running && plan_ok && !self.files.is_empty(),
                    egui::Button::new("Run all"),
                )
                .clicked()
            {
                self.run_all();
            }
            if ui
                .add_enabled(
                    !running && plan_ok && self.selected.is_some(),
                    egui::Button::new("Run selected"),
                )
                .clicked()
            {
                self.run_selected();
            }
            if ui
                .add_enabled(running, egui::Button::new("Stop"))
                .on_hover_text("Cancel the current file and stop")
                .clicked()
            {
                self.stop();
            }
            ui.add_enabled_ui(!running, |ui| {
                ui.checkbox(&mut self.force, "Re-run ok files");
            });
        });
        ui.add_space(crate::tokens::S);
        if self.files.is_empty() {
            crate::tokens::helper(
                ui,
                "Choose an input folder with matching image files. The status line shows any error.",
            );
            return;
        }
        let thumbs = THUMB_SIDE as f32 + 2. * crate::tokens::L + crate::tokens::ROW;
        let list_height = (ui.available_height() - thumbs).max(4. * crate::tokens::ROW);
        self.list_ui(ui, list_height);
        ui.add_space(crate::tokens::S);
        self.thumbs_ui(ui);
    }
    fn list_ui(&mut self, ui: &mut egui::Ui, height: f32) {
        use crate::theme::ThemedScrollArea;
        let p = crate::tokens::palette(ui.ctx());
        let row = crate::tokens::ROW;
        let mut select = None;
        egui::ScrollArea::vertical()
            .id_salt("ibp_files")
            .max_height(height)
            .auto_shrink([false, true])
            .show_rows_themed(ui, row, self.files.len(), |ui, range| {
                for k in range {
                    let f = &self.files[k];
                    let (text, colour) = match &f.status {
                        Status::Pending => ("pending".to_owned(), p.text_muted),
                        Status::Ok => ("ok".to_owned(), p.success),
                        Status::Stopped => ("stopped".to_owned(), p.text_muted),
                        Status::Error(m) => (format!("error: {m}"), p.error),
                    };
                    let time = f
                        .seconds
                        .map(|s| format!("{} s", short(s)))
                        .unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.set_height(row);
                        let name = ui.add_sized(
                            [(ui.available_width() * 0.45).max(80.), row],
                            egui::Button::selectable(self.selected == Some(k), &f.name).truncate(),
                        );
                        if name.clicked() {
                            select = Some(k);
                        }
                        ui.add_sized(
                            [64., row],
                            egui::Label::new(egui::RichText::new(time).color(p.text_muted)),
                        );
                        let status = ui.add(
                            egui::Label::new(egui::RichText::new(&text).color(colour)).truncate(),
                        );
                        if matches!(f.status, Status::Error(_)) {
                            status.on_hover_text(text);
                        }
                    });
                }
            });
        if let Some(k) = select {
            self.selected = Some(k);
        }
    }
    fn thumbs_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.thumb_error {
            crate::tokens::helper(ui, format!("Preview: {error}"));
            return;
        }
        let Some((k, input, output)) = &self.thumbs else {
            return;
        };
        if self.textures.as_ref().is_none_or(|(t, _, _)| t != k) {
            let load = |name: &str, t: &Thumb| {
                ui.ctx().load_texture(
                    name,
                    egui::ColorImage::from_rgb([t.size[1], t.size[0]], &t.rgb),
                    egui::TextureOptions::NEAREST,
                )
            };
            let a = load("ibp-input", input);
            let b = output.as_ref().map(|t| load("ibp-output", t));
            self.textures = Some((*k, a, b));
        }
        let Some((_, a, b)) = &self.textures else {
            return;
        };
        let size = |t: &Thumb| {
            let scale = THUMB_SIDE as f32 / t.size[0].max(t.size[1]) as f32;
            egui::vec2(t.size[1] as f32 * scale, t.size[0] as f32 * scale)
        };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label("Input");
                ui.image((a.id(), size(input)));
            });
            ui.add_space(crate::tokens::L);
            ui.vertical(|ui| {
                ui.label("Result");
                match (b, output) {
                    (Some(b), Some(t)) => {
                        ui.image((b.id(), size(t)));
                    }
                    _ => {
                        crate::tokens::helper(ui, "No output image for this file yet");
                    }
                }
            });
        });
    }
    fn table_ui(&self, ui: &mut egui::Ui) {
        let t = self.table();
        if t.files.is_empty() {
            crate::tokens::helper(ui, "The results table fills as files run ok.");
            return;
        }
        crate::tokens::helper(
            ui,
            format!(
                "Results table: {} rows · File{}",
                t.files.len(),
                t.names.iter().map(|n| format!(", {n}")).collect::<String>()
            ),
        );
        let p = crate::tokens::palette(ui.ctx());
        egui::Grid::new("ibp_table")
            .num_columns(1 + t.names.len().min(3))
            .spacing(egui::vec2(crate::tokens::S, crate::tokens::XS))
            .show(ui, |ui| {
                for (r, file) in t.files.iter().take(8).enumerate() {
                    ui.add(egui::Label::new(egui::RichText::new(file).color(p.text)).truncate());
                    for c in t.columns.iter().take(3) {
                        ui.label(egui::RichText::new(short(c[r])).color(p.text_muted));
                    }
                    ui.end_row();
                }
            });
        if t.files.len() > 8 {
            crate::tokens::helper(ui, format!("… {} more rows", t.files.len() - 8));
        }
    }
    #[cfg(test)]
    pub fn thumbnails(&self) -> Option<(&Thumb, Option<&Thumb>)> {
        self.thumbs.as_ref().map(|(_, a, b)| (a, b.as_ref()))
    }
    #[cfg(test)]
    pub fn thumbs_running(&self) -> bool {
        self.thumb_runner.running()
    }
    #[cfg(test)]
    pub fn listing_running(&self) -> bool {
        self.list_runner.running()
    }
    #[cfg(test)]
    pub fn file_running(&self) -> bool {
        self.file_runner.running()
    }
}

fn returns_label(r: Returns) -> &'static str {
    match r {
        Returns::One => "An image or a struct",
        Returns::Both => "[image, struct]",
    }
}

fn path_edit(ui: &mut egui::Ui, text: &mut String) {
    ui.add(
        crate::tokens::input(egui::TextEdit::singleline(text).char_limit(MAX_PATH))
            .desired_width(ui.available_width()),
    );
}

fn pick_folder(ws: &Snapshot) -> Option<String> {
    #[cfg(test)]
    {
        let _ = ws;
        None
    }
    #[cfg(not(test))]
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(folder) = ws.folder() {
            dialog = dialog.set_directory(folder);
        }
        dialog
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    }
}

/// Export (needs a collected table) and Generate script (needs valid inputs).
fn output_buttons(ui: &mut egui::Ui, export: bool, script: bool) -> Option<super::OutputClick> {
    const WAIT: &str = "Available when the inputs are valid, no batch or command is running \
                        and (for the table) a file ran ok";
    let wide = egui::vec2(ui.available_width(), crate::tokens::CONTROL);
    let mut clicked = None;
    if ui
        .add_enabled(
            export,
            egui::Button::new("Export results table").min_size(wide),
        )
        .on_disabled_hover_text(WAIT)
        .clicked()
    {
        clicked = Some(super::OutputClick::Export);
    }
    if ui
        .add_enabled(script, egui::Button::new("Generate script").min_size(wide))
        .on_disabled_hover_text(WAIT)
        .clicked()
    {
        clicked = Some(super::OutputClick::Script);
    }
    clicked
}

/// A cell array of char vectors.
fn names(outputs: &Outputs, name: &str) -> Result<Vec<String>, String> {
    let bad = || format!("Image Batch Processor: {name} is not a list of file names");
    match outputs.get(name) {
        Some(Array::Cell(c)) => {
            if c.len() > MAX_FILES {
                return Err(format!(
                    "Image Batch Processor: more than {MAX_FILES} files; narrow the pattern"
                ));
            }
            (0..c.len())
                .map(|i| match c.get_linear(i).map_err(|_| bad())?.as_ref() {
                    Array::Character(s) => Ok(String::from_utf16_lossy(s.units())),
                    _ => Err(bad()),
                })
                .collect()
        }
        _ => Err(bad()),
    }
}
fn text(outputs: &Outputs, name: &str) -> String {
    match outputs.get(name) {
        Some(Array::Character(s)) => String::from_utf16_lossy(s.units()),
        _ => String::new(),
    }
}

type FileResult = (Status, f64, Vec<(String, f64)>);
fn file_result(outputs: &Outputs) -> Result<FileResult, String> {
    let seconds = match outputs.get("ibp_probe_seconds") {
        Some(Array::Numeric(m)) if m.data().len() == 1 => m.data()[0],
        _ => return Err("Image Batch Processor: missing file time".into()),
    };
    let status = match text(outputs, "ibp_probe_status").as_str() {
        "ok" => Status::Ok,
        "error" => Status::Error(text(outputs, "ibp_probe_message")),
        other => return Err(format!("Image Batch Processor: unexpected status {other}")),
    };
    let mut values = Vec::new();
    if status == Status::Ok {
        let Some(Array::Table(t)) = outputs.get("ibp_probe_table") else {
            return Err("Image Batch Processor: missing results row".into());
        };
        if t.height() != 1 {
            return Err("Image Batch Processor: expected one results row".into());
        }
        for (name, column) in t.names().iter().zip(t.columns()).skip(1) {
            match column {
                Array::Numeric(m) if m.data().len() == 1 => {
                    values.push((name.clone(), m.data()[0]));
                }
                _ => return Err(format!("Image Batch Processor: {name} is not a scalar")),
            }
        }
    }
    Ok((status, seconds, values))
}

/// A uint8 H-by-W(-by-3) thumbnail; None for an empty array.
fn thumb(outputs: &Outputs, name: &str) -> Result<Option<Thumb>, String> {
    let a = outputs
        .get(name)
        .ok_or_else(|| format!("Image Batch Processor: missing {name}"))?;
    if a.is_empty() {
        return Ok(None);
    }
    let dims = a.dims();
    let (h, w) = (dims[0], dims.get(1).copied().unwrap_or(1));
    let c = dims.get(2).copied().unwrap_or(1);
    let Array::Typed(t) = a else {
        return Err("Image Batch Processor: thumbnail is not uint8".into());
    };
    let TypedData::Uint8(data) = t.data() else {
        return Err("Image Batch Processor: thumbnail is not uint8".into());
    };
    if dims.len() > 3 || !matches!(c, 1 | 3) || h > THUMB_SIDE || w > THUMB_SIDE {
        return Err("Image Batch Processor: unexpected thumbnail size".into());
    }
    let mut rgb = Vec::with_capacity(h * w * 3);
    for row in 0..h {
        for col in 0..w {
            let at = row + col * h;
            if c == 3 {
                rgb.extend([data[at], data[at + h * w], data[at + 2 * h * w]]);
            } else {
                rgb.extend([data[at]; 3]);
            }
        }
    }
    Ok(Some(Thumb { size: [h, w], rgb }))
}
