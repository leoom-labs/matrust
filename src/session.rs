//! Bounded desktop recovery and navigation state, independent of the UI runtime.
pub mod figure_window;
pub mod figures;
pub mod workspace;
pub use figures::SessionStore;
mod export;
mod layout;
mod presets;
pub use export::{ExportFormat, ExportSettings};
pub use presets::{LayoutPresets, MAX_LAYOUT_PRESET_BYTES, MAX_LAYOUT_PRESETS, NamedLayout};
mod preview;
pub use layout::{
    CenterLayout, DesktopLayout, DesktopTheme, EditorColumns, FigureTileSize, FigureTiling,
    PaneMode, PanelLayout, SidebarPanel, ToolTab, ToolTabs, ToolstripTab, WorkspaceDock,
};
pub use preview::{PreviewFormat, PreviewNavigation, PreviewState, PreviewWindow};

use crate::{Result, document::Document, files, simulation::Model};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};
pub const MAX_DOCUMENTS: usize = 32;
pub const MAX_SESSION_BYTES: usize = 32 * 1024 * 1024;
struct BoundedJson<'a>(Vec<u8>, &'a std::sync::atomic::AtomicBool);
impl Write for BoundedJson<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        crate::dsp::check_cancel(self.1).map_err(std::io::Error::other)?;
        let size = self.0.len().saturating_add(bytes.len());
        if size > MAX_SESSION_BYTES {
            return Err(std::io::Error::other(
                "Session exceeds 32 MiB recovery limit; save and close unused scripts",
            ));
        }
        if size > self.0.capacity() {
            let capacity = size
                .max(self.0.capacity().saturating_mul(2))
                .clamp(4096, MAX_SESSION_BYTES);
            self.0
                .try_reserve_exact(capacity - self.0.len())
                .map_err(std::io::Error::other)?;
        }
        for chunk in bytes.chunks(65536) {
            crate::dsp::check_cancel(self.1).map_err(std::io::Error::other)?;
            self.0.extend_from_slice(chunk);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    #[serde(default, skip_serializing_if = "LayoutPresets::is_empty")]
    pub layout_presets: LayoutPresets,
    #[serde(default, deserialize_with = "export::recover_settings")]
    pub export: ExportSettings,
    #[serde(default, deserialize_with = "workspace::recover_manifest")]
    pub workspace_manifest: Option<workspace::Manifest>,
    #[serde(skip)]
    pub workspace: Option<std::collections::BTreeMap<String, crate::Array>>,
    #[serde(default, deserialize_with = "preview::recover_preview")]
    pub preview: Option<PreviewState>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "preview::recover_renderer"
    )]
    pub eps_renderer: Option<PathBuf>,
    #[serde(default, deserialize_with = "figures::recover_manifest")]
    pub figure_manifest: Option<figures::FigureManifest>,
    #[serde(skip)]
    pub figures: Option<figures::FigureSession>,
    #[serde(default, deserialize_with = "layout::recover_layout")]
    pub layout: DesktopLayout,
    pub version: u32,
    pub documents: Vec<Document>,
    pub active: usize,
    pub folder: PathBuf,
    pub recent: Vec<PathBuf>,
    pub history: Vec<String>,
    pub model: Model,
    #[serde(default)]
    pub model_path: Option<PathBuf>,
    #[serde(default)]
    pub model_baseline: Option<Model>,
}
impl Session {
    pub fn validate(&mut self) -> Result<()> {
        self.layout_presets.normalize();
        self.export.normalize();
        if self
            .eps_renderer
            .as_ref()
            .is_some_and(|p| !preview::valid_renderer(p))
        {
            self.eps_renderer = None;
        }
        if let Some(preview) = &mut self.preview {
            preview.navigation.normalize();
        }
        if self.preview.as_ref().is_some_and(|p| !p.valid()) {
            self.preview = None;
        }
        self.layout.normalize();
        if self.version != 1
            || self.documents.is_empty()
            || self.documents.len() > MAX_DOCUMENTS
            || self.active >= self.documents.len()
        {
            return Err("Invalid session version or document list".into());
        }
        self.layout.editor_columns = self
            .layout
            .editor_columns
            .filter(|columns| columns.valid_for(self.documents.len(), self.active));
        for doc in &mut self.documents {
            doc.validate_recovery()?;
        }
        if self.recent.len() > 20
            || self.history.len() > 200
            || self.history.iter().map(String::len).sum::<usize>() > 2_000_000
        {
            return Err("Session history exceeds limit".into());
        }
        self.model.validate_storage()?;
        if let Some(baseline) = &self.model_baseline {
            baseline.validate_storage()?;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = files::read_bounded(path, MAX_SESSION_BYTES)?;
        let mut session: Self =
            serde_json::from_slice(&bytes).map_err(|e| format!("Cannot read session: {e}"))?;
        session.validate()?;
        let recent = std::mem::take(&mut session.recent);
        for path in recent.into_iter().rev() {
            remember(&mut session.recent, path);
        }
        for doc in &mut session.documents {
            if !doc.dirty
                && let Some(path) = &doc.path
                && let Ok(current) = Document::open(path)
            {
                *doc = current;
            }
        }
        Ok(session)
    }
    pub fn save(self, path: &Path) -> Result<()> {
        SessionStore::new(path).save(self)
    }
    fn save_json(mut self, path: &Path, cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
        self.validate()?;
        let mut bytes = BoundedJson(Vec::new(), cancel);
        serde_json::to_writer(&mut bytes, &self).map_err(|e| e.to_string())?;
        publication_checkpoint(cancel)?;
        // The bounded final atomic write is the commit point; a late Stop does not undo it.
        files::atomic_write(path, &bytes.0)
    }
}

pub fn completion_at(text: &str, cursor: usize) -> Result<(String, Range<usize>)> {
    let pos = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .nth(cursor)
        .ok_or("Cursor is outside command")?;
    let identifier = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let start = text[..pos]
        .char_indices()
        .rev()
        .find(|(_, c)| !identifier(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let end = text[pos..]
        .char_indices()
        .find(|(_, c)| !identifier(*c))
        .map_or(text.len(), |(i, _)| pos + i);
    Ok((text[start..pos].into(), start..end))
}

#[derive(Default)]
pub struct History {
    pub entries: Vec<String>,
    cursor: usize,
    draft: String,
}
impl History {
    pub fn from_entries(entries: Vec<String>) -> Self {
        let cursor = entries.len();
        Self {
            entries,
            cursor,
            draft: String::new(),
        }
    }
    pub fn push(&mut self, command: String) {
        if !command.trim().is_empty() && self.entries.last() != Some(&command) {
            self.entries.push(command);
        }
        while self.entries.len() > 200
            || self.entries.iter().map(String::len).sum::<usize>() > 2_000_000
        {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
        self.draft.clear();
    }
    pub fn previous(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        if self.cursor == self.entries.len() {
            self.draft = current.into();
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.entries.get(self.cursor).cloned()
    }
    pub fn browsing(&self) -> bool {
        self.cursor < self.entries.len()
    }
    pub fn next_entry(&mut self) -> String {
        self.cursor = (self.cursor + 1).min(self.entries.len());
        if self.cursor == self.entries.len() {
            self.draft.clone()
        } else {
            self.entries[self.cursor].clone()
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
        self.draft.clear();
    }
}

/// egui cursor offsets count Unicode characters, not bytes.
pub fn selected_text(text: &str, range: Range<usize>) -> Result<String> {
    if range.start >= range.end {
        return Err("Select code in the editor first".into());
    }
    let mut offsets = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()));
    let start = offsets
        .nth(range.start)
        .ok_or("Selection is out of bounds")?;
    let end = offsets
        .nth(range.end - range.start - 1)
        .ok_or("Selection is out of bounds")?;
    Ok(text[start..end].to_owned())
}
pub fn remember(recent: &mut Vec<PathBuf>, path: PathBuf) {
    let path = std::fs::canonicalize(&path).unwrap_or(path);
    recent.retain(|p| p != &path);
    recent.insert(0, path);
    recent.truncate(20);
}

pub struct FolderEntry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}
pub fn folder_entries(path: &Path) -> Result<Vec<FolderEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(2001)
    {
        if entries.len() == 2000 {
            return Err("Folder contains more than 2000 entries; select a smaller folder".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        entries.push(FolderEntry {
            path: entry.path(),
            name: entry.file_name().to_string_lossy().into_owned(),
            directory: kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()),
        });
    }
    entries.sort_by_cached_key(|e| (!e.directory, e.name.to_lowercase()));
    Ok(entries)
}

pub const FUNCTIONS: &[&str] = &[
    "getenv",
    "setenv",
    "ispc",
    "isunix",
    "ismac",
    "computer",
    "version",
    "ver",
    "verLessThan",
    "matlabroot",
    "filesep",
    "pathsep",
    "pwd",
    "cd",
    "dir",
    "ls",
    "mkdir",
    "rmdir",
    "fileread",
    "tempdir",
    "tempname",
    "clock",
    "cputime",
    "date",
    "now",
    "pause",
    "timeit",
    "isdir",
    "copyfile",
    "movefile",
    "delete",
    "movmax",
    "movmin",
    "movmedian",
    "movstd",
    "movvar",
    "repelem",
    "isequaln",
    "isdiag",
    "istril",
    "istriu",
    "isbanded",
    "issymmetric",
    "ishermitian",
    "bandwidth",
    "uniquetol",
    "ismembertol",
    "histcounts2",
    "bitget",
    "bitset",
    "swapbytes",
    "num2hex",
    "hex2num",
    "flipdim",
    "shiftdim",
    "permute",
    "ipermute",
    "filter2",
    "interpft",
    "interp1q",
    "rectint",
    "polyarea",
    "inpolygon",
    "peaks",
    "hsv2rgb",
    "rgb2hsv",
    "atan2d",
    "full",
    "evalc",
    "evalin",
    "assignin",
    "global",
    "persistent",
    "inputname",
    "nargoutchk",
    "nargchk",
    "validateattributes",
    "lasterr",
    "lastwarn",
    "MException",
    "throw",
    "throwAsCaller",
    "onCleanup",
    "cell2struct",
    "struct2cell",
    "substruct",
    "celldisp",
    "mat2cell",
    "sscanf",
    "strread",
    "textread",
    "str2mat",
    "strmatch",
    "findstr",
    "regexptranslate",
    "isstr",
    "setstr",
    "symvar",
    "vectorize",
    "who",
    "rehash",
    "addCause",
    "getReport",
    "properties",
    "subsref",
    "rref",
    "hess",
    "schur",
    "ldl",
    "linsolve",
    "lscov",
    "lsqnonneg",
    "rcond",
    "condest",
    "normest",
    "subspace",
    "sylvester",
    "polyvalm",
    "balance",
    "planerot",
    "mkpp",
    "unmkpp",
    "ppval",
    "residue",
    "blkdiag",
    "cdf2rdf",
    "rsf2csf",
    "struct",
    "fieldnames",
    "isfield",
    "isstruct",
    "isequal",
    "rmfield",
    "setfield",
    "getfield",
    "numfields",
    "error",
    "rethrow",
    "try",
    "catch",
    "mod",
    "rem",
    "fix",
    "sign",
    "hypot",
    "atan2",
    "prod",
    "dot",
    "cross",
    "kron",
    "circshift",
    "trace",
    "logspace",
    "meshgrid",
    "ndgrid",
    "eig",
    "svd",
    "rank",
    "polyfit",
    "rand",
    "randn",
    "randi",
    "rng",
    "tic",
    "toc",
    "sort",
    "sortrows",
    "unique",
    "ismember",
    "union",
    "intersect",
    "setdiff",
    "setxor",
    "diag",
    "tril",
    "triu",
    "flip",
    "flipud",
    "fliplr",
    "sprintf",
    "fprintf",
    "fopen",
    "fclose",
    "fgetl",
    "fgets",
    "feof",
    "ftell",
    "fseek",
    "frewind",
    "fwrite",
    "fread",
    "fscanf",
    "textscan",
    "dlmread",
    "dlmwrite",
    "csvread",
    "csvwrite",
    "type",
    "exist",
    "num2str",
    "int2str",
    "mat2str",
    "strcat",
    "strvcat",
    "strsplit",
    "strjoin",
    "strrep",
    "strtrim",
    "deblank",
    "upper",
    "lower",
    "strcmp",
    "strcmpi",
    "strncmp",
    "strncmpi",
    "strfind",
    "num2cell",
    "iscellstr",
    "blanks",
    "regexp",
    "regexpi",
    "regexprep",
    "str2double",
    "str2num",
    "strjust",
    "isspace",
    "isletter",
    "isstrprop",
    "validatestring",
    "isvarname",
    "datenum",
    "datestr",
    "datevec",
    "weekday",
    "etime",
    "addtodate",
    "eomday",
    "calendar",
    "cumsum",
    "cumprod",
    "diff",
    "trapz",
    "cumtrapz",
    "integral",
    "integral2",
    "integral3",
    "quad",
    "quadgk",
    "dblquad",
    "triplequad",
    "blackman",
    "blackmanharris",
    "nuttallwin",
    "flattopwin",
    "bartlett",
    "triang",
    "rectwin",
    "save",
    "savefig",
    "saveas",
    "exportgraphics",
    "openfig",
    "gcf",
    "load",
    "if",
    "elseif",
    "else",
    "end",
    "for",
    "while",
    "switch",
    "case",
    "otherwise",
    "break",
    "continue",
    "return",
    "function",
    "func2str",
    "str2func",
    "feval",
    "arrayfun",
    "cellfun",
    "nargin",
    "nargout",
    "true",
    "false",
    "logical",
    "double",
    "islogical",
    "isnumeric",
    "complex",
    "i",
    "j",
    "real",
    "imag",
    "conj",
    "isreal",
    "angle",
    "ctranspose",
    "isnan",
    "isinf",
    "isfinite",
    "any",
    "all",
    "nnz",
    "find",
    "and",
    "or",
    "xor",
    "not",
    "axis",
    "bar",
    "clf",
    "close",
    "figure",
    "subplot",
    "tiledlayout",
    "nexttile",
    "sgtitle",
    "grid",
    "hold",
    "legend",
    "scatter",
    "stairs",
    "stem",
    "xlim",
    "ylim",
    "abs",
    "butter_sos",
    "cheby1",
    "cheby1_sos",
    "ceil",
    "clear",
    "clc",
    "format",
    "cos",
    "det",
    "disp",
    "exp",
    "expm1",
    "eye",
    "fft",
    "fileparts",
    "filter",
    "filtfilt",
    "filtic",
    "readmatrix",
    "writematrix",
    "sinc",
    "upsample",
    "downsample",
    "upfirdn",
    "square",
    "sawtooth",
    "chirp",
    "fir1",
    "floor",
    "freqz",
    "freqzmag",
    "fullfile",
    "grpdelay",
    "hamming",
    "hann",
    "kaiser",
    "help",
    "doc",
    "eps",
    "Inf",
    "inf",
    "NaN",
    "nan",
    "cell",
    "char",
    "class",
    "iscell",
    "ischar",
    "isgraphics",
    "gca",
    "get",
    "set",
    "axes",
    "patch",
    "annotation",
    "inv",
    "lu",
    "qr",
    "chol",
    "pinv",
    "null",
    "orth",
    "expm",
    "logm",
    "sqrtm",
    "mldivide",
    "mrdivide",
    "cond",
    "isempty",
    "isscalar",
    "isvector",
    "isrow",
    "iscolumn",
    "ismatrix",
    "ndims",
    "ifft",
    "interp1",
    "impz",
    "length",
    "linspace",
    "log",
    "log1p",
    "log10",
    "log2",
    "max",
    "mean",
    "median",
    "min",
    "mfilename",
    "nextpow2",
    "gcd",
    "lcm",
    "factorial",
    "nchoosek",
    "primes",
    "isprime",
    "factor",
    "gamma",
    "gammaln",
    "beta",
    "betaln",
    "erf",
    "erfc",
    "erfinv",
    "besselj",
    "bessely",
    "besseli",
    "besselk",
    "besselh",
    "airy",
    "gammainc",
    "gammaincinv",
    "betainc",
    "betaincinv",
    "expint",
    "psi",
    "legendre",
    "ellipke",
    "ellipj",
    "erfcx",
    "erfcinv",
    "bitand",
    "bitor",
    "bitxor",
    "bitshift",
    "bitcmp",
    "dec2bin",
    "bin2dec",
    "dec2hex",
    "hex2dec",
    "dec2base",
    "base2dec",
    "nthroot",
    "norm",
    "numel",
    "ode23",
    "ode45",
    "odeget",
    "odeset",
    "ode113",
    "ode15s",
    "ode23s",
    "ode23t",
    "ode23tb",
    "ode15i",
    "decic",
    "deval",
    "odextend",
    "bvp4c",
    "bvp5c",
    "bvpinit",
    "bvpget",
    "bvpset",
    "bvpxtend",
    "dde23",
    "ddeget",
    "ddeset",
    "ddesd",
    "ones",
    "pi",
    "polyval",
    "pow2",
    "phasez",
    "plot",
    "semilogx",
    "semilogy",
    "loglog",
    "xscale",
    "yscale",
    "repmat",
    "reshape",
    "round",
    "run",
    "sin",
    "size",
    "sosfilt",
    "sosfreqz",
    "sosfreqzmag",
    "sosgrpdelay",
    "sosimpz",
    "sosphasez",
    "sosstepz",
    "sqrt",
    "std",
    "sum",
    "stepz",
    "tan",
    "title",
    "transpose",
    "var",
    "whos",
    "xlabel",
    "ylabel",
    "zeros",
    "conv",
    "xcorr",
    "deconv",
    "roots",
    "poly",
    "polyder",
    "polyint",
    "conv2",
    "interp2",
    "spline",
    "pchip",
    "fzero",
    "fminsearch",
    "fminbnd",
    "fft2",
    "ifft2",
    "fftshift",
    "ifftshift",
    "mode",
    "histc",
    "histcounts",
    "cat",
    "horzcat",
    "vertcat",
    "rot90",
    "squeeze",
    "sub2ind",
    "ind2sub",
    "magic",
    "hilb",
    "invhilb",
    "pascal",
    "toeplitz",
    "vander",
    "hadamard",
    "compan",
    "rosser",
    "wilkinson",
    "randperm",
    "accumarray",
    "cummax",
    "cummin",
    "colon",
    "flintmax",
    "realmax",
    "realmin",
    "sinh",
    "cosh",
    "tanh",
    "asinh",
    "acosh",
    "atanh",
    "sind",
    "cosd",
    "tand",
    "asind",
    "acosd",
    "atand",
    "sec",
    "csc",
    "cot",
    "asec",
    "acsc",
    "acot",
    "secd",
    "cscd",
    "cotd",
    "asecd",
    "acscd",
    "acotd",
    "sech",
    "csch",
    "coth",
    "asech",
    "acsch",
    "acoth",
    "deg2rad",
    "rad2deg",
    "cart2pol",
    "pol2cart",
    "cart2sph",
    "sph2cart",
    "cov",
    "corrcoef",
    "movmean",
    "movsum",
    "detrend",
    "discretize",
    "hist",
    "issorted",
    "range",
    "cell2mat",
    "cellstr",
    "deal",
    "structfun",
    "strtok",
    "assert",
    "warning",
    "eval",
    "rat",
    "rats",
    "cast",
    "idivide",
    "int16",
    "int32",
    "int64",
    "int8",
    "intmax",
    "intmin",
    "isa",
    "isfloat",
    "isinteger",
    "single",
    "typecast",
    "uint16",
    "uint32",
    "uint64",
    "uint8",
    "acos",
    "asin",
    "atan",
    "realsqrt",
    "reallog",
    "realpow",
    "plus",
    "minus",
    "times",
    "rdivide",
    "ldivide",
    "mtimes",
    "power",
    "mpower",
    "uminus",
    "uplus",
    "eq",
    "ne",
    "lt",
    "gt",
    "le",
    "ge",
    "nonzeros",
    "perms",
    "gradient",
    "del2",
    "unwrap",
    "hankel",
    "cplxpair",
    "orderfields",
    "narginchk",
    "optimset",
    "zip",
    "unzip",
    "gzip",
    "gunzip",
    "tar",
    "untar",
    "importdata",
    "imread",
    "imwrite",
    "imfinfo",
    "audioread",
    "audiowrite",
    "audioinfo",
    "fileattrib",
    "bsxfun",
    "builtin",
    "which",
    "what",
    "isobject",
    "ismethod",
    "iskeyword",
    "genvarname",
    "namelengthmax",
    "clearvars",
    "functions",
    "localfunctions",
    "dbstack",
    "isequalwithequalnans",
    "bitmax",
    "inline",
    "argnames",
    "formula",
    "mexext",
    "maxNumCompThreads",
    "usejava",
    "isstudent",
    "diary",
    "fftn",
    "ifftn",
    "convn",
    "interp3",
    "interpn",
    "griddata",
    "griddatan",
    "delaunay",
    "delaunayn",
    "convhull",
    "convhulln",
    "dsearchn",
    "tsearchn",
    "voronoin",
    "quad2d",
    "quadl",
    "quadv",
    "freqspace",
    "iqr",
    "containers.Map",
    "keys",
    "values",
    "isKey",
    "remove",
    "inputParser",
    "addRequired",
    "addOptional",
    "addParameter",
    "addParamValue",
    "parse",
    "eigs",
    "svds",
    "condeig",
    "funm",
    "polyeig",
    "qz",
    "ordschur",
    "ordeig",
    "ordqz",
    "gsvd",
    "cholupdate",
    "qrupdate",
    "qrinsert",
    "qrdelete",
    "pcg",
    "gmres",
    "bicg",
    "bicgstab",
    "bicgstabl",
    "cgs",
    "minres",
    "lsqr",
    "qmr",
    "symmlq",
    "tfqmr",
];
pub fn completions<'a>(prefix: &str, variables: impl Iterator<Item = &'a str>) -> Vec<String> {
    if prefix.is_empty() {
        return vec![];
    }
    let mut matches: Vec<_> = FUNCTIONS
        .iter()
        .copied()
        .chain(variables)
        .filter(|n| n.starts_with(prefix))
        .map(str::to_owned)
        .collect();
    matches.sort();
    matches.dedup();
    matches.truncate(20);
    matches
}

fn publication_checkpoint(cancel: &std::sync::atomic::AtomicBool) -> Result<()> {
    #[cfg(test)]
    STOP_BEFORE_COMMIT.with(|armed| {
        if armed.replace(false) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    crate::dsp::check_cancel(cancel)
}
#[cfg(test)]
thread_local! { static STOP_BEFORE_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
