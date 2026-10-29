use matrust::{Matrix, Result, files::MAX_CSV_BYTES};
use std::{
    io::{BufRead, BufReader, Read},
    path::Path,
};
pub fn read_csv(path: &Path) -> Result<Matrix> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_CSV_BYTES as u64 {
        return Err("CSV file exceeds 512 MiB".into());
    }
    let mut reader = BufReader::with_capacity(64 * 1024, file.take(MAX_CSV_BYTES as u64 + 1));
    let mut line = String::new();
    let mut line_number = 0;
    let mut bytes_read = 0;
    let mut values = Vec::new();
    let mut rows = 0;
    let mut cols = None;
    loop {
        line.clear();
        let count = reader
            .read_line(&mut line)
            .map_err(|e| format!("CSV: {e}"))?;
        if count == 0 {
            break;
        }
        bytes_read += count;
        if bytes_read > MAX_CSV_BYTES {
            return Err("CSV file exceeds 512 MiB".into());
        }
        line_number += 1;
        let line = if line_number == 1 {
            line.trim_start_matches('\u{feff}')
        } else {
            &line
        };
        if line.trim().is_empty() {
            continue;
        }
        let mut columns = 0;
        for cell in line.trim_end_matches(['\r', '\n']).split([',', ';', '\t']) {
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
    let cols = cols.unwrap_or(0);
    let mut column_major = vec![0.0; values.len()];
    for r in 0..rows {
        for c in 0..cols {
            column_major[r + c * rows] = values[r * cols + c];
        }
    }
    Matrix::from_column_vec(rows, cols, column_major)
}
