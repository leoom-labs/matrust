//! Serialized model writes and disk-version checks on the existing worker.
use super::*;
use matrust::simulation::{
    Model,
    document::{DiskVersion, ModelFile, SaveOutcome},
};

pub struct Save {
    pub model: Model,
    pub file: ModelFile,
    pub path: PathBuf,
    pub confirmed: Option<DiskVersion>,
}
pub enum Request {
    Save(Box<Save>),
    Check(ModelFile),
}
pub struct Saved {
    pub file: ModelFile,
    pub outcome: SaveOutcome,
}
pub enum Finished {
    Save {
        path: PathBuf,
        confirmed: bool,
        result: matrust::Result<Box<Saved>>,
    },
    Check(matrust::Result<Option<(PathBuf, DiskVersion)>>),
}
impl Request {
    pub fn checking(&self) -> bool {
        matches!(self, Self::Check(_))
    }
    pub fn failure(&self, error: String) -> Finished {
        match self {
            Self::Save(save) => Finished::Save {
                path: save.path.clone(),
                confirmed: save.confirmed.is_some(),
                result: Err(error),
            },
            Self::Check(_) => Finished::Check(Err(error)),
        }
    }
}
pub fn perform(request: Request, cancel: &std::sync::atomic::AtomicBool) -> Finished {
    if cancel.load(Ordering::Relaxed) {
        return request.failure("Model file operation cancelled before starting".into());
    }
    match request {
        Request::Save(save) => {
            let Save {
                model,
                mut file,
                path,
                confirmed,
            } = *save;
            // Once try_save starts, report atomic publication even after Stop.
            let result = file
                .try_save(&model, &path, confirmed.as_ref())
                .map(|outcome| Box::new(Saved { file, outcome }))
                .map_err(|error| format!("{}: {error}", crate::path_label::display(&path)));
            Finished::Save {
                path,
                confirmed: confirmed.is_some(),
                result,
            }
        }
        Request::Check(file) => {
            let result = file
                .external_change()
                .map(|change| change.map(|disk| (file.path.unwrap(), disk)));
            if cancel.load(Ordering::Relaxed) {
                Finished::Check(Err("Model file check cancelled".into()))
            } else {
                Finished::Check(result)
            }
        }
    }
}
impl Workspace {
    pub(super) fn advance_model_write(&mut self) -> bool {
        let Some(request) = self.toolkits.take_model_write() else {
            return false;
        };
        let checking = request.checking();
        let failure = request.failure("Model worker unavailable".into());
        self.submit(Job::ModelWrite(Box::new(request)));
        // Only read-only operations may detach from a blocked OS call on exit.
        self.worker.opening = self.busy && checking;
        if self.busy {
            self.status = if checking {
                "Checking model file..."
            } else {
                "Saving model..."
            }
            .into();
        } else {
            self.toolkits.finish_model_write(failure, false);
        }
        true
    }
    pub(super) fn receive_model_write(&mut self, reply: Finished) {
        self.worker.opening = false;
        let cancelled = self.worker.cancel.load(Ordering::Relaxed);
        self.toolkits.finish_model_write(reply, cancelled);
        self.status = self.toolkits.message.clone();
        self.console.push(self.status.clone());
        self.recovery_dirty = true;
        self.advance_model_read();
    }
}
