//! Typed, live document target. Figure numbers alone cannot authorize actions.
use matrust::{Engine, ObjectIdentity, Result};
use std::sync::atomic::Ordering;
#[derive(Clone)]
pub struct Key {
    pub number: u32,
    pub identity: ObjectIdentity,
}
impl Key {
    pub fn same(&self, other: &Self) -> bool {
        self.number == other.number && self.identity.same_object(&other.identity)
    }
    pub fn check(&self, engine: &Engine) -> Result<()> {
        if engine.cancellation().load(Ordering::Relaxed) {
            return Err("Figure action cancelled".into());
        }
        if !engine
            .figures()
            .any(|(id, f)| id == self.number && self.identity.same_object(&f.identity))
        {
            return Err("This figure document is no longer open".into());
        }
        Ok(())
    }
}
pub struct Select {
    pub key: Key,
    pub ticket: u64,
}
pub struct Acknowledgement {
    pub key: Key,
    pub ticket: u64,
    pub accepted: bool,
}
