//! Desktop launch and replay hooks for Volume Viewer. The engine has no UI dependency.
use super::{Engine, Value, image_basic};
use crate::{
    Array, GraphicsKind, NumericClass, Result, TypedArray,
    plot::{ImageDataClass, VolumeObject, volume::render_volume_viewport},
};
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

pub const MAX_VIEW_PIXELS_PER_EDGE: usize = 256;
const MAX_VOXELS: usize = 32 * 1024 * 1024;
const MAX_SLICE_PIXELS: usize = 1024 * 1024;
pub const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.25..=8.0;

#[derive(Clone, Debug)]
pub enum Launch {
    Empty,
    Volume { name: Option<String>, array: Array },
    Close,
}

type Open = Box<dyn FnMut(Launch) -> Result<()>>;
thread_local! { static OPEN: RefCell<Option<Open>> = RefCell::new(None); }

pub struct Presentation(Option<Open>, PhantomData<Rc<()>>);

impl Drop for Presentation {
    fn drop(&mut self) {
        OPEN.with(|slot| slot.replace(self.0.take()));
    }
}

pub fn present(open: impl FnMut(Launch) -> Result<()> + 'static) -> Presentation {
    Presentation(
        OPEN.with(|slot| slot.replace(Some(Box::new(open)))),
        PhantomData,
    )
}

pub fn volume_shape(array: &Array) -> Result<[usize; 3]> {
    if !matches!(
        array,
        Array::Numeric(_) | Array::Typed(_) | Array::Logical(_)
    ) {
        return Err("volumeViewer: expected a real numeric or logical 3-D array".into());
    }
    let [rows, cols, pages] = *array.dims() else {
        return Err("volumeViewer: expected a nonempty 3-D array".into());
    };
    validate_dimensions([rows, cols, pages])?;
    Ok([rows, cols, pages])
}

pub fn validate_dimensions([rows, cols, pages]: [usize; 3]) -> Result<()> {
    if [rows, cols, pages].contains(&0) {
        return Err("volumeViewer: expected a nonempty 3-D array".into());
    }
    let mut axes = [rows, cols, pages];
    axes.sort_unstable();
    if rows.saturating_mul(cols).saturating_mul(pages) > MAX_VOXELS
        || axes[1].saturating_mul(axes[2]) > MAX_SLICE_PIXELS
    {
        return Err(format!(
            "volumeViewer: volume exceeds {MAX_VOXELS} voxels or {MAX_SLICE_PIXELS} pixels per slice"
        ));
    }
    Ok(())
}

pub fn prepare_volume(array: &Array, cancel: &AtomicBool) -> Result<VolumeObject> {
    let dims = volume_shape(array)?;
    crate::dsp::check_cancel(cancel)?;
    let (data, class) = match array {
        Array::Numeric(m) => (
            copy_values(m.len(), |index| m.data()[index], cancel)?,
            ImageDataClass::Double,
        ),
        Array::Typed(m) => (
            copy_values(m.len(), |index| m.real(index), cancel)?,
            ImageDataClass::Typed(m.class()),
        ),
        Array::Logical(m) => (
            copy_values(m.len(), |index| f64::from(m.data()[index]), cancel)?,
            ImageDataClass::Logical,
        ),
        _ => unreachable!(),
    };
    crate::dsp::check_cancel(cancel)?;
    let volume = VolumeObject::new(dims, data, class);
    crate::dsp::check_cancel(cancel)?;
    Ok(volume)
}

fn copy_values(len: usize, read: impl Fn(usize) -> f64, cancel: &AtomicBool) -> Result<Arc<[f64]>> {
    let mut data = Arc::<[f64]>::new_uninit_slice(len);
    for (start, chunk) in Arc::get_mut(&mut data)
        .unwrap()
        .chunks_mut(4096)
        .enumerate()
    {
        crate::dsp::check_cancel(cancel)?;
        for (offset, value) in chunk.iter_mut().enumerate() {
            value.write(read(start * 4096 + offset));
        }
    }
    // SAFETY: Every element was initialized in the loop above.
    Ok(unsafe { data.assume_init() })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub view: [f64; 2],
    pub size: [usize; 2],
    pub zoom: f64,
    pub background: [u8; 3],
}

impl Viewport {
    pub fn render(self, volume: &VolumeObject, cancel: &AtomicBool) -> Result<Vec<[u8; 4]>> {
        let [width, height] = self.size;
        if !self.view.iter().all(|angle| angle.is_finite())
            || !ZOOM_RANGE.contains(&self.zoom)
            || ![width, height]
                .iter()
                .all(|n| (1..=MAX_VIEW_PIXELS_PER_EDGE).contains(n))
        {
            return Err("Volume Viewer: invalid camera or preview size".into());
        }
        volume.validate()?;
        let mut pixels =
            render_volume_viewport(volume, self.view, (width, height), self.zoom, cancel)?;
        for pixel in &mut pixels {
            let alpha = u32::from(pixel[3]);
            for (value, bg) in pixel[..3].iter_mut().zip(self.background) {
                *value =
                    ((u32::from(*value) * alpha + u32::from(bg) * (255 - alpha) + 127) / 255) as u8;
            }
            pixel[3] = 255;
        }
        Ok(pixels)
    }
}

// Replay the exact app viewport from the Volume object produced by volshow.
// Figure export also draws axes and applies its own layout and raster sizing.
fn replay(
    engine: &Engine,
    handle: &Value,
    camera: &Value,
    size: &Value,
    background: &Value,
) -> Result<Value> {
    let Value::Container(Array::Graphics(handles)) = handle else {
        return Err("Volume Viewer replay requires a live Volume handle".into());
    };
    let handle = handles.scalar_handle()?;
    if handle.kind() != GraphicsKind::Volume {
        return Err("Volume Viewer replay requires a live Volume handle".into());
    }
    let location = engine.locate_graphics(handle)?;
    let volume = engine
        .figures()
        .find(|(id, _)| *id == location.figure)
        .and_then(|(_, figure)| {
            figure
                .all_axes()
                .find(|(position, _)| Some(*position) == location.position)
        })
        .and_then(|(_, axes)| {
            axes.extras
                .volumes
                .iter()
                .find(|volume| volume.identity.same_object(&handle.object))
        })
        .ok_or("Volume Viewer replay could not find the Volume object")?;
    let [az, el, zoom] = replay_numbers(camera, "camera [azimuth elevation zoom]")?;
    let [width, height] = replay_numbers(size, "size [width height]")?;
    let [red, green, blue] = replay_numbers(background, "RGB background")?;
    if ![width, height]
        .iter()
        .all(|&n| n.fract() == 0.0 && (1.0..=MAX_VIEW_PIXELS_PER_EDGE as f64).contains(&n))
        || ![red, green, blue].iter().all(|v| (0.0..=1.0).contains(v))
    {
        return Err("Volume Viewer replay size or background is out of range".into());
    }
    let (width, height) = (width as usize, height as usize);
    let viewport = Viewport {
        view: [az, el],
        size: [width, height],
        zoom,
        background: [red, green, blue].map(|v| (v * 255.0).round() as u8),
    };
    let pixels = viewport.render(volume, &engine.cancel)?;
    let channel_values = (0..3).flat_map(|channel| {
        let pixels = &pixels;
        (0..width)
            .flat_map(move |x| (0..height).map(move |y| f64::from(pixels[y * width + x][channel])))
    });
    let rgb = TypedArray::from_reals(NumericClass::Uint8, height, width * 3, channel_values)?;
    Ok(Value::Container(rgb.with_dims(&[height, width, 3])?.into()))
}

fn replay_numbers<const N: usize>(value: &Value, field: &str) -> Result<[f64; N]> {
    if let Value::Matrix(matrix) = value
        && let Ok(values) = matrix.data().try_into()
    {
        return Ok(values);
    }
    Err(format!("Volume Viewer replay requires numeric {field}"))
}

pub(super) fn open(
    engine: &mut Engine,
    args: &[Value],
    expressions: Option<&[crate::syntax::Expr]>,
    outputs: usize,
) -> Result<Vec<Value>> {
    engine.check_cancel()?;
    if outputs == 1
        && let [operation, handle, camera, size, background] = args
        && image_basic::text(engine, operation).as_deref() == Some("replayViewport")
    {
        return Ok(vec![replay(engine, handle, camera, size, background)?]);
    }
    let usage = "volumeViewer: expected no inputs, (V), (V, 'VolumeType', 'Volume'), or close, and no outputs";
    if outputs != 0 {
        return Err(usage.into());
    }
    let launch = match args {
        [] => Launch::Empty,
        [command]
            if image_basic::text(engine, command)
                .is_some_and(|s| s.eq_ignore_ascii_case("close")) =>
        {
            Launch::Close
        }
        [value] | [value, _, _] => {
            if let [_, option, kind] = args
                && (!image_basic::text(engine, option)
                    .is_some_and(|s| s.eq_ignore_ascii_case("VolumeType"))
                    || !image_basic::text(engine, kind)
                        .is_some_and(|s| s.eq_ignore_ascii_case("Volume")))
            {
                return Err("volumeViewer: the supported option is 'VolumeType', 'Volume'".into());
            }
            if value.dims().len() != 3 {
                return Err("volumeViewer: expected a nonempty 3-D array".into());
            }
            let array = value.clone().array(&engine.cancel)?;
            volume_shape(&array)?;
            let name = match expressions.and_then(|args| args.first()) {
                Some(crate::syntax::Expr::Name(name)) => Some(name.clone()),
                _ => None,
            };
            Launch::Volume { name, array }
        }
        _ => return Err(usage.into()),
    };
    OPEN.with(|slot| {
        let mut slot = slot.borrow_mut();
        slot.as_mut().ok_or("volumeViewer requires the desktop")?(launch)
    })?;
    Ok(Vec::new())
}
