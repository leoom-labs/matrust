//! Clean-room Camera Calibrator launch and export hooks; the engine has no desktop
//! dependency. Behavior reference:
//! https://www.mathworks.com/help/vision/ref/cameracalibrator-app.html
use super::camera::{self, Camera};
use super::{Engine, Value};
use crate::{Array, CellArray, Result, plot::Figure};
use std::{
    cell::RefCell,
    collections::VecDeque,
    marker::PhantomData,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

/// What `cameraCalibrator` asked the desktop to open.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    Empty,
    Folder {
        folder: String,
        square_size: f64,
        units: Option<String>,
    },
    Files {
        files: Vec<String>,
        square_size: f64,
        units: Option<String>,
    },
}

type Open = Box<dyn FnMut(Launch) -> Result<()>>;
thread_local! { static OPEN: RefCell<Option<Open>> = RefCell::new(None); }

pub struct Presentation {
    previous: Option<Open>,
    thread: PhantomData<Rc<()>>,
}
impl Drop for Presentation {
    fn drop(&mut self) {
        OPEN.with(|s| *s.borrow_mut() = self.previous.take());
    }
}
pub fn present(open: impl FnMut(Launch) -> Result<()> + 'static) -> Presentation {
    Presentation {
        previous: OPEN.with(|s| s.replace(Some(Box::new(open)))),
        thread: PhantomData,
    }
}

/// The worker's fitted camera objects, independent of its workspace handles.
#[derive(Clone, Debug)]
pub struct CalibrationValues([Arc<Camera>; 2]);

impl CalibrationValues {
    pub fn capture(engine: &Engine, [params, errors]: [&str; 2]) -> Result<Self> {
        let object = |name| camera::camera_of(engine, &Value::from(engine.array(name)?));
        match (object(params), object(errors)) {
            (Some(params), Some(errors))
                if matches!(params.as_ref(), Camera::Params(_))
                    && matches!(errors.as_ref(), Camera::CalibrationErrors(_)) =>
            {
                Ok(Self([params, errors]))
            }
            _ => Err("Camera Calibrator: fitted objects are missing".into()),
        }
    }
}

/// Owns pending transfers for one app or figure until it closes.
#[derive(Default)]
pub struct TransferScope(Arc<AtomicBool>);

impl TransferScope {
    pub fn stager(&self) -> TransferStager {
        TransferStager {
            #[cfg(not(target_arch = "wasm32"))]
            owner: self.0.clone(),
        }
    }
}

impl Drop for TransferScope {
    fn drop(&mut self) {
        if let Ok(mut pending) = EXPORTS.lock() {
            // Close and remove tickets under the same lock used to stage them.
            self.0.store(true, Ordering::Relaxed);
            pending.retain(|entry| !Arc::ptr_eq(&entry.owner, &self.0));
        }
    }
}

/// A worker can stage on behalf of a scope without extending its lifetime.
#[derive(Clone)]
pub struct TransferStager {
    #[cfg(not(target_arch = "wasm32"))]
    owner: Arc<AtomicBool>,
}

impl TransferStager {
    /// The desktop script action can carry only text. Give it a one-use ticket
    /// for the two values so the workspace worker can import them losslessly.
    pub fn stage_export(&self, values: &CalibrationValues, names: [&str; 2]) -> Result<String> {
        let [params, errors] = names;
        for name in names {
            Engine::validate_workspace_name(name)?;
        }
        if params == errors {
            return Err("Output variable names must be distinct".into());
        }
        let ticket = self.stage_transfer(Transfer::Values(values.clone()))?;
        Ok(format!(
            "[{params}, {errors}] = {ticket}();\n\
             function [cc_params, cc_errors] = {ticket}()\n\
             cc_values = builtin('cameraCalibrator', '{ticket}');\n\
             cc_params = cc_values{{1}};\n\
             cc_errors = cc_values{{2}};\n\
             end\n"
        ))
    }

    /// Publish a figure prepared on the private app runner into the normal figure
    /// workspace. The receiver reports the desktop worker's installation result.
    pub fn stage_figure(&self, figure: Figure) -> Result<(String, mpsc::Receiver<Result<()>>)> {
        let (send, receive) = mpsc::channel();
        let ticket = self.stage_transfer(Transfer::Figure(Box::new(figure), send))?;
        Ok((
            format!(
                "{ticket}();\nfunction {ticket}()\n\
                 cc_figure = builtin('cameraCalibrator', '{ticket}');\nend\n"
            ),
            receive,
        ))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn stage_transfer(&self, transfer: Transfer) -> Result<String> {
        let mut pending = EXPORTS.lock().map_err(|_| "Camera export lock failed")?;
        if self.owner.load(Ordering::Relaxed) {
            return Err("Camera Calibrator: the transfer was cancelled on close".into());
        }
        if pending.len() >= MAX_TRANSFERS {
            return Err(
                "Camera Calibrator: too many pending exports; wait for earlier exports to finish"
                    .into(),
            );
        }
        let ticket = format!("cc_transfer_{}", uuid::Uuid::new_v4().simple());
        pending.push_back(PendingTransfer {
            owner: self.owner.clone(),
            ticket: ticket.clone(),
            transfer,
        });
        Ok(ticket)
    }

    #[cfg(target_arch = "wasm32")]
    fn stage_transfer(&self, _transfer: Transfer) -> Result<String> {
        Err("Camera Calibrator export requires the desktop".into())
    }
}

enum Transfer {
    Values(CalibrationValues),
    Figure(Box<Figure>, mpsc::Sender<Result<()>>),
}

struct PendingTransfer {
    owner: Arc<AtomicBool>,
    ticket: String,
    transfer: Transfer,
}

#[cfg(not(target_arch = "wasm32"))]
const MAX_TRANSFERS: usize = 16;
static EXPORTS: Mutex<VecDeque<PendingTransfer>> = Mutex::new(VecDeque::new());

fn receive_export(engine: &mut Engine, ticket: &str) -> Result<Vec<Value>> {
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("Camera Calibrator export requires the desktop".into());
    }
    let transfer = {
        let mut pending = EXPORTS.lock().map_err(|_| "Camera export lock failed")?;
        pending
            .iter()
            .position(|entry| entry.ticket == ticket)
            .and_then(|index| pending.remove(index))
            .map(|entry| entry.transfer)
            .ok_or("Camera Calibrator: export ticket was not found or was already used")?
    };
    let values = match transfer {
        Transfer::Values(values) => values,
        Transfer::Figure(figure, reply) => {
            let result: Result<Value> = (|| {
                engine.check_cancel()?;
                let mut out = crate::engine::Execution::default();
                let id = engine.graphics("figure", Vec::new(), &mut out)?;
                engine.figure = Some(*figure);
                Ok(id)
            })();
            let _ = reply.send(result.as_ref().map(|_| ()).map_err(Clone::clone));
            return result.map(|id| vec![id]);
        }
    };
    let mut slots = Vec::with_capacity(2);
    // Register fresh handles in the destination engine's object table.
    for value in values.0 {
        engine.check_cancel()?;
        let object = camera::new_object(engine, value.as_ref().clone())?;
        slots.push(Arc::new(object.array(&engine.cancel)?));
    }
    let cell = CellArray::from_shared(1, slots.len(), slots, &engine.cancel)?;
    Ok(vec![Value::Container(Array::Cell(cell))])
}

fn square_size(v: &Value) -> Result<f64> {
    let nums = super::image_basic::numbers(v)
        .ok_or("cameraCalibrator: squareSize must be a positive finite scalar")?;
    match nums.as_slice() {
        [s] if s.is_finite() && *s > 0.0 => Ok(*s),
        _ => Err("cameraCalibrator: squareSize must be a positive finite scalar".into()),
    }
}

pub(super) fn open(e: &mut Engine, args: &[Value], outputs: usize) -> Result<Vec<Value>> {
    if outputs == 1
        && let [arg] = args
        && let Some(ticket) = super::image_basic::text(e, arg)
        && ticket.starts_with("cc_transfer_")
    {
        return receive_export(e, &ticket);
    }
    if !matches!(args.len(), 0 | 2 | 3) || outputs != 0 {
        return Err(
            "cameraCalibrator: expected no inputs or (folderOrFiles, squareSize[, worldUnits]), and no outputs"
                .into(),
        );
    }
    if !OPEN.with(|s| s.borrow().is_some()) {
        return Err("cameraCalibrator requires the desktop".into());
    }
    let launch = if args.is_empty() {
        Launch::Empty
    } else {
        let square_size = square_size(&args[1])?;
        let units = match args.get(2) {
            Some(u) => Some(
                super::image_basic::text(e, u)
                    .ok_or("cameraCalibrator: worldUnits must be text")?,
            ),
            None => None,
        };
        if let Some(u) = &units
            && !matches!(
                u.trim().to_ascii_lowercase().as_str(),
                "mm" | "millimeter"
                    | "millimeters"
                    | "cm"
                    | "centimeter"
                    | "centimeters"
                    | "m"
                    | "meter"
                    | "meters"
                    | "in"
                    | "inch"
                    | "inches"
            )
        {
            return Err("cameraCalibrator: unrecognized worldUnits".into());
        }
        let resolve = |path: &str| -> Result<String> {
            Ok(e.file_path(path, "")?.to_string_lossy().into_owned())
        };
        let first = &args[0];
        if matches!(first, Value::Container(Array::Cell(_))) {
            let files = super::table_fns::text_list(e, first, "imageFiles")?
                .ok_or("cameraCalibrator: imageFiles must be a cell array of file paths")?;
            if files.is_empty() {
                return Err("cameraCalibrator: imageFiles must not be empty".into());
            }
            Launch::Files {
                files: files
                    .iter()
                    .map(|p| resolve(p))
                    .collect::<Result<Vec<_>>>()?,
                square_size,
                units,
            }
        } else if let Some(folder) = super::image_basic::text(e, first) {
            if folder.is_empty() {
                return Err("cameraCalibrator: folder path must not be empty".into());
            }
            Launch::Folder {
                folder: resolve(&folder)?,
                square_size,
                units,
            }
        } else if matches!(first, Value::Container(Array::Str(_))) {
            let files = super::table_fns::text_list(e, first, "imageFiles")?
                .ok_or("cameraCalibrator: imageFiles must be text")?;
            if files.is_empty() {
                return Err("cameraCalibrator: imageFiles must not be empty".into());
            }
            Launch::Files {
                files: files
                    .iter()
                    .map(|p| resolve(p))
                    .collect::<Result<Vec<_>>>()?,
                square_size,
                units,
            }
        } else {
            return Err(
                "cameraCalibrator: first argument must be a folder path or a cell array of image file paths"
                    .into(),
            );
        }
    };
    OPEN.with(|s| s.borrow_mut().as_mut().unwrap()(launch))?;
    Ok(Vec::new())
}
