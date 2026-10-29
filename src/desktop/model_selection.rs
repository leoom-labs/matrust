//! Bounded sorted selection with a primary property target and range anchor.
use super::egui;

#[derive(Default)]
pub(super) struct Selection {
    indices: Vec<usize>,
    primary: usize,
    anchor: usize,
    count: usize,
}
impl Selection {
    pub fn sync(&mut self, count: usize, primary: &mut usize) {
        if count > 0 && *primary != usize::MAX {
            *primary = (*primary).min(count - 1);
        }
        if self.count != count || self.primary != *primary {
            self.replace(
                count,
                if *primary < count {
                    vec![*primary]
                } else {
                    vec![]
                },
                primary,
            );
        }
    }
    pub fn replace(&mut self, count: usize, indices: Vec<usize>, primary: &mut usize) {
        self.indices = indices;
        self.count = count;
        if !self.indices.contains(primary) {
            *primary = self.indices.first().copied().unwrap_or(usize::MAX);
        }
        self.primary = *primary;
        self.anchor = *primary;
    }
    pub fn indices(&self) -> &[usize] {
        &self.indices
    }
    pub fn contains(&self, index: usize) -> bool {
        self.indices.binary_search(&index).is_ok()
    }
    pub fn click(
        &mut self,
        index: usize,
        modifiers: egui::Modifiers,
        count: usize,
        primary: &mut usize,
    ) {
        self.sync(count, primary);
        if index >= count {
            return;
        }
        if modifiers.shift && self.anchor < count {
            let range = self.anchor.min(index)..=self.anchor.max(index);
            if modifiers.ctrl || modifiers.command {
                self.indices.extend(range);
                self.indices.sort_unstable();
                self.indices.dedup();
            } else {
                self.indices = range.collect();
            }
        } else if modifiers.ctrl || modifiers.command {
            match self.indices.binary_search(&index) {
                Ok(position) => {
                    self.indices.remove(position);
                }
                Err(position) => self.indices.insert(position, index),
            }
            self.anchor = index;
        } else {
            self.indices.clear();
            self.indices.push(index);
            self.anchor = index;
        }
        *primary = if self.contains(index) {
            index
        } else {
            self.indices.first().copied().unwrap_or(usize::MAX)
        };
        self.primary = *primary;
    }
    pub fn activate(&mut self, index: usize, primary: &mut usize) {
        *primary = index;
        self.primary = index;
    }
    pub fn all(&mut self, count: usize, primary: &mut usize) {
        self.replace(count, (0..count).collect(), primary);
    }
}
