use crate::{
    Array, ComplexMatrix, LogicalMatrix, Matrix, Result,
    plot::Figure,
    syntax::{self, Expr, Token},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
/// Correctly rounded fused multiply-add. `f64::mul_add` calls the C runtime's `fma`, which on the
/// Windows GNU build is wrong in the last bit for about 1 in 4 inputs (tool/fma_probe.rs, 2026-10-25);
/// `libm::fma` gives the same bits on every platform.
pub(crate) trait Fma {
    fn fma(self, b: f64, c: f64) -> f64;
}
impl Fma for f64 {
    #[inline]
    fn fma(self, b: f64, c: f64) -> f64 {
        libm::fma(self, b, c)
    }
}
mod array_functions;
mod arrays_more;
#[cfg(target_arch = "wasm32")]
mod browser;
mod cell_ops;
mod character_values;
mod chebyshev;
mod clear_variables;
mod complex_math;
mod complex_values;
mod container_values;
mod control_flow;
mod core_more;
mod cplx_linalg;
mod cplx_poly;
mod cplx_qr;
mod cplx_reduce;
mod cplx_svd;
mod dates;
mod eig_more;
mod elementary;
mod env_files;
mod errors;
mod exportgraphics;
mod figure_export;
mod figure_files;
mod file_commands;
mod file_io;
mod files_more;
mod filtic;
mod format_command;
mod fourier;
mod interp1;
mod iter_solvers;
mod lang_extra;
mod lang_more;
mod lang_utils;
mod linalg_more;
mod linear_algebra;
mod linear_algebra2;
mod matrix_build;
mod matrix_files;
mod median;
mod misc_more;
mod near_zero;
mod number_theory;
mod objects;
mod ode;
mod ode_more;
mod poly_interp;
mod polyval;
mod powers_of_two;
mod quadrature;
mod random;
mod rate_conversion;
#[cfg(test)]
mod real_singular_tests;
mod reductions;
mod repmat;
mod sci_more;
mod script_paths;
mod shape_arguments;
mod shape_queries;
mod special_fns;
mod stopwatch;
mod trig_more;
mod xcorr;
pub use figure_files::FigureFileEvent;
mod graphics;
mod graphics_handles;
mod handles;
mod help_command;
mod indexed_assignment;
mod indexing;
mod ismember;
mod logarithms;
mod logical_values;
mod plot_properties;
mod set_operations;
mod sort;
mod sort_rows;
mod stats_more;
mod stored_array;
mod strings;
mod strings2;
mod structs;
mod switch_case;
mod text_format;
mod typed_functions;
mod typed_values;
mod unique;
mod user_functions;
mod value_equality;
mod workspace_assignment;

pub const HELP: &str = concat!(
    "Matrices: A = [1 2; 3 4]; b = [5; 6]; A*b; A\\b; A'; A.^2\n",
    "Vectors: x = 0:0.1:10; y = sin(x); plot(x,y)\n",
    "Indexing: A(1,2), A(:,2), A(1:end), A(:)  (one-based)\n",
    "Indexed writes: A(:,2)=x; A(end+1,:)=row; A(1:10,:)=[]; A(3)=7.\n",
    "Functions: zeros, ones, eye, linspace, reshape, size, numel, length,\n",
    "diag, tril, triu, flip, flipud, fliplr, cumsum, cumprod (real 2-D arrays),\n",
    "Sets: union/intersect/setdiff/setxor(A,B,order); real/logical vectors and shaped empties; sorted/stable and first indices. Other classes, nonempty matrices, rows/legacy flags unsupported.\n",
    "Membership: [Lia,Locb]=ismember(A,B); real/logical vectors and shaped empties, first B indices, NaNs never match. Nonempty matrices, flags and other classes unsupported.\n",
    "Unique: [C,ia,ic]=unique(A[,'sorted'/'stable']); real/logical 2-D, first occurrences, distinct NaNs. Rows/occurrence/legacy/missing options and other data classes unsupported.\n",
    "Row sorting: [B,I]=sortrows(A[,signedColumns][,direction][,'MissingPlacement',value]); real/logical 2-D.\n",
    "Sorting: [B,I]=sort(A[,dim][,'ascend'/'descend'][,'MissingPlacement',value]); real/logical 2-D.\n",
    "Calculus: diff(A[,n[,dim]]) uses the fixed first axis policy of MATLAB R2025a+; dim 1/2 only.\n",
    "trapz/cumtrapz(Y[,dim]) or (X,Y[,dim]): real 2-D data, scalar spacing/vector X.\n",
    "sin, cos, tan, exp, log, sqrt, abs, floor, ceil, round, sum, mean,\n",
    "Powers of two: nextpow2(X), pow2(X), pow2(F,E); real double 2-D only, one output. Binary pow2 admits equal shapes/either scalar and finite exponents truncated toward zero; other broadcasting/storage classes are unsupported.\n",
    "Near-zero functions: log1p/expm1(X); real double 2-D, one output. log1p rejects values below -1; complex and other storage classes are unsupported.\n",
    "Logarithms: log10/log2(X), floating real/complex 2-D; negative real values promote. [F,E]=log2(X) dissects real storage only; complex storage (even empty/zero imaginary) rejects.\n",
    "Statistics: std/var(A[,0|1|[][,dim|'all']][,'omitnan'|'includenan']); real f64, one output. Vector weights/vecdim not implemented.\n",
    "Median: median(A[,dim|'all'][,'omitnan'|'includenan']); real f64. Logical, weights and vecdim pending.\n",
    "Reductions: sum/mean/any/all(A[,dim]); [M,I]=min/max(A[,[],dim]) first index, NaN ignored, complex by abs then angle; min/max(A,B); [r,c,v]=find(A[,n[,'first'|'last']]). det, inv, norm, transpose, disp, plot, title, xlabel, ylabel\n",
    "Text: sprintf/fprintf(fmt,...), num2str, int2str, mat2str, strcat, strsplit, strjoin, strrep, strtrim, deblank, upper, lower, strcmp(i), strncmp(i), strfind, num2cell, iscellstr, blanks; cells and character rows only, no string arrays.\n",
    "Elementwise: mod/rem(x,y), hypot, atan2(y,x), fix, sign; equal shapes or a scalar operand. mod(x,0)=x, rem(x,0)=NaN.\n",
    "Arrays: prod(A[,dim]), dot/cross(A,B[,dim]), kron(A,B), circshift(A,n[,dim]) (dim 1 by default), trace(A), logspace(a,b[,n]), [X,Y]=meshgrid(x[,y]), [X,Y]=ndgrid(x[,y]); 2-D only.\n",
    "Linear algebra: eig(A), [V,D]=eig(A) (V,D: real symmetric or complex Hermitian A), svd(A[,'econ'|0]), [U,S,V]=svd(A), rank(A[,tol]), p=polyfit(x,y,n) (QR least squares); other complex input not yet.\n",
    "Random: rand/randn(n | m,n | [m n]), randi(imax | [lo hi], ...), rng(seed|'default'|'shuffle'); one MT19937 stream per session, rng(0) rand matches MATLAB.\n",
    "Timing: tic; toc; t=toc; id=tic; toc(id).\n",
    "Array emptiness: isempty(A); logical scalar true if any dimension is zero.\n",
    "Array shapes: isscalar/isvector/isrow/iscolumn/ismatrix(A) return logical scalars; ndims(A) returns 2 for admitted 2-D arrays.\n",
    "Containers: C=cell(1,3); C{1}=A; D=C(1); [x,y]=C{1:2}; character rows use single quotes or char(UTF16).\n",
    "Character arrays preserve 2-D UTF16 shapes; string-array storage and cell growth/multi-slot writes are pending.\n",
    "MAT persistence supports real/logical/complex arrays, character arrays and bounded nested cells.\n",
    "Commands: clear, clear A B, clear('A','B'), clc, whos, help. Clear removes current-workspace variables only; all/cache/regexp modes unsupported.\n",
    "clear()/clear variables clear current variables; a variable named variables/all takes precedence. Semicolons suppress output. % comments\n",
    "Files: save('data.mat','A','-v7.3'), load('data.mat','A'); command forms save data.mat A and load data.mat A.\n",
    "MAT save/load default to matlab.mat; save versions -v6/-v7/-v7.3; -v7 -nocompression. No output assignment.\n",
    "File commands use Current Folder; structures, ASCII, append, wildcard/regexp and string variables remain pending.\n",
    "Script paths: mfilename(), mfilename('fullpath'), fileparts(path) (folder output), fullfile(part,...).\n",
    "Named Run uses current editor text and its saved filename; path functions do not change Current Folder.\n",
    "Plots: figure(n), subplot(rows,cols,index), close, clf, plot(x,y,style), scatter, bar, stairs, stem, hold on/off,\n",
    "Figure appearance: Theme light/dark, ThemeMode auto/manual, ColorMode auto/manual; explicit Color stays fixed. Host context is captured per request (CLI Light). Legacy and manually specified series colors stay fixed; newly automatic palette colors follow the resolved figure theme.\n",
    "Plot export: saveas(gcf,'plot.png'); saveas(gcf,'plot','svg'); optional plot-export enables raster/PDF and explicit color EPS saveas(gcf,'plot','epsc').\n",
    "exportgraphics(gcf,'plot.png','Resolution',300): tight figure or live Axes export; plot-export required.\n",
    "Exportgraphics uses a 96 DPI MatRust physical canvas and 2-unit crop margin; Append/custom BackgroundColor values pending.\n",
    "Figure files: f=figure; gcf; savefig('plot.fig'); openfig('plot.fig'); .mrfig is the MatRust native extension (.mfig still works).\n",
    "legend (labels and trailing Location/value): eight compass locations, outside variants, bounded best, vertical bestoutside.\n",
    "grid on/off, xlim, ylim, axis. Up to 16 figures and 64 series.\n",
    "Plot properties: plot(x,y,'LineWidth',2,'Color','red'); trailing pairs affect new series.\n",
    "Layouts: tiledlayout(rows,cols), tiledlayout('flow'), nexttile, nexttile(index,[rows cols]), sgtitle.\n",
    "Shared title: sgtitle([figure_handle,] text, 'Interpreter', 'none'); output Text handles/properties other than Interpreter are unsupported.\n",
    "Positioned axes: subplot('Position',[left bottom width height]); optional OuterPosition and PositionConstraint ('innerposition'/'outerposition'), normalized coordinates.\n",
    "Log plots: semilogx, semilogy, loglog; xscale/yscale('linear' or 'log').\n",
    "Complex line plots: plot/semilogx/semilogy/loglog(Z) use real(Z) vs imag(Z); explicit X,Y use real parts. Other plot kinds require real coordinates.\n",
    "Scripts: if/elseif/else/end, for, while, break, continue and return.\n",
    "Structs: s.a=1; s.b.c='x'; s(2).a=5; s.(name); struct('a',{1,2}); fieldnames, isfield, isstruct, rmfield, getfield, setfield. Struct arrays are 2-D; MAT save/load of structs is pending.\n",
    "Equality: isequal(A,B,...) compares array shapes and values, including structs, cells and function handles; NaN is unequal to itself.\n",
    "Errors: try ... catch err ... end; err.message, err.identifier; error('Pkg:id','fmt %d',n); rethrow(err). Cancellation and run limits are not catchable.\n",
    "Switch: switch x, case 1, ..., case {'a','b'}, ..., otherwise, ..., end; numbers compare by value, text exactly, a cell case matches any element; no fall-through.\n",
    "Function handles: f=@sin; g=@(x,y) x+y*a (captures a when created); f(1), g(1,2), g(); func2str(g), str2func('@(x) x+1'), feval(f,x). Handles cannot be saved to MAT files yet.\n",
    "Mapping: arrayfun(f,A,...) and cellfun(f,C,...) with optional 'UniformOutput',false; cellfun('isempty'|'length'|'numel'|'ndims'|'prodofsize'|'isreal'|'islogical',C). ErrorHandler is not supported yet.\n",
    "Signal toolkit: fir1(order,Wn[,type][,window][,scaleopt]), butter_sos(order,Wn[,type]), filter(b,a,x),\n",
    "Chebyshev-I: b=cheby1(n,Rp,Wp[,type]), [b,a] or [z,p,k]; real digital parameters, order 1..20.\n",
    "cheby1_sos returns bounded SOS; prefer SOS for high orders. Analog/state-space/CTF options pending.\n",
    "Butterworth SOS: one cutoff low/high; two increasing edges bandpass(default)/stop. Band order is 2*order.\n",
    "sosfilt(sos,x), freqz(b,a,n), sosfreqz(sos,n), freqzmag(b,a,n).\n",
    "Zero-phase: filtfilt(b,a,x) or filtfilt(sos,g,x); real rows/columns, SOS (6 columns, several rows, or one row with 4th element 1 and numel(g)<=2) needs 1..64 sections; no options.\n",
    "Streaming: [y,zf]=filter(b,a,x,zi,dim). zi=[] starts from zero; dim=1 or 2.\n",
    "Initial state: filtic(b,a,y[,x]); real vectors, a(1)=1, newest-first histories, column output. Nonunit a(1), other classes and matrix histories unsupported.\n",
    "Fourier: fft/ifft(X[,n[,dim]]); real/logical/complex 2-D, dim 1 or 2, length <=1048576; symmetry flags pending.\n",
    "Text matrices: readmatrix('file.csv'); writematrix(A,'file.csv'); strict numeric comma/semicolon/tab text.\n",
    "Polynomials: polyval(p,x); finite real coefficient vector (1..4096), finite real 2-D queries; ordinary Horner, one output, no scaling/error estimates.\n",
    "Interpolation: interp1(x,v,xq[,'linear'/'nearest'[,fill]]); finite real vectors x/v, distinct x; query-shaped output. Extrapolation/implicit x/matrix samples unsupported.\n",
    "Signals: sinc(x), square(t[,duty_percent]), sawtooth(t[,width]); real 2-D arrays.\n",
    "Rates: upsample/downsample(x,n[,phase]); upfirdn(x,h[,p[,q]]); numeric 2-D, FIR finite-only.\n",
    "Chirp: chirp(t[,f0,t1,f1,method,phase,shape]); real linear/quadratic/logarithmic sweeps.\n",
    "Windows: hann/hamming/blackman(n[,'symmetric'/'periodic']); bartlett/triang/rectwin(n).\n",
    "Cosine windows: blackmanharris/nuttallwin/flattopwin(n[,'symmetric'/'periodic']); double only.\n",
    "Kaiser: kaiser(n[,beta[,'double']]); default beta 0.5, symmetric column output.\n",
    "Pass zf as the next call's zi. State vectors broadcast across channels.\n",
    "Outputs: [rows,cols]=size(A); [~,zf]=filter(b,a,x). Up to 32 targets.\n",
    "Analysis: phasez(b,a,n), grpdelay(b,a,n), impz(b,a,n), stepz(b,a,n).\n",
    "Assignment returns samples; a bare call plots. impz/stepz require n.\n",
    "SOS extensions: sosphasez, sosgrpdelay, sosimpz, sosstepz, sosfreqzmag.\n",
    "Analysis points 1..65536; undefined group delay is NaN.\n",
    "Repetition: repmat(A,n), repmat(A,[r c]) or repmat(A,r,c); real/logical/complex, bounded 2-D output.\n",
    "Shapes: reshape(A,[r c]) or reshape(A,r,c), with one [] inferred dimension; trailing ones supported.\n",
    "zeros/ones accept size vectors (including size(A)); finite negative integer sizes become zero. Two-dimensional output only.\n",
    "Writes support real numeric 1-D/2-D indices, scalar expansion, growth and literal [] deletion.\n",
    "Logical arrays: true, false, logical, double; < <= > >= == ~= & | ~ && ||.\n",
    "Select/update with A(A>0); any, all, nnz, find, isnan, isinf, isfinite.\n",
    "Loops reuse parsed code and stop cooperatively. Limit: 1000000 execution steps.\n",
    "Output is bounded to 2000 entries / 2 MiB per run; use semicolons.\n",
    "Functions: function [y,z]=name(x), local helpers and Current Folder .mr (or .m) files.\n",
    "Private function workspaces, nargin/nargout, return; at most 32 call levels.\n",
    "Complex values support i/j literals, +, -, .*, ./, matrix *, component functions, indexing and both transposes. Array operations allow equal shapes/scalars; complex solves, powers and full toolbox coverage remain pending. Dense complex-double MAT save/load supports -v6/-v7/-v7.3.\n",
    "Type help <name> for a function, for example help plot."
);

#[derive(Clone, Debug)]
pub enum Value {
    Matrix(Matrix),
    Logical(LogicalMatrix),
    Complex(ComplexMatrix),
    Text(String),
    QuotedString(String),
    Container(Array),
    Void,
}
impl Value {
    // Existing evaluation accounting uses eight-byte units. Preserve real and
    // logical charging, and charge both components of complex values.
    fn allocation_units(&self) -> Option<usize> {
        match self {
            Self::Matrix(a) => Some(a.len()),
            Self::Logical(a) => Some(a.len()),
            Self::Complex(a) => Some(a.len() * 2),
            Self::Container(a) => Some(a.bytes().div_ceil(8)),
            Self::Text(s) | Self::QuotedString(s) => Some(s.len().div_ceil(8)),
            _ => None,
        }
    }
    fn shape(&self, cancel: &AtomicBool) -> Result<(usize, usize)> {
        match self {
            Self::Matrix(a) => Ok((a.rows(), a.cols())),
            Self::Logical(a) => Ok((a.rows(), a.cols())),
            Self::Complex(a) => Ok((a.rows(), a.cols())),
            Self::Container(a) => Ok((a.rows(), a.cols())),
            Self::Text(s) => crate::CharacterRow::text_shape(s, cancel),
            _ => Err("Expected an array".into()),
        }
    }
    fn complex(self, cancel: &AtomicBool) -> Result<ComplexMatrix> {
        match self {
            Self::Complex(a) => Ok(a),
            other => {
                ComplexMatrix::from_component_views(other.view(cancel)?.as_ref(), None, cancel)
            }
        }
    }
    fn matrix(self, cancel: &AtomicBool) -> Result<Matrix> {
        match self {
            Self::Matrix(m) => Ok(m),
            Self::Logical(m) => m.to_numeric(cancel),
            Self::Container(Array::Typed(a)) => a.to_matrix(),
            Self::Complex(_) => {
                Err("This operation requires real values; use real or imag explicitly".into())
            }
            value if value.is_character() => value.character_codes(cancel),
            _ => Err("Expected a numeric or logical value".into()),
        }
    }
    fn array(self, cancel: &AtomicBool) -> Result<Array> {
        match self {
            Self::Matrix(m) => Ok(m.into()),
            Self::Logical(m) => Ok(m.into()),
            Self::Complex(m) => Ok(m.into()),
            Self::Container(a) => Ok(a),
            Self::Text(s) => Ok(crate::CharacterRow::from_text(&s, cancel)?.into()),
            _ => Err("Expected an array".into()),
        }
    }
    fn view(&self, cancel: &AtomicBool) -> Result<character_values::NumericView<'_>> {
        use crate::logical::ArrayRef;
        use character_values::NumericView;
        match self {
            Self::Matrix(m) => Ok(NumericView::Borrowed(ArrayRef::Numeric(m))),
            Self::Logical(m) => Ok(NumericView::Borrowed(ArrayRef::Logical(m))),
            Self::Complex(_) => Err("This operation does not support complex values".into()),
            value if value.is_character() => {
                Ok(NumericView::Converted(value.character_codes(cancel)?))
            }
            _ => Err("Expected an array".into()),
        }
    }
    fn logical(self, cancel: &AtomicBool) -> Result<LogicalMatrix> {
        match self {
            Self::Logical(m) => Ok(m),
            Self::Matrix(m) => LogicalMatrix::from_numeric(&m, cancel),
            Self::Container(Array::Typed(a)) => typed_values::to_logical(&a),
            value if value.is_character() => {
                LogicalMatrix::from_numeric(&value.character_codes(cancel)?, cancel)
            }
            Self::Complex(_) => Err("Complex-to-logical conversion is not supported".into()),
            _ => Err("Expected a logical or numeric value".into()),
        }
    }
}
impl From<Array> for Value {
    fn from(a: Array) -> Self {
        match a {
            Array::Numeric(m) => Self::Matrix(m),
            Array::Logical(m) => Self::Logical(m),
            Array::Complex(m) => Self::Complex(m),
            a @ (Array::Character(_)
            | Array::Cell(_)
            | Array::Graphics(_)
            | Array::Struct(_)
            | Array::Typed(_)
            | Array::Handle(_)) => Self::Container(a),
        }
    }
}
#[derive(Default, Debug, Clone)]
pub struct Execution {
    pub output: Vec<String>,
    pub clear_console: bool,
}
#[derive(Clone)]
pub struct FileDiagnostic {
    pub path: std::path::PathBuf,
    pub source: Arc<str>,
    pub diagnostic: syntax::Diagnostic,
}
#[derive(Clone)]
pub struct Engine {
    pub variables: BTreeMap<String, Matrix>,
    pub logical_variables: BTreeMap<String, LogicalMatrix>,
    pub complex_variables: BTreeMap<String, ComplexMatrix>,
    container_variables: BTreeMap<String, Array>,
    pub figure: Option<Figure>,
    other_figures: BTreeMap<u32, Figure>,
    closed_figures: std::collections::BTreeSet<u32>,
    figure_file_events: BTreeMap<u32, FigureFileEvent>,
    pub current_figure: u32,
    graphics_theme_context: crate::plot::BaseTheme,
    pub diagnostic: Option<syntax::Diagnostic>,
    pub file_diagnostic: Option<FileDiagnostic>,
    pub failed_output: Execution,
    cancel: Arc<AtomicBool>,
    evaluated_elements: usize,
    control_steps: usize,
    emitted_bytes: usize,
    emitted_entries: usize,
    /// Output of `fprintf` after its last newline; the next output continues it.
    pending_line: String,
    loop_bytes: usize,
    functions: user_functions::FunctionState,
    evaluation_depth: usize,
    statement_depth: usize,
    file_directory: Option<std::path::PathBuf>,
    document_path: Option<std::path::PathBuf>,
    /// The error in flight, for `catch` to recover its identifier and raw message.
    raised: Option<errors::RaisedError>,
    random: random::Generator,
    stopwatch: stopwatch::Stopwatch,
    fileio: file_io::FileTable,
    envx: env_files::Environment,
    lang_warnings: lang_utils::WarningState,
    lg3: lang_extra::Lg3State,
    objects: objects::ObjTable,
    /// `format short|long|compact|loose` and the console width for echo and `disp`.
    pub display: crate::display_format::DisplayFormat,
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            variables: BTreeMap::new(),
            logical_variables: BTreeMap::new(),
            complex_variables: BTreeMap::new(),
            container_variables: BTreeMap::new(),
            figure: None,
            other_figures: BTreeMap::new(),
            closed_figures: Default::default(),
            figure_file_events: BTreeMap::new(),
            current_figure: 1,
            graphics_theme_context: crate::plot::BaseTheme::Light,
            diagnostic: None,
            file_diagnostic: None,
            failed_output: Execution::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            evaluated_elements: 0,
            control_steps: 0,
            emitted_bytes: 0,
            emitted_entries: 0,
            pending_line: String::new(),
            loop_bytes: 0,
            objects: Default::default(),
            functions: Default::default(),
            evaluation_depth: 0,
            statement_depth: 0,
            file_directory: std::env::current_dir().ok(),
            document_path: None,
            raised: None,
            random: random::Generator::default(),
            stopwatch: stopwatch::Stopwatch::default(),
            fileio: Default::default(),
            envx: Default::default(),
            lang_warnings: Default::default(),
            lg3: Default::default(),
            display: Default::default(),
        }
    }
}
impl Engine {
    /// Host context captured before execution; it is not document metadata.
    pub fn set_graphics_theme_context(&mut self, theme: crate::plot::BaseTheme) {
        self.graphics_theme_context = theme;
    }
    pub fn graphics_theme_context(&self) -> crate::plot::BaseTheme {
        self.graphics_theme_context
    }
    pub fn cancellation(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }
    pub fn execute(&mut self, source: &str) -> Result<Execution> {
        self.execute_source(source, None)
    }
    /// Execute a UTF-8 byte selection using the containing document's local
    /// functions. Unselected statements are validated but never executed.
    /// Diagnostics refer to the complete document.
    pub fn execute_selection(
        &mut self,
        source: &str,
        selection: std::ops::Range<usize>,
    ) -> Result<Execution> {
        self.execute_source(source, Some(selection))
    }
    fn execute_source(
        &mut self,
        source: &str,
        selection: Option<std::ops::Range<usize>>,
    ) -> Result<Execution> {
        self.control_steps = 0;
        self.evaluated_elements = 0;
        self.emitted_bytes = 0;
        self.emitted_entries = 0;
        self.pending_line.clear();
        self.loop_bytes = 0;
        self.functions.reset_run();
        self.evaluation_depth = 0;
        self.statement_depth = 0;
        self.raised = None;
        self.closed_figures.clear();
        self.figure_file_events.clear();
        self.diagnostic = None;
        self.file_diagnostic = None;
        self.failed_output = Execution::default();
        if source.len() <= 1_000_000
            && let Some(range) = &selection
            && source.get(range.clone()).is_none()
        {
            let error = syntax::Diagnostic::new(
                source,
                range.start,
                "Selection must be a valid UTF-8 byte range within the document".into(),
            );
            let message = error.to_string();
            self.diagnostic = Some(error);
            return Err(message);
        }
        let statements = match syntax::parse_detailed(source) {
            Ok(s) => s,
            Err(error) => {
                let message = error.to_string();
                self.diagnostic = Some(error);
                return Err(message);
            }
        };
        let mut execution = Execution::default();
        let mut byte = 0;
        let offset = selection.as_ref().map_or(0, |range| range.start);
        let result = if let Some(range) = selection {
            match syntax::parse_detailed(&source[range]) {
                Ok(selected) => self.execute_selected_program(
                    source,
                    statements,
                    &selected,
                    &mut byte,
                    &mut execution,
                ),
                Err(error) => {
                    byte = error.byte;
                    Err(error.message)
                }
            }
        } else {
            self.execute_program(source, statements, &mut byte, &mut execution)
        };
        let byte = self
            .functions
            .failure
            .as_ref()
            .filter(|(program, _)| program.path.is_none())
            .map_or(offset + byte, |(_, byte)| *byte);
        self.functions.reset_run();
        self.obj_collect_at_statement(true);
        let flushed = if self.pending_line.is_empty() {
            Ok(())
        } else {
            self.emit(&mut execution, String::new())
        };
        match result.and(flushed) {
            Ok(_) => Ok(execution),
            Err(message) => {
                let error = syntax::Diagnostic::new(source, byte, message);
                let message = error.to_string();
                self.diagnostic = Some(error);
                self.failed_output = execution;
                Err(message)
            }
        }
    }
    fn execute_evaluation(
        &mut self,
        statement: &syntax::Evaluation,
        execution: &mut Execution,
    ) -> Result<()> {
        self.obj_collect_at_statement(false);
        if let Some(target) = &statement.list_target {
            return self.cel_assign_outputs(target, &statement.expr, statement.quiet, execution);
        }
        if self.obj_statement(statement, execution)? {
            return Ok(());
        }
        if (!statement.targets.is_empty() || statement.indexed_target.is_some())
            && let Expr::Call(name, _) = &statement.expr
            && matches!(name.as_str(), "tiledlayout" | "nexttile" | "sgtitle")
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            return Err("Graphics handles are unsupported; call this layout command without an output assignment".into());
        }
        if let Some((root, steps)) = &statement.path_target {
            return self.assign_path(root, steps, &statement.expr, statement.quiet, execution);
        }
        if let Some((name, args)) = &statement.indexed_target {
            if matches!(self.container_variables.get(name), Some(Array::Struct(_)))
                && !statement.cell_content_target
            {
                let steps = [syntax::PathStep::Paren(args.clone())];
                return self.assign_path(name, &steps, &statement.expr, statement.quiet, execution);
            }
            if statement.cell_content_target
                || (self.container_variables.contains_key(name)
                    && !matches!(self.container_variables.get(name), Some(Array::Typed(_))))
            {
                return self.assign_container(
                    name,
                    args,
                    statement.cell_content_target,
                    &statement.expr,
                    statement.quiet,
                    execution,
                );
            }
            self.assign_indexed(name, args, &statement.expr, statement.quiet, execution)?;
            return Ok(());
        }
        if statement.targets.is_empty() {
            let call = match &statement.expr {
                Expr::Call(name, args) => Some((name, args.as_slice())),
                Expr::Name(name) => Some((name, &[][..])),
                _ => None,
            };
            // A statement `name` or `name()` runs a folder script; local and
            // file functions keep precedence, and scripts precede builtins.
            if let Some((name, [])) = call
                && !self.contains_array(name)
                && let Some(script) = self.resolve_script(name)?
            {
                return self.run_script(script, name, execution);
            }
            if let Some((name, args)) = call
                && !self.contains_array(name)
                && let Some(function) = self.resolve_function(name)?
            {
                let values = self.function_arguments(args, None, execution)?;
                self.lng_note_inputs(args, values.len());
                let mut results = self.invoke_function(function, values, 0, None, execution)?;
                self.charge_function_outputs(&results)?;
                if let Some(value) = results.pop() {
                    self.assign_workspace_array(
                        "ans",
                        value.array(&self.cancel)?,
                        statement.quiet,
                        execution,
                    )?;
                }
                return Ok(());
            }
        }
        if statement.targets.is_empty() && statement.indexed_target.is_none() {
            if let Expr::Call(name, args) = &statement.expr
                && name == "eval"
                && !self.contains_array(name)
            {
                let values = self.function_arguments(args, None, execution)?;
                self.langx_eval(&values, false, execution)?;
                return Ok(());
            }
            let call = match &statement.expr {
                Expr::Call(name, args) => Some((name, args.as_slice())),
                Expr::Name(name) => Some((name, &[][..])),
                _ => None,
            };
            // Scripts and user functions resolved above keep precedence.
            #[cfg(target_arch = "wasm32")]
            if let Some((name, _)) = call
                && browser::file_command(name, None)
                && !self.contains_array(name)
            {
                return Err(browser::NO_FILES.into());
            }
            if let Some((name, args)) = call
                && !self.contains_array(name)
                && self.envx_statement(name, args, statement.quiet, execution)?
            {
                return Ok(());
            }
            if let Some((name, args)) = call
                && !self.contains_array(name)
                && self.fl2_statement(name, args, statement.quiet, execution)?
            {
                return Ok(());
            }
            if let Some((name, args)) = call
                && matches!(
                    name.as_str(),
                    "fprintf"
                        | "axes"
                        | "patch"
                        | "annotation"
                        | "legend"
                        | "plot"
                        | "semilogx"
                        | "semilogy"
                        | "loglog"
                        | "scatter"
                        | "bar"
                        | "stairs"
                        | "stem"
                        | "set"
                        | "title"
                        | "xlabel"
                        | "ylabel"
                )
                && !self.contains_array(name)
            {
                let values = self.function_arguments(args, None, execution)?;
                self.call(name, values, execution)?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "clear"
                && !self.contains_array(name)
            {
                return self.clear_variables_command(args);
            }
            if let Some((name, args)) = call
                && name == "run"
                && !self.contains_array(name)
            {
                let values = self.function_arguments(args, None, execution)?;
                return self.run_command(values, execution);
            }
            if let Some((name, args)) = call
                && matches!(name.as_str(), "help" | "doc")
                && !self.contains_array(name)
            {
                return self.help_command(name, args, execution);
            }
            if let Some((name, args)) = call
                && name == "format"
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                return self.format_command(args, execution);
            }
            if let Some((name, args)) = call
                && matches!(name.as_str(), "tic" | "toc")
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = self.function_arguments(args, None, execution)?;
                self.stopwatch_call(name, &values, execution, false)?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && matches!(name.as_str(), "save" | "load")
                && !self.contains_array(name)
            {
                return self.file_command(name, args, execution);
            }
            if let Some((name, args)) = call
                && name == "writematrix"
                && !self.contains_array(name)
            {
                let values = self.function_arguments(args, None, execution)?;
                return self.write_matrix_command(self.text_arguments(values)?);
            }
            if let Some((name, args)) = call
                && matches!(
                    name.as_str(),
                    "savefig" | "openfig" | "figure" | "saveas" | "exportgraphics"
                )
                && !self.contains_array(name)
            {
                let values = self.function_arguments(args, None, execution)?;
                let values = self.text_arguments(values)?;

                let values = if matches!(name.as_str(), "figure" | "exportgraphics") {
                    values
                } else {
                    self.graphics_file_arguments(values)?
                };
                if name == "figure" {
                    self.call(name, values, execution)?;
                } else if name == "exportgraphics" {
                    self.export_graphics_command(values)?;
                } else if name == "saveas" {
                    self.figure_export_command(values)?;
                } else {
                    self.figure_file_command(name, values)?;
                }
                return Ok(());
            }
        }
        if statement.targets.is_empty()
            && let Expr::Name(name) = &statement.expr
        {
            match name.as_str() {
                "hold" | "grid" | "legend" | "clf" | "figure" | "close" | "tiledlayout"
                | "nexttile" => {
                    self.graphics(name, Vec::new())?;
                    return Ok(());
                }
                "clc" => {
                    execution.clear_console = true;
                    execution.output.clear();
                    self.pending_line.clear();
                    return Ok(());
                }
                "whos" => {
                    let mut text = String::new();
                    for (n, m) in self.workspace() {
                        self.check_cancel()?;
                        let line = format!(
                            "{n:<18} {:>6} x {:<6} {:>10} bytes  {}\n",
                            m.rows(),
                            m.cols(),
                            m.bytes(),
                            if m.is_complex() {
                                "double (complex)"
                            } else {
                                lang_more::lng_class(&m)
                            }
                        );
                        if text.len() + line.len() > 2 * 1024 * 1024 {
                            return Err("Workspace display exceeds 2 MiB".into());
                        }
                        text.push_str(&line);
                    }
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.emit(execution, text)?;
                    return Ok(());
                }
                _ => {}
            }
        }
        if !statement.targets.is_empty() {
            if let [Some(name)] = statement.targets.as_slice() {
                let value = self
                    .eval_list(&statement.expr, None, execution)?
                    .into_iter()
                    .next()
                    .ok_or("Assignment expression produced no values")?;
                let array = match value {
                    value @ (Value::Matrix(_) | Value::Logical(_) | Value::Complex(_) | Value::Container(_)) => {
                        value.array(&self.cancel)?
                    }
                    value @ Value::Text(_) => value.array(&self.cancel)?,
                    Value::Void => return Err("This command does not return a value".into()),
                    Value::QuotedString(_) => return Err("String array storage is not implemented; use char(...) explicitly for character storage".into()),
                };
                return self.assign_workspace_array(name, array, statement.quiet, execution);
            }
            let outputs = self.eval_outputs(&statement.expr, &statement.targets, execution)?;
            self.assign_values(
                statement.targets.clone(),
                outputs,
                statement.quiet,
                execution,
            )?;
            return Ok(());
        }
        // A bare variable echoes under its own name and leaves `ans` alone (MATLAB).
        if let Expr::Name(name) = &statement.expr
            && let Some(array) = self.array(name)
        {
            if !statement.quiet {
                let text = match &array {
                    Array::Struct(s) => crate::struct_display::named(name, s, &self.display),
                    _ => self.display_text(name, &array)?,
                };
                self.emit(execution, text)?;
            }
            return Ok(());
        }
        self.statsx_no_output(&statement.expr)?;
        let value = match self.handle_statement(&statement.expr, execution)? {
            Some(value) => value,
            None => self.eval(&statement.expr, None, execution)?,
        };
        // MATLAB analysis functions plot only when their result is not requested.
        // Nested calls and assignments retain a numeric column vector.
        if let Expr::Call(name, _) = &statement.expr
            && !self.contains_array(name)
            && let Some((kind, _)) = crate::dsp::Analysis::from_function(name)
            && kind != crate::dsp::Analysis::Magnitude
            && let Value::Matrix(y) = &value
        {
            let x = Matrix::from_column_vec(
                y.len(),
                1,
                (0..y.len())
                    .map(|i| {
                        if kind.time_domain() {
                            i as f64
                        } else {
                            i as f64 / y.len() as f64
                        }
                    })
                    .collect(),
            )?;
            let mut figure = Figure::new(&x, y)?;
            figure.title = kind.label().into();
            figure.xlabel = if kind.time_domain() {
                "Sample"
            } else {
                "Normalized frequency (Nyquist = 1)"
            }
            .into();
            figure.ylabel = if kind.time_domain() {
                "Amplitude"
            } else {
                kind.label()
            }
            .into();
            figure.layout = self.figure.as_ref().and_then(|f| f.layout.clone());
            figure.activate_tile();

            if let Some(previous) = &self.figure {
                figure.identity = previous.identity.clone();
                figure.axes.identity = previous.axes.identity.clone();
            }
            figure.validate()?;
            self.figure = Some(figure);
            return Ok(());
        }
        match value {
            value @ (Value::Matrix(_)
            | Value::Logical(_)
            | Value::Complex(_)
            | Value::Container(_)) => {
                self.assign_workspace_array(
                    "ans",
                    value.array(&self.cancel)?,
                    statement.quiet,
                    execution,
                )?;
            }
            Value::Text(s) => {
                let a = crate::CharacterRow::from_text(&s, &self.cancel)?;
                self.assign_workspace_array("ans", a.into(), statement.quiet, execution)?;
            }
            Value::QuotedString(s) => {
                if !statement.quiet {
                    self.emit(execution, s)?;
                }
            }
            Value::Void => {}
        }
        Ok(())
    }
    fn assign_values(
        &mut self,
        targets: Vec<Option<String>>,
        outputs: Vec<Value>,
        quiet: bool,
        execution: &mut Execution,
    ) -> Result<()> {
        let mut updates = BTreeMap::new();
        let mut printed = Vec::new();
        for (target, value) in targets.into_iter().zip(outputs) {
            if let Some(name) = target {
                let array = value.array(&self.cancel)?;
                if !quiet {
                    printed.push((name.clone(), array.clone()));
                }
                updates.insert(name, array);
            }
        }
        self.merge_workspace(updates)?;
        for (name, array) in printed {
            let text = self.display_text(&name, &array)?;
            self.emit(execution, text)?;
        }
        Ok(())
    }
    pub fn contains_array(&self, name: &str) -> bool {
        self.variables.contains_key(name)
            || self.logical_variables.contains_key(name)
            || self.complex_variables.contains_key(name)
            || self.container_variables.contains_key(name)
    }
    pub fn workspace_count(&self) -> usize {
        self.variables.len()
            + self.logical_variables.len()
            + self.complex_variables.len()
            + self.container_variables.len()
    }
    pub fn remove_array(&mut self, name: &str) {
        self.variables.remove(name);
        self.logical_variables.remove(name);
        self.complex_variables.remove(name);
        self.container_variables.remove(name);
        // Assignment can still own the replacement in a Rust local. Defer
        // collection to a statement boundary or the byte-limit slow path.
    }
    pub fn clear_arrays(&mut self) {
        self.variables.clear();
        self.logical_variables.clear();
        self.complex_variables.clear();
        self.container_variables.clear();
        self.obj_collect_unowned();
    }
    pub fn workspace(&self) -> BTreeMap<String, Array> {
        self.variables
            .iter()
            .map(|(n, a)| (n.clone(), Array::Numeric(a.clone())))
            .chain(
                self.logical_variables
                    .iter()
                    .map(|(n, a)| (n.clone(), Array::Logical(a.clone()))),
            )
            .chain(
                self.complex_variables
                    .iter()
                    .map(|(n, a)| (n.clone(), Array::Complex(a.clone()))),
            )
            .chain(
                self.container_variables
                    .iter()
                    .map(|(n, a)| (n.clone(), a.clone())),
            )
            .collect()
    }
    pub fn array(&self, name: &str) -> Option<Array> {
        if let Some(value) = self.container_variables.get(name) {
            return Some(value.clone());
        }
        self.complex_variables
            .get(name)
            .cloned()
            .map(Array::Complex)
            .or_else(|| {
                self.logical_variables
                    .get(name)
                    .cloned()
                    .map(Array::Logical)
            })
            .or_else(|| self.variables.get(name).cloned().map(Array::Numeric))
            .or_else(|| self.container_variables.get(name).cloned())
    }
    pub fn workspace_bytes(&self) -> usize {
        self.obj_workspace_bytes() + self.obj_workspace_frame_bytes()
    }
    /// Current frame only; the object table is shared by all frames.
    fn obj_workspace_frame_bytes(&self) -> usize {
        self.variables
            .values()
            .map(Matrix::retained_bytes)
            .sum::<usize>()
            + self
                .logical_variables
                .values()
                .map(LogicalMatrix::len)
                .sum::<usize>()
            + self
                .complex_variables
                .values()
                .map(crate::ComplexMatrix::bytes)
                .sum::<usize>()
            + self
                .container_variables
                .values()
                .map(Array::retained_bytes)
                .sum::<usize>()
    }
    fn publish_array(&mut self, name: String, array: Array) {
        self.container_variables.remove(&name);
        match array {
            a @ (Array::Character(_)
            | Array::Cell(_)
            | Array::Graphics(_)
            | Array::Struct(_)
            | Array::Handle(_)
            | Array::Typed(_)) => {
                self.variables.remove(&name);
                self.logical_variables.remove(&name);
                self.complex_variables.remove(&name);
                self.container_variables.insert(name, a);
            }
            Array::Numeric(a) => {
                self.logical_variables.remove(&name);
                self.complex_variables.remove(&name);
                self.variables.insert(name, a);
            }
            Array::Logical(a) => {
                self.variables.remove(&name);
                self.complex_variables.remove(&name);
                self.logical_variables.insert(name, a);
            }
            Array::Complex(a) => {
                self.variables.remove(&name);
                self.logical_variables.remove(&name);
                self.complex_variables.insert(name, a);
            }
        }
    }
    pub fn merge_workspace(&mut self, mut updates: BTreeMap<String, Array>) -> Result<()> {
        for name in updates.keys() {
            Self::validate_workspace_name(name)?;
        }
        let added = updates.keys().filter(|n| !self.contains_array(n)).count();
        if self.workspace_count() + added > 10000 {
            return Err("Workspace exceeds 10000 variables; use clear".into());
        }
        let replaced = updates
            .keys()
            .map(|n| {
                self.variables.get(n).map_or(0, Matrix::retained_bytes)
                    + self.logical_variables.get(n).map_or(0, LogicalMatrix::len)
                    + self
                        .complex_variables
                        .get(n)
                        .map_or(0, crate::ComplexMatrix::bytes)
                    + self
                        .container_variables
                        .get(n)
                        .map_or(0, Array::retained_bytes)
            })
            .sum::<usize>();
        let mut used = self
            .workspace_bytes()
            .checked_sub(replaced)
            .and_then(|v| {
                updates
                    .values()
                    .try_fold(v, |n, a| n.checked_add(a.retained_bytes()))
            })
            .ok_or("Workspace byte accounting overflow")?;
        if used > 256 * 1024 * 1024 {
            let mut candidate = self.workspace();
            candidate.extend(updates);
            let mut stripping = crate::array::coordinate_retention::FactStripper::new(&self.cancel);
            for value in candidate.values_mut() {
                *value = stripping.strip(value)?;
            }
            used = candidate
                .values()
                .try_fold(self.obj_workspace_bytes(), |sum, a| {
                    sum.checked_add(a.retained_bytes())
                })
                .ok_or("Workspace byte accounting overflow")?;
            updates = candidate;
        }
        if used > 256 * 1024 * 1024 {
            return Err("Workspace exceeds its 256 MiB logical storage limit; use clear".into());
        }
        self.check_cancel()?;
        for (name, array) in updates {
            self.publish_array(name, array);
        }
        Ok(())
    }
    fn eval_outputs(
        &mut self,
        expression: &Expr,
        targets: &[Option<String>],
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let count = targets.len();
        #[cfg(target_arch = "wasm32")]
        if let Expr::Call(name, _) = expression
            && browser::file_command(name, None)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            return Err(browser::NO_FILES.into());
        }
        #[cfg(target_arch = "wasm32")]
        if let Expr::Call(name, args) = expression
            && browser::query(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let value = self.browser_query(name, &values)?.expect("browser query");
            if count > 1 {
                return Err("Too many output arguments.".into());
            }
            return Ok(vec![value]);
        }
        if let Some(values) = self.envx_eval_outputs(expression, count, out)? {
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Some(values) = self.fl2_eval_outputs(expression, count, out)? {
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Some(values) = self.lng_eval_outputs(expression, targets, out)? {
            return Ok(values);
        }
        if let Some(values) = self.lg3_outputs(expression, targets, out)? {
            return Ok(values);
        }
        if count >= 2
            && let Expr::Index(base, args) | Expr::Apply(base, args) = expression
        {
            let Value::Container(Array::Handle(handle)) = self.eval(base, None, out)? else {
                return Err("Multiple outputs require a function handle call".into());
            };
            let args = self.function_arguments(args, None, out)?;
            let values = self.call_handle(&handle, args, count, Some(targets), out)?;
            if values.len() != count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::CellContents(name, args) = expression {
            let a = self.array(name).ok_or("Unknown cell variable")?;
            let values = self.cell_contents(&a, args, out)?;
            if values.len() != targets.len() {
                return Err("Cell content output count must match targets".into());
            }
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && !self.contains_array(name)
            && let Some(function) = self.resolve_function(name)?
        {
            let values = self.function_arguments(args, None, out)?;
            self.lng_note_inputs(args, values.len());
            let values = self.invoke_function(function, values, count, Some(targets), out)?;
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && eig_more::handles(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.eigx_outputs(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Some((name, args)) = match expression {
            Expr::Call(name, args) => Some((name, args.as_slice())),
            Expr::Name(name) if name == "peaks" => Some((name, &[][..])),
            _ => None,
        } && matches!(
            name.as_str(),
            "shiftdim" | "inpolygon" | "peaks" | "hsv2rgb" | "rgb2hsv"
        ) && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.msc_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && !self.contains_array(name)
            && let Some(operation) = set_operations::operation(name)
        {
            let maximum = if name == "setdiff" { 2 } else { 3 };
            if !(1..=maximum).contains(&count) {
                return Err(format!("{name} does not support {count} outputs"));
            }
            let values = self.function_arguments(args, None, out)?;
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            let mut outputs = self.set_operation_call(operation, &values, mask)?;
            self.cel_rows_warning(&values, 2, out)?;
            outputs.truncate(count);
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && name == "ismember"
            && !self.contains_array(name)
        {
            if !(1..=2).contains(&count) {
                return Err(format!("ismember does not support {count} outputs"));
            }
            let values = self.function_arguments(args, None, out)?;
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            let mut outputs = self.ismember_call(&values, mask)?;
            self.cel_rows_warning(&values, 2, out)?;
            outputs.truncate(count);
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && name == "unique"
            && !self.contains_array(name)
        {
            if !(1..=3).contains(&count) {
                return Err(format!("unique does not support {count} outputs"));
            }
            let values = self.function_arguments(args, None, out)?;
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            let mut outputs = self.cel_unique_outputs(&values, mask, out)?;
            outputs.truncate(count);
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if count >= 2
            && let Expr::Call(name, args) = expression
            && let Some(outputs) = self.handle_outputs(name, args, targets, out)?
        {
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && matches!(name.as_str(), "min" | "max" | "find")
            && !self.contains_array(name)
        {
            let maximum = if name == "find" { 3 } else { 2 };
            if !(1..=maximum).contains(&count) {
                return Err(format!("{name} does not support {count} outputs"));
            }
            // Character data keeps its stored codes and shape (no text projection).
            let values = self.function_arguments(args, None, out)?;
            if name != "find"
                && let Some(mut outputs) = self.typed_extreme(
                    name,
                    &values,
                    count,
                    targets.get(1).is_some_and(Option::is_some),
                )?
            {
                outputs.truncate(count);
                return Ok(outputs);
            }
            let (values, class) = typed_functions::demote_arguments(values)?;
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            let mut outputs = self.reduction_call(name, &values, count, mask)?;
            outputs.truncate(count);
            if name != "find" {
                typed_functions::restore_first(&mut outputs, class)?;
            }
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && linalg_more::lax_handles(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self
                .lax_outputs(name, &values, count, out)?
                .unwrap_or_default();
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && linear_algebra2::handles(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self
                .linalg2_call_with_output(name, &values, count, out)?
                .unwrap_or_default();
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && ode::is_solver(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            return self.ode_solve(name, values, count, out);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && ode_more::has_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.od2_outputs(name, values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && quadrature::quadx_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.quadx_outputs(name, values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && sci_more::sc2_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.sc2_outputs(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && iter_solvers::slv_is_solver(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            return self.slv_outputs(name, &values, count, out);
        }
        if let Expr::Call(name, args) = expression
            && file_io::handles(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.fileio_outputs(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && arrays_more::arx_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.arx_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && stats_more::has_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.statsx_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && lang_utils::multiple_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.lang_utils_outputs(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && strings2::multiple_outputs(name)
            && !self.contains_array(name)
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.strings2_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && matrix_build::has_outputs(name)
            && !self.contains_array(name)
            && count > 1
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.matbuild_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if count == 1 {
            return match self.eval(expression, None, out)? {
                value @ (Value::Matrix(_)
                | Value::Logical(_)
                | Value::Complex(_)
                | Value::Container(_)) => Ok(vec![value]),
                value @ Value::Text(_) => Ok(vec![value]),
                Value::Void => Err("This command does not return a value".into()),
                Value::QuotedString(_) => {
                    Err("String array storage is not implemented; use char(...) explicitly".into())
                }
            };
        }
        let Expr::Call(name, args) = expression else {
            return Err("Multiple outputs require a function call".into());
        };
        if self.contains_array(name) {
            return Err("Array indexing returns one output".into());
        }
        if name == "gradient" {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.core_more_gradient(&values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if elementary::has_multiple_outputs(name) {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.everyday_outputs_call(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if trig_more::has_multiple_outputs(name) {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.trigx_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Some(outputs) = self.polyopt_outputs(name, args, count, out)? {
            return Ok(outputs);
        }
        if number_theory::handles(name) {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.numtheory_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if special_fns::handles(name) {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.special_fns_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if dates::multiple_outputs(name) {
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.datex_outputs(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if name == "log2" {
            if count != 2 {
                return Err(format!("log2 does not support {count} outputs"));
            }
            let values = self.function_arguments(args, None, out)?;
            let outputs = self
                .logarithm_dissection_call(&values, [targets[0].is_some(), targets[1].is_some()])?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if name == "cheby1" {
            if !(2..=3).contains(&count) {
                return Err("cheby1 supports one/two b,a or three z,p,k outputs; state-space/CTF forms are not implemented".into());
            }
            let values = self.function_arguments(args, None, out)?;
            let outputs = self.chebyshev_call(name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if !matches!(name.as_str(), "filter" | "size" | "sort" | "sortrows")
            || (matches!(name.as_str(), "filter" | "sort" | "sortrows") && count != 2)
        {
            if !crate::help::is_supported(name) {
                return Err(crate::help::unknown_function(name));
            }
            return Err(format!("{name} does not support {count} outputs"));
        }
        // The supported-output preflight above remains before any argument
        // effects; builtins then share the same bounded CSL expansion as calls.
        let values = self.function_arguments(args, None, out)?;
        let values = if name == "size" || values.first().is_some_and(Value::is_character) {
            values
        } else {
            self.text_arguments(values)?
        };
        if name == "size"
            && let Some(outputs) = self.obj_size_outputs(&values, count)?
        {
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if name == "sort" || name == "sortrows" {
            let (sorted, indices) = if name == "sort" {
                self.sort_call(&values, targets[1].is_some())?
            } else {
                self.sort_rows_call(&values, targets[1].is_some())?
            };
            let outputs = vec![
                sorted,
                Value::Matrix(indices.unwrap_or(Matrix::filled(0, 0, 0.)?)),
            ];
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if cplx_poly::routes(name, &values) {
            let outputs = self.cxp_call(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        let outputs = if name == "filter" {
            let (y, zf) = self.filter_call(&values, targets[1].is_some())?;
            vec![
                y,
                match zf {
                    Some(z) => z,
                    None => Matrix::filled(0, 0, 0.)?,
                },
            ]
        } else {
            if values.len() != 1 {
                return Err("Multiple-output size expects one argument".into());
            }
            let shape = values[0].shape(&self.cancel)?;
            (0..count)
                .map(|i| {
                    Matrix::scalar(match i {
                        0 => shape.0 as f64,
                        1 => shape.1 as f64,
                        _ => 1.,
                    })
                })
                .collect()
        };
        for matrix in &outputs {
            self.charge(matrix.len())?;
        }
        self.check_cancel()?;
        Ok(outputs.into_iter().map(Value::Matrix).collect())
    }
    fn charge(&mut self, elements: usize) -> Result<()> {
        self.evaluated_elements = self.evaluated_elements.saturating_add(elements);
        if self.evaluated_elements > 32 * 1024 * 1024 {
            return Err("Statement exceeds its 256 MiB evaluation budget; split the calculation into smaller steps".into());
        }
        Ok(())
    }
    fn filter_call(&self, args: &[Value], final_state: bool) -> Result<(Matrix, Option<Matrix>)> {
        if !(3..=5).contains(&args.len()) {
            return Err("filter expects 3, 4 or 5 arguments".into());
        }
        let numeric = |i: usize| -> Result<&Matrix> {
            match &args[i] {
                Value::Matrix(m) => Ok(m),
                _ => Err("filter requires numeric arguments".into()),
            }
        };
        let dimension = if args.len() == 5 {
            let dim = numeric(4)?.as_scalar()?;
            if dim != 1. && dim != 2. {
                return Err(
                    "filter dimension must be 1 or 2 for the current two-dimensional engine".into(),
                );
            }
            Some(dim as usize)
        } else {
            None
        };
        crate::dsp::filter_with_options(
            crate::dsp::vector(numeric(0)?)?,
            crate::dsp::vector(numeric(1)?)?,
            numeric(2)?,
            crate::dsp::FilterOptions {
                initial: if args.len() >= 4 {
                    Some(numeric(3)?)
                } else {
                    None
                },
                dimension,
                final_state,
            },
            &self.cancel,
        )
    }
    fn check_cancel(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err("Calculation cancelled".into())
        } else {
            Ok(())
        }
    }
    fn eval(&mut self, e: &Expr, end: Option<usize>, out: &mut Execution) -> Result<Value> {
        self.check_cancel()?;
        if self.evaluation_depth >= 128 {
            return Err("Combined expression/function evaluation exceeds 128 levels".into());
        }
        self.evaluation_depth += 1;
        let result = (|| {
            let value = self.eval_inner(e, end, out)?;
            if let Some(units) = value.allocation_units() {
                self.charge(units)?;
            }
            self.check_cancel()?;
            Ok(value)
        })();
        self.evaluation_depth -= 1;
        result
    }
    fn eval_inner(&mut self, e: &Expr, end: Option<usize>, out: &mut Execution) -> Result<Value> {
        self.check_cancel()?;
        if let Some(value) = self.obj_eval(e, out)? {
            return Ok(value);
        }
        let matrix = match e {
            Expr::Number(v) => Matrix::scalar(*v),
            Expr::Imaginary(v) => {
                return Ok(Value::Complex(ComplexMatrix::scalar(
                    crate::Complex64::new(0., *v),
                    &self.cancel,
                )?));
            }
            Expr::String(s) => return Ok(Value::Text(s.clone())),
            Expr::DoubleQuoted(s) => return Ok(Value::QuotedString(s.clone())),
            Expr::Name(n) => match n.as_str() {
                "end" => Matrix::scalar(end.ok_or("end is only valid inside indexing")? as f64),
                _ if self.container_variables.contains_key(n) => {
                    return Ok(self.container_variables[n].clone().into());
                }
                _ if self.complex_variables.contains_key(n) => {
                    return Ok(Value::Complex(self.complex_variables[n].clone()));
                }
                _ if self.logical_variables.contains_key(n) => {
                    return Ok(Value::Logical(self.logical_variables[n].clone()));
                }
                _ if self.variables.contains_key(n) => self.variables[n].clone(),
                "nargin" | "nargout" if self.functions.arguments.is_some() => {
                    self.check_function_input(n)?;
                    let (inputs, outputs) = self.functions.arguments.unwrap();
                    Matrix::scalar(if n == "nargin" { inputs } else { outputs } as f64)
                }
                _ => {
                    if let Some(function) = self.resolve_function(n)? {
                        return Ok(self
                            .invoke_function(function, Vec::new(), 1, None, out)?
                            .remove(0));
                    }
                    match n.as_str() {
                        "true" | "false" => {
                            return Ok(Value::Logical(LogicalMatrix::scalar(n == "true")));
                        }
                        "i" | "j" => {
                            return Ok(Value::Complex(ComplexMatrix::scalar(
                                crate::Complex64::new(0., 1.),
                                &self.cancel,
                            )?));
                        }
                        _ if env_files::envx_handles(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if lang_extra::lg3_handles(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        "pi" => Matrix::scalar(std::f64::consts::PI),
                        "eps" => Matrix::scalar(f64::EPSILON),
                        "Inf" | "inf" => Matrix::scalar(f64::INFINITY),
                        "NaN" | "nan" => Matrix::scalar(crate::MATLAB_NAN),
                        "gcf" | "gca" | "figure" | "tic" | "toc" | "rand" | "randn"
                        | "mfilename" | "flintmax" | "realmax" | "realmin" | "rosser"
                        | "intmax" | "intmin" | "peaks" | "lasterr" | "lastwarn" | "who"
                        | "rehash" | "bvpset" | "ddeset" => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ => {
                            return Err(self.raise(
                                crate::help::unknown_function(n),
                                "MATLAB:UndefinedFunction".into(),
                            ));
                        }
                    }
                }
            },
            Expr::All => return Err("Standalone ':' is only valid inside indexing".into()),
            Expr::Unary(negative, x) => {
                let value = self.eval(x, end, out)?;
                if let Value::Container(Array::Typed(a)) = &value {
                    return if *negative {
                        self.typed_negate(a)
                    } else {
                        Ok(value)
                    };
                }
                if let Value::Complex(a) = value {
                    return Ok(Value::Complex(if *negative {
                        a.negated(&self.cancel)?
                    } else {
                        a
                    }));
                }
                let m = value.matrix(&self.cancel)?;
                if *negative { m.map(|v| -v) } else { m }
            }
            Expr::Not(x) => {
                let value = self.eval(x, end, out)?.logical(&self.cancel)?;
                return Ok(Value::Logical(value.not(&self.cancel)?));
            }
            Expr::Transpose(x) | Expr::NonConjugateTranspose(x) => match self.eval(x, end, out)? {
                Value::Complex(v) => {
                    return Ok(Value::Complex(if matches!(e, Expr::Transpose(_)) {
                        v.adjoint(&self.cancel)?
                    } else {
                        v.transpose(&self.cancel)?
                    }));
                }
                Value::Logical(v) => return Ok(Value::Logical(v.transpose(&self.cancel)?)),
                Value::Container(Array::Typed(v)) => {
                    return Ok(Value::Container(v.transpose()?.into()));
                }
                Value::Container(a @ (Array::Cell(_) | Array::Struct(_))) => {
                    return self.cel_transpose(&a);
                }
                v if v.is_character() => {
                    let codes = v.matrix(&self.cancel)?.transpose();
                    return Ok(Value::Container(self.characters_from_codes(&codes)?.into()));
                }
                v => v.matrix(&self.cancel)?.transpose(),
            },
            Expr::Binary(op, a, b)
                if matches!(
                    op,
                    Token::Equal
                        | Token::NotEqual
                        | Token::Less
                        | Token::LessEqual
                        | Token::Greater
                        | Token::GreaterEqual
                        | Token::And
                        | Token::Or
                        | Token::ShortAnd
                        | Token::ShortOr
                ) =>
            {
                return self.logical_operator(op, a, b, end, out);
            }
            Expr::Binary(op, a, b) => {
                let a = self.eval(a, end, out)?;
                if typed_values::is_typed(&a) {
                    let b = self.eval(b, end, out)?;
                    return self.typed_arithmetic(op, a, b);
                }
                // Unsupported containers and double-quoted literals still fail
                // before right-hand side effects. Char uses the shared conversion.
                if matches!(a, Value::QuotedString(_))
                    || (matches!(a, Value::Container(_)) && !a.is_character())
                {
                    return Err("Arithmetic requires numeric or logical operands".into());
                }
                a.shape(&self.cancel)?;
                let b = self.eval(b, end, out)?;
                if typed_values::is_typed(&b) {
                    return self.typed_arithmetic(op, a, b);
                }
                if matches!(op, Token::Power | Token::DotPower)
                    && let Some(value) = self.complex_power(op == &Token::Power, &a, &b, out)?
                {
                    return Ok(value);
                }
                if matches!(a, Value::Complex(_)) || matches!(b, Value::Complex(_)) {
                    if matches!(op, Token::Slash | Token::Backslash)
                        && let Some(x) =
                            self.cxl_rect_solve(op == &Token::Backslash, &a, &b, out)?
                    {
                        return Ok(x);
                    }
                    return self.complex_arithmetic(op, a, b, out);
                }
                let a = a.matrix(&self.cancel)?;
                let b = b.matrix(&self.cancel)?;
                match op {
                    Token::Plus => a.coordinate_add(&b)?,
                    Token::Minus => a.coordinate_subtract(&b)?,
                    Token::Star => a.mul(&b).map_err(|message| {
                        if a.len() != 1 && b.len() != 1 && a.cols() != b.rows() {
                            self.raise(message, "MATLAB:innerdim".into())
                        } else {
                            message
                        }
                    })?,
                    Token::DotStar => a.coordinate_multiply(&b)?,
                    Token::DotSlash => a.zip(&b, |a, b| a / b)?,
                    Token::DotPower => a.zip(&b, f64::powf)?,
                    Token::Backslash => self.real_divide(true, &a, &b, out)?,
                    Token::Slash => self.real_divide(false, &a, &b, out)?,
                    Token::Power => {
                        let exponent = b.as_scalar()?;
                        if a.len() != 1
                            && a.rows() == a.cols()
                            && exponent < 0.
                            && exponent.fract() == 0.
                            && exponent >= -1_000_000.
                        {
                            self.real_negative_power(&a, -exponent as u64, out)?
                        } else {
                            a.power(exponent)?
                        }
                    }
                    _ => return Err("Unsupported operator".into()),
                }
            }
            Expr::Range(start, step, stop) => {
                return self.range_value(start, step.as_deref(), stop, end, out);
            }
            Expr::CellLiteral(rows) => return self.cell_literal(rows, out),
            Expr::Field(..) | Expr::DynamicField(..) => {
                return self.field_value(e, end, out);
            }
            Expr::Index(base, args) => {
                return self.obj_index_expression(base, args, end, out, false);
            }
            Expr::Brace(base, args) => {
                let base = self.eval(base, end, out)?;
                let mut values = self.brace_values(base, args, out)?;
                if values.len() != 1 {
                    return Err("Cell content expression produces a comma-separated list; use it in concatenation, arguments or matching multiple outputs".into());
                }
                return Ok(values.remove(0));
            }
            Expr::Apply(base, args) => return self.apply_value(base, args, end, out),
            Expr::FunctionHandle(name) => return self.lg3_named_handle(name),
            Expr::Anonymous(function) => return self.anonymous_handle(function, true),
            Expr::CellContents(name, args) => {
                let a = self.array(name).ok_or("Unknown cell variable")?;
                return self.index_container(&a, args, true, out);
            }
            Expr::Matrix(rows) => {
                let mut values = Vec::new();
                for row in rows {
                    let mut cells = Vec::new();
                    for e in row {
                        let list = if e.may_expand() {
                            self.eval_list(e, end, out)?
                        } else {
                            cells.push(self.eval(e, end, out)?);
                            continue;
                        };
                        if cells.len().saturating_add(list.len())
                            > crate::script_containers::MAX_CELL_SLOTS
                        {
                            return Err("Concatenation list exceeds 65536 values".into());
                        }
                        cells.extend(list);
                    }
                    values.push(cells);
                }
                return self.cel_concatenate_warn(values, out);
            }
            Expr::Call(name, args) => {
                if let Some(original) = name
                    .strip_prefix('\u{1}')
                    .and_then(|n| n.strip_suffix("_literal"))
                    && (self.contains_array(original) || self.resolve_function(original)?.is_some())
                    && let [Expr::String(digits)] = args.as_slice()
                {
                    let number = digits
                        .parse::<f64>()
                        .map_err(|_| "Invalid numeric literal")?;
                    return self.eval(
                        &Expr::Call(original.to_owned(), vec![Expr::Number(number)]),
                        end,
                        out,
                    );
                }
                if let Some(handle) = self.handle_variable(name) {
                    return self.call_handle_expression(&handle, args, out);
                }
                if let Some(a) = self.container_variables.get(name).cloned() {
                    if let Some(value) = self.lg3_object_index(&a, args, out)? {
                        return Ok(value);
                    }
                    return self.index_container(&a, args, false, out);
                }
                if let Some(m) = self.complex_variables.get(name).cloned() {
                    return Ok(Value::Complex(self.index_array(&m, args, out, false)?));
                }
                if let Some(m) = self.logical_variables.get(name).cloned() {
                    return Ok(Value::Logical(self.index_array(&m, args, out, false)?));
                }
                if let Some(m) = self.variables.get(name).cloned() {
                    self.index_array(&m, args, out, false)?
                } else {
                    if name == "clear" && self.resolve_function(name)?.is_none() {
                        return Err(
                            "clear requires a standalone statement without output assignment"
                                .into(),
                        );
                    }
                    let mut values = Vec::with_capacity(args.len());
                    for arg in args {
                        let list = if arg.may_expand() {
                            self.eval_list(arg, end, out)?
                        } else {
                            values.push(self.eval(arg, end, out)?);
                            continue;
                        };
                        if values.len().saturating_add(list.len())
                            > crate::script_containers::MAX_CELL_SLOTS
                        {
                            return Err("Argument expansion exceeds 65536 values".into());
                        }
                        values.extend(list);
                    }
                    self.lng_note_inputs(args, values.len());
                    match self.call(name, values, out)? {
                        Value::Matrix(m) => m,
                        other => return Ok(other),
                    }
                }
            }
        };
        Ok(Value::Matrix(matrix))
    }
    fn call(&mut self, name: &str, args: Vec<Value>, out: &mut Execution) -> Result<Value> {
        // The call site's argument names belong to this call only: a user
        // function entered here keeps them, a builtin (cellfun, feval, ...)
        // must not pass them on to functions it calls itself.
        let names = std::mem::take(&mut self.functions.lng.pending_names);
        if let Some(function) = self.resolve_function(name)? {
            self.functions.lng.pending_names = names;
            return Ok(self
                .invoke_function(function, args, 1, None, out)?
                .remove(0));
        }
        #[cfg(target_arch = "wasm32")]
        if browser::file_command(name, Some(&args)) {
            return Err(browser::NO_FILES.into());
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(value) = self.browser_query(name, &args)? {
            return Ok(value);
        }
        if cplx_poly::routes(name, &args) {
            return Ok(self.cxp_call(name, &args, 1, out)?.remove(0));
        }
        if let Some(value) = self.obj_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.cxr_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.cel_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.lg3_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.lax_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.arx_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.msc_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.sc2_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.fl2_call(name, &args, out)? {
            return Ok(value);
        }
        self.functions.lng.call_names = names;
        if let Some(value) = self.lng_call(name, &args, out)? {
            return Ok(value);
        }
        let eig_args = eig_more::handles(name).then(|| args.clone());
        let args = match self.typed_call(name, args, out)? {
            Ok(value) => return Ok(value),
            Err(args) => args,
        };
        if let Some(value) = self.eigx_call(name, eig_args.as_deref().unwrap_or(&args), out)? {
            return Ok(value);
        }
        if name == "run" {
            return Err("Too many output arguments: run returns no value".into());
        }
        if character_values::keeps_character_class(name)
            && args.first().is_some_and(Value::is_character)
        {
            return self.character_class_call(name, args, out);
        }
        if let Some(value) = self.complex_math_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.core_more_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.everyday_math_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.statsx_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.special_fns_call(name, &args)? {
            return Ok(value);
        }
        if name == "isequal" {
            return self.isequal_call(&args);
        }
        if matches!(name, "cheby1" | "cheby1_sos") {
            return Ok(self.chebyshev_call(name, &args, 1)?.remove(0));
        }
        if matches!(name, "nextpow2" | "pow2") {
            return self.powers_of_two_call(name, &args);
        }
        if matches!(name, "log1p" | "expm1") {
            return self.near_zero_call(name, &args);
        }
        if number_theory::handles(name) {
            return Ok(self.numtheory_outputs(name, &args, 1)?.remove(0));
        }
        if name == "filtic" {
            return self.filtic_call(&args);
        }
        if let Some(value) = self.polyopt_call(name, &args, out)? {
            return Ok(value);
        }
        if name == "polyval" {
            return self.polyval_call(&args);
        }
        if name == "interp1" {
            return self.interp1_call(&args);
        }
        if matches!(name, "fft" | "ifft") {
            return self.fourier_call(name, &args);
        }
        if let Some(operation) = set_operations::operation(name) {
            let values = self.set_operation_call(operation, &args, [true, false, false])?;
            self.cel_rows_warning(&args, 2, out)?;
            return Ok(values.into_iter().next().unwrap());
        }
        if name == "ismember" {
            let values = self.ismember_call(&args, [true, false])?;
            self.cel_rows_warning(&args, 2, out)?;
            return Ok(values.into_iter().next().unwrap());
        }
        if name == "unique" {
            return Ok(self
                .cel_unique_outputs(&args, [true, false, false], out)?
                .remove(0));
        }
        if name == "isempty" {
            return self.isempty_call(&args);
        }
        if let Some(value) = self.handle_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.shape_query_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.logarithm_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.trigx_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.envx_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.fileio_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.strings2_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.lang_utils_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.ode_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.od2_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.datex_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.quadx_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.slv_call(name, &args, out)? {
            return Ok(value);
        }
        // Path bounds apply before any generic graphics/text projection. User
        // and local functions have already resolved above and keep precedence.
        self.preflight_script_path_arguments(name, &args)?;
        if let Some(value) = self.graphics_handle_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.container_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.struct_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.error_call(name, &args)? {
            return Ok(value);
        }
        if matches!(name, "mldivide" | "mrdivide")
            && args.len() == 2
            && args.iter().any(|v| matches!(v, Value::Complex(_)))
        {
            if let Some(x) = self.cxl_rect_solve(name == "mldivide", &args[0], &args[1], out)? {
                return Ok(x);
            }
            return self.cxl_divide_call(name == "mldivide", &args, out);
        }
        if let Some(mut value) = self.linalg2_call_with_output(name, &args, 1, out)? {
            return Ok(value.remove(0));
        }
        if let Some(value) = self.script_path_call(name, &args)? {
            return Ok(value);
        }
        if strings::handles(name)
            && let Some(value) = self.text_format_call(name, args.clone(), out)?
        {
            return Ok(value);
        }
        if reductions::handles(name) {
            return Ok(self
                .reduction_call(name, &args, 1, [true, false, false])?
                .remove(0));
        }
        if let Some(value) = self.matbuild_call(name, &args)? {
            return Ok(value);
        }
        // Typed numeric consumers run before legacy text-option projection.
        // In particular, double(char) must not first decode UTF-16 into UTF-8.
        if let Some(value) = self.complex_call(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.logical_call(name, &args)? {
            return Ok(value);
        }
        let raw_args = args;
        let args = self.call_text_arguments(&raw_args)?;
        if name == "repmat" {
            return self.repmat_call(&args);
        }
        if let Some(value) = self.shape_call(name, &args)? {
            return Ok(value);
        }
        if matches!(name, "save" | "load") {
            return Err("File commands require a standalone statement without output assignment; load structure outputs are not supported yet".into());
        }
        if name == "writematrix" {
            return Err(
                "writematrix requires a standalone statement without output assignment".into(),
            );
        }
        if name == "readmatrix" {
            return self.read_matrix_command(args.into_owned());
        }
        if matches!(name, "savefig" | "saveas" | "exportgraphics") {
            return Err(format!(
                "{name} requires a standalone statement without output assignment"
            ));
        }
        if matches!(name, "openfig" | "gcf") {
            return self.figure_file_command(name, args.into_owned());
        }
        if matches!(name, "nargin" | "nargout") {
            if !args.is_empty() {
                return Err(format!(
                    "{name} currently supports only the zero-argument form inside a function"
                ));
            }
            let (inputs, outputs) = self
                .functions
                .arguments
                .ok_or("nargin/nargout require a function workspace")?;
            return Ok(Value::Matrix(Matrix::scalar(
                if name == "nargin" { inputs } else { outputs } as f64,
            )));
        }
        if matches!(
            name,
            "hold"
                | "grid"
                | "legend"
                | "xlim"
                | "ylim"
                | "xscale"
                | "yscale"
                | "axis"
                | "clf"
                | "subplot"
                | "tiledlayout"
                | "nexttile"
                | "sgtitle"
                | "figure"
                | "close"
        ) {
            return self.graphics(name, args.into_owned());
        }
        let arg_count = args.len();
        let arity = |allowed: &[usize]| -> Result<()> {
            if allowed.contains(&arg_count) {
                Ok(())
            } else {
                Err(format!(
                    "{name} expects {} arguments",
                    allowed
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(" or ")
                ))
            }
        };
        if matches!(name, "title" | "xlabel" | "ylabel") {
            arity(&[1, 3, 5])?;
            let mut color = None;
            for pair in args[1..].as_chunks::<2>().0 {
                match (&pair[0], &pair[1]) {
                    (Value::Text(key), Value::Text(value))
                        if key.eq_ignore_ascii_case("Interpreter")
                            && value.eq_ignore_ascii_case("none") => {}
                    (Value::Text(key), value) if key.eq_ignore_ascii_case("Color") => {
                        color = Some(graphics_handles::axes_rgb(value, &self.cancel)?)
                    }
                    _ => {
                        return Err(
                            "Axes text supports literal Interpreter none and RGB Color only".into(),
                        );
                    }
                }
            }
            let Value::Text(s) = &args[0] else {
                return Err("Label must be a quoted string".into());
            };
            if s.len() > 4096 {
                return Err("Plot label exceeds 4096 bytes".into());
            }
            let mut figure = self.figure.clone().unwrap_or_default();
            figure.activate_tile();
            if let Some(rgb) = color {
                let colors = figure.axes.colors.get_or_insert_with(Default::default);
                match name {
                    "title" => colors.title = Some(rgb),
                    "xlabel" => colors.xlabel = Some(rgb),
                    _ => colors.ylabel = Some(rgb),
                }
            }
            match name {
                "title" => figure.title = s.clone(),
                "xlabel" => figure.xlabel = s.clone(),
                _ => figure.ylabel = s.clone(),
            };
            figure.validate()?;
            #[cfg(test)]
            graphics_handles::tests::before_property_publish(&self.cancel);
            self.check_cancel()?;
            self.figure = Some(figure);
            return Ok(Value::Void);
        }
        if name == "disp" {
            arity(&[1])?;
            let text = match &raw_args[0] {
                // The reference prints nothing at all for `disp('')`.
                Value::Text(s) | Value::QuotedString(s) if s.is_empty() => None,
                Value::Text(s) | Value::QuotedString(s) => Some(s.clone()),
                Value::Void => Some(String::new()),
                value => {
                    let array = value.clone().array(&self.cancel)?;
                    crate::display_format::disp(&array, &self.display)?
                }
            };
            if let Some(text) = text {
                self.emit(out, text)?;
            }
            return Ok(Value::Void);
        }
        if name == "sort" {
            return self.sort_call(&args, false).map(|(sorted, _)| sorted);
        }
        if name == "sortrows" {
            return self.sort_rows_call(&args, false).map(|(sorted, _)| sorted);
        }
        if matches!(name, "upsample" | "downsample" | "upfirdn") {
            return self.rate_conversion_call(name, &args);
        }
        if name == "median" {
            return self.median_call(&args);
        }
        let args = if matches!(
            name,
            "plot" | "semilogx" | "semilogy" | "loglog" | "scatter" | "bar"
        ) {
            std::borrow::Cow::Owned(
                args.into_owned()
                    .into_iter()
                    .map(|v| match v {
                        Value::Logical(m) => m.to_numeric(&self.cancel).map(Value::Matrix),
                        v => Ok(v),
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
        } else {
            args
        };
        // Convert only arguments actually requested as numeric, once per call.
        // Text options retain their original meaning and numeric arrays stay shared.
        let numeric_slots = if raw_args.iter().any(Value::is_character) {
            raw_args.len()
        } else {
            0
        };
        let converted: Vec<std::cell::OnceCell<Result<Matrix>>> = (0..numeric_slots)
            .map(|_| std::cell::OnceCell::new())
            .collect();
        let m = |i: usize| -> Result<&Matrix> {
            match args.get(i) {
                Some(Value::Matrix(m)) => Ok(m),
                _ if raw_args.get(i).is_some_and(Value::is_character) => converted[i]
                    .get_or_init(|| raw_args[i].clone().matrix(&self.cancel))
                    .as_ref()
                    .map_err(Clone::clone),
                _ => Err(format!("{name} requires numeric arguments")),
            }
        };
        let scalar = |i: usize| -> Result<f64> { m(i)?.as_scalar() };
        let dimension = |i: usize| -> Result<usize> {
            let v = scalar(i)?;
            if !v.is_finite()
                || v < 0.0
                || v.fract() != 0.0
                || v > crate::matrix::MAX_ELEMENTS as f64
            {
                Err("Dimension must be a bounded nonnegative integer".into())
            } else {
                Ok(v as usize)
            }
        };
        let band = |i: usize, default: crate::dsp::Band| -> Result<crate::dsp::Band> {
            match args.get(i) {
                None => Ok(default),
                Some(Value::Text(s)) => crate::dsp::Band::parse(s),
                _ => Err("Filter type must be a quoted string".into()),
            }
        };
        let result = match name {
            "chirp" => {
                let mut count = args.len();
                if let Some(Value::Text(option)) = args.last() {
                    if option.eq_ignore_ascii_case("complex") {
                        return Err(
                            "chirp complex output is not supported by this real f64 kernel".into(),
                        );
                    }
                    if option.eq_ignore_ascii_case("real") {
                        count -= 1;
                    }
                }
                if !(1..=7).contains(&count) {
                    return Err("chirp accepts t, optional f0/t1/f1/method/phase/shape, and optional trailing 'real'".into());
                }
                let text_option = |index: usize| -> Result<&str> {
                    if index >= count {
                        return Ok("");
                    }
                    match &args[index] {
                        Value::Text(s) => Ok(s),
                        Value::Matrix(a) if a.is_empty() => Ok(""),
                        _ => Err("chirp method/shape must be quoted text or []".into()),
                    }
                };
                let method = crate::dsp::ChirpMethod::parse(text_option(4)?)?;
                let mut options = crate::dsp::ChirpOptions::for_method(method);
                let number_or_default = |index: usize, default: f64| -> Result<f64> {
                    if index >= count || matches!(&args[index],Value::Matrix(a) if a.is_empty()) {
                        Ok(default)
                    } else {
                        scalar(index)
                    }
                };
                options.f0 = number_or_default(1, options.f0)?;
                options.t1 = number_or_default(2, options.t1)?;
                options.f1 = number_or_default(3, options.f1)?;
                options.phase_degrees = number_or_default(5, 0.)?;
                options.shape = crate::dsp::ChirpShape::parse(text_option(6)?)?;
                crate::dsp::chirp(m(0)?, options, &self.cancel)?
            }

            "sinc" => {
                arity(&[1])?;
                crate::dsp::sinc(m(0)?, &self.cancel)?
            }
            "square" | "sawtooth" => {
                arity(&[1, 2])?;
                let parameter = if args.len() == 2 {
                    scalar(1)?
                } else if name == "square" {
                    50.
                } else {
                    1.
                };
                if name == "square" {
                    crate::dsp::square(m(0)?, parameter, &self.cancel)?
                } else {
                    crate::dsp::sawtooth(m(0)?, parameter, &self.cancel)?
                }
            }

            "fir1" => {
                arity(&[2, 3, 4, 5])?;
                let cuts = crate::dsp::vector(m(1)?)?;
                let mut kind = if cuts.len() == 2 {
                    crate::dsp::Band::Bandpass
                } else {
                    crate::dsp::Band::Lowpass
                };
                let mut index = 2;
                // Optional filter type, before the optional window and trailing scaling flag.
                if let Some(Value::Text(s)) = args.get(index)
                    && !s.eq_ignore_ascii_case("scale")
                    && !s.eq_ignore_ascii_case("noscale")
                {
                    if !s.is_empty() {
                        kind = crate::dsp::Band::parse(&s.to_ascii_lowercase())?;
                    }
                    index += 1;
                }
                let mut window = None;
                if let Some(Value::Matrix(w)) = args.get(index) {
                    if !w.is_empty() && w.rows() > 1 && w.cols() > 1 {
                        return Err("FIR window must be a row or column vector".into());
                    }
                    window = Some(w.data());
                    index += 1;
                }
                let mut scale = true;
                if let Some(option) = args.get(index) {
                    scale = match option {
                        Value::Text(s) if s.eq_ignore_ascii_case("scale") => true,
                        Value::Text(s) if s.eq_ignore_ascii_case("noscale") => false,
                        _ => return Err("Expected trailing FIR 'scale' or 'noscale' option".into()),
                    };
                    index += 1;
                }
                if index != args.len() {
                    return Err(
                        "FIR arguments must follow type, window, scale/noscale order".into(),
                    );
                }
                let coefficients = crate::dsp::fir1_with_options(
                    dimension(0)?,
                    cuts,
                    kind,
                    crate::dsp::FirOptions { window, scale },
                    &self.cancel,
                )?;
                Matrix::from_column_vec(1, coefficients.len(), coefficients)?
            }
            "butter_sos" => {
                arity(&[2, 3])?;
                let cuts = crate::dsp::vector(m(1)?)?;
                let default = if cuts.len() == 2 {
                    crate::dsp::Band::Bandpass
                } else {
                    crate::dsp::Band::Lowpass
                };
                crate::dsp::butter_sos_with_cutoffs(
                    dimension(0)?,
                    cuts,
                    band(2, default)?,
                    &self.cancel,
                )?
            }
            "filtfilt" => {
                arity(&[3])?;
                let b = m(0)?;
                let a = m(1)?;
                if b.cols() == 6
                    && (b.rows() > 1 || (b.rows() == 1 && b.data()[3] == 1. && a.len() <= 2))
                {
                    crate::dsp::filtfilt_sos(b, crate::dsp::vector(a)?, m(2)?, &self.cancel)?
                } else {
                    crate::dsp::filtfilt(
                        crate::dsp::vector(b)?,
                        crate::dsp::vector(a)?,
                        m(2)?,
                        &self.cancel,
                    )?
                }
            }
            "filter" => self.filter_call(&args, false)?.0,
            "sosfilt" => {
                arity(&[2])?;
                crate::dsp::sosfilt(m(0)?, m(1)?, &self.cancel)?
            }
            name if crate::dsp::Analysis::from_function(name).is_some() => {
                let (kind, sos) = crate::dsp::Analysis::from_function(name).unwrap();
                let index = if sos { 1 } else { 2 };
                if kind.time_domain() {
                    arity(&[index + 1])?;
                } else {
                    arity(&[index, index + 1])?;
                }
                let n = if args.len() > index {
                    dimension(index)?
                } else {
                    512
                };
                let values = if sos {
                    crate::dsp::analyze_sos(kind, m(0)?, n, &self.cancel)?
                } else {
                    crate::dsp::analyze(
                        kind,
                        crate::dsp::vector(m(0)?)?,
                        crate::dsp::vector(m(1)?)?,
                        n,
                        &self.cancel,
                    )?
                };
                Matrix::from_column_vec(n, 1, values)?
            }
            "freqz" | "sosfreqz" => {
                let is_sos = name == "sosfreqz";
                if is_sos {
                    arity(&[1, 2])?;
                } else {
                    arity(&[2, 3])?;
                }
                let count_index = if is_sos { 1 } else { 2 };
                let n = if args.len() > count_index {
                    dimension(count_index)?
                } else {
                    512
                };
                if n == 0 || n > 65536 {
                    return Err("Frequency response point count must be 1..65536".into());
                }
                let f: Vec<_> = (0..n).map(|i| i as f64 / n as f64).collect();
                let response = if is_sos {
                    crate::dsp::sos_response(m(0)?, &f, &self.cancel)?
                } else {
                    crate::dsp::response(
                        crate::dsp::vector(m(0)?)?,
                        crate::dsp::vector(m(1)?)?,
                        &f,
                        &self.cancel,
                    )?
                };
                let magnitude: Vec<_> = response.iter().map(|v| v[0].hypot(v[1])).collect();
                {
                    let db: Vec<_> = magnitude
                        .iter()
                        .map(|v| 20. * v.max(1e-15).log10())
                        .collect();
                    let mut figure = Figure::new(
                        &Matrix::from_column_slice(n, 1, &f)?,
                        &Matrix::from_column_slice(n, 1, &db)?,
                    )?;
                    figure.title = "Frequency response".into();
                    figure.xlabel = "Normalized frequency (Nyquist = 1)".into();
                    figure.ylabel = "Magnitude (dB)".into();
                    figure.layout = self.figure.as_ref().and_then(|f| f.layout.clone());
                    figure.activate_tile();

                    if let Some(previous) = &self.figure {
                        figure.identity = previous.identity.clone();
                        figure.axes.identity = previous.axes.identity.clone();
                    }
                    figure.validate()?;
                    self.figure = Some(figure);
                    return Ok(Value::Void);
                }
            }
            "kaiser" => {
                arity(&[1, 2, 3])?;
                let n = crate::dsp::window_length(scalar(0)?)?;
                let beta = match args.get(1) {
                    None => 0.5,
                    Some(Value::Matrix(a)) if a.is_empty() => 0.5,
                    _ => scalar(1)?,
                };
                if let Some(option) = args.get(2)
                    && !matches!(option,Value::Text(s) if s=="double")
                {
                    return Err("kaiser supports only real double output; single and periodic options are unsupported".into());
                }
                crate::dsp::kaiser(n, beta, &self.cancel)?
            }
            name if crate::dsp::WindowKind::from_name(name).is_some() => {
                let kind = crate::dsp::WindowKind::from_name(name).unwrap();
                arity(if kind.supports_periodic() {
                    &[1, 2]
                } else {
                    &[1]
                })?;
                let n = crate::dsp::window_length(scalar(0)?)?;
                let periodic = match args.get(1) {
                    None => false,
                    Some(Value::Text(s)) if s == "symmetric" => false,
                    Some(Value::Text(s)) if s == "periodic" => true,
                    _ => return Err("Window sampling must be 'symmetric' or 'periodic'".into()),
                };
                crate::dsp::window(kind, n, periodic, &self.cancel)?
            }
            "eye" => {
                arity(&[1, 2])?;
                let r = dimension(0)?;
                let c = if args.len() == 2 { dimension(1)? } else { r };
                Matrix::identity(r, c)?
            }
            "linspace" => {
                arity(&[2, 3])?;
                let a = scalar(0)?;
                let b = scalar(1)?;
                let n = if args.len() == 3 { dimension(2)? } else { 100 };
                let data: Vec<_> = (0..n)
                    .map(|i| {
                        if i == n - 1 {
                            b
                        } else {
                            a + (b - a) * (i as f64 / (n - 1) as f64)
                        }
                    })
                    .collect();
                Matrix::from_column_slice(1, n, &data)?
            }
            "flip" | "flipud" | "fliplr" | "cumsum" | "cumprod" => {
                arity(if matches!(name, "flipud" | "fliplr") {
                    &[1]
                } else {
                    &[1, 2]
                })?;
                let input = m(0)?;
                let dim = match name {
                    "flipud" => 1,
                    "fliplr" => 2,
                    _ if args.len() == 2 => dimension(1)?,
                    _ => input.first_nonsingleton_dimension(),
                };
                if matches!(name, "cumsum" | "cumprod") {
                    input.cumulative(dim, name == "cumprod", &self.cancel)?
                } else {
                    input.flip_dimension(dim, &self.cancel)?
                }
            }
            "diff" => {
                arity(&[1, 2, 3])?;
                let input = m(0)?;
                let order = if args.len() < 2 || m(1)?.is_empty() {
                    1
                } else {
                    dimension(1)?
                };
                let dim = if args.len() == 3 {
                    dimension(2)?
                } else {
                    input.first_nonsingleton_dimension()
                };
                input.difference(order, dim, &self.cancel)?
            }
            "trapz" | "cumtrapz" => {
                arity(&[1, 2, 3])?;
                let (x, y, dim) = match args.len() {
                    1 => (None, m(0)?, None),
                    2 if m(1)?.len() == 1 => (None, m(0)?, Some(dimension(1)?)),
                    2 => (Some(m(0)?), m(1)?, None),
                    _ => (Some(m(0)?), m(1)?, Some(dimension(2)?)),
                };
                y.trapezoid(x, dim, name == "cumtrapz", &self.cancel)?
            }
            "diag" | "tril" | "triu" => {
                arity(&[1, 2])?;
                let offset = if args.len() == 2 { scalar(1)? } else { 0. };
                if !offset.is_finite()
                    || offset.fract() != 0.
                    || offset.abs() > crate::matrix::MAX_ELEMENTS as f64
                {
                    return Err("Diagonal offset must be a bounded signed integer".into());
                }
                if name == "diag" {
                    m(0)?.diagonal(offset as i64, &self.cancel)?
                } else {
                    m(0)?.triangular(offset as i64, name == "triu", &self.cancel)?
                }
            }
            "size" => {
                arity(&[1, 2])?;
                let a = m(0)?;
                if args.len() == 2 {
                    Matrix::scalar(match dimension(1)? {
                        0 => return Err("Dimension index starts at 1".into()),
                        1 => a.rows() as f64,
                        2 => a.cols() as f64,
                        _ => 1.0,
                    })
                } else {
                    Matrix::from_column_slice(1, 2, &[a.rows() as f64, a.cols() as f64])?
                }
            }
            "numel" | "length" => {
                arity(&[1])?;
                let a = m(0)?;
                Matrix::scalar(if name == "numel" || a.is_empty() {
                    a.len()
                } else {
                    a.rows().max(a.cols())
                } as f64)
            }
            "sin" | "cos" => {
                arity(&[1])?;
                let input = m(0)?;
                if name == "sin" {
                    input.sin()
                } else {
                    input.cos()
                }
            }
            "tan" | "exp" | "log" | "sqrt" | "abs" | "floor" | "ceil" | "round" => {
                arity(&[1])?;
                let input = m(0)?;
                match name {
                    "tan" => input.tan(),
                    "exp" => input.exp(),
                    "log" => input.ln(),
                    "sqrt" => input.sqrt(),
                    "abs" => input.abs(),
                    "floor" => input.floor(),
                    "ceil" => input.ceil(),
                    _ => input.round(),
                }
            }
            "std" | "var" => {
                use crate::matrix::{Dispersion, StatisticsAxis};
                let mut count = args.len();
                let mut omit_nan = false;
                if let Some(Value::Text(flag)) = args.last() {
                    match flag.to_ascii_lowercase().as_str() {
                        "omitnan" | "omitmissing" => {
                            omit_nan = true;
                            count -= 1;
                        }
                        "includenan" | "includemissing" => {
                            count -= 1;
                        }
                        _ => {}
                    }
                }
                if !(1..=3).contains(&count) {
                    return Err(format!(
                        "Use {name}(A[, 0|1|[] [, dim|'all']][, 'omitnan'|'includenan']); vector weights and vector dimensions are not implemented"
                    ));
                }
                let input = m(0)?;
                let population = if count < 2 || m(1)?.is_empty() {
                    false
                } else {
                    if m(1)?.len() != 1 {
                        return Err("Statistics vector weights are not implemented; use scalar normalization 0 or 1".into());
                    }
                    match scalar(1)? {
                        0. => false,
                        1. => true,
                        _ => return Err("Statistics supports scalar normalization 0 or 1; vector weights are not implemented".into()),
                    }
                };
                let axis = if count < 3 {
                    StatisticsAxis::Default
                } else {
                    match &args[2] {
                        Value::Text(value) if value.eq_ignore_ascii_case("all") => StatisticsAxis::All,
                        Value::Matrix(value) if value.len() == 1 => StatisticsAxis::Dimension(dimension(2)?),
                        _ => return Err("Statistics dimension must be a positive integer or 'all'; vector dimensions are not implemented".into()),
                    }
                };
                input.dispersion(
                    if name == "std" {
                        Dispersion::StandardDeviation
                    } else {
                        Dispersion::Variance
                    },
                    population,
                    axis,
                    omit_nan,
                    &self.cancel,
                )?
            }
            "norm" => {
                if matches!(args.first(), Some(Value::Complex(_))) {
                    return self.cxl_norm_call(&args);
                }
                arity(&[1])?;
                let a = m(0)?;
                if a.rows() > 1 && a.cols() > 1 {
                    return Err("norm currently supports vectors only".into());
                }
                Matrix::scalar(a.data().iter().fold(0.0f64, |acc, &x| acc.hypot(x)))
            }
            "inv" => {
                arity(&[1])?;
                if let Some(z @ Value::Complex(_)) = args.first() {
                    return self.cxl_inv(z, out);
                }
                self.real_inverse(m(0)?, out)?
            }
            "det" => {
                arity(&[1])?;
                if let Some(z @ Value::Complex(_)) = args.first() {
                    return self.cxl_det(z);
                }
                m(0)?.det()?
            }
            "transpose" => {
                arity(&[1])?;
                m(0)?.transpose()
            }
            "plot" | "semilogx" | "semilogy" | "loglog" | "scatter" | "bar" | "stairs" | "stem" => {
                let kind = match name {
                    "scatter" => crate::plot::PlotKind::Scatter,
                    "bar" => crate::plot::PlotKind::Bar,
                    "stairs" => crate::plot::PlotKind::Stairs,
                    "stem" => crate::plot::PlotKind::Stem,
                    _ => crate::plot::PlotKind::Line,
                };
                let (data_end, properties) = plot_properties::Properties::parse(&args, kind)?;
                let coordinate_end = if kind == crate::plot::PlotKind::Scatter
                    && data_end == 3
                    && matches!(&args[2], Value::Matrix(_))
                {
                    2 // The validated positional size belongs to properties.
                } else {
                    data_end
                };
                let data_args = &args[..coordinate_end];
                if data_args.is_empty() {
                    return Err("Plot requires data".into());
                }
                let has_complex = data_args.iter().any(|v| matches!(v, Value::Complex(_)));
                if has_complex && !matches!(name, "plot" | "semilogx" | "semilogy" | "loglog") {
                    return Err(format!(
                        "{name} requires real coordinates; use real(Z) and imag(Z) explicitly"
                    ));
                }
                let data_count = data_args
                    .iter()
                    .filter(|v| matches!(v, Value::Matrix(_) | Value::Complex(_)))
                    .count();
                let coordinates = |i: usize| -> Result<crate::plot::PlotData<'_>> {
                    match &data_args[i] {
                        Value::Matrix(m) => Ok(crate::plot::PlotData::Real(m)),
                        Value::Complex(m) => Ok(crate::plot::PlotData::Complex(m)),
                        _ => Err("Plot requires numeric coordinate arrays".into()),
                    }
                };
                let mut series = Vec::new();
                let mut constructed_points = 0usize;
                let mut index = 0;
                while index < data_args.len() {
                    let paired = matches!(
                        data_args.get(index + 1),
                        Some(Value::Matrix(_) | Value::Complex(_))
                    );
                    if has_complex && data_count > 1 && !paired {
                        return Err(
                            "Complex plots need one data array or explicit X,Y pairs".into()
                        );
                    }
                    let y_index = index + usize::from(paired);
                    let mut batch = if has_complex {
                        crate::plot::series_from_plot_data_with_budget(
                            if paired {
                                Some(coordinates(index)?)
                            } else {
                                None
                            },
                            coordinates(y_index)?,
                            &self.cancel,
                            crate::plot::MAX_SERIES - series.len(),
                            crate::plot::MAX_POINTS - constructed_points,
                        )?
                    } else {
                        crate::plot::series_from_matrices(
                            if paired { Some(m(index)?) } else { None },
                            m(y_index)?,
                            kind,
                        )?
                    };
                    index += if paired { 2 } else { 1 };
                    if let Some(Value::Text(spec)) = data_args.get(index) {
                        if matches!(name, "scatter" | "bar")
                            && !(spec.len() == 1 && "rgbcmykw".contains(spec))
                        {
                            return Err("scatter/bar support a single color letter; full line specs require plot".into());
                        }
                        for s in &mut batch {
                            if name == "stem" && spec == "filled" {
                                s.marker_filled = true;
                            } else {
                                s.line_spec(spec)?;
                            }
                        }
                        index += 1;
                    }
                    if name == "stem"
                        && matches!(data_args.get(index),Some(Value::Text(s)) if s == "filled")
                    {
                        for s in &mut batch {
                            s.marker_filled = true;
                        }
                        index += 1;
                    }
                    if series.len() + batch.len() > crate::plot::MAX_SERIES
                        || series
                            .iter()
                            .map(|s: &crate::plot::Series| s.points.len())
                            .sum::<usize>()
                            + batch.iter().map(|s| s.points.len()).sum::<usize>()
                            > crate::plot::MAX_POINTS
                    {
                        return Err("Plot exceeds figure limits".into());
                    }
                    constructed_points += batch.iter().map(|s| s.points.len()).sum::<usize>();
                    properties.apply_new_batch(&mut batch, kind, &self.cancel)?;
                    series.extend(batch);
                }
                let mut figure = if self.figure.as_ref().is_some_and(|f| f.hold) {
                    self.figure.clone().unwrap()
                } else {
                    Figure {
                        identity: self
                            .figure
                            .as_ref()
                            .map_or_else(crate::ObjectIdentity::default, |f| f.identity.clone()),
                        axes: crate::plot::Axes {
                            palette_cycle: self.figure.as_ref().map_or(
                                Some(crate::plot::PaletteCycle::new(
                                    crate::plot::PaletteVersion::MatRustModern1,
                                )),
                                |f| f.axes.palette_cycle.map(crate::plot::PaletteCycle::reset),
                            ),
                            identity: self
                                .figure
                                .as_ref()
                                .map_or_else(crate::ObjectIdentity::default, |f| {
                                    f.axes.identity.clone()
                                }),
                            // Plot replaces data/style, not the user's held
                            // axes layout boundary or imported solved geometry.
                            placement: self.figure.as_ref().and_then(|f| f.axes.placement),
                            xscale: if matches!(name, "semilogx" | "loglog") {
                                crate::plot::AxisScale::Log
                            } else {
                                crate::plot::AxisScale::Linear
                            },
                            yscale: if matches!(name, "semilogy" | "loglog") {
                                crate::plot::AxisScale::Log
                            } else {
                                crate::plot::AxisScale::Linear
                            },
                            ..crate::plot::Axes::default()
                        },
                        layout: self.figure.as_ref().and_then(|f| f.layout.clone()),
                        textboxes: self
                            .figure
                            .as_ref()
                            .map_or_else(Vec::new, |f| f.textboxes.clone()),
                        name: self
                            .figure
                            .as_ref()
                            .map_or_else(String::new, |f| f.name.clone()),
                        color: self.figure.as_ref().map_or([255; 3], |f| f.color),
                        invert_hardcopy: self.figure.as_ref().and_then(|f| f.invert_hardcopy),
                        appearance: self
                            .figure
                            .as_ref()
                            .map_or(Some(crate::plot::FigureAppearance::default()), |f| {
                                f.appearance
                            }),
                    }
                };

                let first = figure.axes.series.len();
                figure.add(series)?;
                figure.validate()?;
                let handles = self.graphics_series_result(&figure, first)?;
                self.check_cancel()?;
                self.figure = Some(figure);
                return Ok(handles);
            }
            _ => {
                return Err(self.raise(
                    crate::help::unknown_function(name),
                    "MATLAB:UndefinedFunction".into(),
                ));
            }
        };
        Ok(Value::Matrix(result))
    }
}
