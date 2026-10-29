//! Bounded layout geometry shared by native and vector backends.
//! Coordinates are screen-like (top increases down); normalized requests are
//! MATLAB-like [left, bottom, width, height]. No sample ownership or UI types.
use super::LegendLocation;
use crate::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegendRect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}
impl LegendRect {
    pub fn new(v: [f64; 4]) -> Self {
        Self {
            left: v[0],
            top: v[1],
            right: v[2],
            bottom: v[3],
        }
    }
    pub fn values(self) -> [f64; 4] {
        [self.left, self.top, self.right, self.bottom]
    }
    pub fn width(self) -> f64 {
        self.right - self.left
    }
    pub fn height(self) -> f64 {
        self.bottom - self.top
    }
    pub fn valid(self) -> bool {
        self.values().iter().all(|x| x.is_finite())
            && self.width().is_finite()
            && self.height().is_finite()
            && self.width() > 0.
            && self.height() > 0.
    }
    pub fn contains(self, other: Self) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }
    pub fn intersection(self, other: Self) -> Option<Self> {
        let r = Self::new([
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        ]);
        r.valid().then_some(r)
    }
    pub fn inset(self, margins: [f64; 4]) -> Self {
        Self::new([
            self.left + margins[0],
            self.top + margins[1],
            self.right - margins[2],
            self.bottom - margins[3],
        ])
    }
    pub fn expand(self, margins: [f64; 4]) -> Self {
        self.inset(margins.map(|v| -v))
    }
    pub fn from_normalized(v: [f64; 4], canvas: Self) -> Self {
        Self::new([
            canvas.left + v[0] * canvas.width(),
            canvas.top + (1. - v[1] - v[3]) * canvas.height(),
            canvas.left + (v[0] + v[2]) * canvas.width(),
            canvas.top + (1. - v[1]) * canvas.height(),
        ])
    }
    pub fn normalized(self, canvas: Self) -> [f64; 4] {
        [
            (self.left - canvas.left) / canvas.width(),
            (canvas.bottom - self.bottom) / canvas.height(),
            self.width() / canvas.width(),
            self.height() / canvas.height(),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionConstraint {
    InnerPosition,
    OuterPosition,
}
impl PositionConstraint {
    pub fn parse(value: &str) -> Result<Self> {
        if value.eq_ignore_ascii_case("innerposition") {
            Ok(Self::InnerPosition)
        } else if value.eq_ignore_ascii_case("outerposition") {
            Ok(Self::OuterPosition)
        } else {
            Err("PositionConstraint must be innerposition or outerposition".into())
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::InnerPosition => "innerposition",
            Self::OuterPosition => "outerposition",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AxesPlacement {
    pub requested_inner: [f64; 4],
    pub requested_outer: [f64; 4],
    pub constraint: PositionConstraint,
    /// Imported coordinates already include MATLAB's layout adjustments.
    /// Retain until an explicit layout edit, never clear during paint/reopen.
    pub imported_solved: bool,
}
impl AxesPlacement {
    pub fn validate(self) -> Result<()> {
        super::Position::from_normalized(self.requested_inner)?;
        super::Position::from_normalized(self.requested_outer)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LegendGeometry {
    pub data: LegendRect,
    pub legend: Option<LegendRect>,
    pub outer: LegendRect,
}

/// Size is measured by the adapter. Inside layout does not change the data box.
/// Outside layout reserves space outside decorations, within the held outer box.
pub fn legend_geometry(
    outer: LegendRect,
    inner: LegendRect,
    margins: [f64; 4],
    extent: Option<[f64; 2]>,
    location: LegendLocation,
    fixed_inner: bool,
) -> Result<LegendGeometry> {
    if !outer.valid() || !inner.valid() || !margins.iter().all(|v| v.is_finite() && *v >= 0.) {
        return Err("Invalid legend layout bounds".into());
    }
    let Some([width, height]) = extent else {
        let data = if fixed_inner {
            inner
        } else {
            outer.inset(margins)
        };
        if !data.valid() || data.width() < 16. || data.height() < 16. || !outer.contains(data) {
            return Err(
                "Axes decorations leave no useful plot area; enlarge the held bounds".into(),
            );
        }
        return Ok(LegendGeometry {
            data,
            legend: None,
            outer,
        });
    };
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return Err("Invalid legend extent".into());
    }
    let anchor = location.fixed_anchor();
    if !location.is_outside() {
        if !outer.contains(inner) || inner.width() < 16. || inner.height() < 16. {
            return Err("Inner axes must fit the available canvas with a useful plot area".into());
        }
        let available = inner.inset([4.; 4]);
        let legend = anchored(
            available,
            [width.min(available.width()), height.min(available.height())],
            anchor,
        )?;
        return Ok(LegendGeometry {
            data: inner,
            legend: Some(legend),
            outer,
        });
    }
    let gap = 8.;
    let mut data = if fixed_inner {
        inner
    } else {
        outer.inset(margins)
    };
    let side = location.outside_side().unwrap();
    if !fixed_inner {
        match side {
            0 => data.left += width + gap,
            1 => data.top += height + gap,
            2 => data.right -= width + gap,
            _ => data.bottom -= height + gap,
        }
    }
    if !data.valid() || !outer.contains(data) || data.width() < 16. || data.height() < 16. {
        return Err(
            "Legend leaves no useful plot area; enlarge OuterPosition or reduce the legend".into(),
        );
    }
    let decorated = data.expand(margins);
    let available = match side {
        0 => LegendRect::new([
            decorated.left - gap - width,
            decorated.top,
            decorated.left - gap,
            decorated.bottom,
        ]),
        1 => LegendRect::new([
            decorated.left,
            decorated.top - gap - height,
            decorated.right,
            decorated.top - gap,
        ]),
        2 => LegendRect::new([
            decorated.right + gap,
            decorated.top,
            decorated.right + gap + width,
            decorated.bottom,
        ]),
        _ => LegendRect::new([
            decorated.left,
            decorated.bottom + gap,
            decorated.right,
            decorated.bottom + gap + height,
        ]),
    };
    let legend = anchored(available, [width, height], anchor)?;
    if !outer.contains(legend) {
        return Err(
            "Outside legend exceeds the held bounds; use outerposition or enlarge the allocation"
                .into(),
        );
    }
    Ok(LegendGeometry {
        data,
        legend: Some(legend),
        outer,
    })
}

pub fn anchored(
    available: LegendRect,
    extent: [f64; 2],
    anchor: LegendLocation,
) -> Result<LegendRect> {
    if !available.valid()
        || extent.iter().any(|v| !v.is_finite() || *v <= 0.)
        || extent[0] > available.width()
        || extent[1] > available.height()
    {
        return Err("Legend cannot fit its allocation".into());
    }
    let [x, y] = anchor.alignment();
    let l = available.left + (available.width() - extent[0]) * x;
    let t = available.top + (available.height() - extent[1]) * y;
    let result = LegendRect::new([l, t, l + extent[0], t + extent[1]]);
    if !result.valid() || !available.contains(result) {
        return Err("Legend bounds are not representable".into());
    }
    Ok(result)
}

/// Simultaneous, order-independent allocation from original requested boxes.
/// Horizontal growth is limited against original Y-overlapping boxes; vertical
/// growth then uses the completed horizontal envelopes. No live state mutation.
pub fn outer_allocations(
    inner: &[LegendRect],
    canvas: LegendRect,
    margins: [f64; 4],
) -> Result<Vec<LegendRect>> {
    if inner.len() > 16 || !canvas.valid() || margins.iter().any(|v| !v.is_finite() || *v < 0.) {
        return Err("Invalid axes allocation request".into());
    }
    for (i, a) in inner.iter().enumerate() {
        if !a.valid() || !canvas.contains(*a) || inner[..i].iter().any(|b| a.overlaps(*b)) {
            return Err("Axes allocation boxes must be disjoint and inside the figure".into());
        }
    }
    let mut out = inner.to_vec();
    for (i, a) in inner.iter().enumerate() {
        let mut left = (a.left - margins[0]).max(canvas.left);
        let mut right = (a.right + margins[2]).min(canvas.right);
        for (j, b) in inner.iter().enumerate() {
            if i == j || a.top >= b.bottom || a.bottom <= b.top {
                continue;
            }
            if b.right <= a.left {
                left = left.max(b.right + (a.left - b.right) * 0.5);
            }
            if b.left >= a.right {
                right = right.min(a.right + (b.left - a.right) * 0.5);
            }
        }
        out[i].left = left;
        out[i].right = right;
    }
    let horizontal = out.clone();
    for (i, a) in inner.iter().enumerate() {
        let mut top = (a.top - margins[1]).max(canvas.top);
        let mut bottom = (a.bottom + margins[3]).min(canvas.bottom);
        for (j, b) in inner.iter().enumerate() {
            if i == j
                || horizontal[i].left >= horizontal[j].right
                || horizontal[i].right <= horizontal[j].left
            {
                continue;
            }
            if b.bottom <= a.top {
                top = top.max(b.bottom + (a.top - b.bottom) * 0.5);
            }
            if b.top >= a.bottom {
                bottom = bottom.min(a.bottom + (b.top - a.bottom) * 0.5);
            }
        }
        out[i].top = top;
        out[i].bottom = bottom;
    }
    for (i, a) in out.iter().enumerate() {
        if !a.contains(inner[i]) || !canvas.contains(*a) || out[..i].iter().any(|b| a.overlaps(*b))
        {
            return Err("Cannot derive disjoint outer axes allocations".into());
        }
    }
    Ok(out)
}
