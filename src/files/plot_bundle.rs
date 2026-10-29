//! Compact ordinary MATLAB reconstruction plus an immutable genuine MAT generation.
//! The original inline writer remains available with its existing 64 MiB limit.
use crate::{Array, CharacterRow, Matrix, Result, plot::Figure};
use std::{
    collections::BTreeMap,
    fmt,
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// Keep the script and this companion directory together when copying/moving.
/// Old successful generations are retained: another script may still use them.
pub struct MatlabPlotBundle {
    pub script: PathBuf,
    pub companion_directory: PathBuf,
    pub data: PathBuf,
}
pub(super) struct Data {
    arrays: BTreeMap<String, Array>,
    texts: BTreeMap<String, String>,
}
impl Data {
    pub(super) fn text(&self, value: &str) -> std::result::Result<String, fmt::Error> {
        self.texts.get(value).cloned().ok_or(fmt::Error)
    }
    fn add_text(&mut self, value: &str, cancel: &AtomicBool) -> Result<()> {
        if !self.texts.contains_key(value) {
            let name = format!("mr_text_{}", self.texts.len() + 1);
            self.arrays.insert(
                name.clone(),
                Array::Character(CharacterRow::from_text(value, cancel)?),
            );
            self.texts.insert(value.to_owned(), name);
        }
        Ok(())
    }
    fn new(figure: &Figure, cancel: &AtomicBool) -> Result<Self> {
        let mut data = Self {
            arrays: BTreeMap::new(),
            texts: BTreeMap::new(),
        };
        data.add_text(&figure.name, cancel)?;
        if let Some(layout) = &figure.layout {
            data.add_text(&layout.title, cancel)?;
        }
        let mut index = 0;
        for (_, axes) in figure.all_axes() {
            for label in [&axes.title, &axes.xlabel, &axes.ylabel] {
                data.add_text(label, cancel)?;
            }
            for series in &axes.series {
                crate::dsp::check_cancel(cancel)?;
                index += 1;
                data.add_text(&series.label, cancel)?;
                if let Some(coordinates) =
                    series.points.patch().and_then(|p| p.script_coordinates())
                {
                    if coordinates.corners.iter().flatten().any(|c| {
                        matches!(c, crate::plot::coordinates::ScriptCoordinate::Constant(_))
                    }) {
                        let values = (0..2)
                            .flat_map(|axis| {
                                coordinates
                                    .corners
                                    .iter()
                                    .map(move |corner| match corner[axis] {
                                        crate::plot::coordinates::ScriptCoordinate::Constant(
                                            value,
                                        ) => value,
                                        _ => 0.,
                                    })
                            })
                            .collect();
                        data.arrays.insert(
                            format!("mr_constants_{index}"),
                            Array::Numeric(Matrix::from_column_vec(4, 2, values)?),
                        );
                    }
                    if coordinates.has_steps() {
                        data.arrays.insert(
                            format!("mr_steps_{index}"),
                            Array::Numeric(Matrix::from_column_vec(
                                4,
                                4,
                                coordinates.step_operands(),
                            )?),
                        );
                    }
                    for (source, backing) in coordinates.sources.iter().enumerate() {
                        data.arrays.insert(
                            format!("mr_coordinates_{index}_{}", source + 1),
                            Array::Numeric(backing.matrix(cancel)?),
                        );
                    }
                    continue;
                }
                // Model storage is shared interleaved XY. MAT requires column-major
                // values; one bounded conversion copy (<=16 MiB across the figure)
                // preserves original sample bits and does not mutate any aliases.
                let mut values = Vec::new();
                values
                    .try_reserve_exact(
                        series
                            .points
                            .len()
                            .checked_mul(2)
                            .ok_or("Sample size overflow")?,
                    )
                    .map_err(|_| "Cannot allocate plot companion samples")?;
                for column in 0..2 {
                    for (index, point) in series.points.iter().enumerate() {
                        if index.is_multiple_of(1024) {
                            crate::dsp::check_cancel(cancel)?;
                        }
                        values.push(point[column]);
                    }
                }
                data.arrays.insert(
                    format!("mr_samples_{index}"),
                    Array::Numeric(Matrix::from_column_vec(series.points.len(), 2, values)?),
                );
            }
        }
        for textbox in &figure.textboxes {
            data.add_text(&textbox.text, cancel)?;
        }
        crate::dsp::check_cancel(cancel)?;
        Ok(data)
    }
}

/// Save a compact .m script and a uniquely named sibling MAT-data directory.
/// Parser/editor limits remain unchanged. Compact Patch reconstruction retains
/// original coordinate sources and ordered operations in the shared geometry
/// bank; ordinary Bar output reaches that bank through public matrix operations.
/// Publication is script-last. A failed attempt removes only its own unpublished
/// generation; it never removes data referenced by an older successful script.
pub fn write_matlab_plot_bundle_with_cancel(
    path: &Path,
    figure: &Figure,
    cancel: &AtomicBool,
) -> Result<MatlabPlotBundle> {
    write_matlab_plot_bundle_with_theme(path, figure, crate::plot::BaseTheme::Light, cancel)
}
pub fn write_matlab_plot_bundle_with_theme(
    path: &Path,
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<MatlabPlotBundle> {
    prepare_bundle(path, figure, theme, cancel)?
        .publish(true, cancel, || Ok(()))?
        .ok_or_else(|| "Missing companion generation".into())
}
#[cfg(test)]
fn write_checked(
    path: &Path,
    figure: &Figure,
    cancel: &AtomicBool,
    mut before_publication: impl FnMut(&Path) -> Result<()>,
) -> Result<MatlabPlotBundle> {
    let prepared = prepare_bundle(path, figure, crate::plot::BaseTheme::Light, cancel)?;
    before_publication(
        &prepared
            .generation
            .as_ref()
            .unwrap()
            .path()
            .join("data.mat"),
    )?;
    prepared
        .publish(true, cancel, || Ok(()))?
        .ok_or_else(|| "Missing companion generation".into())
}

/// Owns an unpublished script and any fresh companion generation. Dropping this
/// value removes only that attempt. Publication consumes it; there is no exposed
/// temporary path which could outlive generation ownership.
pub struct PreparedMatlabPlotScript {
    path: PathBuf,
    staged: tempfile::NamedTempFile,
    generation: Option<tempfile::TempDir>,
}
impl PreparedMatlabPlotScript {
    /// `before_publication` runs after staging, before the final cancellation
    /// checkpoint and atomic rename. Callers can retain destination-version
    /// checks. `replace=false` uses atomic no-clobber publication.
    pub fn publish(
        self,
        replace: bool,
        cancel: &AtomicBool,
        before_publication: impl FnOnce() -> Result<()>,
    ) -> Result<Option<MatlabPlotBundle>> {
        crate::dsp::check_cancel(cancel)?;
        before_publication()?;
        crate::dsp::check_cancel(cancel)?;
        if replace {
            super::persist_atomic_checked(
                self.staged,
                &self.path,
                |_| {},
                || crate::dsp::check_cancel(cancel),
            )?;
        } else {
            self.staged
                .persist_noclobber(&self.path)
                .map_err(|error| error.error.to_string())?;
        }
        Ok(self.generation.map(|generation| {
            let companion_directory = generation.keep();
            let data = companion_directory.join("data.mat");
            MatlabPlotBundle {
                script: self.path,
                companion_directory,
                data,
            }
        }))
    }
}

pub(super) fn stage_text(
    path: &Path,
    text: &str,
    generation: Option<tempfile::TempDir>,
    cancel: &AtomicBool,
) -> Result<PreparedMatlabPlotScript> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    for chunk in text.as_bytes().chunks(65536) {
        crate::dsp::check_cancel(cancel)?;
        staged.write_all(chunk).map_err(|e| e.to_string())?;
    }
    staged.as_file().sync_all().map_err(|e| e.to_string())?;
    Ok(PreparedMatlabPlotScript {
        path: path.to_owned(),
        staged,
        generation,
    })
}

/// Preserve small inline scripts byte-for-byte. If ordinary source admission
/// fails, store original samples/text in a sibling MAT generation and parse the
/// compact reconstruction with the same limits. Neither route executes code.
/// This guarantees source admission, not all graphics replay capacities.
pub fn prepare_matlab_plot_reconstruction(
    path: &Path,
    figure: &Figure,
    cancel: &AtomicBool,
) -> Result<PreparedMatlabPlotScript> {
    prepare_matlab_plot_reconstruction_with_theme(
        path,
        figure,
        crate::plot::BaseTheme::Light,
        cancel,
    )
}
pub fn prepare_matlab_plot_reconstruction_with_theme(
    path: &Path,
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<PreparedMatlabPlotScript> {
    crate::dsp::check_cancel(cancel)?;
    super::validate_matlab_plot_script_path(path)?;
    figure.validate()?;
    super::plot_script::baseline_replay_preflight(figure, cancel)?;
    if let Some(script) = super::plot_script::replay_inline(figure, theme, cancel)? {
        crate::dsp::check_cancel(cancel)?;
        return stage_text(path, &script, None, cancel);
    }
    crate::dsp::check_cancel(cancel)?;
    prepare_bundle(path, figure, theme, cancel)
}

pub fn write_matlab_plot_reconstruction_with_cancel(
    path: &Path,
    figure: &Figure,
    cancel: &AtomicBool,
) -> Result<Option<MatlabPlotBundle>> {
    write_matlab_plot_reconstruction_with_theme(path, figure, crate::plot::BaseTheme::Light, cancel)
}
pub fn write_matlab_plot_reconstruction_with_theme(
    path: &Path,
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<Option<MatlabPlotBundle>> {
    prepare_matlab_plot_reconstruction_with_theme(path, figure, theme, cancel)?.publish(
        true,
        cancel,
        || Ok(()),
    )
}

pub(super) fn validate_loading_name(path: &Path) -> Result<()> {
    let stem = path
        .file_stem()
        .and_then(|p| p.to_str())
        .ok_or("Invalid script filename")?;
    if ["load", "clear", "mfilename", "fileparts", "fullfile"]
        .iter()
        .any(|n| stem.eq_ignore_ascii_case(n))
    {
        return Err(
            "Choose a script filename that does not shadow its file-loading functions".into(),
        );
    }
    Ok(())
}

fn prepare_bundle(
    path: &Path,
    figure: &Figure,
    theme: crate::plot::BaseTheme,
    cancel: &AtomicBool,
) -> Result<PreparedMatlabPlotScript> {
    crate::dsp::check_cancel(cancel)?;
    super::validate_matlab_plot_script_path(path)?;
    validate_loading_name(path)?;
    figure.validate()?;
    super::plot_script::baseline_replay_preflight(figure, cancel)?;
    let data = Data::new(figure, cancel)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let generation = tempfile::Builder::new()
        .prefix("mr_plot_")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let relative = generation
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid companion directory")?;
    // tempfile's fixed ASCII prefix/random suffix is data, never MATLAB syntax.
    if !relative
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("Invalid generated companion directory".into());
    }
    let mut script = super::plot_script::ReplayText(String::new());
    script
        .0
        .push_str(&super::bundle_relocation::header(relative)?);
    writeln!(&mut script, "% Keep this script and {relative}/data.mat together.\nload(fullfile(fileparts(mfilename('fullpath')),'{relative}','data.mat'));")
        .map_err(|_| "Cannot allocate compact reconstruction script")?;
    super::plot_script::encode(&mut script, figure, theme, cancel, Some(&data)).map_err(|_| {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            "Cancelled"
        } else {
            "Compact reconstruction exceeds the supported script source size or cannot be encoded"
        }
    })?;
    // Do not relax source/token/node bounds to admit generated code.
    crate::syntax::parse(&script.0)?;
    crate::dsp::check_cancel(cancel)?;
    let data_path = generation.path().join("data.mat");
    super::save_array_workspace(&data_path, &data.arrays, cancel)?;
    super::bundle_relocation::write_manifest(generation.path(), relative, &script.0, cancel)?;
    crate::dsp::check_cancel(cancel)?;
    stage_text(path, &script.0, Some(generation), cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn cancelled_after_data_staging_keeps_previous_pair_and_cleans_only_new_generation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plot_bundle.m");
        let cancel = AtomicBool::new(false);
        let mut figure = Figure::default();
        figure
            .add(vec![crate::plot::Series::new(
                vec![[1., 2.]],
                crate::plot::PlotKind::Line,
            )])
            .unwrap();
        let old = write_matlab_plot_bundle_with_cancel(&path, &figure, &cancel).unwrap();
        let script = std::fs::read(&path).unwrap();
        let bytes = std::fs::read(&old.data).unwrap();
        let entries = || {
            std::fs::read_dir(directory.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect::<std::collections::BTreeSet<_>>()
        };
        let before = entries();
        let mut reached = false;
        let error = write_checked(&path, &figure, &cancel, |new_data| {
            assert!(new_data.is_file());
            assert_ne!(new_data, old.data.as_path());
            reached = true;
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        })
        .err()
        .unwrap();
        assert!(reached);
        assert_eq!(error, "Calculation cancelled");
        assert_eq!(entries(), before);
        assert_eq!(std::fs::read(&path).unwrap(), script);
        assert_eq!(std::fs::read(&old.data).unwrap(), bytes);
        cancel.store(false, Ordering::Relaxed);
        figure.title = "Replacement".into();
        let new = write_matlab_plot_bundle_with_cancel(&path, &figure, &cancel).unwrap();
        assert_ne!(new.companion_directory, old.companion_directory);
        assert!(new.data.is_file());
        assert_eq!(std::fs::read(&old.data).unwrap(), bytes);
        assert_ne!(std::fs::read(&path).unwrap(), script);
    }
}
