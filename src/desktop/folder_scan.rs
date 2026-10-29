//! One bounded background directory scanner; superseded requests never publish.
use matrust::{Result, session::FolderEntry};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

const MAX_ENTRIES: usize = 100_000;
const MAX_VISITED: usize = 1_000_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;

pub struct Listing {
    pub entries: Vec<FolderEntry>,
    pub visited: usize,
    pub truncated: bool,
}
pub fn scan(path: &Path, filter: &str, cancel: &AtomicBool) -> Result<Listing> {
    scan_limited(path, filter, cancel, MAX_ENTRIES, MAX_VISITED, MAX_BYTES)
}
fn scan_limited(
    path: &Path,
    filter: &str,
    cancel: &AtomicBool,
    max_entries: usize,
    max_visited: usize,
    max_bytes: usize,
) -> Result<Listing> {
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err("Cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    if filter.len() > 1024 {
        return Err("Filename filter exceeds 1024 bytes".into());
    }
    let filter = filter.to_lowercase();
    let mut entries = Vec::new();
    let mut visited = 0;
    let mut bytes = 0;
    let mut truncated = false;
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        check()?;
        if visited == max_visited {
            truncated = true;
            break;
        }
        visited += 1;
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let key = name.to_lowercase();
        if !key.contains(&filter) {
            continue;
        }
        let path = entry.path();
        let cost = path.as_os_str().as_encoded_bytes().len()
            + name.len()
            + key.len()
            + std::mem::size_of::<(String, FolderEntry)>();
        if entries.len() == max_entries || cost > max_bytes - bytes {
            truncated = true;
            break;
        }
        bytes += cost;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let directory = kind.is_dir() || (kind.is_symlink() && path.is_dir());
        entries.push((
            key,
            FolderEntry {
                path,
                name,
                directory,
            },
        ));
    }
    check()?;
    entries.sort_unstable_by(|(a, x), (b, y)| {
        (!x.directory, a, &x.name).cmp(&(!y.directory, b, &y.name))
    });
    check()?;
    Ok(Listing {
        entries: entries.into_iter().map(|(_, entry)| entry).collect(),
        visited,
        truncated,
    })
}

struct Request {
    resolve: bool,
    generation: u64,
    path: PathBuf,
    filter: String,
    cancel: Arc<AtomicBool>,
}
pub struct Response {
    pub directory: Option<PathBuf>,
    pub generation: u64,
    pub result: Result<Listing>,
}
#[derive(Default)]
struct State {
    pending: Option<Request>,
    response: Option<Response>,
    stop: bool,
}
pub struct Scanner {
    state: Arc<(Mutex<State>, Condvar)>,
    cancel: Arc<AtomicBool>,
    generation: u64,
    thread: Option<JoinHandle<()>>,
}
impl Scanner {
    pub fn new(context: eframe::egui::Context) -> Self {
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let shared = state.clone();
        let thread = std::thread::spawn(move || {
            loop {
                let request = {
                    let (lock, wake) = &*shared;
                    let mut state = lock.lock().unwrap();
                    while state.pending.is_none() && !state.stop {
                        state = wake.wait(state).unwrap();
                    }
                    if state.stop {
                        return;
                    }
                    state.pending.take().unwrap()
                };
                let mut directory = None;
                let result = (|| {
                    if request.cancel.load(Ordering::Relaxed) {
                        return Err("Cancelled".into());
                    }
                    let path = if request.resolve {
                        std::fs::canonicalize(&request.path).map_err(|e| e.to_string())?
                    } else {
                        request.path.clone()
                    };
                    let listing = scan(&path, &request.filter, &request.cancel)?;
                    if request.resolve {
                        directory = Some(path);
                    }
                    Ok(listing)
                })();
                let mut state = shared.0.lock().unwrap();
                if state.stop {
                    return;
                }
                if !request.cancel.load(Ordering::Relaxed) {
                    state.response = Some(Response {
                        directory,
                        generation: request.generation,
                        result,
                    });
                    drop(state);
                    context.request_repaint_of(eframe::egui::ViewportId::ROOT);
                }
            }
        });
        Self {
            state,
            cancel: Arc::new(AtomicBool::new(false)),
            generation: 0,
            thread: Some(thread),
        }
    }
    pub fn request(&mut self, path: PathBuf, filter: String) -> u64 {
        self.submit(path, filter, false)
    }
    pub fn navigate(&mut self, path: PathBuf) -> u64 {
        self.submit(path, String::new(), true)
    }
    fn submit(&mut self, path: PathBuf, filter: String, resolve: bool) -> u64 {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation = self.generation.wrapping_add(1);
        let mut state = self.state.0.lock().unwrap();
        state.response = None;
        state.pending = Some(Request {
            resolve,
            generation: self.generation,
            path,
            filter,
            cancel: self.cancel.clone(),
        });
        self.state.1.notify_one();
        self.generation
    }
    pub fn take(&self) -> Option<Response> {
        self.state.0.lock().unwrap().response.take()
    }
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let mut state = self.state.0.lock().unwrap();
        state.pending = None;
        state.response = None;
    }
}

#[cfg(test)]
#[path = "folder_scan_tests.rs"]
mod tests;
impl Drop for Scanner {
    fn drop(&mut self) {
        self.cancel();
        self.state.0.lock().unwrap().stop = true;
        self.state.1.notify_one();
        // OS directory calls are not interruptible. Do not block window teardown
        // on a disconnected share; the owned thread exits after that call returns.
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
    }
}
