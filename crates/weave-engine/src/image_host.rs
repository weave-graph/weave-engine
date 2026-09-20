//! Experimental trusted single-owner host profile. No browser durability is provided here.
use crate::{err, Engine, Result, SystemClock};
use rusqlite::Connection;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const IMAGE_LIMIT: usize = 8 * 1024 * 1024;

impl Engine {
    /// Opens the experimental DELETE-journal profile under exclusive host ownership.
    /// The host must prevent other connections and fence success on durable image storage.
    /// Restoring an image is trusted local storage recovery, never remote graph admission.
    pub fn open_single_owner_image(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection_profile(Connection::open(path)?, Arc::new(SystemClock), || {}, true)
    }

    /// Copies a quiescent committed image without resetting the operation clock.
    /// Returned bytes are not a durable receipt. This experiment caps the complete store at 8 MiB.
    pub fn export_single_owner_image(&self) -> Result<Vec<u8>> {
        if !self.conn.is_autocommit() {
            return Err(err(
                "E_IMAGE_PROFILE",
                "cannot export an active transaction",
            ));
        }
        let mode: String = self
            .conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if mode != "delete" {
            return Err(err(
                "E_IMAGE_PROFILE",
                "single-owner image journal unavailable",
            ));
        }
        self.conn.cache_flush()?;
        let path = self
            .conn
            .path()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| err("E_IMAGE_PROFILE", "file-backed image required"))?;
        let io_error = |_| err("E_IMAGE_STORAGE", "database image unavailable");
        let mut file = std::fs::File::open(path).map_err(io_error)?;
        let len = file.metadata().map_err(io_error)?.len();
        if len > IMAGE_LIMIT as u64 {
            return Err(err(
                "E_IMAGE_BUDGET",
                "experimental image capacity exceeded",
            ));
        }
        let mut image = vec![0; len as usize];
        file.read_exact(&mut image).map_err(io_error)?;
        let mut extra = [0];
        if file.read(&mut extra).map_err(io_error)? != 0 || !image.starts_with(b"SQLite format 3\0")
        {
            return Err(err("E_IMAGE_STORAGE", "database image changed or invalid"));
        }
        Ok(image)
    }
}
