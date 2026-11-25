//! `image`, `imagesc` and `imshow`, and the image object's properties.
//!
//! An image is an axes-owned [`ImageObject`] (src/plot/image.rs) holding raw
//! colour data; renderers resolve it through the axes colormap and CLim, so a
//! later `colormap`/`clim` recolours it. Semantics follow the MATLAB/Octave
//! documentation: `image` maps doubles directly onto 1-based colormap rows and
//! integers onto 0-based rows, M-by-N-by-3 arrays are truecolor, `imagesc`
//! scales through CLim (set to the data range or the given limits), and
//! `imshow` shows intensity images in gray(256) over the class range (or a
//! DisplayRange), keeps one data unit square (`axis image`) and hides the axes.
//! On axes that are not held, all three replace the plot, show the 2-D view
//! and reverse the y direction (`axis ij`).
use super::{Engine, Execution, Value};
use crate::engine::image_basic::{self, Img, Kind};
use crate::plot::{Axes, Figure, ImageDataClass, ImageMapping, ImageObject, colormap};
use crate::{Array, GraphicsArray, GraphicsHandle, GraphicsKind, Matrix, NumericClass, Result};
use std::sync::Arc;

/// Range of an intensity image's class (MATLAB `getrangefromclass`).
fn class_range(kind: Kind) -> [f64; 2] {
    match kind {
        Kind::Typed(NumericClass::Uint8) => [0., 255.],
        Kind::Typed(NumericClass::Uint16) => [0., 65535.],
        Kind::Typed(NumericClass::Int16) => [-32768., 32767.],
        Kind::Typed(NumericClass::Int8) => [-128., 127.],
        Kind::Typed(NumericClass::Uint32) => [0., 4294967295.],
        Kind::Typed(NumericClass::Int32) => [-2147483648., 2147483647.],
        // intmin/intmax as f64. 2^63-1 and 2^64-1 round to 2^63 and 2^64.
        Kind::Typed(c @ (NumericClass::Int64 | NumericClass::Uint64)) => {
            [c.min() as f64, c.max() as f64]
        }
        _ => [0., 1.],
    }
}

fn numbers(value: &Value, what: &str) -> Result<Vec<f64>> {
    image_basic::numbers(value).ok_or_else(|| format!("{what} must be numeric"))
}

fn limits_of(value: &Value, what: &str) -> Result<[f64; 2]> {
    match numbers(value, what)?.as_slice() {
        [a, b] if a.is_finite() && b.is_finite() && a < b => Ok([*a, *b]),
        _ => Err(format!("{what} must be two finite increasing values")),
    }
}

/// XData/YData from a vector: its first and last element.
fn span_of(value: &Value, what: &str) -> Result<[f64; 2]> {
    match numbers(value, what)?.as_slice() {
        [a] if a.is_finite() => Ok([*a, *a]),
        [a, .., b] if a.is_finite() && b.is_finite() => Ok([*a, *b]),
        _ => Err(format!("{what} must be a nonempty finite vector")),
    }
}

/// Builds an image from CData. `scaled` selects CDataMapping 'scaled' for
/// one-plane data; three planes are always truecolor.
fn image_object(
    e: &Engine,
    name: &str,
    value: &Value,
    scaled: bool,
) -> Result<(ImageObject, Kind)> {
    let img = image_basic::image(e, name, value)?;
    let zero_based = !matches!(img.kind, Kind::Double | Kind::Typed(NumericClass::Single));
    let mapping = match (img.c, img.kind) {
        (3, kind) => ImageMapping::TrueColor {
            full_scale: match kind {
                Kind::Typed(NumericClass::Uint8) => 255.,
                Kind::Typed(NumericClass::Uint16) => 65535.,
                _ => 1.,
            },
        },
        (1, _) if scaled => ImageMapping::Scaled,
        (1, _) => ImageMapping::Direct { zero_based },
        _ => return Err(format!("{name}: CData must be M-by-N or M-by-N-by-3")),
    };
    let mut object = ImageObject::new(img.h, img.w, Arc::from(img.data), mapping);
    object.data_class = match img.kind {
        Kind::Double => ImageDataClass::Double,
        Kind::Logical => ImageDataClass::Logical,
        Kind::Typed(class) => ImageDataClass::Typed(class),
    };
    object.direct_zero_based = zero_based;
    object.validate().map_err(|err| format!("{name}: {err}"))?;
    Ok((object, img.kind))
}

fn is_text(value: &Value) -> bool {
    matches!(value, Value::Text(_)) || matches!(value, Value::Container(Array::Str(_)))
}

impl Engine {
    pub(in crate::engine) fn image_draw(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Value> {
        let positional = args.iter().position(is_text).unwrap_or(args.len());
        let (lead, pairs) = args.split_at(positional);
        // imshow(filename, ...) reads the file first.
        let (lead, pairs): (Vec<Value>, &[Value]) =
            if name == "imshow" && lead.is_empty() && !pairs.is_empty() {
                let file = pairs[0].clone();
                let mut rest = &pairs[1..];
                let mut lead = vec![self.call("imread", vec![file], out)?];
                while let Some(v) = rest.first().filter(|v| !is_text(v)) {
                    lead.push(v.clone());
                    rest = &rest[1..];
                }
                (lead, rest)
            } else {
                (lead.to_vec(), pairs)
            };
        if !pairs.len().is_multiple_of(2) {
            return Err(format!("{name}: name-value arguments must come in pairs"));
        }
        let mut x_data = None;
        let mut y_data = None;
        let mut mapping_name: Option<bool> = None;
        let mut visible = None;
        let mut display_range: Option<Option<[f64; 2]>> = None;
        let mut map_arg: Option<Vec<[f64; 3]>> = None;
        for pair in pairs.chunks(2) {
            let key = super::text(&pair[0])?.to_ascii_lowercase();
            match key.as_str() {
                "xdata" => x_data = Some(span_of(&pair[1], "XData")?),
                "ydata" => y_data = Some(span_of(&pair[1], "YData")?),
                "visible" => {
                    visible = Some(match super::text(&pair[1])?.to_ascii_lowercase().as_str() {
                        "on" => true,
                        "off" => false,
                        _ => return Err("Visible must be 'on' or 'off'".into()),
                    });
                }
                "cdatamapping" if name != "imshow" => {
                    mapping_name =
                        Some(match super::text(&pair[1])?.to_ascii_lowercase().as_str() {
                            "scaled" => true,
                            "direct" => false,
                            _ => return Err("CDataMapping must be 'scaled' or 'direct'".into()),
                        })
                }
                "displayrange" if name == "imshow" => {
                    display_range = Some(match numbers(&pair[1], "DisplayRange")?.as_slice() {
                        [] => None,
                        _ => Some(limits_of(&pair[1], "DisplayRange")?),
                    })
                }
                "colormap" if name == "imshow" => map_arg = Some(colormap_rows(&pair[1])?),
                "initialmagnification" | "border" | "interpolation" | "reduce"
                    if name == "imshow" => {}
                other => {
                    return Err(format!(
                        "{name}: unsupported property '{other}' (supported: XData, YData{})",
                        if name == "imshow" {
                            ", DisplayRange, Colormap, InitialMagnification, Border"
                        } else {
                            ", CDataMapping"
                        }
                    ));
                }
            }
        }
        let mut clim: Option<[f64; 2]> = None;
        let (cdata, xy) = match (name, lead.as_slice()) {
            (_, [c]) => (c, None),
            ("imagesc", [c, lim]) => {
                clim = Some(limits_of(lim, "imagesc color limits")?);
                (c, None)
            }
            ("imshow", [c, second]) => {
                let dims = second.dims();
                if dims.len() == 2
                    && dims[1] == 3
                    && dims[0] > 0
                    && !(dims[0] == 1 && c.dims().len() > 2)
                {
                    map_arg = Some(colormap_rows(second)?);
                } else {
                    display_range =
                        Some(match numbers(second, "imshow display range")?.as_slice() {
                            [] => None,
                            _ => Some(limits_of(second, "imshow display range")?),
                        });
                }
                (c, None)
            }
            ("image" | "imagesc", [x, y, c]) => (c, Some((x, y))),
            ("imagesc", [x, y, c, lim]) => {
                clim = Some(limits_of(lim, "imagesc color limits")?);
                (c, Some((x, y)))
            }
            _ => {
                return Err(match name {
                    "image" => "Use image(C), image(x, y, C) or Name,Value pairs".into(),
                    "imagesc" => "Use imagesc(C), imagesc(x, y, C) or imagesc(..., clims)".into(),
                    _ => "Use imshow(I), imshow(I, [low high]), imshow(X, map) or imshow(filename)"
                        .into(),
                });
            }
        };
        let indexed = map_arg.is_some() && name == "imshow";
        let scaled = match name {
            "imagesc" => true,
            "imshow" => !indexed,
            _ => mapping_name.unwrap_or(false),
        };
        let (mut object, kind) = image_object(self, name, cdata, scaled)?;
        if let Some((x, y)) = xy {
            object.x = span_of(x, "x")?;
            object.y = span_of(y, "y")?;
        }
        if let Some(x) = x_data {
            object.x = x;
        }
        if let Some(y) = y_data {
            object.y = y;
        }
        let command_mapping = object.mapping;
        let truecolor = matches!(command_mapping, ImageMapping::TrueColor { .. });
        let held = self.figure.as_ref().is_some_and(|f| f.hold);
        let previous = self.figure.clone();
        let mut figure: Figure = self.surface_target();
        super::plot_carry_over(previous.as_ref(), &mut figure, &mut [])?;
        let axes_defaults = self.graphics_plot_axes_defaults(&mut figure)?;
        self.apply_image_defaults(&mut object, &figure.identity)?;
        if let Some(visible) = visible {
            object.visible = visible;
        }
        if matches!(name, "imagesc" | "imshow") || mapping_name.is_some() {
            object.mapping = command_mapping;
        } else if let (ImageMapping::Direct { zero_based }, ImageMapping::Direct { .. }) =
            (command_mapping, object.mapping)
        {
            object.mapping = ImageMapping::Direct { zero_based };
        }
        let axes: &mut Axes = &mut figure.axes;
        if !held {
            axes.view = Axes::default().view;
            axes.extras.y_reversed = true;
            // Octave 10.3 __img__: a replacing image/imagesc puts grid and ticks on top.
            axes.extras.layer = crate::plot::AxesLayer::Top;
        }
        let token = GraphicsHandle::axes_object(
            &figure.identity,
            &axes.identity,
            &object.identity,
            GraphicsKind::Image,
        );
        let data_range = object.value_range();
        let scaled = object.mapping == ImageMapping::Scaled;
        axes.extras.images.push(object);
        match name {
            // image(..., 'CDataMapping', 'scaled') also sets automatic limits.
            "imagesc" | "image" if name == "imagesc" || scaled => {
                let range = clim.or_else(|| {
                    let mut range: Option<[f64; 2]> = None;
                    for image in axes
                        .extras
                        .images
                        .iter()
                        .filter(|i| i.mapping == ImageMapping::Scaled)
                    {
                        if let Some(r) = image.value_range() {
                            range = Some(range.map_or(r, |a| [a[0].min(r[0]), a[1].max(r[1])]));
                        }
                    }
                    range
                });
                axes.extras.clim = Some(match range {
                    Some([a, b]) if a < b => [a, b],
                    Some([a, _]) => [a - 1., a + 1.],
                    None => [0., 1.],
                });
            }
            "imshow" => {
                if indexed {
                    axes.extras.colormap = map_arg.map(Arc::new);
                } else if !truecolor {
                    let range = match display_range {
                        Some(Some(r)) => r,
                        Some(None) => match data_range {
                            Some([a, b]) if a < b => [a, b],
                            _ => class_range(kind),
                        },
                        None => class_range(kind),
                    };
                    axes.extras.clim = Some(range);
                    axes.extras.colormap = Some(Arc::new(match map_arg {
                        Some(map) => map,
                        None => colormap::named("gray", 256)?.unwrap_or_default(),
                    }));
                }
                axes.extras.equal_aspect = true;
                axes.extras.axis_off = true;
            }
            _ => {}
        }
        figure.prepare_legend_layout()?;
        figure.validate()?;
        self.check_cancel()?;
        self.graphics_finish_axes_defaults(&figure, axes_defaults);
        self.figure = Some(figure);
        Ok(Value::Container(
            GraphicsArray::scalar(token, &self.cancel)?.into(),
        ))
    }
}

fn colormap_rows(value: &Value) -> Result<Vec<[f64; 3]>> {
    let data = numbers(value, "Colormap")?;
    let dims = value.dims();
    if dims.len() != 2 || dims[1] != 3 || dims[0] == 0 {
        return Err("Colormap must be an m-by-3 matrix".into());
    }
    let m = dims[0];
    if m > crate::plot::MAX_COLORMAP_ROWS {
        return Err("Colormap length exceeds 4096 rows".into());
    }
    let rows: Vec<[f64; 3]> = (0..m)
        .map(|i| [data[i], data[i + m], data[i + 2 * m]])
        .collect();
    if !rows.iter().flatten().all(|v| (0. ..=1.).contains(v)) {
        return Err("Colormap values must be in the range [0, 1]".into());
    }
    Ok(rows)
}

/// get(h, Name) on an image.
pub(super) fn image_get(
    e: &Engine,
    image: &ImageObject,
    name: &str,
    parent: Value,
) -> Result<Value> {
    let span =
        |r: [f64; 2]| -> Result<Value> { Ok(Value::Matrix(Matrix::from_column_slice(1, 2, &r)?)) };
    Ok(match name {
        "type" => Value::Text("image".into()),
        "parent" => parent,
        "cdata" => {
            let planes = if matches!(image.mapping, ImageMapping::TrueColor { .. }) {
                3
            } else {
                1
            };
            let kind = match image.data_class {
                ImageDataClass::Double => Kind::Double,
                ImageDataClass::Logical => Kind::Logical,
                ImageDataClass::Typed(class) => Kind::Typed(class),
            };
            image_basic::to_value(
                e,
                Img {
                    h: image.rows,
                    w: image.cols,
                    c: planes,
                    kind,
                    data: image.data.to_vec(),
                },
            )?
        }
        "xdata" => span(image.x)?,
        "ydata" => span(image.y)?,
        "cdatamapping" => Value::Text(
            if image.mapping == ImageMapping::Scaled {
                "scaled"
            } else {
                "direct"
            }
            .into(),
        ),
        "visible" => Value::Text(if image.visible { "on" } else { "off" }.into()),
        _ => {
            return Err(
                "Supported image queries: Type, Parent, CData, XData, YData, CDataMapping, Visible"
                    .into(),
            );
        }
    })
}

/// set(h, Name, Value) on an image.
pub(super) fn image_set(e: &Engine, image: &mut ImageObject, pair: &[Value]) -> Result<()> {
    let name = super::text(&pair[0])?.to_ascii_lowercase();
    match name.as_str() {
        "cdata" => {
            let scaled = image.mapping == ImageMapping::Scaled;
            let (mut new, _) = image_object(e, "set", &pair[1], scaled)?;
            new.identity = image.identity.clone();
            new.visible = image.visible;
            if new.rows == image.rows && new.cols == image.cols {
                new.x = image.x;
                new.y = image.y;
            }
            *image = new;
        }
        "xdata" => image.x = span_of(&pair[1], "XData")?,
        "ydata" => image.y = span_of(&pair[1], "YData")?,
        "visible" => {
            image.visible = match super::text(&pair[1])?.to_ascii_lowercase().as_str() {
                "on" => true,
                "off" => false,
                _ => return Err("Visible must be 'on' or 'off'".into()),
            }
        }
        "cdatamapping" => {
            if matches!(image.mapping, ImageMapping::TrueColor { .. }) {
                return Ok(());
            }
            image.mapping = match super::text(&pair[1])?.to_ascii_lowercase().as_str() {
                "scaled" => ImageMapping::Scaled,
                "direct" => ImageMapping::Direct {
                    zero_based: image.direct_zero_based,
                },
                _ => return Err("CDataMapping must be 'scaled' or 'direct'".into()),
            }
        }
        _ => {
            return Err(format!(
                "Supported image properties: CData, XData, YData, CDataMapping, Visible; '{}' is not supported yet",
                super::text(&pair[0])?
            ));
        }
    }
    image.validate()
}
