//! Streaming numeric CSV with bounded batches and ordered parallel parsing.
use crate::{Matrix, Result, matrix::MAX_ELEMENTS};
use rayon::prelude::*;
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

mod partition;

const BATCH_BYTES: usize = 2 * 1024 * 1024;
const JOB_BYTES: usize = 64 * 1024;

pub(super) fn read(path: &Path) -> Result<Matrix> {
    read_with_cancel(path, &AtomicBool::new(false))
}

pub(super) fn read_with_cancel(path: &Path, cancel: &AtomicBool) -> Result<Matrix> {
    read_impl::<false>(path, None, cancel)
}

pub(super) fn read_strict(
    path: &Path,
    delimiter: Option<char>,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    if delimiter.is_some_and(|c| !matches!(c, ',' | ';' | '\t')) {
        return Err("Numeric text delimiter must be comma, semicolon or tab".into());
    }
    read_impl::<true>(path, delimiter, cancel)
}

fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Calculation cancelled".into())
    } else {
        Ok(())
    }
}
// Keep record boundaries while checking cancellation between bounded copies.
// `limit` is the remaining file-byte budget including any existing batch prefix.
// A blocking filesystem read itself cannot be interrupted by this token.
fn read_record(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<usize> {
    let initial = output.len();
    if initial > limit {
        return Err("CSV file exceeds 512 MiB".into());
    }
    loop {
        check(cancel)?;
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("CSV: {error}")),
        };
        check(cancel)?;
        let available = &available[..available.len().min(JOB_BYTES)];
        if available.is_empty() {
            return Ok(output.len() - initial);
        }
        let newline = memchr::memchr(b'\n', available);
        let count = newline.map_or(available.len(), |position| position + 1);
        let end = output.len().checked_add(count).ok_or("CSV size overflow")?;
        if end > limit {
            return Err("CSV file exceeds 512 MiB".into());
        }
        if end > output.capacity() {
            // Bound the requested capacity, including a final record in a growing
            // file. Allocator bookkeeping/RSS is outside this logical bound.
            let capacity = end.max(output.capacity().saturating_mul(2)).min(limit);
            output
                .try_reserve_exact(capacity - output.len())
                .map_err(|_| "Cannot allocate CSV record buffer".to_owned())?;
        }
        output.extend_from_slice(&available[..count]);
        reader.consume(count);
        check(cancel)?;
        if newline.is_some() {
            return Ok(output.len() - initial);
        }
    }
}

fn separator<const STRICT: bool>(c: char, delimiter: char) -> bool {
    if STRICT {
        c == delimiter
    } else {
        matches!(c, ',' | ';' | '\t')
    }
}
// A record made only of tab separators still contains missing fields. Do
// not discard it as whitespace before tab-delimiter inference or parsing.
// Keep the established permissive blank-line behavior of the legacy reader.
fn blank_record<const STRICT: bool>(line: &str, delimiter: char) -> bool {
    line.trim().is_empty() && (!STRICT || !matches!(delimiter, '\0' | '\t') || !line.contains('\t'))
}

fn detect(line: &str) -> Result<char> {
    let mut chosen = None;
    for c in line.chars().filter(|&c| matches!(c, ',' | ';' | '\t')) {
        if chosen.is_some_and(|old| old != c) {
            return Err(
                "Mixed delimiters in the first data record; specify one Delimiter explicitly"
                    .into(),
            );
        }
        chosen = Some(c);
    }
    Ok(chosen.unwrap_or(','))
}

fn read_impl<const STRICT: bool>(
    path: &Path,
    delimiter: Option<char>,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    check(cancel)?;
    let mut delimiter = delimiter.unwrap_or('\0');
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    if length > super::MAX_CSV_BYTES as u64 {
        return Err("CSV file exceeds 512 MiB".into());
    }
    // Measurements show batching regresses small files and single-worker loads.
    if length < 256 * 1024 || rayon::current_num_threads() == 1 {
        return serial::<STRICT>(file, delimiter, cancel);
    }
    let mut reader =
        BufReader::with_capacity(JOB_BYTES, file.take(super::MAX_CSV_BYTES as u64 + 1));
    let mut bytes = Vec::with_capacity(BATCH_BYTES);
    let mut consumed = 0;
    let mut line_number = 1;
    let mut cols = None;
    let mut values = Vec::new();
    loop {
        check(cancel)?;
        bytes.clear();
        reader
            .by_ref()
            .take(BATCH_BYTES as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("CSV: {e}"))?;
        if bytes.is_empty() {
            break;
        }
        // A record is never divided between jobs or batches. The outer Take also
        // bounds unusually long records and files that grow while being read.
        if bytes.last() != Some(&b'\n') {
            read_record(
                &mut reader,
                &mut bytes,
                super::MAX_CSV_BYTES - consumed,
                cancel,
            )?;
        }
        consumed += bytes.len();
        if consumed > super::MAX_CSV_BYTES {
            return Err("CSV file exceeds 512 MiB".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| "CSV is not valid UTF-8")?;
        let text = if line_number == 1 {
            text.trim_start_matches('\u{feff}')
        } else {
            text
        };
        if cols.is_none() {
            let first = text
                .split('\n')
                .find(|s| !blank_record::<STRICT>(s, delimiter));
            if STRICT
                && delimiter == '\0'
                && let Some(first) = first
            {
                delimiter = detect(first)?;
            }
            cols = first.map(|s| s.split(|c| separator::<STRICT>(c, delimiter)).count());
            if let Some(c) = cols {
                Matrix::checked_len(1, c)?;
            }
        }
        let Some(columns) = cols else {
            line_number += partition::newline_count(text.as_bytes(), cancel)?;
            continue;
        };
        let remaining = MAX_ELEMENTS - values.len();
        if text.len() >= 256 * 1024 && rayon::current_num_threads() > 1 {
            let (jobs, next_line) = partition::jobs(text, line_number, cancel)?;
            line_number = next_line;
            if jobs.len() == 1 {
                // There is no independent parsing job to schedule for a single
                // unusually wide record. Decode into the final accumulator while
                // retaining field order, physical line errors, and checkpoints.
                let (part, line) = jobs[0];
                parse_single_job_into::<STRICT>(
                    &mut values,
                    part,
                    line,
                    columns,
                    remaining,
                    delimiter,
                    cancel,
                )?;
                Matrix::checked_len(values.len() / columns, columns)?;
            } else {
                // Indexed collection preserves physical input order and deterministic
                // errors. Only this bounded batch is parsed concurrently.
                let results: Vec<_> = jobs
                    .par_iter()
                    .map(|&(part, line)| {
                        parse::<STRICT>(part, line, columns, remaining, delimiter, cancel)
                    })
                    .collect();
                for result in results {
                    append(&mut values, result?, columns)?;
                }
            }
        } else {
            append(
                &mut values,
                parse::<STRICT>(text, line_number, columns, remaining, delimiter, cancel)?,
                columns,
            )?;
            line_number += partition::newline_count(text.as_bytes(), cancel)?;
        }
    }
    check(cancel)?;
    let cols = cols.unwrap_or(0);
    let rows = values.len().checked_div(cols).unwrap_or(0);
    if cols <= 1 || rows <= 1 {
        return Matrix::from_column_vec(rows, cols, values);
    }
    let mut output = vec![0.; values.len()];
    // Contiguous output tiles avoid concurrent writes to adjacent cache lines.
    let fill = |(tile, out): (usize, &mut [f64])| -> Result<()> {
        check(cancel)?;
        let mut index = tile * 8192;
        let mut rest = out;
        while !rest.is_empty() {
            let column = index / rows;
            let row = index % rows;
            let count = rest.len().min(rows - row);
            let (part, tail) = rest.split_at_mut(count);
            for (r, value) in part.iter_mut().enumerate() {
                *value = values[(row + r) * cols + column];
            }
            index += count;
            rest = tail;
        }
        Ok(())
    };
    if values.len() >= 262144 && rayon::current_num_threads() > 1 {
        output.par_chunks_mut(8192).enumerate().try_for_each(fill)?;
    } else {
        output.chunks_mut(8192).enumerate().try_for_each(fill)?;
    }
    check(cancel)?;
    Matrix::from_column_vec(rows, cols, output)
}

// Only the already-serial single-job branch uses this accumulator. The
// multi-job parser below retains its original allocation and collection policy.
fn grow_single_job(values: &mut Vec<f64>, required: usize) -> Result<()> {
    if required > MAX_ELEMENTS {
        return Err(format!("Matrix exceeds the {MAX_ELEMENTS}-element limit"));
    }
    if required > values.capacity() {
        let capacity = required.next_power_of_two().min(MAX_ELEMENTS);
        values
            .try_reserve_exact(capacity - values.len())
            .map_err(|_| "Cannot allocate CSV value buffer".to_owned())?;
    }
    Ok(())
}

#[inline]
fn single_job_check(cancel: &AtomicBool, _length: usize) -> Result<()> {
    #[cfg(test)]
    single_job_tests::checkpoint(cancel, _length);
    check(cancel)
}

fn parse_single_job_into<const STRICT: bool>(
    values: &mut Vec<f64>,
    text: &str,
    first_line: usize,
    cols: usize,
    remaining: usize,
    delimiter: char,
    cancel: &AtomicBool,
) -> Result<()> {
    let initial = values.len();
    for (offset, line) in text.split_terminator('\n').enumerate() {
        single_job_check(cancel, values.len())?;
        if blank_record::<STRICT>(line, delimiter) {
            continue;
        }
        let line_number = first_line + offset;
        if values.len() - initial + cols > remaining {
            return Err(format!("Matrix exceeds the {MAX_ELEMENTS}-element limit"));
        }
        let start = values.len();
        // The complete row's admitted column count bounds requested growth.
        // Power-of-two reservations preserve the accumulator's existing budget
        // policy without a second decoded vector. Allocator/RSS is not a quota.
        grow_single_job(values, start + cols)?;
        for cell in line
            .trim_end_matches('\r')
            .split(|c| separator::<STRICT>(c, delimiter))
        {
            if (values.len() - start).is_multiple_of(8192) {
                single_job_check(cancel, values.len())?;
            }
            if values.len() - start == cols {
                return Err(format!("Inconsistent column count on line {line_number}"));
            }
            values.push(
                cell.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("Invalid numeric field on line {line_number}"))?,
            );
        }
        if values.len() - start != cols {
            return Err(format!("Inconsistent column count on line {line_number}"));
        }
    }
    single_job_check(cancel, values.len())?;
    Ok(())
}

fn append(values: &mut Vec<f64>, part: Vec<f64>, cols: usize) -> Result<()> {
    Matrix::checked_len((values.len() + part.len()) / cols, cols)?;
    if values.is_empty() {
        // Start growth on a power-of-two boundary. Adopting a job's estimated
        // capacity can otherwise reserve almost twice the decoded matrix limit.
        values.reserve_exact(part.len().next_power_of_two().min(MAX_ELEMENTS));
        values.extend(part);
    } else {
        values.extend(part);
    }
    Ok(())
}

fn parse<const STRICT: bool>(
    text: &str,
    first_line: usize,
    cols: usize,
    remaining: usize,
    delimiter: char,
    cancel: &AtomicBool,
) -> Result<Vec<f64>> {
    let mut values = Vec::with_capacity((text.len() / 20).min(remaining));
    for (offset, line) in text.split_terminator('\n').enumerate() {
        check(cancel)?;
        if blank_record::<STRICT>(line, delimiter) {
            continue;
        }
        let line_number = first_line + offset;
        if values.len() + cols > remaining {
            return Err(format!("Matrix exceeds the {MAX_ELEMENTS}-element limit"));
        }
        let start = values.len();
        for cell in line
            .trim_end_matches('\r')
            .split(|c| separator::<STRICT>(c, delimiter))
        {
            if (values.len() - start).is_multiple_of(8192) {
                check(cancel)?;
            }
            if values.len() - start == cols {
                return Err(format!("Inconsistent column count on line {line_number}"));
            }
            values.push(
                cell.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("Invalid numeric field on line {line_number}"))?,
            );
        }
        if values.len() - start != cols {
            return Err(format!("Inconsistent column count on line {line_number}"));
        }
    }
    check(cancel)?;
    Ok(values)
}

fn serial<const STRICT: bool>(
    file: std::fs::File,
    mut delimiter: char,
    cancel: &AtomicBool,
) -> Result<Matrix> {
    let mut reader =
        BufReader::with_capacity(64 * 1024, file.take(super::MAX_CSV_BYTES as u64 + 1));
    let mut line = Vec::new();
    let mut line_number = 0;
    let mut bytes_read = 0;
    let mut values = Vec::new();
    let mut rows = 0;
    let mut cols = None;
    loop {
        check(cancel)?;
        line.clear();
        let count = read_record(
            &mut reader,
            &mut line,
            super::MAX_CSV_BYTES - bytes_read,
            cancel,
        )?;
        if count == 0 {
            break;
        }
        bytes_read += count;
        if bytes_read > super::MAX_CSV_BYTES {
            return Err("CSV file exceeds 512 MiB".into());
        }
        line_number += 1;
        let line = std::str::from_utf8(&line).map_err(|_| "CSV is not valid UTF-8")?;
        let line = if line_number == 1 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        if blank_record::<STRICT>(line, delimiter) {
            continue;
        }
        if STRICT && delimiter == '\0' {
            delimiter = detect(line)?;
        }
        let mut columns: usize = 0;
        for cell in line
            .trim_end_matches(['\r', '\n'])
            .split(|c| separator::<STRICT>(c, delimiter))
        {
            if columns.is_multiple_of(8192) {
                check(cancel)?;
            }
            columns += 1;
            Matrix::checked_len(rows + 1, columns)?;
            let value = cell
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("Invalid numeric field on line {line_number}"))?;
            values.push(value);
        }
        if cols.is_some_and(|n| n != columns) {
            return Err(format!("Inconsistent column count on line {}", line_number));
        }
        cols = Some(columns);
        rows += 1;
    }
    check(cancel)?;
    let cols = cols.unwrap_or(0);
    if cols <= 1 || rows <= 1 {
        return Matrix::from_column_vec(rows, cols, values);
    }
    let mut column_major = vec![0.0; values.len()];
    for r in 0..rows {
        check(cancel)?;
        for c in 0..cols {
            if c.is_multiple_of(8192) {
                check(cancel)?;
            }
            column_major[r + c * rows] = values[r * cols + c];
        }
    }
    check(cancel)?;
    Matrix::from_column_vec(rows, cols, column_major)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_read_cancels_during_a_long_record_without_reading_the_tail() {
        struct CancelAfterChunk<'a> {
            data: Vec<u8>,
            offset: usize,
            cancel: &'a AtomicBool,
        }
        impl Read for CancelAfterChunk<'_> {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("The record reader must use the bounded BufRead interface")
            }
        }
        impl BufRead for CancelAfterChunk<'_> {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                Ok(&self.data[self.offset..])
            }
            fn consume(&mut self, amount: usize) {
                self.offset += amount;
                self.cancel.store(true, Ordering::Relaxed);
            }
        }
        let cancel = AtomicBool::new(false);
        let mut reader = CancelAfterChunk {
            data: vec![b'1'; JOB_BYTES * 4],
            offset: 0,
            cancel: &cancel,
        };
        let mut output = Vec::new();
        let error = read_record(&mut reader, &mut output, JOB_BYTES * 4, &cancel).unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(cancel.load(Ordering::Relaxed));
        assert_eq!(reader.offset, JOB_BYTES);
        assert_eq!(output.len(), JOB_BYTES);
        assert_eq!(output.capacity(), JOB_BYTES);
    }

    #[test]
    fn record_read_preserves_boundaries_eof_and_limit_without_oversized_growth() {
        let cancel = AtomicBool::new(false);
        let mut reader = BufReader::with_capacity(2, std::io::Cursor::new(b"1,2\r\n3,4"));
        let mut output = Vec::new();
        assert_eq!(
            read_record(&mut reader, &mut output, 8, &cancel).unwrap(),
            5
        );
        assert_eq!(output, b"1,2\r\n");
        output.clear();
        assert_eq!(
            read_record(&mut reader, &mut output, 3, &cancel).unwrap(),
            3
        );
        assert_eq!(output, b"3,4");
        output.clear();
        assert_eq!(
            read_record(&mut reader, &mut output, 0, &cancel).unwrap(),
            0
        );

        let mut reader = std::io::Cursor::new(b"x\n");
        let mut output = vec![b'0'; 70];
        assert_eq!(
            read_record(&mut reader, &mut output, 128, &cancel).unwrap(),
            2
        );
        assert_eq!(output.len(), 72);
        assert!(output.capacity() <= 128);
        let before = output.clone();
        let mut too_long = std::io::Cursor::new([b'1'; 57]);
        assert_eq!(
            read_record(&mut too_long, &mut output, 128, &cancel).unwrap_err(),
            "CSV file exceeds 512 MiB"
        );
        assert_eq!(output, before);
        assert_eq!(too_long.position(), 0);
        assert!(output.capacity() <= 128);
    }

    #[test]
    fn record_search_keeps_first_newline_across_alignment_and_chunk_edges() {
        let cancel = AtomicBool::new(false);
        // Cover short records, SIMD-sized boundaries and the bounded 64 KiB
        // search boundary. The trailing newline must remain for the next call.
        for prefix in [
            0,
            1,
            7,
            15,
            16,
            31,
            32,
            63,
            64,
            JOB_BYTES - 1,
            JOB_BYTES,
            JOB_BYTES + 1,
        ] {
            for alignment in [0, 1, 3, 15] {
                let mut input = vec![b'x'; alignment + prefix];
                input.extend_from_slice(b"\nlast\n");
                let mut reader = std::io::Cursor::new(&input[alignment..]);
                let mut output = b"batch-prefix".to_vec();
                let initial = output.len();
                assert_eq!(
                    read_record(&mut reader, &mut output, initial + prefix + 1, &cancel).unwrap(),
                    prefix + 1
                );
                assert_eq!(&output[..initial], b"batch-prefix");
                assert!(
                    output[initial..initial + prefix]
                        .iter()
                        .all(|&byte| byte == b'x')
                );
                assert_eq!(output.last(), Some(&b'\n'));
                assert_eq!(reader.position(), (prefix + 1) as u64);
                output.clear();
                assert_eq!(
                    read_record(&mut reader, &mut output, 5, &cancel).unwrap(),
                    5
                );
                assert_eq!(output, b"last\n");
                output.clear();
                assert_eq!(
                    read_record(&mut reader, &mut output, 0, &cancel).unwrap(),
                    0
                );
            }
        }
        let mut reader = std::io::Cursor::new(b"\nuntouched");
        let mut output = Vec::new();
        assert_eq!(
            read_record(&mut reader, &mut output, 0, &cancel).unwrap_err(),
            "CSV file exceeds 512 MiB"
        );
        assert_eq!(reader.position(), 0);
        assert!(output.is_empty());
    }

    #[test]
    fn strict_batches_reject_decoded_budget_mixed_fields_and_precancellation() {
        let cancel = AtomicBool::new(false);
        assert!(
            parse::<true>("1;2\n3;4", 1, 2, 3, ';', &cancel)
                .unwrap_err()
                .contains("limit")
        );
        assert!(
            parse::<true>("1,2\n3;4", 7, 2, 16, ',', &cancel)
                .unwrap_err()
                .contains("line 8")
        );
        assert!(
            parse::<true>("1,2", 1, 2, 16, ',', &AtomicBool::new(true))
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(detect("1,2;3").is_err());
        assert_eq!(detect("1\t2\t3").unwrap(), '\t');
    }
    #[test]
    fn batches_reject_decoded_budget_before_appending() {
        assert!(
            parse::<false>("1,2\n3,4", 1, 2, 3, ',', &AtomicBool::new(false))
                .unwrap_err()
                .contains("limit")
        );
        let mut values = vec![0.; MAX_ELEMENTS];
        assert!(append(&mut values, vec![1.], 1).is_err());
        assert_eq!(values.len(), MAX_ELEMENTS);
    }
}

#[cfg(test)]
mod vector_tests;

#[cfg(test)]
#[path = "csv/single_job_tests.rs"]
mod single_job_tests;
