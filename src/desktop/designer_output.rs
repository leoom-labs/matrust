//! Filter Designer outputs that leave the app: the saved script (written on the
//! worker) and the block-model block built from the designed coefficients.
//! Workspace export and Apply are ordinary scripts (`Action::Script`).
use super::filter_spec::{Coefficients, Spec};
use matrust::{Result, simulation::Block};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

/// Save the generated script to a file.
pub struct Save {
    pub script: String,
    pub path: PathBuf,
    pub volume: Option<matrust::Array>,
    pub labels: Option<(String, matrust::Array)>,
}

pub fn save(request: Save, cancel: &AtomicBool) -> Result<PathBuf> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Filter operation cancelled".into());
    }
    if request.volume.is_some() || request.labels.is_some() {
        return save_volume_script(
            request.script,
            request.path,
            request.volume,
            request.labels,
            cancel,
        );
    }
    // Entering atomic_write starts a noncancellable write/sync/rename section;
    // a completed publication is reported even after a late Stop.
    matrust::files::atomic_write(&request.path, request.script.as_bytes())?;
    Ok(request.path)
}

fn save_volume_script(
    script: String,
    path: PathBuf,
    volume: Option<matrust::Array>,
    labels: Option<(String, matrust::Array)>,
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let name = format!("volume_{}.mat", uuid::Uuid::new_v4().simple());
    let companion = path.with_file_name(&name);
    let (header, body) = script.split_at(script.split_inclusive('\n').take(2).map(str::len).sum());
    let mut vars = std::collections::BTreeMap::new();
    let mut load_names = Vec::new();
    if let Some(volume) = volume {
        vars.insert("vv_volume".into(), volume);
        load_names.push("'vv_volume'".to_owned());
    }
    if let Some((label_name, label_array)) = labels {
        load_names.push(format!("'{label_name}'"));
        vars.insert(
            label_name,
            crate::apps::volume_viewer::portable_label_array(label_array, cancel)?,
        );
    }
    let load_args = load_names.join(", ");
    let source = format!(
        "{header}% Keep this script and {name} together.\n\
         load(fullfile(fileparts(mfilename('fullpath')), '{name}'), {load_args});\n{body}"
    );
    matrust::files::save_array_workspace_uncompressed(&companion, &vars, cancel)?;
    let result = matrust::dsp::check_cancel(cancel)
        .and_then(|()| matrust::files::atomic_write(&path, source.as_bytes()));
    if let Err(error) = result {
        let _ = std::fs::remove_file(companion);
        return Err(error);
    }
    Ok(path)
}

/// Block-model block for designed coefficients: a Filter block for direct
/// form, an SOS Filter block (gain folded into section 1) for sections.
pub fn block(coefficients: &Coefficients) -> Result<Block> {
    match coefficients {
        Coefficients::Tf { b, a } => {
            if a.first().is_none_or(|a0| *a0 == 0.) {
                return Err("The design has no valid denominator".into());
            }
            Ok(Block::Filter {
                b: b.clone(),
                a: a.clone(),
            })
        }
        Coefficients::Sos { sos, g } => {
            let mut sections = sos.clone();
            let first = sections
                .first_mut()
                .ok_or("The design returned no sections")?;
            for v in &mut first[..3] {
                *v *= g;
            }
            Ok(Block::SosFilter { sections })
        }
    }
}

impl crate::Workspace {
    pub(super) fn submit_design_save(&mut self, spec: &Spec, path: PathBuf) {
        if self.busy {
            return;
        }
        let script = match spec.script() {
            Ok(script) => script,
            Err(error) => {
                self.status = error.clone();
                self.toolkits.message = error;
                return;
            }
        };
        if let Err(error) = self.toolkits.begin_design_save(spec.title()) {
            self.status = error.clone();
            self.toolkits.message = error;
            return;
        }
        self.submit(crate::worker::Job::SaveDesign(Box::new(Save {
            script,
            path,
            volume: None,
            labels: None,
        })));
        if self.busy {
            self.status = self.toolkits.message.clone();
        } else {
            self.toolkits.finish_design_save(Err(self.status.clone()));
        }
    }
    pub(super) fn complete_design_save(&mut self, result: Result<PathBuf>) {
        // Apps save their generated scripts through this same job.
        if let Some(kind) = self.toolkits.apps.take_saving() {
            self.complete_app_script_save(kind, result);
            return;
        }
        self.toolkits.finish_design_save(result);
        self.status = self.toolkits.message.clone();
    }
}
