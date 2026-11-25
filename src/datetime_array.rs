//! Immutable, bounded two-dimensional `datetime` storage in column-major
//! order. Each element is a wall-clock tuple, not an instant: a day number
//! (the serial-day numbering of `datenum`, day 1 = 1-Jan-0000 in the proleptic
//! Gregorian calendar) and the nanoseconds since that day's midnight, always in
//! `[0, 86_400e9)`. Unzoned values are local wall-clock components; the `Utc`
//! zone marks the same fields as UTC. `NaT` has the day number `i64::MIN`,
//! outside every real date. Properties (format, zone) live beside the shared
//! element storage, so a property write copies only the metadata.
use crate::Result;
use std::sync::Arc;

/// Element cap shared by datetime and duration arrays (16 Mi elements).
pub const MAX_ELEMENTS: usize = 16 * 1024 * 1024;
/// Nanoseconds in one 24-hour day.
pub const NANOS_PER_DAY: i64 = 86_400_000_000_000;
/// Default display format.
pub const DEFAULT_FORMAT: &str = "dd-MMM-yyyy HH:mm:ss";
/// Display format for values made from dates only.
pub const DEFAULT_DATE_FORMAT: &str = "dd-MMM-yyyy";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DateTimeValue {
    pub days: i64,
    pub nanos: i64,
}

impl DateTimeValue {
    pub const NAT: Self = Self {
        days: i64::MIN,
        nanos: 0,
    };
    pub fn is_nat(self) -> bool {
        self.days == i64::MIN
    }
    /// Total nanoseconds since the serial epoch (exact; NaT gives None).
    pub fn total_nanos(self) -> Option<i128> {
        (!self.is_nat())
            .then(|| i128::from(self.days) * i128::from(NANOS_PER_DAY) + i128::from(self.nanos))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TimeZone {
    #[default]
    Unzoned,
    Utc,
}

impl TimeZone {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unzoned => "",
            Self::Utc => "UTC",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DatetimeArray {
    // u32 keeps Array at 56 bytes (elements are capped at 2^24).
    rows: u32,
    cols: u32,
    data: Arc<[DateTimeValue]>,
    format: Arc<str>,
    /// False while the format is the date-only default chosen at
    /// construction: a derived array holding a time of day then shows the
    /// full default format.
    explicit_format: bool,
    zone: TimeZone,
}

pub fn checked_len(rows: usize, cols: usize, what: &str) -> Result<usize> {
    let n = rows
        .checked_mul(cols)
        .ok_or_else(|| format!("{what} array dimensions overflow"))?;
    if n > MAX_ELEMENTS || rows > u32::MAX as usize || cols > u32::MAX as usize {
        return Err(format!("{what} array exceeds 16777216 elements"));
    }
    Ok(n)
}

impl DatetimeArray {
    pub fn new(
        rows: usize,
        cols: usize,
        data: Vec<DateTimeValue>,
        format: impl Into<Arc<str>>,
        zone: TimeZone,
    ) -> Result<Self> {
        if checked_len(rows, cols, "datetime")? != data.len() {
            return Err("datetime array shape does not match its elements".into());
        }
        let format: Arc<str> = format.into();
        let mut array = Self {
            rows: rows as u32,
            cols: cols as u32,
            data: data.into(),
            explicit_format: &*format != DEFAULT_DATE_FORMAT,
            format,
            zone,
        };
        array.widen_default();
        Ok(array)
    }
    /// The date-only default format becomes the full default once an element
    /// has a time of day.
    fn widen_default(&mut self) {
        if !self.explicit_format
            && &*self.format == DEFAULT_DATE_FORMAT
            && self.data.iter().any(|v| !v.is_nat() && v.nanos != 0)
        {
            self.format = DEFAULT_FORMAT.into();
        }
    }
    pub fn rows(&self) -> usize {
        self.rows as usize
    }
    pub fn cols(&self) -> usize {
        self.cols as usize
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn data(&self) -> &[DateTimeValue] {
        &self.data
    }
    pub fn format(&self) -> &str {
        &self.format
    }
    pub fn zone(&self) -> TimeZone {
        self.zone
    }
    pub fn bytes(&self) -> usize {
        self.data.len() * 16
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
    /// Same elements (shared), new display format set by the user.
    pub fn with_format(&self, format: impl Into<Arc<str>>) -> Self {
        Self {
            format: format.into(),
            explicit_format: true,
            ..self.clone()
        }
    }
    /// Same elements (shared), new zone label.
    pub fn with_zone(&self, zone: TimeZone) -> Self {
        Self {
            zone,
            ..self.clone()
        }
    }
    /// Same metadata, new elements.
    pub fn with_data(&self, rows: usize, cols: usize, data: Vec<DateTimeValue>) -> Result<Self> {
        if checked_len(rows, cols, "datetime")? != data.len() {
            return Err("datetime array shape does not match its elements".into());
        }
        let mut array = Self {
            rows: rows as u32,
            cols: cols as u32,
            data: data.into(),
            format: self.format.clone(),
            explicit_format: self.explicit_format,
            zone: self.zone,
        };
        array.widen_default();
        Ok(array)
    }
    /// Same elements (shared), new shape of equal element count.
    pub fn reshaped(&self, rows: usize, cols: usize) -> Result<Self> {
        if rows.checked_mul(cols) != Some(self.len())
            || rows > u32::MAX as usize
            || cols > u32::MAX as usize
        {
            return Err("reshape: the number of elements must not change".into());
        }
        Ok(Self {
            rows: rows as u32,
            cols: cols as u32,
            ..self.clone()
        })
    }
    pub fn transposed(&self) -> Self {
        let mut data = Vec::with_capacity(self.len());
        for r in 0..self.rows() {
            for c in 0..self.cols() {
                data.push(self.data[c * self.rows() + r]);
            }
        }
        Self {
            rows: self.cols,
            cols: self.rows,
            data: data.into(),
            format: self.format.clone(),
            explicit_format: self.explicit_format,
            zone: self.zone,
        }
    }
    pub fn select(&self, rows: usize, cols: usize, positions: &[usize]) -> Result<Self> {
        let data = positions
            .iter()
            .map(|&i| {
                self.data
                    .get(i)
                    .copied()
                    .ok_or("Index exceeds array bounds")
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.with_data(rows, cols, data)
    }
}

impl std::fmt::Display for DatetimeArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}x{} datetime]", self.rows, self.cols)
    }
}
