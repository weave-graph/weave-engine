//! Native authority time. Clock installation belongs to the trusted embedding host.
use super::{err, Engine, Result};
use std::sync::{
    atomic::{AtomicI64, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

/// A trusted host clock. Never deserialized from a graph, program, or request.
pub trait TrustedClock: Send + Sync {
    fn unix_millis(&self) -> Result<i64>;
}

#[derive(Default)]
pub struct SystemClock;
impl TrustedClock for SystemClock {
    fn unix_millis(&self) -> Result<i64> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| unavailable())?
            .as_millis();
        i64::try_from(millis).map_err(|_| unavailable())
    }
}

/// Explicit test/embedding-host clock. Installing it is a trusted constructor action.
/// Changing it during an operation does not change that operation's captured time.
pub struct ManualClock {
    time: AtomicI64,
    samples: AtomicUsize,
}
impl ManualClock {
    pub fn new(unix_millis: i64) -> Self {
        Self {
            time: AtomicI64::new(unix_millis),
            samples: AtomicUsize::new(0),
        }
    }
    pub fn set(&self, unix_millis: i64) {
        self.time.store(unix_millis, Ordering::SeqCst);
    }
    pub fn samples(&self) -> usize {
        self.samples.load(Ordering::SeqCst)
    }
}
impl TrustedClock for ManualClock {
    fn unix_millis(&self) -> Result<i64> {
        self.samples.fetch_add(1, Ordering::SeqCst);
        Ok(self.time.load(Ordering::SeqCst))
    }
}
fn unavailable() -> super::Error {
    err("E_CLOCK_UNAVAILABLE", "trusted operation clock unavailable")
}
#[derive(Default)]
struct State {
    last_sample: Option<i64>,
    active: Option<(i64, usize)>,
}
pub(crate) struct OperationClock {
    source: Arc<dyn TrustedClock>,
    state: Arc<Mutex<State>>,
}
pub(crate) struct ClockScope(
    Arc<Mutex<State>>,
    Option<super::read_budget::ReadScope>,
    Option<super::authorization::Scope>,
);
impl OperationClock {
    pub(crate) fn new(source: Arc<dyn TrustedClock>) -> Self {
        Self {
            source,
            state: Arc::new(Mutex::new(State::default())),
        }
    }
    fn enter(&self) -> Result<ClockScope> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if let Some((_, depth)) = &mut state.active {
            *depth = depth.checked_add(1).ok_or_else(unavailable)?;
        } else {
            let now = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.source.unix_millis()
            }))
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
            if now < 0 || state.last_sample.is_some_and(|last| now < last) {
                return Err(unavailable());
            }
            state.last_sample = Some(now);
            state.active = Some((now, 1));
        }
        Ok(ClockScope(self.state.clone(), None, None))
    }
    fn current(&self) -> Result<i64> {
        self.state
            .lock()
            .map_err(|_| unavailable())?
            .active
            .map(|(now, _)| now)
            .ok_or_else(unavailable)
    }
}
impl Drop for ClockScope {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            if let Some((_, depth)) = &mut state.active {
                *depth -= 1;
                if *depth == 0 {
                    state.active = None;
                }
            }
        }
    }
}
impl Engine {
    /// Must follow BEGIN/SAVEPOINT. A real table read establishes a deferred snapshot
    /// before sampling. Nested scopes retain the outer timestamp, even after clock updates.
    pub(crate) fn operation_scope(&self) -> Result<ClockScope> {
        if self.conn.is_autocommit() {
            return Err(err(
                "E_CLOCK_CONTEXT",
                "authority operation requires a storage snapshot",
            ));
        }
        self.conn
            .query_row("SELECT EXISTS(SELECT 1 FROM heads)", [], |r| {
                r.get::<_, bool>(0)
            })?;
        let mut scope = self.operation_clock.enter()?;
        scope.1 = Some(self.read_budget.enter());
        scope.2 = Some(self.authorization.enter());
        Ok(scope)
    }
    pub(crate) fn operation_write_scope(&self) -> Result<ClockScope> {
        if self.conn.is_autocommit() {
            return Err(err(
                "E_CLOCK_CONTEXT",
                "authority mutation requires a transaction",
            ));
        }
        // Reserve the writer before the captured-time check, without changing records.
        self.conn
            .execute("UPDATE heads SET revision=revision WHERE 0", [])?;
        self.operation_scope()
    }
    pub(crate) fn operation_time(&self) -> Result<i64> {
        self.operation_clock.current()
    }
    pub(crate) fn optional_read_transaction(&self) -> Result<Option<rusqlite::Transaction<'_>>> {
        if self.conn.is_autocommit() {
            Ok(Some(self.conn.unchecked_transaction()?))
        } else {
            Ok(None)
        }
    }
}

/// Restore the manual transaction boundary before propagating a trusted callback panic.
/// Ordinary errors still use each caller's existing rollback and error semantics.
pub(crate) fn rollback_unwind<T>(
    outcome: std::thread::Result<T>,
    connection: &rusqlite::Connection,
    rollback: &str,
) -> T {
    match outcome {
        Ok(value) => value,
        Err(panic) => {
            // Preserve the original panic. SQLite errors cannot be returned while unwinding.
            let _ = connection.execute_batch(rollback);
            std::panic::resume_unwind(panic)
        }
    }
}
