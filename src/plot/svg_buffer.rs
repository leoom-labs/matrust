//! The exportgraphics path bounds generated SVG before each allocation.
use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};
pub(super) const SVG_LIMIT: usize = 32 * 1024 * 1024;
pub(super) struct SvgBuffer<'a> {
    text: String,
    cancel: Option<&'a AtomicBool>,
    failed: bool,
}
impl<'a> SvgBuffer<'a> {
    pub(super) fn new(text: String, cancel: Option<&'a AtomicBool>) -> Self {
        let mut output = Self {
            text: String::new(),
            cancel,
            failed: false,
        };
        let _ = fmt::Write::write_str(&mut output, &text);
        output
    }
    pub(super) fn checkpoint(&self) -> usize {
        self.text.len()
    }
    pub(super) fn rewind(&mut self, checkpoint: usize) {
        // Only caller-owned UTF-8 boundaries obtained from checkpoint().
        // Cancellation/output-limit failure remains sticky after rewinding.
        self.text.truncate(checkpoint);
    }
    pub(super) fn failed(&self) -> bool {
        self.failed
    }
    pub(super) fn stopped(&mut self, index: usize) -> bool {
        if !self.failed
            && index.is_multiple_of(4096)
            && self
                .cancel
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            self.failed = true;
        }
        self.failed
    }
    pub(super) fn push_str(&mut self, text: &str) {
        let _ = fmt::Write::write_str(self, text);
    }
    pub(super) fn finish(self) -> String {
        if self.failed {
            String::new()
        } else {
            self.text
        }
    }
}
impl fmt::Write for SvgBuffer<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.failed {
            return Err(fmt::Error);
        }
        if let Some(cancel) = self.cancel {
            let Some(size) = self.text.len().checked_add(text.len()) else {
                self.failed = true;
                return Err(fmt::Error);
            };
            if cancel.load(Ordering::Relaxed) || size > SVG_LIMIT {
                self.failed = true;
                return Err(fmt::Error);
            }
            if size > self.text.capacity() {
                let capacity = size
                    .max(self.text.capacity().saturating_mul(2))
                    .min(SVG_LIMIT);
                self.text
                    .try_reserve_exact(capacity - self.text.len())
                    .map_err(|_| {
                        self.failed = true;
                        fmt::Error
                    })?;
            }
        }
        self.text.push_str(text);
        Ok(())
    }
}
// Legacy output keeps its existing acceptance behavior. The bounded export
// updates its small root attributes in place without an unbounded clone.
pub(super) fn replace_first(
    mut text: String,
    needle: &str,
    replacement: &str,
    cancel: Option<&AtomicBool>,
) -> Option<String> {
    let Some(cancel) = cancel else {
        return Some(text.replacen(needle, replacement, 1));
    };
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    let start = text.find(needle)?;
    let length = text
        .len()
        .checked_sub(needle.len())?
        .checked_add(replacement.len())?;
    if length > SVG_LIMIT {
        return None;
    }
    if length > text.len() {
        text.try_reserve_exact(length - text.len()).ok()?;
    }
    text.replace_range(start..start + needle.len(), replacement);
    Some(text)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;
    #[test]
    fn bounded_svg_rejects_growth_and_cancel_without_a_partial_result() {
        let cancel = AtomicBool::new(false);
        let mut buffer = SvgBuffer::new(String::new(), Some(&cancel));
        let chunk = "x".repeat(4096);
        for _ in 0..SVG_LIMIT / 4096 {
            buffer.write_str(&chunk).unwrap();
        }
        assert!(buffer.text.capacity() <= SVG_LIMIT);
        assert!(buffer.write_str("x").is_err());
        assert!(buffer.finish().is_empty());
        let mut buffer = SvgBuffer::new("prefix".into(), Some(&cancel));
        cancel.store(true, Ordering::Relaxed);
        assert!(write!(buffer, "{}", 1.25).is_err());
        assert!(buffer.finish().is_empty());
    }
    #[test]
    fn bounded_subplot_header_growth_is_checked_before_mutation() {
        let cancel = AtomicBool::new(false);
        assert_eq!(
            replace_first(
                "<svg width=1>body</svg>".into(),
                "width=1",
                "x=2 width=3",
                Some(&cancel)
            )
            .unwrap(),
            "<svg x=2 width=3>body</svg>"
        );
        let text = "x".repeat(SVG_LIMIT);
        assert!(replace_first(text, "x", "xx", Some(&cancel)).is_none());
        assert!(replace_first("abc".into(), "missing", "new", Some(&cancel)).is_none());
        cancel.store(true, Ordering::Relaxed);
        assert!(replace_first("abc".into(), "a", "b", Some(&cancel)).is_none());
    }
    #[test]
    fn bounded_skipped_point_poll_observes_cancel_without_any_write() {
        let cancel = AtomicBool::new(false);
        let mut buffer = SvgBuffer::new("prefix".into(), Some(&cancel));
        assert!(!buffer.stopped(0));
        cancel.store(true, Ordering::Relaxed);
        assert!(!buffer.stopped(4095));
        assert!(buffer.stopped(4096));
        assert!(buffer.stopped(4097));
        assert!(buffer.finish().is_empty());
    }
}
