//! Native Apply captures the selected signal before publishing coefficients.
//! All work and the shared Engine metadata clone stay on the existing worker.
use matrust::{Array, Engine, Execution, Matrix, Result, dsp, simulation::Block};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug)]
pub enum Design {
    Fir(super::fir_designer::Design),
    Iir(super::iir_designer::Design),
}

pub struct Request {
    pub design: Design,
    pub input: String,
    pub folder: PathBuf,
    pub view: Option<(u32, super::worker::HistoryView)>,
}

pub fn perform(engine: &mut Engine, request: Request) -> Result<Execution> {
    before_publish(engine, request, || {})
}

fn before_publish(
    engine: &mut Engine,
    request: Request,
    checkpoint: impl FnOnce(),
) -> Result<Execution> {
    let cancel = engine.cancellation();
    dsp::check_cancel(&cancel)?;
    if !matrust::files::valid_name(&request.input) {
        return Err("Input must be a workspace variable name".into());
    }
    // Array/Matrix clones retain the immutable column-major allocation. In
    // particular, b/sos/y is read before any candidate output is installed.
    let input = match engine.array(&request.input) {
        Some(Array::Numeric(input)) => input,
        Some(_) => return Err("Filter Designer Apply requires a real numeric input".into()),
        None => return Err(format!("Unknown workspace input '{}'", request.input)),
    };
    let (name, coefficients, result) = match request.design {
        Design::Fir(design) => {
            let Block::Filter { b, a } = design.block(&cancel)? else {
                return Err("FIR design did not produce filter coefficients".into());
            };
            let result = dsp::filter(&b, &a, &input, &cancel)?;
            let coefficients = Matrix::from_column_vec(1, b.len(), b)?;
            ("b", coefficients, result)
        }
        Design::Iir(design) => {
            let coefficients = design.coefficients(&cancel)?;
            let result = dsp::sosfilt(&coefficients, &input, &cancel)?;
            ("sos", coefficients, result)
        }
    };
    dsp::check_cancel(&cancel)?;
    // Engine already derives Clone: arrays, cell contents, function programs
    // and plot sample storage remain shared; maps/figure metadata are bounded.
    // The clone also shares this exact cancellation flag and creates no pool.
    let mut candidate = engine.clone();
    candidate.set_function_directory(&request.folder)?;
    candidate.set_file_directory(&request.folder)?;
    if let Some((id, view)) = request.view {
        if id != candidate.current_figure {
            return Err("Filter result figure changed before Apply".into());
        }
        let figure = candidate
            .figure
            .as_ref()
            .ok_or("Filter result figure no longer exists")?;
        let mut figure = super::worker::with_history_view(figure, Some(view))?;
        if view.1[0] {
            figure.xlim = None;
        }
        if view.1[1] {
            figure.ylim = None;
        }
        candidate.figure = Some(figure);
    }
    candidate.merge_workspace(BTreeMap::from([
        (name.into(), coefficients.into()),
        ("y".into(), result.into()),
    ]))?;
    // Retain ordinary current-axes/hold/layout and public plot/title dispatch.
    // Generated exported scripts remain ordinary scripts; native Apply uses
    // the same typed built-in design/filter kernels as the native block path.
    let output = candidate.execute("plot(y); title('Filtered signal');")?;
    checkpoint();
    dsp::check_cancel(&cancel)?;
    *engine = candidate;
    Ok(output)
}

#[cfg(test)]
#[path = "designer_apply_tests.rs"]
mod tests;
