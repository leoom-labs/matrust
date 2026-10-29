//! Model reads use the existing worker; decisions and publication stay on the UI.
use super::*;
use matrust::simulation::{Model, document::ModelFile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Open,
    Confirmed,
    Reload,
}
pub struct Request {
    pub path: PathBuf,
    pub purpose: Purpose,
}
pub struct Finished {
    pub purpose: Purpose,
    pub source: Option<PathBuf>,
    pub imported: bool,
    pub result: matrust::Result<(Model, ModelFile)>,
}
pub fn read(request: Request, cancel: &std::sync::atomic::AtomicBool) -> Finished {
    let imported = request
        .path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("slx"));
    let result = if cancel.load(Ordering::Relaxed) {
        Err("Model opening cancelled".into())
    } else {
        let opened = if imported {
            matrust::simulation::slx::import(&request.path, cancel)
                .map(|loaded| (loaded.model, ModelFile::imported()))
        } else {
            ModelFile::open(&request.path)
        };
        let result = opened
            .map_err(|error| format!("{}: {error}", crate::path_label::display(&request.path)));
        if cancel.load(Ordering::Relaxed) {
            Err("Model opening cancelled".into())
        } else {
            result
        }
    };
    Finished {
        purpose: request.purpose,
        source: Some(request.path),
        imported,
        result,
    }
}
impl Workspace {
    pub(super) fn advance_model_read(&mut self) {
        // Capture queued-open intent before an unrelated worker can finish.
        if self.toolkits.model_read_pending() {
            self.document_save.model_read_started();
        }
        if self.busy {
            return;
        }
        if self.advance_model_write() {
            return;
        }
        let Some(request) = self.toolkits.take_model_read() else {
            return;
        };
        self.submit(Job::ReadModel(request));
        self.worker.opening = self.busy;
        if self.busy {
            self.status = "Opening block model...".into();
        } else {
            self.toolkits.finish_model_read(Finished {
                purpose: self.toolkits.read_purpose(),
                source: None,
                imported: false,
                result: Err(self.status.clone()),
            });
            self.document_save.model_read_completed(false);
        }
    }
    pub(super) fn receive_model_read(&mut self, mut reply: Finished) {
        self.worker.opening = false;
        if self.worker.cancel.load(Ordering::Relaxed) {
            reply.result = Err("Model opening cancelled; local work retained".into());
        }
        let success = reply.result.is_ok();
        let selected = self.toolkits.selected_tool();
        self.toolkits.finish_model_read(reply);
        // Work the user moved to meanwhile keeps the main area; the tab is added.
        if !self.document_save.model_read_completed(success) && success {
            match selected {
                Some(tool) => self.toolkits.select_tool(tool),
                None => self.toolkits.deselect_tool(),
            }
        }
        self.status = self.toolkits.message.clone();
        if success {
            self.recovery_dirty = true;
        } else {
            self.console.push(self.status.clone());
        }
        self.advance_model_read();
    }
}
