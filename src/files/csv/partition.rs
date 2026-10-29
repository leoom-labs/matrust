//! Preserve the reader's job boundaries while bounding each newline scan.
use super::{JOB_BYTES, check};
use crate::Result;
use std::sync::atomic::AtomicBool;
pub(super) type Jobs<'a> = Vec<(&'a str, usize)>;

#[cfg(test)]
thread_local! {
    static CANCEL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static SCANNED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[inline]
fn checkpoint(cancel: &AtomicBool) -> Result<()> {
    // Deterministic per-thread test hook only. No hook state/code in release.
    #[cfg(test)]
    CANCEL_AFTER.with(|remaining| {
        if let Some(count) = remaining.get() {
            if count == 0 {
                remaining.set(None);
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                remaining.set(Some(count - 1));
            }
        }
    });
    check(cancel)
}

#[inline]
fn scanned(_count: usize) {
    #[cfg(test)]
    SCANNED_BYTES.with(|total| total.set(total.get() + _count));
}

fn newline_after(bytes: &[u8], start: usize, cancel: &AtomicBool) -> Result<Option<usize>> {
    checkpoint(cancel)?;
    for (chunk_index, chunk) in bytes[start..].chunks(JOB_BYTES).enumerate() {
        let found = memchr::memchr(b'\n', chunk);
        scanned(chunk.len());
        checkpoint(cancel)?;
        if let Some(position) = found {
            return Ok(Some(start + chunk_index * JOB_BYTES + position));
        }
    }
    Ok(None)
}

pub(super) fn newline_count(bytes: &[u8], cancel: &AtomicBool) -> Result<usize> {
    checkpoint(cancel)?;
    let mut count = 0;
    for chunk in bytes.chunks(JOB_BYTES) {
        count += memchr::memchr_iter(b'\n', chunk).count();
        scanned(chunk.len());
        checkpoint(cancel)?;
    }
    Ok(count)
}

pub(super) fn jobs<'a>(
    text: &'a str,
    initial_line: usize,
    cancel: &AtomicBool,
) -> Result<(Jobs<'a>, usize)> {
    check(cancel)?;
    let mut jobs = Vec::new();
    let mut offset = 0;
    let mut line = initial_line;
    while offset < text.len() {
        let search = offset.saturating_add(JOB_BYTES).min(text.len());
        let end = newline_after(text.as_bytes(), search, cancel)?
            .map_or(text.len(), |position| position + 1);
        // Offset/end are zero, EOF, or immediately after an ASCII newline, so
        // both are valid UTF8 boundaries even when search is inside a codepoint.
        let part = &text[offset..end];
        jobs.push((part, line));
        line = line
            .checked_add(newline_count(part.as_bytes(), cancel)?)
            .ok_or("CSV line number overflow")?;
        offset = end;
    }
    check(cancel)?;
    Ok((jobs, line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CANCEL_AFTER.with(|x| x.set(None));
            SCANNED_BYTES.with(|x| x.set(0));
        }
    }
    #[test]
    fn scans_cancel_after_one_bounded_chunk_and_retry_without_leaked_state() {
        let _reset = Reset;
        let cancel = AtomicBool::new(false);
        let no_newline = vec![b'1'; JOB_BYTES * 4];
        let many_newlines = vec![b'\n'; JOB_BYTES * 4];
        for count in [false, true] {
            cancel.store(false, Ordering::Relaxed);
            CANCEL_AFTER.with(|x| x.set(Some(1)));
            SCANNED_BYTES.with(|x| x.set(0));
            let result = if count {
                newline_count(&many_newlines, &cancel).map(Some)
            } else {
                newline_after(&no_newline, 0, &cancel)
            };
            assert!(result.unwrap_err().contains("cancelled"));
            assert!(cancel.load(Ordering::Relaxed));
            CANCEL_AFTER.with(|x| assert_eq!(x.get(), None));
            SCANNED_BYTES.with(|x| assert_eq!(x.get(), JOB_BYTES));
            cancel.store(false, Ordering::Relaxed);
            assert_eq!(
                newline_count(&many_newlines, &cancel).unwrap(),
                JOB_BYTES * 4
            );
            assert_eq!(newline_after(&no_newline, 0, &cancel).unwrap(), None);
        }
    }
    #[test]
    fn exact_boundaries_line_counts_and_utf8_slices_are_preserved() {
        let cancel = AtomicBool::new(false);
        for first in [
            0,
            1,
            JOB_BYTES - 1,
            JOB_BYTES,
            JOB_BYTES + 1,
            3 * JOB_BYTES + 7,
        ] {
            let text = format!("{}\n\nlast", "x".repeat(first));
            let (parts, next) = jobs(&text, 17, &cancel).unwrap();
            assert_eq!(next, 19);
            assert_eq!(parts.iter().map(|(s, _)| *s).collect::<String>(), text);
            let end = if first >= JOB_BYTES {
                first + 1
            } else if first + 1 >= JOB_BYTES {
                first + 2
            } else {
                text.len()
            };
            assert_eq!(parts[0], (&text[..end], 17));
            if end < text.len() {
                assert_eq!(
                    parts[1],
                    (&text[end..], if end == first + 1 { 18 } else { 19 })
                );
            }
        }
        let text = format!("{}\n\ntail", "é".repeat(JOB_BYTES / 2 + 1));
        let (parts, next) = jobs(&text, 7, &cancel).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].0.len(), JOB_BYTES + 3);
        assert_eq!(parts[1], ("\ntail", 8));
        assert_eq!(next, 9);
        assert_eq!(jobs("", 3, &cancel).unwrap(), (vec![], 3));
        assert!(jobs("", 3, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn long_single_record_partition_cancels_before_job_publication() {
        let _reset = Reset;
        let text = "1".repeat(JOB_BYTES * 5);
        let cancel = AtomicBool::new(false);
        CANCEL_AFTER.with(|x| x.set(Some(1)));
        SCANNED_BYTES.with(|x| x.set(0));
        assert!(jobs(&text, 1, &cancel).unwrap_err().contains("cancelled"));
        SCANNED_BYTES.with(|x| assert_eq!(x.get(), JOB_BYTES));
        cancel.store(false, Ordering::Relaxed);
        let (parts, next) = jobs(&text, 1, &cancel).unwrap();
        assert_eq!(parts, vec![(text.as_str(), 1)]);
        assert_eq!(next, 1);
    }
}
