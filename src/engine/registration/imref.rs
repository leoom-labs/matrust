//! imref2d: 2-D spatial referencing of an m-by-n image, written from the
//! public MathWorks reference page. Pixel (r, c) is centred at intrinsic
//! (x, y) = (c, r) and covers [c-0.5, c+0.5] by [r-0.5, r+0.5]; world
//! coordinates follow linearly from XWorldLimits and YWorldLimits:
//!
//! - PixelExtentInWorldX = diff(XWorldLimits) / n, ImageExtentInWorldX = diff(XWorldLimits)
//! - xWorld = XWorldLimits(1) + (xIntrinsic - 0.5) * PixelExtentInWorldX (likewise y with m)
//! - the defaults are XWorldLimits = [0.5 n+0.5] and YWorldLimits = [0.5 m+0.5]
//! - imref2d(size, dx, dy) sets XWorldLimits = dx*[0.5 n+0.5], YWorldLimits = dy*[0.5 m+0.5]
//!
//! Settable: XWorldLimits, YWorldLimits (the pixel extents follow) and
//! ImageSize (the world limits stay, the pixel extents follow).
use crate::Result;

const WHO: &str = "imref2d";

#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine) struct Imref2d {
    /// [rows cols], positive integers.
    pub size: [f64; 2],
    /// [xmin xmax], increasing.
    pub x: [f64; 2],
    /// [ymin ymax], increasing.
    pub y: [f64; 2],
}

/// Rows and columns from an image size of 2 or 3 elements; only the first
/// two are kept, as the reference page says for size(RGB).
pub(in crate::engine) fn image_size(values: &[f64]) -> Result<[f64; 2]> {
    if !(2..=3).contains(&values.len()) {
        return Err(format!(
            "{WHO}: imageSize must be a 2-element vector [rows cols] (a third element is ignored)"
        ));
    }
    if values
        .iter()
        .any(|&v| !(v.is_finite() && v >= 1. && v.fract() == 0. && v <= 2f64.powi(53)))
    {
        return Err(format!("{WHO}: imageSize must contain positive integers"));
    }
    Ok([values[0], values[1]])
}

fn limits(what: &str, values: &[f64]) -> Result<[f64; 2]> {
    if values.len() != 2 {
        return Err(format!(
            "{WHO}: {what} must be a 2-element vector [min max]"
        ));
    }
    if !(values[0].is_finite() && values[1].is_finite()) || values[0] >= values[1] {
        return Err(format!(
            "{WHO}: {what} must be finite and increasing ([min max] with min < max)"
        ));
    }
    Ok([values[0] + 0., values[1] + 0.])
}

impl Imref2d {
    /// imref2d(imageSize): XWorldLimits [0.5 n+0.5], YWorldLimits [0.5 m+0.5].
    pub(in crate::engine) fn new(size: [f64; 2]) -> Self {
        Self {
            size,
            x: [0.5, size[1] + 0.5],
            y: [0.5, size[0] + 0.5],
        }
    }
    /// imref2d(imageSize, xWorldLimits, yWorldLimits).
    pub(in crate::engine) fn with_limits(size: [f64; 2], x: &[f64], y: &[f64]) -> Result<Self> {
        Ok(Self {
            size,
            x: limits("xWorldLimits", x)?,
            y: limits("yWorldLimits", y)?,
        })
    }
    /// imref2d(imageSize, pixelExtentInWorldX, pixelExtentInWorldY).
    pub(in crate::engine) fn with_extents(size: [f64; 2], dx: f64, dy: f64) -> Result<Self> {
        if !(dx.is_finite() && dx > 0. && dy.is_finite() && dy > 0.) {
            return Err(format!(
                "{WHO}: the pixel extents must be positive finite numbers"
            ));
        }
        let r = Self {
            size,
            x: [0.5 * dx, (size[1] + 0.5) * dx],
            y: [0.5 * dy, (size[0] + 0.5) * dy],
        };
        // Huge sizes times tiny extents can collapse the interval.
        limits("the world limits", &r.x)?;
        limits("the world limits", &r.y)?;
        Ok(r)
    }
    pub(in crate::engine) fn pixel_extent_x(&self) -> f64 {
        (self.x[1] - self.x[0]) / self.size[1]
    }
    pub(in crate::engine) fn pixel_extent_y(&self) -> f64 {
        (self.y[1] - self.y[0]) / self.size[0]
    }
    pub(in crate::engine) fn world_to_intrinsic(&self, xw: f64, yw: f64) -> (f64, f64) {
        (
            0.5 + (xw - self.x[0]) / self.pixel_extent_x(),
            0.5 + (yw - self.y[0]) / self.pixel_extent_y(),
        )
    }
    pub(in crate::engine) fn intrinsic_to_world(&self, xi: f64, yi: f64) -> (f64, f64) {
        (
            self.x[0] + (xi - 0.5) * self.pixel_extent_x(),
            self.y[0] + (yi - 0.5) * self.pixel_extent_y(),
        )
    }
    /// Inside or on the border of the world limits.
    pub(in crate::engine) fn contains(&self, xw: f64, yw: f64) -> bool {
        self.x[0] <= xw && xw <= self.x[1] && self.y[0] <= yw && yw <= self.y[1]
    }
    /// (row, column) of the pixel nearest to the point (intrinsic coordinates
    /// rounded half away from zero, the far border kept in the last pixel);
    /// NaN for points outside the image.
    pub(in crate::engine) fn world_to_subscript(&self, xw: f64, yw: f64) -> (f64, f64) {
        if !self.contains(xw, yw) {
            return (f64::NAN, f64::NAN);
        }
        let (xi, yi) = self.world_to_intrinsic(xw, yw);
        (
            yi.round().clamp(1., self.size[0]),
            xi.round().clamp(1., self.size[1]),
        )
    }
    pub(in crate::engine) const PROPERTIES: [&'static str; 9] = [
        "XWorldLimits",
        "YWorldLimits",
        "ImageSize",
        "PixelExtentInWorldX",
        "PixelExtentInWorldY",
        "ImageExtentInWorldX",
        "ImageExtentInWorldY",
        "XIntrinsicLimits",
        "YIntrinsicLimits",
    ];
    pub(in crate::engine) const METHODS: [&'static str; 5] = [
        "contains",
        "intrinsicToWorld",
        "sizesMatch",
        "worldToIntrinsic",
        "worldToSubscript",
    ];
    /// A property as a real row vector or scalar.
    pub(in crate::engine) fn property(&self, name: &str) -> Option<Vec<f64>> {
        Some(match name {
            "XWorldLimits" => self.x.to_vec(),
            "YWorldLimits" => self.y.to_vec(),
            "ImageSize" => self.size.to_vec(),
            "PixelExtentInWorldX" => vec![self.pixel_extent_x()],
            "PixelExtentInWorldY" => vec![self.pixel_extent_y()],
            "ImageExtentInWorldX" => vec![self.x[1] - self.x[0]],
            "ImageExtentInWorldY" => vec![self.y[1] - self.y[0]],
            "XIntrinsicLimits" => vec![0.5, self.size[1] + 0.5],
            "YIntrinsicLimits" => vec![0.5, self.size[0] + 0.5],
            _ => return None,
        })
    }
    /// A copy with XWorldLimits, YWorldLimits or ImageSize changed.
    pub(in crate::engine) fn with_property(&self, name: &str, value: &[f64]) -> Result<Self> {
        let mut r = self.clone();
        match name {
            "XWorldLimits" => r.x = limits("XWorldLimits", value)?,
            "YWorldLimits" => r.y = limits("YWorldLimits", value)?,
            "ImageSize" => r.size = image_size(value)?,
            _ if Self::PROPERTIES.contains(&name) => {
                return Err(format!(
                    "You cannot set the read-only property '{name}' of imref2d."
                ));
            }
            _ => {
                return Err(format!(
                    "Unrecognized property '{name}' for class 'imref2d'."
                ));
            }
        }
        Ok(r)
    }
}
