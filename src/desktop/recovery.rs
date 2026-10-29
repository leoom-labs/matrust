use matrust::{Result, session::Session};
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::mpsc::{self, Receiver, SyncSender},
    thread::JoinHandle,
};
struct Request {
    session: Session,
    ack: Option<SyncSender<Result<()>>>,
}
/// One writer owns the session lock. Screenshot/demo runs use no session store.
pub struct Recovery {
    sender: Option<SyncSender<Request>>,
    pub errors: Receiver<String>,
    thread: Option<JoinHandle<()>>,
    _lock: File,
}
impl Recovery {
    pub fn open(path: PathBuf) -> Result<Self> {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        let parent = path.parent().ok_or("Invalid recovery path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(parent.join("session.lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock().map_err(|_|"Another MatRust window owns session recovery; this window will not overwrite its state")?;
        let (sender, receive) = mpsc::sync_channel::<Request>(1);
        let (errors_tx, errors) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut store = matrust::session::SessionStore::new(&path);
            while let Ok(request) = receive.recv() {
                let result = store.save(request.session);
                if let Err(e) = &result {
                    let _ = errors_tx.send(e.clone());
                }
                if let Some(ack) = request.ack {
                    let _ = ack.send(result);
                }
            }
        });
        Ok(Self {
            sender: Some(sender),
            errors,
            thread: Some(thread),
            _lock: lock,
        })
    }
    pub fn queue(&self, session: Session) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|s| s.try_send(Request { session, ack: None }).is_ok())
    }
    pub fn flush(&self, session: Session) -> Result<()> {
        let (ack, receive) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .ok_or("Recovery stopped")?
            .send(Request {
                session,
                ack: Some(ack),
            })
            .map_err(|e| e.to_string())?;
        receive.recv().map_err(|e| e.to_string())?
    }
}
impl Drop for Recovery {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
