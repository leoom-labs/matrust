//! 1-D discrete wavelet transform builtins (Wavelet Toolbox core): wfilters,
//! dwt, idwt, dwtmode, wavedec, waverec, appcoef, detcoef, wrcoef, wmaxlev,
//! wthresh, thselect, wden and wdenoise. Built from the public function
//! documentation and the published algorithms; expected values in
//! tests/wavelet_dwt.rs come from PyWavelets and the reference script
//! tool/refs/wavelet_dwt/wavelet_dwt.py.
//! Multiresolution analysis (modwt, imodwt, modwtmra, emd, vmd, ewt) lives in
//! mra.rs and its siblings; tests/wavelet_mra.rs, tool/refs/wavelet_mra/.
use super::{Engine, Execution, Value};
use crate::Result;
use crate::syntax::Expr;

mod builtins;
mod coeffs;
mod cwt;
mod cwt_bank;
mod cwt_kernel;
mod denoise;
mod emd;
mod ewt;
mod filters;
mod modwt;
mod mra;
mod transform;
mod vmd;

pub(super) const NAMES: &[&str] = &[
    "wfilters", "dwt", "idwt", "dwtmode", "wavedec", "waverec", "appcoef", "detcoef", "wrcoef",
    "wmaxlev", "wthresh", "thselect", "wden", "wdenoise",
];

pub(super) fn handles(name: &str) -> bool {
    NAMES.contains(&name) || mra::NAMES.contains(&name) || cwt::NAMES.contains(&name)
}

/// Calls answered ahead of the generic dispatch: a cwtfilterbank method (wt,
/// freqz, wavelets, scales, ...), chosen by its first argument because the
/// names are shared with other builtins, and icwt of the 3-D transform of a
/// complex signal (ahead of the N-D guard).
pub(super) fn method(name: &str, args: &[Value]) -> bool {
    cwt_bank::is_method(name, args) || (name == "icwt" && args.first().is_some_and(Value::is_nd))
}

/// Constructors that are also called without parentheses (`fb = cwtfilterbank`).
pub(super) fn bare(name: &str) -> bool {
    name == "cwtfilterbank"
}

/// The same test before the arguments are evaluated (multiple outputs): the
/// first argument is a filter bank variable or a cwtfilterbank(...) call.
pub(super) fn method_call(e: &Engine, name: &str, args: &[Expr]) -> bool {
    cwt_bank::METHODS.contains(&name)
        && match args.first() {
            Some(Expr::Name(v)) => e
                .container_variables
                .get(v)
                .is_some_and(cwt_bank::is_bank_array),
            Some(Expr::Call(n, _)) => n == "cwtfilterbank",
            _ => false,
        }
}

/// Tag value of the cwtfilterbank language object (see lang_more::lng_tag).
pub(super) const CWT_BANK_TAG: f64 = cwt_bank::TAG;

/// Names that print when called as a statement (no outputs).
pub(super) fn statement(name: &str) -> bool {
    name == "dwtmode"
}

/// The global extension mode of `dwtmode` (default 'sym').
#[derive(Clone, Debug, Default)]
pub(super) struct WaveletState {
    mode: transform::Mode,
}

pub(super) fn call(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    nargout: usize,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    e.check_cancel()?;
    let args = e.text_arguments(args.to_vec())?;
    if mra::NAMES.contains(&name) {
        return mra::call(e, name, &args, nargout);
    }
    if cwt::NAMES.contains(&name) || cwt_bank::is_method(name, &args) {
        return cwt::call(e, name, &args, nargout);
    }
    builtins::call(e, name, &args, nargout, out)
}
