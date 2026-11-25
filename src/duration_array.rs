//! Immutable, bounded two-dimensional `duration` storage in column-major
//! order: f64 milliseconds per element (NaN allowed) and one display format
//! for the array. Calendar durations carry separate years/months/days/time
//! components in the same shared record; no fixed length is assigned to a
//! calendar month. A format write copies only the metadata.
use crate::Result;
use crate::datetime_array::checked_len;
use std::sync::Arc;

/// Default display format.
pub const DEFAULT_FORMAT: &str = "hh:mm:ss";

pub(crate) fn checked_calendar_len(rows: usize, cols: usize) -> Result<usize> {
    let n = checked_len(rows, cols, "calendarDuration")?;
    if n > 128 * 1024 * 1024 / 40 {
        return Err("calendarDuration: storage exceeds 128 MiB".into());
    }
    Ok(n)
}

#[derive(Debug)]
struct DurationData {
    fixed: Box<[f64]>,
    calendar: Option<Box<[[f64; 4]]>>,
}

#[derive(Clone, Debug)]
pub struct DurationArray {
    rows: usize,
    cols: usize,
    data: Arc<DurationData>,
    format: Arc<str>,
}

impl DurationArray {
    pub fn new(
        rows: usize,
        cols: usize,
        data: Vec<f64>,
        format: impl Into<Arc<str>>,
    ) -> Result<Self> {
        if checked_len(rows, cols, "duration")? != data.len() {
            return Err("duration array shape does not match its elements".into());
        }
        Ok(Self {
            rows,
            cols,
            data: Arc::new(DurationData {
                fixed: data.into_boxed_slice(),
                calendar: None,
            }),
            format: format.into(),
        })
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    pub fn cols(&self) -> usize {
        self.cols
    }
    pub fn len(&self) -> usize {
        self.data.fixed.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.fixed.is_empty()
    }
    /// Milliseconds, column-major, for fixed durations. Calendar values use
    /// NaN sentinels here; inspect `calendar_data` for their actual components.
    pub fn data(&self) -> &[f64] {
        &self.data.fixed
    }
    pub fn format(&self) -> &str {
        &self.format
    }
    pub fn bytes(&self) -> usize {
        self.len() * if self.is_calendar() { 40 } else { 8 }
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
    pub fn is_calendar(&self) -> bool {
        self.data.calendar.is_some()
    }
    pub fn calendar_data(&self) -> Option<&[[f64; 4]]> {
        self.data.calendar.as_deref()
    }
    pub fn new_calendar(rows: usize, cols: usize, values: Vec<[f64; 4]>) -> Result<Self> {
        if checked_calendar_len(rows, cols)? != values.len() {
            return Err("calendarDuration: shape mismatch".into());
        }
        let n = values.len();
        Ok(Self {
            rows,
            cols,
            data: Arc::new(DurationData {
                fixed: vec![f64::NAN; n].into_boxed_slice(),
                calendar: Some(values.into_boxed_slice()),
            }),
            format: "ymdt".into(),
        })
    }
    pub fn with_format(&self, format: impl Into<Arc<str>>) -> Self {
        Self {
            format: format.into(),
            ..self.clone()
        }
    }
    pub fn with_data(&self, rows: usize, cols: usize, data: Vec<f64>) -> Result<Self> {
        if self.is_calendar() {
            return Err("calendarDuration: this operation is not supported".into());
        }
        Self::new(rows, cols, data, self.format.clone())
    }
    pub fn reshaped(&self, rows: usize, cols: usize) -> Result<Self> {
        if rows.checked_mul(cols) != Some(self.len()) {
            return Err("reshape: the number of elements must not change".into());
        }
        Ok(Self {
            rows,
            cols,
            ..self.clone()
        })
    }
    pub fn transposed(&self) -> Self {
        let mut data = Vec::with_capacity(self.len());
        for r in 0..self.rows {
            for c in 0..self.cols {
                data.push(self.data.fixed[c * self.rows + r]);
            }
        }
        if let Some(values) = self.calendar_data() {
            let mut v = Vec::with_capacity(self.len());
            for r in 0..self.rows {
                for c in 0..self.cols {
                    v.push(values[c * self.rows + r]);
                }
            }
            return Self::new_calendar(self.cols, self.rows, v).expect("same size");
        }
        Self {
            rows: self.cols,
            cols: self.rows,
            data: Arc::new(DurationData {
                fixed: data.into_boxed_slice(),
                calendar: None,
            }),
            format: self.format.clone(),
        }
    }
    pub fn select(&self, rows: usize, cols: usize, positions: &[usize]) -> Result<Self> {
        if let Some(values) = self.calendar_data() {
            checked_calendar_len(rows, cols)?;
            let v = positions
                .iter()
                .map(|&i| {
                    values
                        .get(i)
                        .copied()
                        .ok_or_else(|| "Index exceeds array bounds".to_string())
                })
                .collect::<Result<Vec<_>>>()?;
            return Self::new_calendar(rows, cols, v);
        }
        let data = positions
            .iter()
            .map(|&i| {
                self.data
                    .fixed
                    .get(i)
                    .copied()
                    .ok_or("Index exceeds array bounds")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.with_data(rows, cols, data)
    }
}

impl std::fmt::Display for DurationArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}x{} {}]",
            self.rows,
            self.cols,
            if self.is_calendar() {
                "calendarDuration"
            } else {
                "duration"
            }
        )
    }
}
