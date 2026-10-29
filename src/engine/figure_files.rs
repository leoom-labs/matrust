//! Script figure files report exact file identity to desktop document tracking.
use super::{Engine, Value};
use crate::{
    Matrix, Result,
    files::figure_version::{self, SaveOutcome, Version},
    plot::{Figure, SnapshotStamp},
};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct FigureFileEvent {
    pub id: u32,
    pub path: PathBuf,
    pub version: Version,
    pub opened: bool,
    stamp: SnapshotStamp,
}
impl FigureFileEvent {
    /// True only for the same metadata and immutable sample allocations.
    pub fn matches(&self, figure: &Figure) -> bool {
        self.stamp.matches(figure)
    }
}

impl Engine {
    /// Drain successful file operations from the last script, including work
    /// completed before a later error. One event per live figure; no retained samples.
    pub fn take_figure_file_events(&mut self) -> Vec<FigureFileEvent> {
        std::mem::take(&mut self.figure_file_events)
            .into_values()
            .collect()
    }

    pub(super) fn figure_file_command(&mut self, name: &str, args: Vec<Value>) -> Result<Value> {
        self.check_cancel()?;
        if name == "gcf" {
            if !args.is_empty() {
                return Err("gcf takes no arguments".into());
            }
            self.select_figure(self.current_figure)?;
            return Ok(Value::Matrix(Matrix::scalar(self.current_figure as f64)));
        }
        let text = |value: &Value| -> Result<String> {
            match value {
                Value::Text(value) => Ok(value.clone()),
                _ => Err("Figure filename must be quoted text".into()),
            }
        };
        let (id, filename) = match (name, args.as_slice()) {
            ("savefig", []) => (self.current_figure, "Untitled.fig".into()),
            ("savefig", [filename]) => (self.current_figure, text(filename)?),
            ("savefig", [id, filename]) => {
                let id = id.clone().matrix(&self.cancel)?.as_scalar()?;
                if !id.is_finite() || id.fract() != 0. || !(1. ..=1_000_000.).contains(&id) {
                    return Err("Figure number must be an integer in 1..1000000".into());
                }
                (id as u32, text(filename)?)
            }
            ("openfig", [filename]) => (0, text(filename)?),
            ("openfig", [filename, Value::Text(option)]) if option == "new" => (0, text(filename)?),
            _ => return Err("Use savefig(filename), savefig(number,filename), or openfig(filename[, 'new']); figure arrays, format versions, reuse and visibility options are not supported yet".into()),
        };
        let path = self.file_path(&filename, ".fig")?;
        if !path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
            ["fig", "mrfig", "mfig"]
                .iter()
                .any(|e| v.eq_ignore_ascii_case(e))
        }) {
            return Err(
                "Editable figures require .fig or the MatRust .mrfig (or .mfig) extension".into(),
            );
        }
        if name == "savefig" {
            let figure = self
                .figures()
                .find(|(number, _)| *number == id)
                .map(|(_, figure)| figure)
                .ok_or("Figure does not exist; create or select a figure first")?;
            // An explicit script save replaces its destination. Subsequent GUI
            // saves use this exact version for their normal disk-conflict checks.
            let SaveOutcome::Saved(version) = figure_version::save_with_theme(
                &path,
                figure,
                None,
                self.graphics_theme_context,
                &self.cancel,
            )?
            else {
                return Err("Unexpected figure save conflict".into());
            };
            let stamp = SnapshotStamp::new(figure);
            self.figure_file_events.insert(
                id,
                FigureFileEvent {
                    id,
                    path,
                    version,
                    opened: false,
                    stamp,
                },
            );
            Ok(Value::Void)
        } else {
            let ids = self.figure_ids();
            if ids.len() >= 16 {
                return Err("At most 16 figures can be open; close unused figures".into());
            }
            let (figure, version) = figure_version::load(&path, &self.cancel)?;
            self.check_cancel()?;
            let id = (1..=17)
                .find(|id| !ids.contains(id))
                .ok_or("No figure number available")?;
            let stamp = SnapshotStamp::new(&figure);
            let result = crate::GraphicsArray::scalar(
                crate::GraphicsHandle::figure(&figure.identity),
                &self.cancel,
            )?;
            self.check_cancel()?;
            self.select_figure(id)?;
            self.figure = Some(figure);
            self.figure_file_events.insert(
                id,
                FigureFileEvent {
                    id,
                    path,
                    version,
                    opened: true,
                    stamp,
                },
            );
            Ok(Value::Container(result.into()))
        }
    }
}
