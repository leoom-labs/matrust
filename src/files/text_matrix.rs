//! Strict numeric delimited-text exchange used by readmatrix/writematrix.
use crate::{Array, Matrix, Result, dsp::check_cancel};
use std::{
    io::{self, BufWriter, Write},
    path::Path,
    sync::atomic::AtomicBool,
};

pub fn read_matrix_text(
    path: &Path,
    delimiter: Option<char>,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    super::csv::read_strict(path, delimiter, cancel)
}

/// Stream a bounded numeric/logical text file to a sibling temporary file, then
/// publish atomically. Inputs stay borrowed; encoded output uses a 64-KiB buffer.
pub fn write_matrix_text(
    path: &Path,
    array: &Array,
    delimiter: char,
    cancel: &AtomicBool,
) -> Result<()> {
    check_cancel(cancel)?;
    if !matches!(delimiter, ',' | ';' | '\t') {
        return Err("Numeric text delimiter must be comma, semicolon or tab".into());
    }
    let rows = if array.is_empty() { 0 } else { array.rows() };
    match array {
        Array::Character(_)
        | Array::Cell(_)
        | Array::Graphics(_)
        | Array::Struct(_)
        | Array::Typed(_)
        | Array::Handle(_) => {
            Err("Numeric text export does not support character, cell, struct, function handle or graphics handle arrays".into())
        }
        Array::Complex(_) => Err(
            "Complex numeric text export is not implemented; use explicit component variables"
                .into(),
        ),
        Array::Numeric(a) => write_values(
            path,
            rows,
            a.cols(),
            |r, c| a.get(r, c),
            delimiter as u8,
            cancel,
            super::MAX_CSV_BYTES,
        ),
        Array::Logical(a) => write_values(
            path,
            rows,
            a.cols(),
            |r, c| u8::from(a.get(r, c)),
            delimiter as u8,
            cancel,
            super::MAX_CSV_BYTES,
        ),
    }
}

struct LimitedWriter<W> {
    inner: W,
    bytes: usize,
    limit: usize,
}
impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.len() > self.limit - self.bytes {
            return Err(io::Error::other("CSV exceeds its encoded file size limit"));
        }
        let written = self.inner.write(data)?;
        self.bytes += written;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn write_values<T: std::fmt::Display>(
    path: &Path,
    rows: usize,
    cols: usize,
    value: impl Fn(usize, usize) -> T,
    delimiter: u8,
    cancel: &AtomicBool,
    limit: usize,
) -> Result<()> {
    check_cancel(cancel)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    {
        let mut output = LimitedWriter {
            inner: BufWriter::with_capacity(64 * 1024, temporary.as_file_mut()),
            bytes: 0,
            limit,
        };
        for row in 0..rows {
            check_cancel(cancel)?;
            for col in 0..cols {
                if col.is_multiple_of(8192) {
                    check_cancel(cancel)?;
                }
                if col > 0 {
                    output.write_all(&[delimiter]).map_err(|e| e.to_string())?;
                }
                write!(output, "{}", value(row, col)).map_err(|e| e.to_string())?;
            }
            output.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        check_cancel(cancel)?;
        output.flush().map_err(|e| e.to_string())?;
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    check_cancel(cancel)?;
    // Publication (including bounded Windows sharing retries) is one final
    // noninterruptible phase. An error never removes the previous destination.
    super::persist_atomic(temporary, path, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn streaming_text_limit_and_mid_encode_cancellation_preserve_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.csv");
        std::fs::write(&path, b"keep").unwrap();
        let cancel = AtomicBool::new(false);
        assert!(
            write_values(&path, 2, 3, |_, _| 12345., b',', &cancel, 16)
                .unwrap_err()
                .contains("limit")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        assert!(
            write_values(
                &path,
                1,
                20000,
                |_, c| {
                    if c == 9000 {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    c
                },
                b',',
                &cancel,
                super::super::MAX_CSV_BYTES
            )
            .unwrap_err()
            .contains("cancelled")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
