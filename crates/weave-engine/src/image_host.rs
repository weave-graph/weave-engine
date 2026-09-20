//! Experimental trusted single-owner host profile. No browser durability is provided here.
use crate::{err, Engine, Result, SystemClock};
use rusqlite::{Connection, OpenFlags};
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

    /// Reopens a previously acknowledged image without silently creating or initializing it.
    /// Supported historical nonzero schema versions still use the ordinary migration path.
    pub fn open_restored_single_owner_image(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::default() & !OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 0 {
            return Err(err(
                "E_IMAGE_UNINITIALIZED",
                "restored image is not an initialized store",
            ));
        }
        Self::from_connection_profile(conn, Arc::new(SystemClock), || {}, true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HostContext, ManualClock};
    use serde_json::json;

    #[test]
    fn image_export_requires_quiescent_delete_profile() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open_single_owner_image(dir.path().join("image.db")).unwrap();
        engine.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert_eq!(
            engine.export_single_owner_image().unwrap_err().code,
            "E_IMAGE_PROFILE"
        );
        engine.conn.execute_batch("ROLLBACK").unwrap();
        assert!(engine
            .export_single_owner_image()
            .unwrap()
            .starts_with(b"SQLite format 3\0"));
        let ordinary = Engine::open(dir.path().join("wal.db")).unwrap();
        assert_eq!(
            ordinary.export_single_owner_image().unwrap_err().code,
            "E_IMAGE_PROFILE"
        );
    }

    #[test]
    fn oversized_committed_image_is_refused_before_copy() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open_single_owner_image(dir.path().join("image.db")).unwrap();
        engine
            .conn
            .execute_batch("CREATE TABLE fixture_blob(data BLOB)")
            .unwrap();
        engine
            .conn
            .execute(
                "INSERT INTO fixture_blob VALUES (zeroblob(?1))",
                [IMAGE_LIMIT as i64],
            )
            .unwrap();
        assert!(engine.conn.is_autocommit());
        assert_eq!(
            engine.export_single_owner_image().unwrap_err().code,
            "E_IMAGE_BUDGET"
        );
    }

    #[test]
    fn restored_sqlite_image_cannot_silently_initialize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ordinary.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE unrelated(value TEXT)")
            .unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        assert!(before.starts_with(b"SQLite format 3\0"));
        assert_eq!(
            Engine::open_restored_single_owner_image(&path)
                .err()
                .unwrap()
                .code,
            "E_IMAGE_UNINITIALIZED"
        );
        assert_eq!(before, std::fs::read(path).unwrap());
    }

    #[test]
    fn exporting_does_not_reset_the_engine_clock() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(10));
        let mut engine = Engine::from_connection_profile(
            Connection::open(dir.path().join("image.db")).unwrap(),
            clock.clone(),
            || {},
            true,
        )
        .unwrap();
        let host = HostContext::new("owner", ["g".into()]);
        let program = serde_json::from_value(json!({"version":"0.18.0","commands":[{
            "op":"commit","graph_id":"g","data":{"nodes":[]}
        }]}))
        .unwrap();
        engine.execute(&program, &host).unwrap();
        let query = serde_json::from_value(
            json!({"version":"0.18.0","commands":[{"op":"query","query":{"graph_id":"g"}}]}),
        )
        .unwrap();
        let samples = clock.samples();
        engine.export_single_owner_image().unwrap();
        assert_eq!(clock.samples(), samples);
        clock.set(9);
        assert_eq!(
            engine.execute(&query, &host).unwrap_err().code,
            "E_CLOCK_UNAVAILABLE"
        );
        clock.set(10);
        engine.execute(&query, &host).unwrap();
    }
}
