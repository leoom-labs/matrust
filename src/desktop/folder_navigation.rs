//! Bounded navigation history; commit only after background resolution and scanning succeed.
use super::*;

#[derive(Default)]
pub struct Navigation {
    paths: Vec<PathBuf>,
    index: usize,
    pub focus_address: bool,
    /// Ctrl+L: the Files card swaps its icons for the address field.
    pub address_open: bool,
}
impl Navigation {
    pub fn destination(&self, forward: bool) -> Option<(usize, PathBuf)> {
        let index = if forward {
            self.index.checked_add(1)?
        } else {
            self.index.checked_sub(1)?
        };
        self.paths.get(index).cloned().map(|path| (index, path))
    }
    fn visit(&mut self, path: PathBuf) {
        if self.paths.get(self.index) == Some(&path) {
            return;
        }
        self.paths.truncate(self.index + 1);
        self.paths.push(path);
        if self.paths.len() > 64 {
            self.paths.remove(0);
        }
        self.index = self.paths.len() - 1;
    }
}

pub fn address_id() -> egui::Id {
    egui::Id::new("current_folder_address")
}
pub(super) fn address_text(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if let Some(network) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{network}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    }
}
impl Workspace {
    pub(super) fn change_folder(&mut self, path: PathBuf) -> bool {
        self.visit_folder(path, None)
    }
    pub(super) fn step_folder(&mut self, forward: bool) {
        if let Some((index, path)) = self.folder_navigation.destination(forward) {
            self.visit_folder(path, Some(index));
        }
    }
    fn visit_folder(&mut self, path: PathBuf, index: Option<usize>) -> bool {
        if self.busy || self.file_action_pending() {
            self.status = "Finish the current operation before changing folder".into();
            return false;
        }
        let path = if path.is_absolute() {
            path
        } else {
            self.folder.join(path)
        };
        self.resolve_folder(path, index);
        self.folder_text = address_text(&self.folder);
        true
    }
    pub(super) fn commit_folder(&mut self, path: PathBuf, index: Option<usize>) {
        self.cancel_folder_search();
        if let Some(index) = index {
            self.folder_navigation.index = index;
        } else {
            self.folder_navigation.visit(path.clone());
        }
        self.recovery_dirty |= self.folder != path;
        self.folder_browser.filter.clear();
        self.folder_browser.filter_open = false;
        self.folder = path;
        self.folder_text = address_text(&self.folder);
        self.status = "Current Folder ready".into();
    }
    pub(super) fn folder_shortcuts(&mut self, ctx: &egui::Context, pending: bool) {
        if pending || self.busy {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)) {
            self.step_folder(false);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight)) {
            self.step_folder(true);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowUp))
            && let Some(parent) = self.folder.parent()
        {
            self.change_folder(parent.to_owned());
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::L)) {
            self.folder_navigation.focus_address = true;
            self.folder_navigation.address_open = !self.folder_search.visible;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.refresh_folder();
        }
    }
    pub(super) fn folder_address_focus(&mut self, response: &egui::Response) {
        if self.folder_navigation.focus_address {
            self.folder_navigation.focus_address = false;
            response.request_focus();
            let ctx = &response.ctx;
            let mut state =
                egui::text_edit::TextEditState::load(ctx, address_id()).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.folder_text.chars().count()),
                )));
            state.store(ctx, address_id());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_navigation_history_bounds_forks_and_same_location_keeps_forward() {
        assert_eq!(
            address_text(std::path::Path::new(r"\\?\UNC\server\share\data")),
            r"\\server\share\data"
        );
        assert_eq!(
            address_text(std::path::Path::new(r"\\?\C:\data")),
            r"C:\data"
        );
        assert_eq!(
            address_text(std::path::Path::new(r"\\server\share")),
            r"\\server\share"
        );
        let mut history = Navigation::default();
        for n in 0..80 {
            history.visit(PathBuf::from(format!("folder-{n}")));
        }
        assert_eq!(history.paths.len(), 64);
        assert_eq!(history.paths[0], PathBuf::from("folder-16"));
        history.index = 20;
        let current = history.paths[20].clone();
        let next = history.destination(true);
        history.visit(current);
        assert_eq!(history.destination(true), next);
        history.visit(PathBuf::from("new-branch"));
        assert_eq!(history.paths.len(), 22);
        assert!(history.destination(true).is_none());
        assert_eq!(history.destination(false).unwrap().0, 20);
    }
}
