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
fn graphics_text_lines(
    value: &Value,
    cancel: &AtomicBool,
) -> Result<(String, Option<crate::plot::text_markup::TextSource>)> {
    let lines = match value {
        Value::Text(s) => return Ok((s.clone(), None)),
        Value::QuotedString(s) => {
            return Ok((
                s.clone(),
                Some(crate::plot::text_markup::TextSource::Strings {
                    rows: 1,
                    cols: 1,
                    values: vec![Some(s.clone())],
                }),
            ));
        }
        Value::Container(Array::Cell(c)) => {
            if c.len() > 256 {
                return Err("Text exceeds 256 lines".into());
            }
            let mut lines = Vec::new();
            let mut bytes = 0usize;
            for i in 0..c.len() {
                let item = c.get_linear(i)?;
                match item.as_ref() {
                    Array::Character(a) if a.is_text_row() => {
                        let line = a.to_utf8(cancel)?;
                        bytes = bytes.saturating_add(line.len() + 1);
                        if bytes > crate::plot::MAX_TEXT_BYTES + 1 {
                            return Err("Text exceeds 4096 bytes".into());
                        }
                        lines.push(line);
                    }
                    _ => return Err("Text cells must contain character rows".into()),
                }
            }
            lines
        }
        Value::Container(Array::Str(a))
            if a.len() <= 256
                && a.data().iter().flatten().map(|s| s.len()).sum::<usize>()
                    + a.len().saturating_sub(1)
                    <= crate::plot::MAX_TEXT_BYTES =>
        {
            a.data()
                .iter()
                .map(|s| s.as_deref().unwrap_or("").to_owned())
                .collect()
        }
        Value::Container(Array::Character(a)) if a.rows() <= 256 => {
            let mut lines = Vec::new();
            for row in 0..a.rows() {
                let units: Vec<_> = (0..a.cols())
                    .map(|col| a.units()[col * a.rows() + row])
                    .collect();
                lines.push(String::from_utf16(&units).map_err(|_| "Invalid UTF-16 text")?);
            }
            lines
        }
        _ => {
            return Err(
                "Text must be character text, a string array or cell array of character rows"
                    .into(),
            );
        }
    };
    let joined = lines.join("\n");
    if joined.len() > crate::plot::MAX_TEXT_BYTES {
        return Err("Text exceeds 4096 bytes".into());
    }
    use crate::plot::text_markup::TextSource;
    let source = match value {
        Value::Container(Array::Cell(_)) => TextSource::Cell(lines),
        Value::Container(Array::Str(a)) => TextSource::Strings {
            rows: a.rows(),
            cols: a.cols(),
            values: a
                .data()
                .iter()
                .map(|s| s.as_ref().map(|v| v.to_string()))
                .collect(),
        },
        Value::Container(Array::Character(a)) => TextSource::Character {
            rows: a.rows(),
            cols: a.cols(),
            units: a.units().to_vec(),
        },
        _ => unreachable!(),
    };
    Ok((joined, Some(source)))
}
fn graphics_text_value(
    source: &crate::plot::text_markup::TextSource,
    cancel: &AtomicBool,
) -> Result<Value> {
    use crate::plot::text_markup::TextSource;
    let array = match source {
        TextSource::Cell(lines) => {
            let mut cells = Vec::new();
            for line in lines {
                cells.push(Arc::new(
                    crate::CharacterRow::from_text(line, cancel)?.into(),
                ));
            }
            crate::CellArray::from_shared(cells.len(), 1, cells, cancel)?.into()
        }
        TextSource::Strings { rows, cols, values } => Array::Str(crate::StringArray::new(
            *rows,
            *cols,
            values
                .iter()
                .map(|s| s.as_ref().map(|v| Arc::<str>::from(v.as_str())))
                .collect(),
        )?),
        TextSource::Character { rows, cols, units } => {
            crate::CharacterRow::from_column_major(*rows, *cols, units.clone(), cancel)?.into()
        }
    };
    Ok(Value::Container(array))
}
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
pub mod app_designer_model;
pub mod app_launchers;
mod arguments;
mod array_functions;
mod arrays_more;
#[cfg(target_arch = "wasm32")]
mod browser;
mod categorical_fns;
mod cell_ops;
mod character_values;
mod chebyshev;
mod classdef;
mod clear_variables;
pub mod comm_app_hooks;
pub mod compare;
pub mod compare_builtin;
mod complex_math;
mod complex_values;
mod container_values;
mod control_design;
mod control_flow;
mod control_lti;
mod control_reduce;
mod control_response;
mod core_more;
mod cplx_linalg;
mod cplx_poly;
mod cplx_qr;
mod cplx_reduce;
mod cplx_svd;
mod curve_fit;
pub(crate) mod dates;
pub(crate) mod datetime_fns;
mod dialogs;
pub mod econ_app_hooks;
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
mod ui_components;
pub use files_more::{DicomBrowserPresentation, present_dicom_browser};
mod dicom;
#[doc(hidden)]
pub use dicom::collection::with_dicom_collection_count;
pub mod fuzzy_app_hooks;
pub mod implay;
mod limits;
pub mod mpc_app_hooks;
pub mod neural_app_hooks;
mod table_fns;
pub use limits::Limits;
mod audio;
pub mod camcal_app_hooks;
mod camera;
mod comm;
mod comm_wave;
mod econ;
mod econ_tests;
pub mod econ_vol;
mod filter_extras;
mod filtic;
mod format_command;
mod fourier;
mod fuzzy;
mod graphics_root;
mod iir;
mod import_options;
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
mod mpc;
mod nd;
mod nd_reduce;
mod nd_shape;
mod near_zero;
pub mod nnet;
mod number_theory;
mod objects;
mod ode;
mod ode_more;
mod optim_con;
mod optim_global;
mod optim_nl;
mod packages;
mod poly_interp;
mod polyval;
mod powers_of_two;
mod profiler;
mod quadrature;
pub mod random;
mod rate_conversion;
#[cfg(test)]
mod real_singular_tests;
mod reductions;
pub mod regest_app_hooks;
mod registration;
mod repmat;
mod scalar_loops;
mod sci_more;
mod script_paths;
mod shape_arguments;
mod shape_queries;
mod signal_spectral;
mod signal_timedomain;
mod sparse_fns;
mod sparse_order;
mod sparse_precond;
mod sparse_random;
mod sparse_solve;
mod special_fns;
mod spline_toolbox;
mod stopwatch;
#[cfg(test)]
mod tests;
mod trig_more;
mod video;
#[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
pub use audio::audio_output_sample;
mod volume;
pub mod volume_viewer_app_hooks;
mod xcorr;
pub use figure_files::FigureFileEvent;
mod graphics;
mod graphics_handles;
mod graphics_ticks;
mod handles;
mod help_command;
mod host_io;
mod image_basic;
mod image_color;
pub use dialogs::{DialogReply, DialogRequest, FileFilter, MessageIcon};
pub use host_io::{GraphicsFrame, HostIo};
pub use ui_components::snapshot::{
    UiEvent, UiNodeSnapshot, UiSnapshot, UiTableCell, UiWidgetSnapshot, UiWindowSnapshot,
    capture_ui_snapshot, dispatch_ui_event,
};
pub use ui_components::{GridSpan, GridTrack, UiKind, UiLayout, dispatch_ui_event_or_warn};
mod classifier_objects;
mod digital_filter;
mod dist_fit;
mod dist_objects;
mod filter_analysis;
mod image_morph;
mod image_segment;
mod indexed_assignment;
mod indexing;
mod ismember;
mod linear_models;
mod logarithms;
mod logical_values;
mod nnet_plots;
mod outliers;
mod parallel;
mod pde;
pub mod pde_app_model;
mod pde_geometry;
mod pde_mesh;
mod pde_mesher;
#[doc(hidden)]
pub use pde_mesher::{MeshOpts, TriMesh, pde_mesh_from_dl};
pub mod pde_model;
pub mod pde_modeler_app_hooks;
mod pde_plot;
mod pde_solve;
pub(crate) mod plot_properties;
mod plot_time_axes;
mod plots_2d_more;
mod plots_compose;
mod set_operations;
mod sort;
mod sort_rows;
mod stats_dist;
mod stats_models;
mod stats_plots;
mod sysid;
mod wavelet;

mod stats_gaps;
mod stats_more;
mod stats_tests;
mod stored_array;
mod string_class;
mod strings;
mod strings2;
mod structs;
mod switch_case;
pub(crate) mod text_format;
mod typed_functions;
mod typed_values;
mod unique;
mod user_functions;
mod value_equality;
mod workspace_assignment;
// jsondecode/jsonencode, readcell/writecell, xlsread and spreadsheet readmatrix.
mod readers;
pub use mpc::estimator::MpcEstimator;
pub use mpc::prediction::MpcPredictionModel;
pub use objects::{mpc_prediction_model, mpc_prediction_ss};

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
    "Statistics: std/var(A[,0|1|[][,dim|'all']][,'omitnan'|'includenan']); real f64, one output. Vector weights not implemented.\n",
    "Median: median(A[,dim|'all'][,'omitnan'|'includenan']); real f64. Weights pending.\n",
    "Reductions: sum/mean/any/all(A[,dim]); [M,I]=min/max(A[,[],dim]) first index, NaN ignored, complex by abs then angle; min/max(A,B); [r,c,v]=find(A[,n[,'first'|'last']]). det, inv, norm, transpose, disp, plot, title, xlabel, ylabel\n",
    "Text: sprintf/fprintf(fmt,...), num2str, int2str, mat2str, strcat, strsplit, strjoin, strrep, strtrim, deblank, upper, lower, strcmp(i), strncmp(i), strfind, num2cell, iscellstr, blanks; cells and character rows only, no string arrays.\n",
    "Elementwise: mod/rem(x,y), hypot, atan2(y,x), fix, sign; equal shapes or a scalar operand. mod(x,0)=x, rem(x,0)=NaN.\n",
    "Arrays: prod(A[,dim]), dot/cross(A,B[,dim]), kron(A,B), circshift(A,n[,dim]) (first non-singleton dim by default), trace(A), logspace(a,b[,n]), [X,Y]=meshgrid(x[,y]), [X,Y]=ndgrid(x[,y]); 2-D only.\n",
    "Linear algebra: eig(A), [V,D]=eig(A) (V,D: real symmetric or complex Hermitian A), svd(A[,'econ'|0]), [U,S,V]=svd(A), rank(A[,tol]), p=polyfit(x,y,n) (QR least squares); other complex input not yet.\n",
    "Random: rand/randn(n | m,n | [m n]), randi(imax | [lo hi], ...), s=rng, rng(s|seed|gen|'default'|'shuffle'); 'twister' (default, rng(0) rand matches MATLAB), 'philox', 'combRecursive'.\n",
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
    "grid on/off, xlim, ylim, axis. Up to 16 figures with bounded plot storage.\n",
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
    "Loops reuse parsed code and stop cooperatively. Browser: 1,000,000 execution steps per run. Desktop/CLI: no step limit; Stop / Ctrl+C ends a run.\n",
    "Browser output: 2,000 entries / 2 MiB per run and 2 MiB per displayed value. Desktop/CLI: no per-run output limit; 32 MiB per displayed value. Use semicolons.\n",
    "Functions: function [y,z]=name(x), local helpers and Current Folder .mr (or .m) files.\n",
    "Private function workspaces, nargin/nargout, return; at most 32 call levels.\n",
    "Complex values support i/j literals, +, -, .*, ./, matrix *, component functions, indexing and both transposes. Array operations allow equal shapes/scalars; complex solves, powers and full toolbox coverage remain pending. Dense complex-double MAT save/load supports -v6/-v7/-v7.3.\n",
    "Tables: T=table(a,b), T.a, T(rows,vars), T{rows,vars}, T.Properties, sortrows(T,vars), [T1;T2], readtable/writetable (CSV); see help table.\n",
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
fn round14_decimal_text(abs_sci: &str, n: i32) -> Option<String> {
    let (mantissa, exp_str) = abs_sci.split_once('e')?;
    let exp: i32 = exp_str.parse().ok()?;
    let mut digits: Vec<u8> = mantissa.bytes().filter(|&b| b != b'.').collect();
    if digits.is_empty() {
        return None;
    }
    let keep = i64::from(exp) + i64::from(n) + 1;
    if keep <= 0 {
        if keep == 0 && digits[0] >= b'5' {
            return Some(format!("1e{}", -i64::from(n)));
        }
        return Some("0".to_owned());
    }
    let keep = keep as usize;
    if keep >= digits.len() {
        return None;
    }
    let round_up = digits[keep] >= b'5';
    digits.truncate(keep);
    if round_up {
        let mut carry = true;
        for d in digits.iter_mut().rev() {
            if *d < b'9' {
                *d += 1;
                carry = false;
                break;
            }
            *d = b'0';
        }
        if carry {
            digits.insert(0, b'1');
        }
    }
    let s = String::from_utf8(digits).ok()?;
    Some(format!("{s}e{}", -i64::from(n)))
}

// Shared decimal and significant rounding for real, typed and complex values.
// Uses 15 significant decimal digits, then rounds half away from zero.
fn round14_digits(x: f64, n: i32) -> f64 {
    if !x.is_finite() || x == 0. {
        return x;
    }
    if (-22..=22).contains(&n) {
        let scale = 10_f64.powi(n.abs());
        let y = if n >= 0 { x * scale } else { x / scale };
        // Account for both scaling error and the 15-digit decimal representation.
        // At larger magnitudes the decimal representation can discard the digit
        // being rounded, so leave those values to the text path.
        let magnitude = y.abs();
        let margin = 1e-6 + magnitude * 1e-14;
        if magnitude < 1e14 && (magnitude.fract() - 0.5).abs() > margin {
            return if n >= 0 {
                y.round() / scale
            } else {
                y.round() * scale
            };
        }
    }
    let Some(text) = round14_decimal_text(&format!("{:.14e}", x.abs()), n) else {
        return x;
    };
    let result: f64 = text.parse().unwrap_or(x);
    if result.is_finite() {
        result.copysign(x)
    } else {
        x
    }
}

// Perform scaling in the input precision, including for significant digits.
fn round14_single(x: f32, n: i32, significant: bool) -> f32 {
    if !x.is_finite() || x == 0. {
        return x;
    }
    let digits = if significant {
        n.saturating_sub((x as f64).abs().log10().floor() as i32)
            .saturating_sub(1)
    } else {
        n
    };
    let Some(text) = round14_decimal_text(&format!("{:.6e}", x.abs()), digits) else {
        return x;
    };
    let result: f32 = text.parse().unwrap_or(x);
    if result.is_finite() {
        result.copysign(x)
    } else {
        x
    }
}

fn round14(engine: &Engine, args: &[Value]) -> Result<Value> {
    use crate::typed_array::TypedArray;
    if !(2..=3).contains(&args.len()) {
        return Err("round expects two or three inputs".into());
    }
    if !matches!(
        &args[1],
        Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_))
    ) {
        return Err("round: N must be a finite integer scalar".into());
    }
    let n = args[1].clone().matrix(&engine.cancel)?;
    if n.len() != 1 || !n.data()[0].is_finite() || n.data()[0].fract() != 0. {
        return Err("round: N must be a finite integer scalar".into());
    }
    let n = n.data()[0].clamp(-1000., 1000.) as i32;
    let significant = match args.get(2) {
        None => false,
        Some(Value::Text(s) | Value::QuotedString(s)) if s.eq_ignore_ascii_case("decimals") => {
            false
        }
        Some(Value::Text(s) | Value::QuotedString(s)) if s.eq_ignore_ascii_case("significant") => {
            true
        }
        Some(Value::Container(Array::Character(c))) => {
            let s = c.to_utf8(&engine.cancel)?;
            if s.eq_ignore_ascii_case("significant") {
                true
            } else if s.eq_ignore_ascii_case("decimals") {
                false
            } else {
                return Err("round: invalid rounding mode".into());
            }
        }
        _ => return Err("round: invalid rounding mode".into()),
    };
    if significant && n <= 0 {
        return Err("round: significant digits must be positive".into());
    }
    let f = |x: f64| {
        let digits = if significant && x.is_finite() && x != 0. {
            n.saturating_sub(x.abs().log10().floor() as i32)
                .saturating_sub(1)
        } else {
            n
        };
        round14_digits(x, digits)
    };
    let result = match &args[0] {
        Value::Container(Array::Typed(a)) => {
            if a.is_integer() && !significant && n >= 0 {
                return Ok(args[0].clone());
            }
            if a.is_integer() {
                let values = (0..a.len())
                    .take_while(|i| i % 8192 != 0 || engine.check_cancel().is_ok())
                    .map(|i| {
                        let x = a.int(i);
                        let digits = if significant && x != 0 {
                            n - (x.unsigned_abs().ilog10() as i32) - 1
                        } else {
                            n
                        };
                        if digits >= 0 {
                            x
                        } else if digits < -19 {
                            0
                        } else {
                            let scale = 10_i128.pow((-digits) as u32);
                            let q = x / scale;
                            let rem = x % scale;
                            (q + if rem.abs() * 2 >= scale {
                                x.signum()
                            } else {
                                0
                            }) * scale
                        }
                    });
                let output = TypedArray::from_ints(a.class(), a.rows(), a.cols(), values);
                engine.check_cancel()?;
                Value::Container(Array::Typed(output?))
            } else {
                let values = (0..a.len())
                    .take_while(|i| i % 8192 != 0 || engine.check_cancel().is_ok())
                    .map(|i| round14_single(a.real(i) as f32, n, significant) as f64);
                let output = TypedArray::from_reals(a.class(), a.rows(), a.cols(), values);
                engine.check_cancel()?;
                Value::Container(Array::Typed(output?))
            }
        }
        Value::Complex(z) => {
            let mut data = Vec::with_capacity(z.len());
            for chunk in z.data().chunks(8192) {
                engine.check_cancel()?;
                data.extend(
                    chunk
                        .iter()
                        .map(|v| crate::Complex64::new(f(v.re), f(v.im))),
                );
            }
            engine.narrowed(ComplexMatrix::from_column_vec(
                z.rows(),
                z.cols(),
                data,
                &engine.cancel,
            )?)?
        }
        v => {
            let m = v.clone().matrix(&engine.cancel)?;
            let mut data = Vec::with_capacity(m.len());
            for chunk in m.data().chunks(8192) {
                engine.check_cancel()?;
                data.extend(chunk.iter().map(|&x| f(x)));
            }
            Value::Matrix(Matrix::from_column_vec(m.rows(), m.cols(), data)?)
        }
    };
    result.with_dims(&args[0].dims())
}

impl Value {
    /// Logical MATLAB dimensions; folded storage remains shared.
    pub fn dims(&self) -> std::borrow::Cow<'_, [usize]> {
        match self {
            Self::Matrix(a) => a.dims(),
            Self::Logical(a) => a.dims(),
            Self::Complex(a) => a.dims(),
            Self::Container(a) => a.dims(),
            Self::Text(s) => std::borrow::Cow::Owned(vec![1, s.encode_utf16().count()]),
            Self::QuotedString(_) => std::borrow::Cow::Owned(vec![1, 1]),
            Self::Void => std::borrow::Cow::Owned(vec![0, 0]),
        }
    }
    pub fn ndims(&self) -> usize {
        self.dims().len()
    }
    pub fn is_nd(&self) -> bool {
        match self {
            Self::Matrix(a) => a.is_nd(),
            Self::Logical(a) => a.is_nd(),
            Self::Complex(a) => a.is_nd(),
            Self::Container(a) => a.is_nd(),
            _ => false,
        }
    }
    pub fn with_dims(&self, dims: &[usize]) -> Result<Self> {
        match self {
            Self::Matrix(a) => Ok(Self::Matrix(a.with_dims(dims)?)),
            Self::Logical(a) => Ok(Self::Logical(a.with_dims(dims)?)),
            Self::Complex(a) => Ok(Self::Complex(a.with_dims(dims)?)),
            Self::Container(a) => Ok(Self::Container(a.with_dims(dims)?)),
            _ => Err("This value cannot be reshaped as an array".into()),
        }
    }
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
            Self::QuotedString(_) => Ok((1, 1)),
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
            Self::Container(Array::Sparse(s)) => s.to_dense(cancel),
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
            Self::QuotedString(s) => Ok(Array::Str(crate::StringArray::scalar(s)?)),
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
            Self::Container(Array::Sparse(s)) => Ok(NumericView::Converted(s.to_dense(cancel)?)),
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
            Self::Container(Array::Sparse(s)) if s.is_logical() => s.to_dense_logical(cancel),
            Self::Container(Array::Sparse(s)) => {
                LogicalMatrix::from_numeric(&s.to_dense(cancel)?, cancel)
            }
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
            Array::Str(s) => string_class::value(s),
            a @ (Array::Character(_)
            | Array::Cell(_)
            | Array::Graphics(_)
            | Array::Struct(_)
            | Array::Typed(_)
            | Array::Handle(_)
            | Array::Categorical(_)
            | Array::Datetime(_)
            | Array::Duration(_)
            | Array::Table(_)
            | Array::Sparse(_)) => Self::Container(a),
        }
    }
}
#[derive(Default, Debug, Clone)]
pub struct Execution {
    pub output: Vec<String>,
    pub clear_console: bool,
    /// True only for a run whose host consumes completed lines immediately.
    /// This is crate-private so non-console callers cannot accidentally opt in.
    pub(crate) stream: bool,
}
impl Execution {
    /// Construct a non-streaming result for a host-side operation.
    pub fn with_output(output: Vec<String>, clear_console: bool) -> Self {
        Self {
            output,
            clear_console,
            stream: false,
        }
    }
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
    pub(in crate::engine) linked_axes: Vec<crate::plot::AxisLink>,
    closed_figures: std::collections::BTreeSet<u32>,
    figure_file_events: BTreeMap<u32, FigureFileEvent>,
    pub current_figure: u32,
    graphics_theme_context: crate::plot::BaseTheme,
    pub diagnostic: Option<syntax::Diagnostic>,
    pub file_diagnostic: Option<FileDiagnostic>,
    pub failed_output: Execution,
    cancel: Arc<AtomicBool>,
    /// Step, output and memory budgets of this host (`Limits::browser()` by default).
    limits: Limits,
    evaluated_elements: usize,
    control_steps: usize,
    emitted_bytes: usize,
    emitted_entries: usize,
    /// Output of `fprintf` after its last newline; the next output continues it.
    pending_line: String,
    loop_bytes: usize,
    functions: user_functions::FunctionState,
    nested: user_functions::nested::Store,
    evaluation_depth: usize,
    statement_depth: usize,
    file_directory: Option<std::path::PathBuf>,
    dicom_dictionary: dicom::custom_dict::Dictionary,
    document_path: Option<std::path::PathBuf>,
    /// The error in flight, for `catch` to recover its identifier and raw message.
    raised: Option<errors::RaisedError>,
    probe_mode: bool,
    probe_count: usize,
    try_depth: usize,
    probe_control_error: bool,
    probe_source: Option<Arc<str>>,
    random: random::Generator,
    stopwatch: stopwatch::Stopwatch,
    profiler: profiler::Profiler,
    fileio: file_io::FileTable,
    /// msgbox and waitbar windows (dialogs.rs).
    dialogs: dialogs::DialogState,
    /// UI component tree (`uifigure`, `uibutton`, ... in ui_components/).
    ui: ui_components::UiState,
    envx: env_files::Environment,
    lang_warnings: lang_utils::WarningState,
    lg3: lang_extra::Lg3State,
    objects: objects::ObjTable,
    #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
    audio: audio::Session,
    /// `format short|long|compact|loose` and the console width for echo and `disp`.
    pub display: crate::display_format::DisplayFormat,
    /// Answers `input` and bare `pause`; shared by clones of the engine.
    host_io: host_io::SharedHost,
    /// Status of an accepted `exit`/`quit`; shared by clones of the engine.
    exit_request: host_io::ExitRequest,
    /// Chart records of histogram, errorbar, polarplot and the other 2-D charts.
    p2: plots_2d_more::P2State,
    /// The serial parallel pool (parallel.rs).
    par: parallel::ParState,
    /// Animated-line limits (plots_compose.rs).
    compose: plots_compose::ComposeState,
    /// Rebuildable surf/mesh records (graphics_handles/surface_commands.rs).
    surf: graphics_handles::SurfaceState,
    /// Sparse-to-full limit and dispatch state (sparse_fns.rs).
    sparse: sparse_fns::SparseState,
    /// The global `dwtmode` extension mode (wavelet/).
    wavelet: wavelet::WaveletState,
    /// Root graphics object (`groot` / handle `0`) and session defaults.
    root: graphics_root::RootState,
    figure_defaults: BTreeMap<usize, graphics_root::GraphicsDefaults>,
    axes_creation_defaults: BTreeMap<u64, graphics_root::AxesCreationDefaults>,
    hidden_figures: std::collections::BTreeSet<usize>,
    ode_plot: ode::OdePlotState,
    /// Hidden target standing for a missing struct field (structs.rs); its `end` is 0.
    field_end_target: Option<String>,
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
            linked_axes: Vec::new(),
            closed_figures: Default::default(),
            figure_file_events: BTreeMap::new(),
            current_figure: 1,
            graphics_theme_context: crate::plot::BaseTheme::Light,
            diagnostic: None,
            file_diagnostic: None,
            failed_output: Execution::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            limits: Limits::browser(),
            evaluated_elements: 0,
            control_steps: 0,
            emitted_bytes: 0,
            emitted_entries: 0,
            pending_line: String::new(),
            loop_bytes: 0,
            objects: Default::default(),
            #[cfg(all(feature = "audio", not(target_arch = "wasm32")))]
            audio: Default::default(),
            functions: Default::default(),
            nested: Default::default(),
            evaluation_depth: 0,
            statement_depth: 0,
            file_directory: std::env::current_dir().ok(),
            dicom_dictionary: Default::default(),
            document_path: None,
            raised: None,
            probe_mode: std::env::var_os("MATRUST_PROBE").is_some_and(|v| v == "1"),
            probe_count: 0,
            try_depth: 0,
            probe_control_error: false,
            probe_source: None,
            random: random::Generator::default(),
            stopwatch: stopwatch::Stopwatch::default(),
            profiler: profiler::Profiler::default(),
            fileio: Default::default(),
            dialogs: Default::default(),
            ui: Default::default(),
            envx: Default::default(),
            lang_warnings: Default::default(),
            lg3: Default::default(),
            display: Default::default(),
            host_io: None,
            exit_request: Default::default(),
            p2: Default::default(),
            par: Default::default(),
            compose: Default::default(),
            surf: Default::default(),
            sparse: Default::default(),
            wavelet: Default::default(),
            root: Default::default(),
            figure_defaults: BTreeMap::new(),
            axes_creation_defaults: BTreeMap::new(),
            hidden_figures: std::collections::BTreeSet::new(),
            ode_plot: Default::default(),
            field_end_target: None,
        }
    }
}
/// Class of a struct-backed LTI model value (`tf`, `zpk`, `ss`, `pid` or
/// `pidstd`), as `class` reports it; `None` for every other value.
pub fn lti_class(array: &Array) -> Option<&'static str> {
    control_lti::class(array)
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
    pub(in crate::engine) fn clear_raised_error(&mut self) {
        self.raised = None;
    }
    /// An engine with the run limits of a host (`Limits::local()` for desktop and CLI).
    pub fn with_limits(limits: Limits) -> Self {
        let mut engine = Self::default();
        engine.limits_set(limits);
        engine
    }
    pub fn limits_set(&mut self, limits: Limits) {
        self.limits = limits;
        self.display.max_bytes = limits.display_bytes;
    }
    pub fn limits_active(&self) -> Limits {
        self.limits
    }
    /// Execute Command Window, -batch or --eval text: workspace names are
    /// variables for command syntax (`x -1` subtracts when x exists).
    pub fn execute(&mut self, source: &str) -> Result<Execution> {
        self.execute_source(source, None, false)
    }
    /// Execute a whole script's text (editor or browser Run): command syntax
    /// is decided lexically, as for a script file run by `run`, so a
    /// workspace variable named `close` does not stop `close all`.
    pub fn execute_script(&mut self, source: &str) -> Result<Execution> {
        self.execute_source(source, None, true)
    }
    /// Execute a UTF-8 byte selection using the containing document's local
    /// functions. Unselected statements are validated but never executed.
    /// Diagnostics refer to the complete document, which parses lexically;
    /// the selection runs as Command Window text.
    pub fn execute_selection(
        &mut self,
        source: &str,
        selection: std::ops::Range<usize>,
    ) -> Result<Execution> {
        self.execute_source(source, Some(selection), true)
    }
    fn execute_source(
        &mut self,
        source: &str,
        selection: Option<std::ops::Range<usize>>,
        document: bool,
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
        self.probe_count = 0;
        self.try_depth = 0;
        self.probe_control_error = false;
        self.probe_source = self.probe_mode.then(|| Arc::from(source));
        self.closed_figures.clear();
        self.figure_file_events.clear();
        self.diagnostic = None;
        self.file_diagnostic = None;
        self.failed_output = Execution::default();
        self.hostio_clear_exit();
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
        // Command Window and -batch text: workspace names are variables for
        // command syntax (`x -1` subtracts when x exists). A document is a
        // script: lexical, like `run('name')`.
        let parsed = if document {
            syntax::parse_detailed(source)
        } else {
            syntax::parse_in_workspace(source, &|n| self.contains_array(n))
        };
        let statements = match parsed {
            Ok(s) => s,
            Err(error) => {
                let message = error.to_string();
                self.diagnostic = Some(error);
                return Err(message);
            }
        };
        let mut execution = Execution {
            stream: self.hostio_streams_output(),
            ..Execution::default()
        };
        let mut byte = 0;
        let offset = selection.as_ref().map_or(0, |range| range.start);
        let result = if let Some(range) = selection {
            match syntax::parse_in_workspace(&source[range], &|n| self.contains_array(n)) {
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
        // An accepted exit/quit unwound the run through cancellation; the run
        // itself ended normally and keeps its output.
        let exiting = self.hostio_exit_unwind();
        self.obj_collect_at_statement(true);
        let flushed = if self.pending_line.is_empty() {
            Ok(())
        } else {
            self.emit(&mut execution, String::new())
        };
        match result.and(flushed) {
            Ok(_) => Ok(execution),
            Err(_) if exiting => Ok(execution),
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
        if econ_vol::statement(self, statement, execution)? {
            return Ok(());
        }
        if filter_analysis::statement(self, statement, execution)? {
            return Ok(());
        }
        if statement.targets.is_empty()
            && statement.indexed_target.is_none()
            && statement.path_target.is_none()
            && let Expr::Call(name, args) = &statement.expr
            && name == "summarize"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
            && econ::expression_method(self, name, args)?
        {
            // nargout = 0 prints the summary instead of displaying ans.
            let values = string_class::arguments(self, name, args, execution)?;
            if econ::accepts(self, name, &values) {
                econ::call(self, name, &values, 0, execution)?;
                return Ok(());
            }
        }
        if statement.targets.is_empty()
            && statement.indexed_target.is_none()
            && statement.path_target.is_none()
            && let Expr::Call(name, args) = &statement.expr
            && matches!(name.as_str(), "sim" | "review")
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, execution)?;
            if values
                .first()
                .is_some_and(|v| objects::mpc_value(self, v).is_some())
            {
                mpc::outputs(self, name, &values, 0, execution)?;
            } else {
                let value = self.call(name, values, execution)?;
                if !matches!(value, Value::Void) {
                    self.assign_workspace_array(
                        "ans",
                        value.array(&self.cancel)?,
                        statement.quiet,
                        execution,
                    )?;
                }
            }
            return Ok(());
        }
        if statement.targets.is_empty()
            && statement.indexed_target.is_none()
            && statement.path_target.is_none()
            && let Expr::Call(name, args) = &statement.expr
            && name == "web"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, execution)?;
            self.call(name, values, execution)?;
            return Ok(());
        }
        if self.pkg_statement(statement, execution)? {
            return Ok(());
        }
        if self.obj_statement(statement, execution)? {
            return Ok(());
        }
        if self.cls_statement(statement, execution)? {
            return Ok(());
        }
        if let Some((root, steps)) = &statement.path_target {
            return self.assign_path(root, steps, &statement.expr, statement.quiet, execution);
        }
        if let Some((name, args)) = &statement.indexed_target {
            if matches!(self.container_variables.get(name), Some(Array::Table(_))) {
                return self.tbl_assign_statement(
                    name,
                    args,
                    statement.cell_content_target,
                    &statement.expr,
                    statement.quiet,
                    execution,
                );
            }
            if matches!(self.container_variables.get(name), Some(Array::Struct(_)))
                && !statement.cell_content_target
            {
                let steps = [syntax::PathStep::Paren(args.clone())];
                return self.assign_path(name, &steps, &statement.expr, statement.quiet, execution);
            }
            if statement.cell_content_target
                || (self.container_variables.contains_key(name)
                    && !self.array(name).is_some_and(|a| a.is_nd())
                    && !matches!(
                        self.container_variables.get(name),
                        Some(
                            Array::Typed(_)
                                | Array::Categorical(_)
                                | Array::Datetime(_)
                                | Array::Duration(_)
                                | Array::Sparse(_)
                        )
                    ))
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
            if let Some((name, args)) = call
                && name == "warning"
                && !self.contains_array(name)
            {
                let values = string_class::arguments(self, name, args, execution)?;
                let values = match sparse_fns::call(self, name, values, execution)? {
                    Ok(_) => return Ok(()),
                    Err(values) => values,
                };
                if let Err(values) = self.typed_call(name, values, execution)? {
                    self.langx_warning(&values, false, execution)?;
                }
                return Ok(());
            }
        }
        if statement.targets.is_empty() && statement.indexed_target.is_none() {
            if let Expr::Call(name, args) = &statement.expr
                && name == "eval"
                && !self.contains_array(name)
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::eval_text(self, |engine| engine.langx_eval(&values, false, execution))?;
                return Ok(());
            }
            let call = match &statement.expr {
                Expr::Call(name, args) => Some((name, args.as_slice())),
                Expr::Name(name) => Some((name, &[][..])),
                _ => None,
            };
            // Scripts and user functions resolved above keep precedence.
            if let Some((name, args)) = call
                && !self.contains_array(name)
                && self.dlg_statement(name, args, statement.quiet, execution)?
            {
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "damp"
                && !self.contains_array(name)
            {
                let values = string_class::arguments(self, "damp", args, execution)?;
                let values = sysid::lti_args(self, values, execution)?;
                self.ctl_data("damp", &values, 0, execution)?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && spline_toolbox::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                if let Some(value) = profiler::dispatch(self, name, |engine| {
                    spline_toolbox::call(engine, name, &values, 0, execution)
                })?
                .pop()
                {
                    self.assign_workspace_array(
                        "ans",
                        value.array(&self.cancel)?,
                        statement.quiet,
                        execution,
                    )?;
                }
                return Ok(());
            }
            if let Some((name, args)) = call
                && signal_spectral::plots(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    signal_spectral::call(engine, name, &values, 0)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && pde_model::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    pde_model::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && graphics_handles::gobj_statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                if name == "alphamap" {
                    volume::show::alphamap_call(self, &values, 0)?;
                } else {
                    self.call(name, values, execution)?;
                }
                return Ok(());
            }
            if let Some((name, args)) = call
                && image_morph::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                // Images are N-D: no generic N-D argument guard.
                let values = self.function_arguments(args, None, execution)?;
                profiler::dispatch(self, name, |engine| {
                    engine.image_morph_call(name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "phasedelay"
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    filter_extras::call(engine, name, &values, 0)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "findpeaks"
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    signal_timedomain::call(engine, name, &values, 0)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && (volume::iso::handles(name) || volume::slice_viewer::handles(name))
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    if volume::slice_viewer::handles(name) {
                        volume::slice_viewer::call(engine, name, &values, 0, execution)
                    } else {
                        volume::iso::call(engine, name, &values, 0, execution)
                    }
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && graphics_handles::surface_names(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    graphics_handles::surface_call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "imshowpair"
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                // Images are N-D: no generic N-D argument guard.
                let values = self.function_arguments(args, None, execution)?;
                profiler::dispatch(self, name, |engine| {
                    image_basic::registration_call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && matches!(name.as_str(), "plotmf" | "gensurf" | "plotfis" | "writefis")
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    fuzzy::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && comm_wave::is_plot(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = self.function_arguments(args, None, execution)?;
                profiler::dispatch(self, name, |engine| {
                    comm_wave::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && (matches!(name.as_str(), "gplot" | "histfit" | "boxplot")
                    || nnet_plots::is_plot(name))
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    if nnet_plots::handles(name) {
                        nnet_plots::call(engine, name, &values, 0, execution)
                    } else {
                        stats_plots::call(engine, name, &values, 0, execution)
                    }
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "voronoi"
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = self.function_arguments(args, None, execution)?;
                profiler::dispatch(self, name, |engine| {
                    engine.sc2_outputs(name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && dist_fit::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    dist_fit::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && wavelet::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    wavelet::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && plots_2d_more::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    plots_2d_more::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && plots_compose::statement(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                profiler::dispatch(self, name, |engine| {
                    plots_compose::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && control_response::plots(name)
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                let values = sysid::lti_args(self, values, execution)?;
                if name == "margin" && self.obj_call(name, &values, execution)?.is_some() {
                    return Ok(());
                }
                profiler::dispatch(self, name, |engine| {
                    control_response::call(engine, name, &values, 0, execution)
                })?;
                return Ok(());
            }
            #[cfg(target_arch = "wasm32")]
            if let Some((name, _)) = call
                && browser::file_command(name, None)
                && !self.contains_array(name)
            {
                return Err(browser::NO_FILES.into());
            }
            if let Some((name, args)) = call
                && name == "rng"
                && !args.is_empty()
                && !self.contains_array(name)
                && self.resolve_function(name)?.is_none()
            {
                let values = string_class::arguments(self, name, args, execution)?;
                self.rng_call(&values)?;
                return Ok(());
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
                && !self.contains_array(name)
                && self.rdx_statement(name, args, statement.quiet, execution)?
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
                        | "plot3"
                        | "scatter3"
                        | "bar"
                        | "stairs"
                        | "stem"
                        | "set"
                        | "title"
                        | "xlabel"
                        | "ylabel"
                        | "zlabel"
                )
                && !self.contains_array(name)
            {
                let values = string_class::arguments(self, name, args, execution)?;
                self.call(name, values, execution)?;
                return Ok(());
            }
            if let Some((name, args)) = call
                && name == "clear"
                && !self.contains_array(name)
            {
                return self.clear_variables_command(args, execution);
            }
            if let Some((name, args)) = call
                && name == "run"
                && !self.contains_array(name)
            {
                let values = string_class::arguments(self, name, args, execution)?;
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
                let values = string_class::arguments(self, name, args, execution)?;
                if self.profiler.enabled {
                    profiler::builtin(self, name, |engine| {
                        engine.stopwatch_call(name, &values, execution, false)
                    })?;
                } else {
                    self.stopwatch_call(name, &values, execution, false)?;
                }
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
                let values = string_class::arguments(self, name, args, execution)?;
                return self.write_matrix_command(self.text_arguments(values)?);
            }
            if let Some((name, args)) = call
                && matches!(
                    name.as_str(),
                    "savefig" | "openfig" | "figure" | "saveas" | "exportgraphics"
                )
                && !self.contains_array(name)
            {
                let values = string_class::arguments(self, name, args, execution)?;
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
                "legend" => {
                    self.call(name, Vec::new(), execution)?;
                    return Ok(());
                }
                "hold" | "grid" | "clf" | "cla" | "figure" | "close" | "tiledlayout"
                | "nexttile" => {
                    self.graphics(name, Vec::new(), execution)?;
                    return Ok(());
                }
                "clc" => {
                    execution.clear_console = true;
                    execution.output.clear();
                    self.pending_line.clear();
                    return Ok(());
                }
                "whos" => {
                    let text = self.whos_workspace(None)?;
                    self.emit(execution, text)?;
                    return Ok(());
                }
                _ => {}
            }
        }
        if !statement.targets.is_empty() {
            if let [Some(name)] = statement.targets.as_slice() {
                // Real scalar arithmetic in loops: no temporaries, in-place store.
                if statement.quiet && scalar_loops::assign(self, name, &statement.expr, execution)?
                {
                    return Ok(());
                }
                // Assignment keeps shared string storage, including large scalar
                // payloads that would otherwise pass through QuotedString text.
                if let Expr::Name(source) = &statement.expr
                    && let Some(array @ Array::Str(_)) = self.array(source)
                {
                    self.charge(array.bytes().div_ceil(8))?;
                    return self.assign_workspace_array(name, array, statement.quiet, execution);
                }
                let value = self
                    .eval_list(&statement.expr, None, execution)?
                    .into_iter()
                    .next()
                    .ok_or("Assignment expression produced no values")?;
                let array = match value {
                    value @ (Value::Matrix(_)
                    | Value::Logical(_)
                    | Value::Complex(_)
                    | Value::Container(_)) => value.array(&self.cancel)?,
                    value @ Value::Text(_) => value.array(&self.cancel)?,
                    Value::Void => return Err("This command does not return a value".into()),
                    value @ Value::QuotedString(_) => value.array(&self.cancel)?,
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
                    Array::Struct(s)
                        if control_lti::class(&array).is_none()
                            && sysid::class(&array).is_none() =>
                    {
                        crate::struct_display::named(name, s, &self.display)
                    }
                    _ => self.display_text(name, &array)?,
                };
                self.emit(execution, text)?;
            }
            return Ok(());
        }
        if let Expr::Call(name, args) = &statement.expr
            && name == "summary"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, execution)?;
            self.tbl_outputs(name, &values, &[], 0, execution)?;
            return Ok(());
        }
        if let Expr::Call(name, args) = &statement.expr
            && name == "rlocus"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, execution)?;
            control_design::cds_call(self, name, &values, 0, execution)?;
            return Ok(());
        }
        if let Expr::Call(name, args) = &statement.expr
            && econ_tests::is_correlogram(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, execution)?;
            econ_tests::call(self, name, &values, 0, execution)?;
            return Ok(());
        }
        if let Expr::Call(name, args) = &statement.expr
            && matches!(name.as_str(), "subplot" | "tiledlayout" | "nexttile")
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, execution)?;
            self.call(name, values, execution)?;
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
            value @ Value::QuotedString(_) => {
                self.assign_workspace_array(
                    "ans",
                    value.array(&self.cancel)?,
                    statement.quiet,
                    execution,
                )?;
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
            if classdef::is_object(&array) {
                self.cls_echo(&name, &array, execution)?;
                continue;
            }
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
    /// Adds or replaces one workspace variable under the workspace's name,
    /// count and byte limits (`merge_workspace`).
    pub fn insert_array(&mut self, name: &str, array: Array) -> Result<()> {
        self.merge_workspace(BTreeMap::from([(name.to_owned(), array)]))
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
            .filter(|(n, _)| !sparse_fns::is_hidden_name(n))
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
        self.obj_workspace_bytes()
            + self.obj_workspace_frame_bytes()
            + user_functions::nested::caller_bytes(self)
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
                if matches!(a, Array::Sparse(_)) {
                    self.sparse.seen = true;
                }
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
        if used > limits::memory_bytes(&self.limits) {
            self.nested.sweep_unowned();
            let mut candidate = self.workspace();
            candidate.extend(updates);
            let mut stripping = crate::array::coordinate_retention::FactStripper::new(&self.cancel);
            for value in candidate.values_mut() {
                *value = stripping.strip(value)?;
            }
            used = candidate
                .values()
                .try_fold(
                    self.obj_workspace_bytes() + user_functions::nested::caller_bytes(self),
                    |sum, a| sum.checked_add(a.retained_bytes()),
                )
                .ok_or("Workspace byte accounting overflow")?;
            updates = candidate;
        }
        if used > limits::memory_bytes(&self.limits) {
            return Err(limits::workspace_error(&self.limits));
        }
        self.check_cancel()?;
        for (name, array) in updates {
            // A loaded cell or struct may hold sparse values the multi-output hook must see.
            if matches!(array, Array::Cell(_) | Array::Struct(_)) {
                self.sparse.seen = true;
            }
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
        let bare_call;
        let expression = if count > 1
            && let Expr::Name(name) = expression
        {
            if self.contains_array(name) {
                return Err("Too many output arguments.".into());
            }
            bare_call = Expr::Call(name.clone(), Vec::new());
            &bare_call
        } else {
            expression
        };
        if let Some(values) = sparse_fns::outputs(self, expression, targets, out)? {
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if count > 1
            && let Expr::Call(name, _) = expression
            && matches!(name.as_str(), "profile" | "profsave")
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            return Err(format!("{name}: Too many output arguments."));
        }
        if let Expr::Call(name, args) = expression
            && name == "sim"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            if values
                .first()
                .is_some_and(|v| objects::mpc_value(self, v).is_some())
            {
                let outputs = mpc::outputs(self, name, &values, count, out)?;
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            if count > 1
                && values
                    .first()
                    .is_some_and(|v| objects::nnet_value(self, v).is_some())
            {
                return nnet::call(self, name, &values, count);
            }
            let outputs = if count <= 1 {
                vec![self.call(name, values, out)?]
            } else {
                sysid::call(self, name, &values, count, out)?
                    .ok_or("sim: unsupported first argument or output count")?
            };
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && sysid::owns(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                sysid::call(engine, name, &values, count, out)
            })?
            .unwrap_or_default();
            if values.len() < count {
                return Err(format!("{name}: too many output arguments"));
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && pde::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let args = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                pde::call(engine, name, &args, count, out)
            })?;
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && pde_geometry::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let args = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                pde_geometry::call(engine, name, &args, count, out)
            })?;
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && pde_model::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let args = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                pde_model::call(engine, name, &args, count, out)
            })?;
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && let Some(Expr::Name(var)) = args.first()
            && name == "tf"
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
            && let Some(f) = self.filter_var(var)
        {
            if args.len() > 1 || count > 2 {
                return Err("tf: use [b, a] = tf(d) on a digitalFilter".into());
            }
            let values = digital_filter::expand_args(&f, &[])?;
            self.charge_function_outputs(&values)?;
            return Ok(values.into_iter().take(count).collect());
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && linear_models::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                linear_models::call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Some(values) = self.pkg_outputs(expression, targets, out)? {
            return Ok(values);
        }
        if let Some(values) = self.cls_outputs(expression, targets, out)? {
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && registration::multi_output(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                registration::call(engine, name, &values, count, out)
            })?
            .ok_or_else(|| format!("{name} does not support {count} outputs"))?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && camera::multi_output(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                camera::call(engine, name, &values, count, out)
            })?
            .ok_or_else(|| format!("{name} does not support {count} outputs"))?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        // `[a, b] = object.method(...)` on a camera object.
        if let Expr::Index(base, args) = expression
            && count > 1
            && let Expr::Field(inner, member) = base.as_ref()
            && camera::is_method(member)
            && let Expr::Name(var) = inner.as_ref()
            && let Some(object) = self.array(var)
            && camera::camera_of(self, &Value::from(object.clone())).is_some()
        {
            let mut values = vec![Value::from(object)];
            values.extend(self.function_arguments(args, None, out)?);
            let values = camera::call(self, member, &values, count, out)?
                .ok_or_else(|| format!("{member} does not support {count} outputs"))?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        // `[a, b] = object.method(...)` on a registration object.
        if let Expr::Index(base, args) = expression
            && count > 1
            && let Expr::Field(inner, member) = base.as_ref()
            && (registration::multi_output(member) || member == "contains")
            && let Expr::Name(var) = inner.as_ref()
            && let Some(object) = self.array(var)
            && registration::geo_of(self, &Value::from(object.clone())).is_some()
        {
            let mut values = vec![Value::from(object)];
            values.extend(self.function_arguments(args, None, out)?);
            let values = registration::call(self, member, &values, count, out)?
                .ok_or_else(|| format!("{member} does not support {count} outputs"))?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && curve_fit::multi_output(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                curve_fit::call(engine, name, &values, count, out)
            })?
            .ok_or_else(|| format!("{name} does not support {count} outputs"))?;
            if values.len() < count {
                return Err(format!("{name} does not support {count} outputs"));
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && control_design::cds_handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                control_design::cds_call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && control_reduce::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                control_reduce::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && (volume::iso::handles(name) || volume::slice_viewer::handles(name))
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                if volume::slice_viewer::handles(name) {
                    volume::slice_viewer::call(engine, name, &values, count, out)
                } else {
                    volume::iso::call(engine, name, &values, count, out)
                }
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && graphics_handles::surface_names(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            if name == "sphere" {
                let outputs = profiler::dispatch(self, name, |engine| {
                    graphics_handles::sphere_coordinates(engine, &values, count)
                })?;
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            return profiler::dispatch(self, name, |engine| {
                graphics_handles::surface_call(engine, name, &values, count, out)
            })
            .map(|v| vec![v]);
        }
        if let Expr::Call(name, args) = expression
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
            && econ::expression_method(self, name, args)?
        {
            let values = string_class::arguments(self, name, args, out)?;
            if econ::accepts(self, name, &values) {
                return econ::call(self, name, &values, count, out);
            }
        }
        if let Expr::Call(name, args) = expression
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
            && (econ_vol::handles(name) || econ_vol::expression_method(self, name, args)?)
        {
            let values = string_class::arguments(self, name, args, out)?;
            if econ_vol::handles(name) || econ_vol::method(self, name, &values) {
                return profiler::dispatch(self, name, |engine| {
                    econ_vol::call(engine, name, &values, count, out)
                });
            }
        }
        if let Expr::Call(name, args) = expression
            && econ_tests::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                econ_tests::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && comm::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                comm::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && fuzzy::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                fuzzy::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && comm_wave::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            return profiler::dispatch(self, name, |engine| {
                comm_wave::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && (stats_plots::handles(name) || nnet_plots::handles(name))
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                if nnet_plots::handles(name) {
                    nnet_plots::call(engine, name, &values, count, out)
                } else {
                    stats_plots::call(engine, name, &values, count, out)
                }
            })?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && dist_fit::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                dist_fit::call(engine, name, &values, count, out)
            })?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && nnet::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return nnet::call(self, name, &values, count);
        }
        if let Expr::Call(name, args) = expression
            && wavelet::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                wavelet::call(engine, name, &values, count, out)
            })?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && wavelet::method_call(self, name, args)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                wavelet::call(engine, name, &values, count, out)
            })?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && parallel::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = profiler::dispatch(self, name, |engine| {
                parallel::call(engine, name, &values, count, out)
            })?;
            if values.len() < count {
                return Err("Too many output arguments.".into());
            }
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && plots_2d_more::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                plots_2d_more::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && plots_compose::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                plots_compose::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, _) | Expr::Name(name) = expression
            && name == "view"
            && count >= 2
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = match expression {
                Expr::Call(_, args) => string_class::arguments(self, name, args, out)?,
                _ => Vec::new(),
            };
            let mut view = self.call(name, values, out)?;
            if !matches!(view, Value::Matrix(_)) {
                // A setter such as view(3) returns nothing: report the new angles.
                view = self.call(name, Vec::new(), out)?;
            }
            return graphics_handles::view_outputs(view, count);
        }
        if let Expr::Call(name, args) = expression
            && name == "title"
            && count >= 2
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            if count > 2 {
                return Err("title: too many outputs".into());
            }
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = graphics_handles::heading_call(self, name, &values, count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && control_response::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = sysid::lti_args(self, values, out)?;
            if name == "margin"
                && let Some(value) = self.obj_call(name, &values, out)?
            {
                if count > 1 {
                    return Err("margin: classifier margin returns one output".into());
                }
                return Ok(vec![value]);
            }
            return profiler::dispatch(self, name, |engine| {
                control_response::call(engine, name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && control_lti::data_name(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let values = sysid::lti_args(self, values, out)?;
            let outputs = self.ctl_data(name, &values, count, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
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
            let values = string_class::arguments(self, name, args, out)?;
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
        if let Some(values) = self.hostio_outputs(expression, count, out)? {
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Some(values) = self.dlg_eval_outputs(expression, count, out)? {
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Some(values) = self.fl2_eval_outputs(expression, count, out)? {
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Some(values) = self.rdx_eval_outputs(expression, count, out)? {
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
        if (count > 1 && expression.may_expand()) || matches!(expression, Expr::CellContents(..)) {
            let mut values = self.eval_list(expression, None, out)?;
            if count <= 1 && values.len() != count {
                return Err("Cell content output count must match targets".into());
            }
            if values.len() < count {
                return Err("Insufficient number of outputs from right hand side of equal sign to satisfy assignment.".into());
            }
            values.truncate(count);
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && !self.contains_array(name)
            && let Some(function) = self.resolve_function(name)?
        {
            let values = string_class::arguments(self, name, args, out)?;
            self.lng_note_inputs(args, values.len());
            self.functions.lng.pending_nv_start =
                args.iter().position(|a| matches!(a, Expr::NameValue(_)));
            let values = self.invoke_function(function, values, count, Some(targets), out)?;
            self.charge_function_outputs(&values)?;
            return Ok(values);
        }
        if let Expr::Call(name, args) = expression
            && eig_more::handles(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.eigx_outputs(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Some((name, args)) = match expression {
            Expr::Call(name, args) => Some((name, args.as_slice())),
            Expr::Name(name) if name == "peaks" || name == "bucky" => Some((name, &[][..])),
            _ => None,
        } && matches!(
            name.as_str(),
            "shiftdim" | "inpolygon" | "peaks" | "hsv2rgb" | "rgb2hsv" | "bucky"
        ) && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            if name == "shiftdim"
                && let Some(mut outputs) = nd_shape::shiftdim(self, &values)?
            {
                outputs.truncate(count);
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.msc_outputs(name, &values, count)
            })?;
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
            let values = string_class::arguments(self, name, args, out)?;
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            let mut outputs = profiler::dispatch(self, name, |engine| {
                engine.set_operation_call(operation, &values, mask)
            })?;
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
            let values = string_class::arguments(self, name, args, out)?;
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            if let Some(outputs) = categorical_fns::outputs(self, name, &values, count)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            let mut outputs =
                profiler::dispatch(self, name, |engine| engine.ismember_call(&values, mask))?;
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
            let values = string_class::arguments(self, name, args, out)?;
            if let Some(outputs) = categorical_fns::outputs(self, name, &values, count)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            if let Some(outputs) = self.dtm_outputs(name, &values, count, out)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            let mut outputs = profiler::dispatch(self, name, |engine| {
                engine.cel_unique_outputs(&values, mask, out)
            })?;
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
            // Evaluate once, then choose the N-D or the original 2-D kernel.
            let values = string_class::arguments(self, name, args, out)?;
            if let Some(outputs) = categorical_fns::outputs(self, name, &values, count)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            if let Some(outputs) = self.dtm_outputs(name, &values, count, out)? {
                return Ok(outputs);
            }
            if values.iter().any(Value::is_nd) || nd_reduce::needs_dispatch(name, &values) {
                let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
                if let Some(outputs) = nd::multi_outputs(self, name, &values, count, mask)? {
                    return Ok(outputs);
                }
            }
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
            let mut outputs = profiler::dispatch(self, name, |engine| {
                engine.reduction_call(name, &values, count, mask)
            })?;
            outputs.truncate(count);
            if name != "find" {
                typed_functions::restore_first(&mut outputs, class)?;
            }
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && matches!(name.as_str(), "predict" | "kfoldPredict")
            && count > 1
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let Some(model) = values.first() else {
                return Err(format!("{name}: expected a model"));
            };
            if let Some(outputs) = self.obj_predict_outputs(name, model, &values[1..])? {
                if count > outputs.len() {
                    return Err(format!("{name}: too many outputs"));
                }
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs.into_iter().take(count).collect());
            }
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && linalg_more::lax_handles(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
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
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.linalg2_call_with_output(name, &values, count, out)
            })?
            .unwrap_or_default();
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && ode::is_solver(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                engine.ode_solve(name, values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && ode_more::has_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.od2_outputs(name, values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && quadrature::quadx_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.quadx_outputs(name, values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Name(name) = expression
            && name == "flow"
            && count > 1
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let outputs = volume::data::call(self, name, &[], count)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && sci_more::sc2_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.sc2_outputs(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && iter_solvers::slv_is_solver(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                engine.slv_outputs(name, &values, count, out)
            });
        }
        if let Expr::Call(name, args) = expression
            && file_io::handles(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.fileio_outputs(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && arrays_more::arx_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.arx_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && stats_more::has_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.statsx_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && image_color::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                image_color::call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && image_basic::registration_handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                image_basic::registration_call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && camera::detect::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                camera::detect::call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && image_segment::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                image_segment::call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && image_morph::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = self.function_arguments(args, None, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.image_morph_call(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && stats_tests::handles(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.stt_call(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && stats_gaps::handles(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.stats_gaps_call(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && outliers::handles(name)
            && !self.contains_array(name)
            && self.resolve_function(name)?.is_none()
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                outliers::call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && stats_dist::stats_dist_has_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.stats_dist_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && stats_models::NAMES.contains(&name.as_str())
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.stm_outputs(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && lang_utils::multiple_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.lang_utils_outputs(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && strings2::multiple_outputs(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.strings2_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && matrix_build::has_outputs(name)
            && !self.contains_array(name)
            && count > 1
        {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.matbuild_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if let Expr::Call(name, args) = expression
            && count > 1
            && signal_spectral::handles(name)
            && !self.contains_array(name)
        {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                signal_spectral::call(engine, name, &values, count)
            });
        }
        if count == 1 {
            return match self.eval(expression, None, out)? {
                value @ (Value::Matrix(_)
                | Value::Logical(_)
                | Value::Complex(_)
                | Value::Container(_)) => Ok(vec![value]),
                value @ Value::Text(_) => Ok(vec![value]),
                Value::Void => Err("This command does not return a value".into()),
                value @ Value::QuotedString(_) => Ok(vec![value]),
            };
        }
        let Expr::Call(name, args) = expression else {
            return Err("Multiple outputs require a function call".into());
        };
        if self.contains_array(name) {
            return Err("Array indexing returns one output".into());
        }
        if signal_timedomain::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                signal_timedomain::call(engine, name, &values, count)
            });
        }
        if iir::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                iir::call(engine, name, &values, count, out)
            });
        }
        if filter_analysis::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                filter_analysis::call(engine, name, &values, count, out)
            });
        }
        if filter_extras::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            return profiler::dispatch(self, name, |engine| {
                filter_extras::call(engine, name, &values, count)
            });
        }
        if name == "gradient" {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.core_more_gradient(&values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if elementary::has_multiple_outputs(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.everyday_outputs_call(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if trig_more::has_multiple_outputs(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.trigx_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if spline_toolbox::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                spline_toolbox::call(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Some(outputs) = self.polyopt_outputs(name, args, count, out)? {
            return Ok(outputs);
        }
        if mpc::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                mpc::outputs(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if optim_con::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                optim_con::ocn_outputs(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if optim_global::og_handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                optim_global::og_outputs(engine, name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if optim_nl::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = if name == "optimoptions" {
                if let Some(value) = optim_con::ocn_options_builder(self, &values, out)? {
                    if count > 1 {
                        return Err("optimoptions supports one output".into());
                    }
                    vec![value]
                } else {
                    profiler::dispatch(self, name, |engine| {
                        optim_nl::onl_outputs(engine, name, &values, count, out)
                    })?
                }
            } else {
                profiler::dispatch(self, name, |engine| {
                    optim_nl::onl_outputs(engine, name, &values, count, out)
                })?
            };
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if number_theory::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.numtheory_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if special_fns::handles(name) {
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.special_fns_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if dates::multiple_outputs(name) {
            let values = string_class::arguments(self, name, args, out)?;
            if let Some(outputs) = categorical_fns::outputs(self, name, &values, count)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            if let Some(outputs) = self.dtm_outputs(name, &values, count, out)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.datex_outputs(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if name == "log2" {
            if count != 2 {
                return Err(format!("log2 does not support {count} outputs"));
            }
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.logarithm_dissection_call(
                    &values,
                    [targets[0].is_some(), targets[1].is_some()],
                )
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if name == "cheby1" {
            if !(2..=3).contains(&count) {
                return Err("cheby1 supports one/two b,a or three z,p,k outputs; state-space/CTF forms are not implemented".into());
            }
            let values = string_class::arguments(self, name, args, out)?;
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.chebyshev_call(name, &values, count)
            })?;
            self.charge_function_outputs(&outputs)?;
            self.check_cancel()?;
            return Ok(outputs);
        }
        if table_fns::owns_outputs(name) {
            let names: Vec<String> = args
                .iter()
                .map(|a| match a {
                    Expr::Name(n) => n.clone(),
                    _ => String::new(),
                })
                .collect();
            let values = string_class::arguments(self, name, args, out)?;
            if let Some(outputs) = self.tbl_outputs(name, &values, &names, count, out)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
        }
        if datetime_fns::owns_outputs(name) {
            let values = string_class::arguments(self, name, args, out)?;
            if let Some(outputs) = categorical_fns::outputs(self, name, &values, count)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
            if let Some(outputs) = self.dtm_outputs(name, &values, count, out)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
        }
        if !matches!(name.as_str(), "filter" | "size" | "sort" | "sortrows")
            || (matches!(name.as_str(), "filter" | "sort" | "sortrows") && count != 2)
        {
            if econ::method(name) || econ_vol::METHODS.contains(&name.as_str()) {
                let values = string_class::arguments(self, name, args, out)?;
                if econ::accepts(self, name, &values) {
                    return econ::call(self, name, &values, count, out);
                }
                if econ_vol::method(self, name, &values) {
                    return profiler::dispatch(self, name, |engine| {
                        econ_vol::call(engine, name, &values, count, out)
                    });
                }
                return Err(self.raise(
                    econ_vol::undefined_method(name),
                    "MATLAB:UndefinedFunction".into(),
                ));
            }
            if !crate::help::is_supported(name) {
                return Err(crate::help::unknown_function(name));
            }
            return Err(format!("{name} does not support {count} outputs"));
        }
        // The supported-output preflight above remains before any argument
        // effects; builtins then share the same bounded CSL expansion as calls.
        let values = string_class::arguments(self, name, args, out)?;
        if let Some(outputs) = self.tbl_outputs(name, &values, &[], count, out)? {
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Some(outputs) = categorical_fns::outputs(self, name, &values, count)? {
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if let Some(outputs) = self.dtm_outputs(name, &values, count, out)? {
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        if values.iter().any(Value::is_nd) {
            let mask = std::array::from_fn(|i| targets.get(i).is_some_and(Option::is_some));
            if let Some(outputs) = nd::multi_outputs(self, name, &values, count, mask)? {
                self.charge_function_outputs(&outputs)?;
                return Ok(outputs);
            }
        }
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
                profiler::dispatch(self, name, |engine| {
                    engine.sort_call(&values, targets[1].is_some())
                })?
            } else {
                profiler::dispatch(self, name, |engine| {
                    engine.sort_rows_call(&values, targets[1].is_some())
                })?
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
            let outputs = profiler::dispatch(self, name, |engine| {
                engine.cxp_call(name, &values, count, out)
            })?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs);
        }
        let outputs = if name == "filter" {
            let (y, zf) = profiler::dispatch(self, name, |engine| {
                engine.filter_call(&values, targets[1].is_some())
            })?;
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
            profiler::dispatch(self, name, |engine| {
                let shape = match &values[0] {
                    Value::Container(a) => control_lti::io_shape(a).unwrap_or((a.rows(), a.cols())),
                    value => value.shape(&engine.cancel)?,
                };
                Ok((0..count)
                    .map(|i| {
                        Matrix::scalar(match i {
                            0 => shape.0 as f64,
                            1 => shape.1 as f64,
                            _ => 1.,
                        })
                    })
                    .collect())
            })?
        };
        for matrix in &outputs {
            self.charge(matrix.len())?;
        }
        self.check_cancel()?;
        Ok(outputs.into_iter().map(Value::Matrix).collect())
    }
    fn charge(&mut self, elements: usize) -> Result<()> {
        self.evaluated_elements = self.evaluated_elements.saturating_add(elements);
        if let Some(units) = self.limits.evaluation_units
            && self.evaluated_elements > units
        {
            return Err(limits::evaluation_error(units));
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
        if let Some(result) = scalar_loops::evaluate(self, e)? {
            return Ok(scalar_loops::value(result));
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
        const END_OUTSIDE_INDEX: &str = "end is only valid inside indexing";
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
            Expr::NameValue(s) => return Ok(Value::Text(s.clone())),
            Expr::Name(n) => match n.as_str() {
                "end" => Matrix::scalar(end.ok_or(END_OUTSIDE_INDEX)? as f64),
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
                _ => {
                    if let Some(function) = self.resolve_function(n)? {
                        return Ok(self
                            .invoke_function(function, Vec::new(), 1, None, out)?
                            .remove(0));
                    }
                    match n.as_str() {
                        "nargin" | "nargout" if self.functions.arguments.is_some() => {
                            let (inputs, outputs) = self.functions.arguments.unwrap();
                            Matrix::scalar(if n == "nargin" { inputs } else { outputs } as f64)
                        }
                        "mpcActiveSetOptions" | "mpcmoveopt" | "mpcsimopt" => {
                            return self.call(n, Vec::new(), out);
                        }
                        "missing" | "newline" | "audiorecorder" => {
                            return self.call(n, Vec::new(), out);
                        }
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
                        _ if datetime_fns::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if lang_extra::lg3_handles(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if graphics_handles::gobj_bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if ui_components::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if parallel::handles(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if fuzzy::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if econ_vol::handles(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if nnet::constructor(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if wavelet::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if registration::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if camera::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        _ if pde_model::bare(n) => {
                            return self.call(n, Vec::new(), out);
                        }
                        "pi" => Matrix::scalar(std::f64::consts::PI),
                        "eps" => Matrix::scalar(f64::EPSILON),
                        "Inf" | "inf" => Matrix::scalar(f64::INFINITY),
                        "NaN" | "nan" => Matrix::scalar(crate::MATLAB_NAN),
                        "gcf" | "gca" | "groot" | "axes" | "figure" | "tiledlayout"
                        | "nexttile" | "tic" | "toc" | "profile" | "profsave" | "rand"
                        | "randn" | "rng" | "mfilename" | "flintmax" | "realmax" | "realmin"
                        | "rosser" | "intmax" | "intmin" | "peaks" | "lasterr" | "lastwarn"
                        | "warning" | "flow" | "who" | "rehash" | "edit" | "bvpset" | "ddeset"
                        | "odeset" | "optimset" | "view" | "xlim" | "ylim" | "zlim" | "axis"
                        | "animatedline" | "line" | "balredOptions" | "sphere" | "ishold"
                        | "movegui" | "bucky" => {
                            return self.call(n, Vec::new(), out);
                        }
                        // A bare exit/quit statement ends the run (host_io.rs).
                        "exit" | "quit" => return self.call(n, Vec::new(), out),
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
                if datetime_fns::is_dt(&value) {
                    return self.dtm_unary(*negative, value);
                }
                if let Value::Container(Array::Sparse(s)) = &value {
                    return Ok(sparse_fns::unary(s, *negative));
                }
                if control_lti::is_lti(&value) {
                    return if *negative {
                        control_lti::negate(self, &value)
                    } else {
                        Ok(value)
                    };
                }
                if value.is_nd() {
                    return nd::unary_operator(value, *negative, false);
                }
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
                let input = self.eval(x, end, out)?;
                let input = sparse_fns::densify(self, "not", input)?;
                if input.is_nd() {
                    return nd::unary_operator(input, false, true);
                }
                let value = input.logical(&self.cancel)?;
                return Ok(Value::Logical(value.not(&self.cancel)?));
            }
            Expr::Transpose(x) | Expr::NonConjugateTranspose(x) => {
                let value = self.eval(x, end, out)?;
                if let Some(c) = categorical_fns::of(&value) {
                    return categorical_fns::transpose(self, c);
                }
                if datetime_fns::is_dt(&value) {
                    return self.dtm_transpose(&value);
                }
                if let Value::Container(Array::Sparse(s)) = &value {
                    return sparse_fns::transpose(s);
                }
                if string_class::is_string(&value) {
                    return self.str_transpose(&value);
                }
                // A 1-by-1 struct transposes to itself, which would make D'\B
                // solve with the untransposed matrix.
                if linear_algebra2::is_decomposition(&value) {
                    return Err(
                        "decomposition: transposing a decomposition object is not supported yet"
                            .into(),
                    );
                }
                if value.is_nd() {
                    return Err(nd::TRANSPOSE_ND.into());
                }
                match value {
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
                }
            }
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
                if categorical_fns::is_cat(&a) {
                    let b = self.eval(b, end, out)?;
                    return categorical_fns::binary(self, op, &a, &b);
                }
                if datetime_fns::is_dt(&a) {
                    let b = self.eval(b, end, out)?;
                    return self.dtm_binary(op, &a, &b);
                }
                if sparse_fns::is_sparse(&a) {
                    let b = self.eval(b, end, out)?;
                    return sparse_fns::binary(self, op, a, b, out);
                }
                if string_class::is_string(&a) {
                    let b = self.eval(b, end, out)?;
                    return self.str_binary(op, &a, &b);
                }
                if control_lti::is_lti(&a) {
                    let b = self.eval(b, end, out)?;
                    return self.ctl_binary(op, &a, &b, out);
                }
                if a.is_nd() {
                    let b = self.eval(b, end, out)?;
                    if let Some(value) = nd::binary(self, op, &a, &b)? {
                        return Ok(value);
                    }
                }
                if typed_values::is_typed(&a) {
                    let b = self.eval(b, end, out)?;
                    if categorical_fns::is_cat(&b) {
                        return categorical_fns::binary(self, op, &a, &b);
                    }
                    if datetime_fns::is_dt(&b) {
                        return self.dtm_binary(op, &a, &b);
                    }
                    if string_class::is_string(&b) {
                        return self.str_binary(op, &a, &b);
                    }
                    if b.is_nd()
                        && let Some(value) = nd::binary(self, op, &a, &b)?
                    {
                        return Ok(value);
                    }
                    return self.typed_arithmetic(op, a, b);
                }
                if linear_algebra2::is_decomposition(&a) {
                    let b = self.eval(b, end, out)?;
                    return linear_algebra2::decomposition_binary(self, op, &a, &b, out);
                }
                // Remaining unsupported containers fail before right-hand side
                // effects. Char uses the shared numeric conversion.
                if matches!(a, Value::QuotedString(_))
                    || (matches!(a, Value::Container(_)) && !a.is_character())
                {
                    return Err("Arithmetic requires numeric or logical operands".into());
                }
                a.shape(&self.cancel)?;
                let b = self.eval(b, end, out)?;
                if categorical_fns::is_cat(&b) {
                    return categorical_fns::binary(self, op, &a, &b);
                }
                if datetime_fns::is_dt(&b) {
                    return self.dtm_binary(op, &a, &b);
                }
                if sparse_fns::is_sparse(&b) {
                    return sparse_fns::binary(self, op, a, b, out);
                }
                if string_class::is_string(&b) {
                    return self.str_binary(op, &a, &b);
                }
                if control_lti::is_lti(&b) {
                    return self.ctl_binary(op, &a, &b, out);
                }
                if linear_algebra2::is_decomposition(&b) {
                    return linear_algebra2::decomposition_binary(self, op, &a, &b, out);
                }
                if b.is_nd()
                    && let Some(value) = nd::binary(self, op, &a, &b)?
                {
                    return Ok(value);
                }
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
                let expands =
                    a.len() != 1 && b.len() != 1 && (a.rows(), a.cols()) != (b.rows(), b.cols());
                match op {
                    Token::Plus if expands => {
                        elementary::real_elementwise(self, &a, &b, |a, b| a + b)?
                    }
                    Token::Minus if expands => {
                        elementary::real_elementwise(self, &a, &b, |a, b| a - b)?
                    }
                    Token::DotStar if expands => {
                        elementary::real_elementwise(self, &a, &b, |a, b| a * b)?
                    }
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
                    Token::DotSlash => elementary::real_elementwise(self, &a, &b, |a, b| a / b)?,
                    Token::DotBackslash => {
                        elementary::real_elementwise(self, &b, &a, |b, a| b / a)?
                    }
                    Token::DotPower => elementary::real_elementwise(self, &a, &b, f64::powf)?,
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
                if args.iter().any(|arg| matches!(arg, Expr::NameValue(_))) {
                    return Err("Name=Value syntax is not supported for indexing".into());
                }
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
                if values.iter().flatten().any(control_lti::is_lti) {
                    return control_lti::literal(self, values, out);
                }
                if values
                    .iter()
                    .flatten()
                    .any(|v| objects::fis_value(self, v).is_some())
                {
                    return fuzzy::properties::concatenate(self, values);
                }
                if values.iter().flatten().any(table_fns::is_table) {
                    return self.tbl_concat(values, out);
                }
                if values.iter().flatten().any(categorical_fns::is_cat) {
                    return categorical_fns::concatenate(self, values);
                }
                if values.iter().flatten().any(datetime_fns::is_dt) {
                    return self.dtm_concat(values);
                }
                if values.iter().flatten().any(sparse_fns::is_sparse) {
                    return sparse_fns::concatenate(self, values);
                }
                if values.iter().flatten().any(Value::is_nd) {
                    return nd::literal(self, values);
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
                if args.iter().any(|arg| matches!(arg, Expr::NameValue(_)))
                    && (self.container_variables.contains_key(name)
                        || self.complex_variables.contains_key(name)
                        || self.logical_variables.contains_key(name)
                        || self.variables.contains_key(name))
                {
                    return Err("Name=Value syntax is not supported for indexing".into());
                }
                if let Some(Array::Sparse(s)) = self.container_variables.get(name) {
                    let s = s.clone();
                    return sparse_fns::index(self, &s, args, out);
                }
                if let Some(a) = self.container_variables.get(name).cloned() {
                    // An inline object is called, not indexed, whatever its argument count.
                    if let Some(value) = self.lg3_object_index(&a, args, out)? {
                        return Ok(value);
                    }
                    if let Some(value) = sysid::index(self, &a, args, out)? {
                        return Ok(value);
                    }
                    if (a.is_nd() || args.len() > 2)
                        && !matches!(control_lti::class(&a), Some("tf" | "zpk" | "ss"))
                    {
                        return nd::index(self, Value::Container(a), args, out);
                    }
                    return self.index_container(&a, args, false, out);
                }
                if let Some(m) = self.complex_variables.get(name).cloned() {
                    if m.is_nd() || args.len() > 2 {
                        return nd::index(self, Value::Complex(m), args, out);
                    }
                    return Ok(Value::Complex(self.index_array(&m, args, out, false)?));
                }
                if let Some(m) = self.logical_variables.get(name).cloned() {
                    if m.is_nd() || args.len() > 2 {
                        return nd::index(self, Value::Logical(m), args, out);
                    }
                    return Ok(Value::Logical(self.index_array(&m, args, out, false)?));
                }
                if let Some(m) = self.variables.get(name).cloned() {
                    if m.is_nd() || args.len() > 2 {
                        return nd::index(self, Value::Matrix(m), args, out);
                    }
                    self.index_array(&m, args, out, false)?
                } else {
                    if name == "clear" && self.resolve_function(name)?.is_none() {
                        return Err(
                            "clear requires a standalone statement without output assignment"
                                .into(),
                        );
                    }
                    let mut values = Vec::with_capacity(args.len());
                    let mut nv_start = None;
                    for arg in args {
                        if nv_start.is_none() && matches!(arg, Expr::NameValue(_)) {
                            nv_start = Some(values.len());
                        }
                        let evaluated = if arg.may_expand() {
                            self.eval_list(arg, end, out).map(Some)
                        } else {
                            self.eval(arg, end, out).map(|value| {
                                values.push(value);
                                None
                            })
                        };
                        let Some(list) = (match evaluated {
                            Err(ref error)
                                if error == END_OUTSIDE_INDEX
                                    && !crate::help::is_matlab_name(name)
                                    && !crate::help::is_supported(name)
                                    && self.resolve_function(name)?.is_none() =>
                            {
                                return Err(self.raise(
                                    crate::help::unknown_function(name),
                                    "MATLAB:UndefinedFunction".into(),
                                ));
                            }
                            result => result?,
                        }) else {
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
                    self.functions.lng.pending_nv_start = nv_start;
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
        self.graphics_call_with_line_defaults(name, args, out, false)
    }

    fn graphics_call_with_line_defaults(
        &mut self,
        name: &str,
        args: Vec<Value>,
        out: &mut Execution,
        primitive_line: bool,
    ) -> Result<Value> {
        // The call site's argument names belong to this call only: a user
        // function entered here keeps them, a builtin (cellfun, feval, ...)
        // must not pass them on to functions it calls itself.
        let names = std::mem::take(&mut self.functions.lng.pending_names);
        let nv_start = self.functions.lng.pending_nv_start.take();
        // A user object argument: its class's method (or object query) first.
        let object_first = args.first().is_some_and(classdef::is_object_value);
        if object_first {
            // A method keeps the call site's names (inputname); the object
            // queries answered here take none.
            self.functions.lng.pending_names = names.clone();
            self.functions.lng.pending_nv_start = nv_start;
        }
        if name == "isprop"
            && let Some(o) = args.first().and_then(|v| objects::fis_value(self, v))
            && self.resolve_function(name)?.is_none()
        {
            return fuzzy::properties::method(self, &o, name, &args[1..], out)?
                .ok_or_else(|| "fuzzy: unsupported property query".into());
        }
        if name == "isprop"
            && let Some(net) = args.first().and_then(|v| objects::nnet_value(self, v))
            && self.resolve_function(name)?.is_none()
        {
            return nnet::object_method(self, &net, name, &args[1..])?
                .ok_or_else(|| "network: unsupported property query".into());
        }
        if name == "isprop"
            && args
                .first()
                .is_some_and(|v| objects::triangulation_value(self, v).is_some())
            && self.resolve_function(name)?.is_none()
        {
            return self
                .obj_call(name, &args, out)?
                .ok_or_else(|| "delaunayTriangulation: unsupported property query".into());
        }
        // isprop on imref2d and the transformation objects: before the
        // classdef query, which only knows classdef and graphics objects.
        if name == "isprop"
            && args
                .first()
                .is_some_and(|v| registration::geo_of(self, v).is_some())
            && self.resolve_function(name)?.is_none()
        {
            let mut values = registration::call(self, name, &args, 1, out)?
                .ok_or("isprop: unsupported property query")?;
            return Ok(values.swap_remove(0));
        }
        if name == "isprop"
            && args
                .first()
                .is_some_and(|v| camera::camera_of(self, v).is_some())
            && self.resolve_function(name)?.is_none()
        {
            let mut values = camera::call(self, name, &args, 1, out)?
                .ok_or("isprop: unsupported property query")?;
            return Ok(values.swap_remove(0));
        }
        // An imported, local or private function comes before the method.
        let handled = match object_first.then(|| self.cls_shadowed(name)) {
            Some(Ok(true)) => Ok(None),
            Some(Err(error)) => Err(error),
            _ => self.cls_call(name, &args, 1, None, out),
        };
        if object_first {
            self.functions.lng.pending_names.clear();
            self.functions.lng.pending_nv_start = None;
        }
        if let Some(mut values) = handled? {
            return Ok(values.swap_remove(0));
        }
        if let Some(function) = self.resolve_function(name)? {
            self.functions.lng.pending_names = names;
            self.functions.lng.pending_nv_start = nv_start;
            return Ok(self
                .invoke_function(function, args, 1, None, out)?
                .remove(0));
        }
        if self.profiler.enabled {
            return profiler::builtin(self, name, |engine| {
                engine.profiler_builtin_call(name, args, names, out, primitive_line)
            });
        }
        self.profiler_builtin_call(name, args, names, out, primitive_line)
    }
    fn profiler_builtin_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
        names: Vec<String>,
        out: &mut Execution,
        primitive_line: bool,
    ) -> Result<Value> {
        if let Some(launch) = app_launchers::Launch::for_name(name) {
            return launch.open(name, args.len(), 1).map(|()| Value::Void);
        }
        // Check before type-specific repmat dispatchers interpret [] as a size.
        if name == "repmat"
            && let [_, Value::Matrix(factors)] = args.as_slice()
            && factors.is_empty()
        {
            return Err(
                "Replication factors must be a row vector of integers or integer scalars.".into(),
            );
        }
        // imref2d and the 2-D transformation objects: before the string
        // functions (contains) and the generic object queries (isa, ...).
        if let Some(mut values) = registration::call(self, name, &args, 1, out)? {
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.swap_remove(0)
            });
        }
        // cameraIntrinsics, cameraParameters and the calibration functions.
        if let Some(mut values) = camera::call(self, name, &args, 1, out)? {
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.swap_remove(0)
            });
        }
        if let Some(mut values) = sysid::call(self, name, &args, 1, out)? {
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.swap_remove(0)
            });
        }
        if econ::accepts(self, name, &args) {
            return Ok(econ::call(self, name, &args, 1, out)?.remove(0));
        }
        if econ_tests::handles(name) {
            return Ok(econ_tests::call(self, name, &args, 1, out)?.remove(0));
        }
        if econ_vol::handles(name) || econ_vol::method(self, name, &args) {
            return Ok(econ_vol::call(self, name, &args, 1, out)?.remove(0));
        }
        if comm::handles(name) {
            return Ok(comm::call(self, name, &args, 1, out)?.remove(0));
        }
        if fuzzy::handles(name) {
            return Ok(fuzzy::call(self, name, &args, 1, out)?.remove(0));
        }
        if comm_wave::handles(name) {
            return Ok(comm_wave::call(self, name, &args, 1, out)?.remove(0));
        }
        if name == "sim"
            && args
                .first()
                .is_some_and(|v| objects::mpc_value(self, v).is_some())
        {
            let outputs = mpc::outputs(self, name, &args, 1, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs.into_iter().next().unwrap());
        }
        if nnet::handles(name)
            || (name == "sim"
                && args
                    .first()
                    .is_some_and(|v| objects::nnet_value(self, v).is_some()))
        {
            return Ok(nnet::call(self, name, &args, 1)?.remove(0));
        }
        if wavelet::method(name, &args) {
            return Ok(wavelet::call(self, name, &args, 1, out)?.swap_remove(0));
        }
        if let Some(mut values) = curve_fit::call(self, name, &args, 1, out)? {
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.swap_remove(0)
            });
        }
        if let Some(value) = ui_components::call(self, name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.dlg_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(mut values) = categorical_fns::outputs(self, name, &args, 1)? {
            return Ok(values.remove(0));
        }
        if let Some(value) = self.tbl_call(name, &args, &names, out)? {
            return Ok(value);
        }
        if let Some(value) = import_options::call(self, name, &args, out)? {
            return Ok(value);
        }
        if matches!(name, "margin" | "edge")
            && let Some(value) = self.obj_call(name, &args, out)?
        {
            return Ok(value);
        }
        if image_color::handles(name) {
            return Ok(image_color::call(self, name, &args, 1, out)?.remove(0));
        }
        if image_basic::registration_handles(name) {
            return Ok(image_basic::registration_call(self, name, &args, 1, out)?.remove(0));
        }
        // Before the N-D and class guards: images are N-D and integer-typed.
        if camera::detect::handles(name) {
            return Ok(camera::detect::call(self, name, &args, 1, out)?.remove(0));
        }
        if image_segment::handles(name) {
            return Ok(image_segment::call(self, name, &args, 1, out)?.remove(0));
        }
        if image_morph::handles(name) {
            return Ok(self.image_morph_call(name, &args, 1, out)?.remove(0));
        }
        // Before the datetime dispatch: timetables and duration windows.
        if outliers::handles(name) {
            return Ok(outliers::call(self, name, &args, 1, out)?.remove(0));
        }
        if let Some(value) = self.dtm_call(name, &args, out)? {
            return Ok(value);
        }
        let args = match sparse_fns::call(self, name, args, out)? {
            Ok(value) => return Ok(value),
            Err(args) => args,
        };
        // dicominfo owns scalar filename/type validation, including string arrays.
        if name != "dicominfo"
            && !(name == "circshift" && args.first().is_some_and(string_class::is_string))
        {
            string_class::guard(name, &args)?;
        }
        if let Some(value) = self.str_call(name, &args, out)? {
            return Ok(value);
        }
        // Image functions take H-by-W-by-C arrays: before the N-D guard.
        if let Some(value) = self.image_basic_call(name, &args)? {
            return Ok(value);
        }
        if volume::iso::handles(name) {
            return Ok(volume::iso::call(self, name, &args, 1, out)?.remove(0));
        }
        if volume::slice_viewer::handles(name) {
            return Ok(volume::slice_viewer::call(self, name, &args, 1, out)?.remove(0));
        }
        if nd::needs_dispatch(name, &args) {
            nd::guard(name, &args)?;
            if let Some(value) = nd::call(self, name, &args, out)? {
                return Ok(value);
            }
        }
        if signal_spectral::handles(name) {
            return Ok(signal_spectral::call(self, name, &args, 1)?.remove(0));
        }
        if control_design::cds_handles(name) {
            return Ok(control_design::cds_call(self, name, &args, 1, out)?.remove(0));
        }
        if control_reduce::handles(name) {
            return Ok(control_reduce::call(self, name, &args, 1, out)?.remove(0));
        }
        if signal_timedomain::handles(name) {
            return Ok(signal_timedomain::call(self, name, &args, 1)?.remove(0));
        }
        if iir::handles(name) {
            return Ok(iir::call(self, name, &args, 1, out)?.remove(0));
        }
        if filter_analysis::handles(name) {
            return Ok(filter_analysis::call(self, name, &args, 1, out)?.remove(0));
        }
        if filter_extras::handles(name) {
            return Ok(filter_extras::call(self, name, &args, 1)?.remove(0));
        }
        if control_response::handles(name) {
            return Ok(control_response::call(self, name, &args, 1, out)?.remove(0));
        }
        if graphics_handles::surface_names(name) {
            return graphics_handles::surface_call(self, name, &args, 1, out);
        }
        if plots_2d_more::handles(name) {
            return Ok(plots_2d_more::call(self, name, &args, 1, out)?.remove(0));
        }
        if plots_compose::handles(name) {
            let count = usize::from(!plots_compose::no_output(name));
            let values = plots_compose::call(self, name, &args, count, out)?;
            return Ok(values.into_iter().next().unwrap_or(Value::Void));
        }
        #[cfg(target_arch = "wasm32")]
        if browser::file_command(name, Some(&args)) {
            return Err(browser::NO_FILES.into());
        }
        #[cfg(target_arch = "wasm32")]
        if let Some(value) = self.browser_query(name, &args)? {
            return Ok(value);
        }
        if let Some(value) = self.ctl_call(name, &args, out)? {
            return Ok(value);
        }
        if cplx_poly::routes(name, &args) {
            return Ok(self.cxp_call(name, &args, 1, out)?.remove(0));
        }
        if nnet_plots::handles(name) {
            return Ok(nnet_plots::call(self, name, &args, 1, out)?.remove(0));
        }
        if stats_plots::handles(name) {
            let mut values = stats_plots::call(self, name, &args, 1, out)?;
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.remove(0)
            });
        }
        if dist_fit::handles(name) {
            let mut values = dist_fit::call(self, name, &args, 1, out)?;
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.remove(0)
            });
        }
        if wavelet::handles(name) {
            let mut values = wavelet::call(self, name, &args, 1, out)?;
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.remove(0)
            });
        }
        if parallel::handles(name) || parallel::method(self, name, &args) {
            let mut values = parallel::call(self, name, &args, 1, out)?;
            return Ok(if values.is_empty() {
                Value::Void
            } else {
                values.remove(0)
            });
        }
        if linear_models::handles(name) {
            return Ok(linear_models::call(self, name, &args, 1, out)?.remove(0));
        }
        if name == "designfilt" {
            let f = digital_filter::designfilt(self, &args)?;
            return self.filter_new(f);
        }
        if let Some(value) = dist_objects::call(self, name, &args)? {
            return Ok(value);
        }
        if name == "circshift" && args.first().is_some_and(string_class::is_string) {
            return self
                .cel_call(name, &args, out)?
                .ok_or_else(|| "circshift: unsupported string input".into());
        }
        if let Some(value) = self.obj_call(name, &args, out)? {
            return Ok(value);
        }
        if name == "round" && args.len() >= 2 {
            return round14(self, &args);
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
        if let Some(value) = self.rdx_call(name, &args, out)? {
            return Ok(value);
        }
        self.functions.lng.call_names = names;
        if let Some(value) = self.lng_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = self.hostio_call(name, &args, out)? {
            return Ok(value);
        }
        if stats_tests::handles(name) {
            return Ok(self.stt_call(name, &args, 1, out)?.remove(0));
        }
        // Before the generic integer/single conversion: the module applies
        // its own class rule, the same for every output count.
        if stats_gaps::handles(name) {
            return Ok(self.stats_gaps_call(name, &args, 1)?.remove(0));
        }
        if stats_models::NAMES.contains(&name) {
            return Ok(self.stm_outputs(name, &args, 1, out)?.remove(0));
        }
        if mpc::handles(name) {
            let outputs = mpc::outputs(self, name, &args, 1, out)?;
            self.charge_function_outputs(&outputs)?;
            return Ok(outputs.into_iter().next().unwrap());
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
        if let Some(value) = self.stats_dist_call(name, &args)? {
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
        if spline_toolbox::handles(name) {
            let mut values = spline_toolbox::call(self, name, &args, 1, out)?;
            return Ok(values.swap_remove(0));
        }
        if let Some(value) = self.polyopt_call(name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = optim_con::ocn_call(self, name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = optim_global::og_call(self, name, &args, out)? {
            return Ok(value);
        }
        if let Some(value) = optim_nl::onl_call(self, name, &args, out)? {
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
        if pde::handles(name) {
            return pde::call(self, name, &args, 1, out).map(|mut v| v.remove(0));
        }
        if pde_geometry::handles(name) {
            return pde_geometry::call(self, name, &args, 1, out).map(|mut v| v.remove(0));
        }
        if pde_model::handles(name) {
            return pde_model::call(self, name, &args, 1, out).map(|mut v| v.remove(0));
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
            if let [arg] = args.as_ref() {
                let arity = |function: &user_functions::ResolvedFunction| {
                    let (count, variable) = if name == "nargin" {
                        (function.input_count(), function.variadic())
                    } else {
                        (function.output_count(), function.variadic_output())
                    };
                    Value::Matrix(Matrix::scalar(
                        count as f64 * if variable { -1.0 } else { 1.0 },
                    ))
                };
                let (target, scope) = match arg {
                    Value::Container(Array::Handle(h)) => {
                        if let Some(function) = user_functions::nested::bound(h) {
                            return Ok(arity(function));
                        }
                        if let Some((anonymous, _)) = h.anonymous_parts() {
                            let count = if name == "nargin" {
                                let count = anonymous.params.len() as f64;
                                if anonymous
                                    .params
                                    .last()
                                    .is_some_and(|param| param == "varargin")
                                {
                                    -count
                                } else {
                                    count
                                }
                            } else {
                                1.0
                            };
                            return Ok(Value::Matrix(Matrix::scalar(count)));
                        }
                        (h.name().unwrap_or_default().to_owned(), h.scope().cloned())
                    }
                    Value::Text(s) => (s.clone(), None),
                    _ => return Err(format!("{name}: expected a function name or handle")),
                };
                let caller = scope
                    .as_ref()
                    .and_then(|s| s.downcast_ref::<user_functions::HandleScope>())
                    .map(|s| self.ctx_enter_handle(s));
                let resolved = self.resolve_function(&target);
                if let Some(caller) = caller {
                    self.ctx_leave_handle(caller);
                }
                if let Some(function) = resolved? {
                    return Ok(arity(&function));
                }
                if crate::session::FUNCTIONS.contains(&target.as_str()) {
                    return Err(format!("{name} is not known for builtin {target}"));
                }
                return Err(self.raise(
                    format!("Function {target} does not exist."),
                    "MATLAB:narginout:functionDoesnotExist".into(),
                ));
            }
            if !args.is_empty() {
                return Err("Too many input arguments.".into());
            }
            let (inputs, outputs) = self
                .functions
                .arguments
                .ok_or("nargin/nargout require a function workspace")?;
            return Ok(Value::Matrix(Matrix::scalar(
                if name == "nargin" { inputs } else { outputs } as f64,
            )));
        }
        if graphics_ticks::is_tick_or_colororder(name) {
            return self.graphics_ticks_or_colororder(name, args.into_owned());
        }
        if matches!(
            name,
            "hold"
                | "grid"
                | "xlim"
                | "ylim"
                | "zlim"
                | "view"
                | "xscale"
                | "yscale"
                | "axis"
                | "clf"
                | "cla"
                | "subplot"
                | "tiledlayout"
                | "nexttile"
                | "linkaxes"
                | "sgtitle"
                | "figure"
                | "close"
        ) {
            return self.graphics(name, args.into_owned(), out);
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
        if matches!(name, "xlabel" | "ylabel" | "zlabel") {
            if args.is_empty() || args.len() > 11 || args.len().is_multiple_of(2) {
                return Err(format!(
                    "{name} expects a label and optional Name,Value pairs"
                ));
            }
            let mut color = None;
            let mut interpreter = crate::plot::text_markup::Interpreter::Tex;
            let mut style = crate::plot::LabelStyle::default();
            for pair in args[1..].as_chunks::<2>().0 {
                match (&pair[0], &pair[1]) {
                    (Value::Text(key), Value::Text(value))
                        if key.eq_ignore_ascii_case("Interpreter") =>
                    {
                        interpreter = crate::plot::text_markup::Interpreter::parse(value)?;
                    }
                    (Value::Text(key), value) if key.eq_ignore_ascii_case("Color") => {
                        color = Some(graphics_handles::axes_rgb(value, &self.cancel)?)
                    }
                    (Value::Text(key), value) if key.eq_ignore_ascii_case("FontSize") => {
                        let size = graphics_handles::dot_syntax::real_scalar(value, "FontSize")?;
                        crate::plot::validate_font_size(size)?;
                        style.font_size = Some(size);
                    }
                    (Value::Text(key), value) if key.eq_ignore_ascii_case("FontWeight") => {
                        style.bold = Some(graphics_handles::dot_syntax::text_font_weight(value)?);
                    }
                    (Value::Text(key), value) if key.eq_ignore_ascii_case("FontAngle") => {
                        style.italic = Some(graphics_handles::dot_syntax::text_font_angle(value)?);
                    }
                    _ => {
                        return Err(
                            "Axes text supports Interpreter, RGB Color, FontSize, FontWeight and FontAngle".into(),
                        );
                    }
                }
            }
            let (s, lines) = graphics_text_lines(&raw_args[0], &self.cancel)?;
            if s.len() > 4096 {
                return Err("Plot label exceeds 4096 bytes".into());
            }
            let mut figure = self.figure.clone().unwrap_or_default();
            figure.activate_tile();
            if let Some(rgb) = color {
                let colors = figure.axes.colors.get_or_insert_with(Default::default);
                match name {
                    "xlabel" => colors.xlabel = Some(rgb),
                    "zlabel" => return Err("zlabel Color is not supported yet".into()),
                    _ => colors.ylabel = Some(rgb),
                }
            }
            let index = match name {
                "xlabel" => 1,
                "ylabel" => 2,
                _ => 3,
            };
            figure.axes.extras.label_interpreters[index] = interpreter;
            figure.axes.extras.label_lines[index] = lines;
            let target_style = &mut figure.axes.extras.label_styles[index - 1];
            target_style.font_size = style.font_size.or(target_style.font_size);
            target_style.bold = style.bold.or(target_style.bold);
            target_style.italic = style.italic.or(target_style.italic);
            match name {
                "xlabel" => figure.xlabel = s.clone(),
                "zlabel" => figure.zlabel = s.clone(),
                _ => figure.ylabel = s.clone(),
            };
            figure.validate()?;
            #[cfg(test)]
            graphics_handles::tests::before_property_publish(&self.cancel);
            self.check_cancel()?;
            let identity = match index {
                1 => &figure.axes.extras.xlabel_identity,
                2 => &figure.axes.extras.ylabel_identity,
                _ => &figure.axes.extras.zlabel_identity,
            };
            let handle = crate::GraphicsHandle::axes_object(
                &figure.identity,
                &figure.axes.identity,
                identity,
                crate::GraphicsKind::Text,
            );
            let result = crate::GraphicsArray::scalar(handle, &self.cancel)?;
            self.figure = Some(figure);
            return Ok(Value::Container(result.into()));
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
                    if let Some(text) = volume::show::volume_display(self, &array) {
                        Some(text)
                    } else {
                        crate::display_format::disp(&array, &self.display)?
                    }
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
            "plot" | "semilogx" | "semilogy" | "loglog" | "scatter" | "bar" | "plot3" | "scatter3"
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
            "gausswin" | "tukeywin" | "chebwin" | "taylorwin" | "barthannwin" | "bohmanwin"
            | "parzenwin" => {
                let max_args = match name {
                    "taylorwin" => 3,
                    "gausswin" | "tukeywin" | "chebwin" => 2,
                    _ => 1,
                };
                arity(&(1..=max_args).collect::<Vec<_>>())?;
                let n = crate::dsp::window_length(scalar(0)?)?;
                let param = |i: usize, default: f64| -> Result<f64> {
                    match args.get(i) {
                        None => Ok(default),
                        Some(Value::Matrix(a)) if a.is_empty() => Ok(default),
                        Some(_) => scalar(i),
                    }
                };
                use crate::dsp::ParamWindow as W;
                let kind = match name {
                    "gausswin" => W::Gauss(param(1, 2.5)?),
                    "tukeywin" => W::Tukey(param(1, 0.5)?),
                    "chebwin" => {
                        let r = param(1, 100.)?;
                        if !(r.is_finite() && r > 0.) {
                            return Err(
                                "chebwin: R must be a positive finite sidelobe attenuation in dB"
                                    .into(),
                            );
                        }
                        W::Cheb(r)
                    }
                    "taylorwin" => {
                        let nbar = param(1, 4.)?;
                        let sll = param(2, -30.)?;
                        if !(nbar.is_finite() && nbar >= 1. && nbar.fract() == 0.) {
                            return Err("taylorwin: NBAR must be a positive integer".into());
                        }
                        if !(sll.is_finite() && sll < 0.) {
                            return Err("taylorwin: SLL must be negative (dB)".into());
                        }
                        W::Taylor(nbar as usize, sll)
                    }
                    "barthannwin" => W::BartHann,
                    "bohmanwin" => W::Bohman,
                    _ => W::Parzen,
                };
                crate::dsp::param_window(kind, n, &self.cancel)?
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
                if args.is_empty() {
                    Matrix::identity(1, 1)?
                } else {
                    if matches!(&args[..], [Value::Matrix(a)] if a.rows() == 1 && a.len() > 2) {
                        return Err("N-dimensional arrays are not supported.".into());
                    }
                    let dims = shape_arguments::shape_dims(&args, None)?;
                    if dims.len() > 2 {
                        return Err("N-dimensional arrays are not supported.".into());
                    }
                    Matrix::identity(dims[0], dims[1])?
                }
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
                        "Use {name}(A[, 0|1|[] [, dim|'all']][, 'omitnan'|'includenan']); vector weights are not implemented"
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
                        Value::Text(value) if value.eq_ignore_ascii_case("all") => {
                            StatisticsAxis::All
                        }
                        Value::Matrix(value) if value.len() == 1 => {
                            StatisticsAxis::Dimension(dimension(2)?)
                        }
                        _ => {
                            return Err(
                                "Statistics dimension must be a positive integer or 'all'".into()
                            );
                        }
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
            "plot" | "semilogx" | "semilogy" | "loglog" | "scatter" | "bar" | "stairs" | "stem"
            | "plot3" | "scatter3" => {
                let kind = match name {
                    "scatter" | "scatter3" => crate::plot::PlotKind::Scatter,
                    "bar" => crate::plot::PlotKind::Bar,
                    "stairs" => crate::plot::PlotKind::Stairs,
                    "stem" => crate::plot::PlotKind::Stem,
                    _ => crate::plot::PlotKind::Line,
                };
                // plot3/scatter3 read X,Y,Z triplets through the same loop.
                let three = matches!(name, "plot3" | "scatter3");
                if three && args.len() < 3 {
                    return Err(format!(
                        "{name}: not enough input arguments; X, Y and Z are required"
                    ));
                }
                if name == "scatter" && args.is_empty() {
                    return Err("scatter: not enough input arguments; X and Y are required".into());
                }
                let (data_end, properties) = if kind == crate::plot::PlotKind::Scatter {
                    let coord_count = if three {
                        3
                    } else if args.len() == 1
                        || !args.get(1).is_some_and(plot_time_axes::is_plot_data_arg)
                    {
                        1
                    } else {
                        2
                    };
                    plot_properties::Properties::parse_scatter(&args, coord_count)?
                } else if name == "bar" {
                    plot_properties::Properties::parse_bar(&args)?
                } else {
                    plot_properties::Properties::parse(&args, kind)?
                };
                let coordinate_end = data_end;
                let data_args = &args[..coordinate_end];
                if data_args.is_empty() {
                    return Err("Plot requires data".into());
                }
                let has_complex = data_args.iter().any(|v| matches!(v, Value::Complex(_)));
                let has_non_numeric = data_args.iter().any(plot_time_axes::is_non_numeric_data);
                if has_complex
                    && (has_non_numeric
                        || !matches!(name, "plot" | "semilogx" | "semilogy" | "loglog"))
                {
                    return Err(format!(
                        "{name} requires real coordinates; use real(Z) and imag(Z) explicitly"
                    ));
                }
                let data_count = data_args
                    .iter()
                    .filter(|v| plot_time_axes::is_plot_data_arg(v))
                    .count();
                let coordinates = |i: usize| -> Result<crate::plot::PlotData<'_>> {
                    match &data_args[i] {
                        Value::Matrix(m) => Ok(crate::plot::PlotData::Real(m)),
                        Value::Complex(m) => Ok(crate::plot::PlotData::Complex(m)),
                        _ => Err("Plot requires numeric coordinate arrays".into()),
                    }
                };
                let mut ruler_kinds: [Option<crate::plot::RulerKind>; 3] = [
                    plot_time_axes::existing_ruler_for_plot(self, 0).cloned(),
                    plot_time_axes::existing_ruler_for_plot(self, 1).cloned(),
                    plot_time_axes::existing_ruler_for_plot(self, 2).cloned(),
                ];
                let mut series = Vec::new();
                let mut constructed_points = 0usize;
                let mut index = 0;
                while index < data_args.len() {
                    if three {
                        if data_args.len() - index < 3
                            || !data_args[index..index + 3]
                                .iter()
                                .all(plot_time_axes::is_plot_data_arg)
                        {
                            return Err(format!(
                                "{name}: data must be given as X, Y, Z triplets of real numeric arrays"
                            ));
                        }
                        let (xm, xk) = plot_time_axes::extract_axis_data(
                            &data_args[index],
                            ruler_kinds[0].as_ref(),
                            "x",
                            &self.cancel,
                        )?;
                        let (ym, yk) = plot_time_axes::extract_axis_data(
                            &data_args[index + 1],
                            ruler_kinds[1].as_ref(),
                            "y",
                            &self.cancel,
                        )?;
                        let (zm, zk) = plot_time_axes::extract_axis_data(
                            &data_args[index + 2],
                            ruler_kinds[2].as_ref(),
                            "z",
                            &self.cancel,
                        )?;
                        ruler_kinds = [Some(xk), Some(yk), Some(zk)];
                        let mut batch = graphics_handles::series_from_triplet(
                            name,
                            [Ok(&xm), Ok(&ym), Ok(&zm)],
                            kind,
                        )?;
                        self.apply_line_defaults(
                            &mut batch,
                            self.figure.as_ref().map(|f| &f.identity),
                            primitive_line,
                        )?;
                        index += 3;
                        if let Some(Value::Text(spec)) = data_args.get(index) {
                            for s in &mut batch {
                                s.line_spec(spec)?;
                            }
                            index += 1;
                        }
                        if series.len() + batch.len() > crate::plot::MAX_PLOT_SERIES
                            || series
                                .iter()
                                .chain(&batch)
                                .map(|s: &crate::plot::Series| s.points.len())
                                .sum::<usize>()
                                > crate::plot::MAX_POINTS
                        {
                            return Err("Plot exceeds figure limits".into());
                        }
                        properties.apply_new_batch(&mut batch, kind, &self.cancel)?;
                        series.extend(batch);
                        continue;
                    }
                    let paired = data_args
                        .get(index + 1)
                        .is_some_and(plot_time_axes::is_plot_data_arg);
                    if has_complex && data_count > 1 && !paired {
                        return Err(
                            "Complex plots need one data array or explicit X,Y pairs".into()
                        );
                    }
                    let y_index = index + usize::from(paired);
                    let mut batch = if has_complex {
                        if let Some(ex) = &ruler_kinds[0]
                            && !ex.is_numeric()
                        {
                            return Err(format!(
                                "Values plotted against x-axis must be {} values",
                                ex.kind_word()
                            ));
                        }
                        if let Some(ex) = &ruler_kinds[1]
                            && !ex.is_numeric()
                        {
                            return Err(format!(
                                "Values plotted against y-axis must be {} values",
                                ex.kind_word()
                            ));
                        }
                        ruler_kinds[0] = Some(crate::plot::RulerKind::Numeric);
                        ruler_kinds[1] = Some(crate::plot::RulerKind::Numeric);
                        crate::plot::series_from_plot_data_with_budget(
                            if paired {
                                Some(coordinates(index)?)
                            } else {
                                None
                            },
                            coordinates(y_index)?,
                            &self.cancel,
                            crate::plot::MAX_PLOT_SERIES - series.len(),
                            crate::plot::MAX_POINTS - constructed_points,
                        )?
                    } else {
                        let xm = if paired {
                            let (xm, xk) = plot_time_axes::extract_axis_data(
                                &data_args[index],
                                ruler_kinds[0].as_ref(),
                                "x",
                                &self.cancel,
                            )?;
                            ruler_kinds[0] = Some(xk);
                            Some(xm)
                        } else {
                            if let Some(ex) = &ruler_kinds[0]
                                && !ex.is_numeric()
                            {
                                return Err(format!(
                                    "Values plotted against x-axis must be {} values",
                                    ex.kind_word()
                                ));
                            }
                            ruler_kinds[0] = Some(crate::plot::RulerKind::Numeric);
                            None
                        };
                        let (ym, yk) = plot_time_axes::extract_axis_data(
                            &data_args[y_index],
                            ruler_kinds[1].as_ref(),
                            "y",
                            &self.cancel,
                        )?;
                        ruler_kinds[1] = Some(yk);
                        let (xm, ym) = if kind == crate::plot::PlotKind::Bar
                            && matches!(
                                ruler_kinds[0],
                                Some(crate::plot::RulerKind::Categorical { .. })
                            )
                            && let Some(xm) = xm
                        {
                            let (sx, sy) = plot_time_axes::sort_bar_categorical(xm, ym)?;
                            (Some(sx), sy)
                        } else {
                            (xm, ym)
                        };
                        crate::plot::series_from_matrices(xm.as_ref(), &ym, kind)?
                    };
                    self.apply_line_defaults(
                        &mut batch,
                        self.figure.as_ref().map(|f| &f.identity),
                        primitive_line,
                    )?;
                    index += if paired { 2 } else { 1 };
                    if let Some(Value::Text(spec)) = data_args.get(index) {
                        if matches!(name, "scatter" | "bar")
                            && !(spec.len() == 1 && "rgbcmykw".contains(spec))
                        {
                            return Err(plot_properties::LINE_SPEC_UNSUPPORTED.into());
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
                    if series.len() + batch.len() > crate::plot::MAX_PLOT_SERIES
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
                            // A replacing plot3/scatter3 starts in view(3);
                            // a replacing 2-D plot resets to view(2).
                            view: if three {
                                crate::plot::view3d::VIEW_3D
                            } else {
                                crate::plot::view3d::VIEW_2D
                            },
                            ..crate::plot::Axes::default()
                        },
                        layout: self.figure.as_ref().and_then(|f| f.layout.clone()),
                        // Labels a chart placed on the replaced axes go with it.
                        textboxes: plots_2d_more::p2_kept_textboxes(self),
                        annotations: self
                            .figure
                            .as_ref()
                            .map_or_else(Vec::new, |f| f.annotations.clone()),
                        name: self
                            .figure
                            .as_ref()
                            .map_or_else(String::new, |f| f.name.clone()),
                        color: self.figure.as_ref().map_or([255; 3], |f| f.color),
                        invert_hardcopy: self.figure.as_ref().and_then(|f| f.invert_hardcopy),
                        alphamap: self.figure.as_ref().and_then(|f| f.alphamap.clone()),
                        slice_viewers: Vec::new(),
                        orthoslice_viewers: Vec::new(),
                        appearance: self
                            .figure
                            .as_ref()
                            .map_or(Some(crate::plot::FigureAppearance::default()), |f| {
                                f.appearance
                            }),
                        position: self
                            .figure
                            .as_ref()
                            .map_or(crate::plot::DEFAULT_FIGURE_POSITION, |f| f.position),
                        units: self.figure.as_ref().map_or(Default::default(), |f| f.units),
                        number_title: self.figure.as_ref().is_none_or(|f| f.number_title),
                        visible: self.figure.as_ref().is_none_or(|f| f.visible),
                        resize: self.figure.as_ref().is_none_or(|f| f.resize),
                        window_state: self
                            .figure
                            .as_ref()
                            .map_or(Default::default(), |f| f.window_state),
                    }
                };

                if let Some(old) = self.figure.as_ref().filter(|f| !f.hold) {
                    // Replacement clears the active axes, not the other tiles' viewers.
                    let id = &old.axes.identity;
                    figure.slice_viewers.clone_from(&old.slice_viewers);
                    figure
                        .slice_viewers
                        .retain(|s| !s.axes_identity.same_object(id));
                    figure
                        .orthoslice_viewers
                        .clone_from(&old.orthoslice_viewers);
                    figure
                        .orthoslice_viewers
                        .retain(|o| !o.axes_identities.iter().any(|a| a.same_object(id)));
                }
                graphics_handles::plot_carry_over(self.figure.as_ref(), &mut figure, &mut series)?;
                let command_yscale = figure.axes.yscale;
                let axes_defaults = self.graphics_plot_axes_defaults(&mut figure)?;
                if self.figure.as_ref().is_none_or(|f| !f.hold) {
                    if matches!(name, "semilogx" | "loglog") {
                        figure.axes.xscale = crate::plot::AxisScale::Log;
                    }
                    if matches!(name, "semilogy" | "loglog") {
                        figure.axes.yscale = command_yscale;
                    }
                    if three {
                        figure.axes.view = crate::plot::view3d::VIEW_3D;
                    }
                }
                let first = figure.axes.series.len();
                figure.add(series)?;
                for (axis, kind) in ruler_kinds.into_iter().enumerate() {
                    if let Some(kind) = kind {
                        figure.axes.extras.rulers[axis].kind = kind;
                    }
                }
                graphics_handles::plot_apply_color_order(&mut figure, first);
                plot_properties::refresh_scatter_clim(&mut figure.axes);
                let axes_id = figure.axes.identity.clone();
                figure.sync_linked_axes(&axes_id, true, true);
                figure.validate()?;
                let handles = self.graphics_series_result(&figure, first)?;
                self.check_cancel()?;
                self.graphics_finish_axes_defaults(&figure, axes_defaults);
                self.figure = Some(figure);
                graphics::sync_engine_linked_axes(self, &axes_id, true, true);
                return Ok(handles);
            }
            _ => {
                return Err(self.raise(
                    if econ_vol::METHODS.contains(&name) {
                        econ_vol::undefined_method(name)
                    } else {
                        crate::help::unknown_function(name)
                    },
                    "MATLAB:UndefinedFunction".into(),
                ));
            }
        };
        Ok(Value::Matrix(result))
    }
}
