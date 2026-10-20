//! Structured request logs: one JSON object per line.
//!
//! A request line holds the request ID, method, matched route template (never
//! the raw path or query), status, error code, tenant, principal and
//! duration. Headers, credentials and bodies are never logged, and neither
//! are graph IDs, so denied topology stays out of the log.
use serde_json::{json, Value};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// A line-oriented JSON log sink. Clones share the sink.
#[derive(Clone)]
pub struct Logger {
    sink: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Logger {
    pub fn new(sink: impl Write + Send + 'static) -> Self {
        Self {
            sink: Arc::new(Mutex::new(Box::new(sink))),
        }
    }
    /// Log to standard error.
    pub fn stderr() -> Self {
        Self::new(std::io::stderr())
    }
    /// Discard every line.
    pub fn discard() -> Self {
        Self::new(std::io::sink())
    }
    /// Write one event. `fields` must be a JSON object; it is merged after
    /// the timestamp, level and event name.
    pub fn event(&self, level: &str, event: &str, fields: Value) {
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut line = json!({"ts_ms": ts_ms, "level": level, "event": event});
        if let (Some(line), Value::Object(fields)) = (line.as_object_mut(), fields) {
            line.extend(fields);
        }
        if let Ok(mut sink) = self.sink.lock() {
            let _ = writeln!(sink, "{line}");
            let _ = sink.flush();
        }
    }
}

/// A `Write` that appends to a shared buffer, for tests that read the log.
#[derive(Clone, Default)]
pub struct MemorySink(pub Arc<Mutex<Vec<u8>>>);

impl MemorySink {
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log buffer")).into_owned()
    }
}

impl Write for MemorySink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| std::io::Error::other("log buffer poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
