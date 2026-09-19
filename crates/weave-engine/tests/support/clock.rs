//! Shared fixed-host fixture clock for legacy regression scenarios and recovery probes.
//! This is test code, not a way to submit time through production requests.
#![allow(dead_code)]
use std::{cell::Cell, path::Path, sync::Arc};
use weave_engine::{Engine, Result, TrustedClock};
thread_local! { static NOW: Cell<i64> = const { Cell::new(0) }; }
struct FixtureClock;
impl TrustedClock for FixtureClock {
    fn unix_millis(&self) -> Result<i64> {
        Ok(NOW.get())
    }
}
pub fn memory() -> Result<Engine> {
    Engine::memory_with_clock(Arc::new(FixtureClock))
}
pub fn open(path: impl AsRef<Path>) -> Result<Engine> {
    Engine::open_with_clock(path, Arc::new(FixtureClock))
}
pub fn at<T>(time: i64, f: impl FnOnce() -> T) -> T {
    NOW.set(time);
    f()
}
