//! Prepared desktop exports. Encoding and destination review run only on the worker.
use super::{
    export_batch::{Stamp, stamp},
    image_export,
    worker::ExportRequest,
};
use matrust::{Result, files, plot::BaseTheme};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub enum Input {
    Preview(image_export::Request),
    Figure(Box<ExportRequest>),
}
pub enum Action {
    Prepare(Input),
    Commit(Plan),
}
pub struct Request {
    pub ticket: u64,
    pub action: Action,
}
pub struct Finished {
    pub ticket: u64,
    pub result: Result<Outcome>,
}
pub enum Outcome {
    Review(Plan),
    Saved(PathBuf),
}
enum Encoded {
    File(tempfile::TempPath),
    Script(files::PreparedMatlabPlotScript),
}
pub struct Plan {
    path: PathBuf,
    expected: Option<Stamp>,
    source: Option<PathBuf>,
    encoded: Encoded,
}
impl Plan {
    pub fn destination(&self) -> &Path {
        &self.path
    }
    pub fn replaces(&self) -> bool {
        self.expected.is_some()
    }
    fn verify(&self, cancel: &AtomicBool) -> Result<()> {
        if let Some(source) = &self.source {
            image_export::different_destination(source, &self.path)?;
        }
        if stamp(&self.path, cancel)? != self.expected {
            return Err(
                "Destination changed after export preparation; choose the destination again".into(),
            );
        }
        super::image_load::check(cancel)
    }
}
pub fn perform(request: Request, theme: BaseTheme, cancel: &AtomicBool) -> Finished {
    let result = match request.action {
        Action::Prepare(input) => prepare(input, theme, cancel).and_then(|plan| {
            if plan.replaces() {
                Ok(Outcome::Review(plan))
            } else {
                commit(plan, cancel).map(Outcome::Saved)
            }
        }),
        Action::Commit(plan) => commit(plan, cancel).map(Outcome::Saved),
    };
    Finished {
        ticket: request.ticket,
        result,
    }
}
fn destination(path: &Path) -> Result<PathBuf> {
    let name = path.file_name().ok_or("Choose an export file name")?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent.canonicalize().map_err(|e| e.to_string())?.join(name))
}
pub fn prepare(input: Input, theme: BaseTheme, cancel: &AtomicBool) -> Result<Plan> {
    prepare_checked(input, theme, cancel, || {})
}
fn prepare_checked(
    input: Input,
    theme: BaseTheme,
    cancel: &AtomicBool,
    after_encode: impl FnOnce(),
) -> Result<Plan> {
    super::image_load::check(cancel)?;
    let selected = match &input {
        Input::Preview(r) => &r.path,
        Input::Figure(r) => &r.path,
    };
    let path = destination(selected)?;
    let expected = stamp(&path, cancel)?;
    let source = match &input {
        Input::Preview(r) => Some(r.source.clone()),
        Input::Figure(_) => None,
    };
    if let Some(source) = &source {
        image_export::different_destination(source, &path)?;
    }
    let encoded = if let Input::Figure(request) = &input
        && path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("m"))
    {
        Encoded::Script(files::prepare_matlab_plot_reconstruction_with_theme(
            &path,
            &request.figure,
            theme,
            cancel,
        )?)
    } else {
        let suffix = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or("Choose a supported export extension")?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".matrust-export-")
            .suffix(&format!(".{suffix}"))
            .tempfile_in(path.parent().ok_or("Missing destination folder")?)
            .map_err(|e| e.to_string())?;
        match &input {
            Input::Preview(request) => {
                let bytes = image_export::encode(request, cancel)?;
                for chunk in bytes.chunks(65536) {
                    super::image_load::check(cancel)?;
                    temporary.write_all(chunk).map_err(|e| e.to_string())?;
                }
                temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            }
            Input::Figure(request) => {
                // Codecs use their established atomic writer. Close the original
                // temp handle first, as batch export does, to allow Windows rename.
                let temporary = temporary.into_temp_path();
                if path
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("csv"))
                {
                    files::write_figure_csv_with_cancel(&temporary, &request.figure, cancel)?;
                } else if let Some(canvas) = request.vector {
                    matrust::export::export_vector_in_theme_with_cancel(
                        &request.figure,
                        &temporary,
                        canvas,
                        theme,
                        cancel,
                    )?;
                } else {
                    matrust::export::export_figure_in_theme_with_cancel(
                        &request.figure,
                        &temporary,
                        [request.width, request.height],
                        theme,
                        cancel,
                    )?;
                }
                let plan = Plan {
                    path,
                    expected,
                    source,
                    encoded: Encoded::File(temporary),
                };
                after_encode();
                plan.verify(cancel)?;
                return Ok(plan);
            }
        }
        Encoded::File(temporary.into_temp_path())
    };
    let plan = Plan {
        path,
        expected,
        source,
        encoded,
    };
    after_encode();
    plan.verify(cancel)?;
    Ok(plan)
}
pub fn commit(plan: Plan, cancel: &AtomicBool) -> Result<PathBuf> {
    match &plan.encoded {
        Encoded::File(_) => {
            // publish_checked performs the final complete destination stamp.
            // Keep the source-alias check before that guard without reading twice.
            if let Some(source) = &plan.source {
                image_export::different_destination(source, &plan.path)?;
            }
        }
        Encoded::Script(_) => plan.verify(cancel)?,
    }
    let Plan {
        path,
        expected,
        source: _,
        encoded,
    } = plan;
    match encoded {
        Encoded::File(temporary) => super::export_batch::publish_checked(
            &path,
            &expected,
            temporary,
            cancel,
            "Destination changed after export preparation; choose the destination again",
        )?,
        Encoded::Script(script) => {
            script.publish(expected.is_some(), cancel, || {
                if stamp(&path, cancel)? != expected {
                    return Err("Destination changed after export preparation; choose the destination again".into());
                }
                Ok(())
            })?;
        }
    }
    Ok(path)
}
#[cfg(test)]
#[path = "single_export_tests.rs"]
mod tests;
