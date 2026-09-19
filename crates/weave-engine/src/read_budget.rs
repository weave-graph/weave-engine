//! Shared counters for nested reads in one host operation. No process RSS claim.
use super::{err, Result};
use std::sync::{Arc, Mutex};
pub(crate) const READ_BYTES: usize = 128 * 1024 * 1024;
const READ_CALLS: usize = 4096;
struct Session {
    nesting: usize,
    bytes: usize,
    calls: usize,
}
#[derive(Clone, Default)]
pub(crate) struct ReadBudget(Arc<Mutex<Option<Session>>>);
pub(crate) struct ReadScope(ReadBudget);
impl ReadBudget {
    pub(crate) fn enter(&self) -> ReadScope {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let session = slot.get_or_insert(Session {
            nesting: 0,
            bytes: READ_BYTES,
            calls: READ_CALLS,
        });
        session.nesting += 1;
        ReadScope(self.clone())
    }
    pub(crate) fn remaining(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(READ_BYTES, |s| s.bytes)
    }
    pub(crate) fn request(&self) -> Result<()> {
        if let Some(s) = self.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            s.calls = s
                .calls
                .checked_sub(1)
                .ok_or_else(|| err("E_BUDGET", "cumulative graph-read count exceeded"))?;
        }
        Ok(())
    }
    pub(crate) fn charge(&self, bytes: usize) -> Result<()> {
        if let Some(s) = self.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            s.bytes = s
                .bytes
                .checked_sub(bytes)
                .ok_or_else(|| err("E_BUDGET", "cumulative graph-read byte budget exceeded"))?;
        }
        Ok(())
    }
}
impl Drop for ReadScope {
    fn drop(&mut self) {
        let mut slot = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = slot.as_mut() {
            s.nesting -= 1;
            if s.nesting == 0 {
                *slot = None;
            }
        }
    }
}
