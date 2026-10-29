//! Bounded, sequential exports using the existing codecs and shared figure data.
use matrust::{Result, export, files, plot::Figure, session::ExportSettings};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

const MAX_EXISTING_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Stamp {
    bytes: u64,
    hash: blake3::Hash,
}

#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub figure: Figure,
    expected: Option<Stamp>,
}
impl Item {
    pub fn replaces(&self) -> bool {
        self.expected.is_some()
    }
}

#[derive(Clone)]
pub struct Plan {
    pub folder: PathBuf,
    pub items: Vec<Item>,
    pub settings: ExportSettings,
    pub theme: matrust::plot::BaseTheme,
}
pub struct Prepare {
    pub folder: PathBuf,
    pub figures: Vec<(u32, Figure)>,
    pub settings: ExportSettings,
}
pub struct Run {
    pub plan: Plan,
    pub completed: usize,
    pub replace: bool,
    pub progress: Arc<AtomicUsize>,
}
pub struct Outcome {
    pub completed: usize,
    pub error: Option<String>,
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
pub(super) fn stamp(path: &Path, cancel: &AtomicBool) -> Result<Option<Stamp>> {
    check(cancel)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !metadata.file_type().is_file() {
        return Err("Destination must be a regular file, not a directory or symbolic link".into());
    }
    if metadata.len() > MAX_EXISTING_BYTES {
        return Err("Existing destination exceeds the 1 GiB review limit".into());
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut bytes = 0;
    loop {
        check(cancel)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_EXISTING_BYTES {
            return Err("Destination grew beyond the 1 GiB review limit".into());
        }
        hash.update(&buffer[..count]);
    }
    if bytes != metadata.len() {
        return Err("Destination changed while being reviewed; choose the folder again".into());
    }
    Ok(Some(Stamp {
        bytes,
        hash: hash.finalize(),
    }))
}
fn validate(settings: ExportSettings, figures: impl Iterator<Item = Figure>) -> Result<()> {
    if settings.format.raster() {
        export::validate_size(settings.width, settings.height)?;
    }
    if settings.format.vector() {
        settings.vector.validate()?;
    }
    for figure in figures {
        figure.validate()?;
    }
    Ok(())
}
pub fn prepare(request: Prepare, cancel: &AtomicBool) -> Result<Plan> {
    check(cancel)?;
    if request.figures.is_empty() || request.figures.len() > 16 {
        return Err("Export between 1 and 16 open figures".into());
    }
    let mut ids = BTreeSet::new();
    if request.figures.iter().any(|(id, _)| !ids.insert(*id)) {
        return Err("Duplicate figure number in export".into());
    }
    validate(
        request.settings,
        request.figures.iter().map(|(_, f)| f.clone()),
    )?;
    let folder = fs::canonicalize(&request.folder).map_err(|e| e.to_string())?;
    if !folder.is_dir() {
        return Err("Choose an existing destination folder".into());
    }
    let items = request
        .figures
        .into_iter()
        .map(|(id, figure)| {
            let path = folder.join(request.settings.format.file_name(id));
            if request.settings.format == matrust::session::ExportFormat::MatlabScript {
                files::validate_matlab_plot_script_path(&path)?;
            }
            let expected = stamp(&path, cancel)
                .map_err(|e| format!("{}: {e}", crate::path_label::display(&path)))?;
            Ok(Item {
                path,
                figure,
                expected,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Plan {
        folder,
        items,
        settings: request.settings,
        theme: matrust::plot::BaseTheme::Light,
    })
}
pub fn encode(
    figure: &Figure,
    path: &Path,
    settings: ExportSettings,
    theme: matrust::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<()> {
    check(cancel)?;
    if settings.format == matrust::session::ExportFormat::Csv {
        files::write_figure_csv_with_cancel(path, figure, cancel)
    } else if settings.format == matrust::session::ExportFormat::MatlabScript {
        files::write_matlab_plot_reconstruction_with_theme(path, figure, theme, cancel).map(|_| ())
    } else if settings.format.vector() {
        export::export_vector_in_theme_with_cancel(figure, path, settings.vector, theme, cancel)
    } else {
        export::export_figure_in_theme_with_cancel(
            figure,
            path,
            [settings.width, settings.height],
            theme,
            cancel,
        )
    }
}
fn publish(item: &Item, temporary: tempfile::TempPath, cancel: &AtomicBool) -> Result<()> {
    publish_checked(
        &item.path,
        &item.expected,
        temporary,
        cancel,
        "Destination changed after review; choose the folder again before replacing it",
    )
}
pub(super) fn publish_checked(
    path: &Path,
    expected: &Option<Stamp>,
    temporary: tempfile::TempPath,
    cancel: &AtomicBool,
    changed_destination: &'static str,
) -> Result<()> {
    if &stamp(path, cancel)? != expected {
        return Err(changed_destination.into());
    }
    check(cancel)?;
    // Missing destinations have an atomic no-clobber guarantee. Replacement is
    // best-effort content checking, not a cross-process lock; another process can
    // still write between the final check and rename.
    let result = if expected.is_some() {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    };
    result.map_err(|e| e.error.to_string())
}
pub fn run(request: Run, cancel: &AtomicBool, repaint: impl Fn()) -> Outcome {
    let mut completed = request.completed;
    let result = (|| {
        check(cancel)?;
        if request.plan.items.is_empty()
            || request.plan.items.len() > 16
            || completed > request.plan.items.len()
        {
            return Err("Invalid export progress".into());
        }
        validate(
            request.plan.settings,
            request
                .plan
                .items
                .iter()
                .skip(completed)
                .map(|item| item.figure.clone()),
        )?;
        if !request.replace
            && request
                .plan
                .items
                .iter()
                .skip(completed)
                .any(Item::replaces)
        {
            return Err("Confirm replacement of existing files before exporting".into());
        }
        request.progress.store(completed, Ordering::Relaxed);
        for item in request.plan.items.iter().skip(request.completed) {
            let result = (|| {
                check(cancel)?;
                if request.plan.settings.format == matrust::session::ExportFormat::MatlabScript {
                    let prepared = files::prepare_matlab_plot_reconstruction_with_theme(
                        &item.path,
                        &item.figure,
                        request.plan.theme,
                        cancel,
                    )?;
                    prepared.publish(item.replaces(), cancel, || {
                        if stamp(&item.path, cancel)? != item.expected {
                            return Err("Destination changed after review; choose the folder again before replacing it".into());
                        }
                        Ok(())
                    })?;
                    return Ok(());
                }
                // Each output is encoded once, closed and flushed by the existing
                // codec before publication. No second encoded buffer or file copy.
                let temporary = tempfile::Builder::new()
                    .prefix(".matrust-export-")
                    .suffix(&format!(
                        ".{}",
                        request.plan.settings.format.extensions()[0]
                    ))
                    .tempfile_in(&request.plan.folder)
                    .map_err(|e| e.to_string())?
                    .into_temp_path();
                encode(
                    &item.figure,
                    &temporary,
                    request.plan.settings,
                    request.plan.theme,
                    cancel,
                )?;
                publish(item, temporary, cancel)
            })();
            result.map_err(|e| format!("{}: {e}", crate::path_label::display(&item.path)))?;
            completed += 1;
            request.progress.store(completed, Ordering::Relaxed);
            repaint();
        }
        Ok(())
    })();
    Outcome {
        completed,
        error: result.err(),
    }
}

#[cfg(test)]
#[path = "export_batch_tests.rs"]
mod tests;
