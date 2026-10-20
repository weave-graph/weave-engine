//! One tenant: its own store and its own audit log (ADR-0007 section 6).
//!
//! A tenant's directory holds `store.db`, the engine store with its
//! registry and graphs, and `audit.db`, its audit log. A request reaches a
//! tenant only through the tenant its credential is bound to, so no request
//! can name a store outside its tenant.
//!
//! Engine execution takes SQLite's write lock (the operation clock reserves
//! it for every program), so the service runs one request at a time per
//! tenant on one connection. Requests wait for at most the queue timeout in
//! a bounded queue, and tenants run in parallel.
use crate::audit::AuditLog;
use crate::control::TenantRecord;
use crate::error::{codes, ApiError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;
use weave_engine::{Engine, TrustedClock};

/// Name of a tenant's engine store inside its directory.
pub const STORE_FILE: &str = "store.db";
/// Name of a tenant's audit log inside its directory.
pub const AUDIT_FILE: &str = "audit.db";

/// The engine and the audit log of one tenant, used together under one lock.
pub struct TenantState {
    pub engine: Engine,
    pub audit: AuditLog,
    /// Quota usage of the tenant's principals, under the same lock, so a
    /// request's check, write and count are atomic.
    pub quotas: crate::quota::Quotas,
}

pub struct Tenant {
    pub record: TenantRecord,
    pub dir: PathBuf,
    state: Mutex<TenantState>,
    queue: Semaphore,
    checkouts: AtomicU64,
}

/// The directory of a tenant under the service data directory.
pub fn tenant_dir(data_dir: &Path, tenant_id: &str) -> PathBuf {
    data_dir.join("tenants").join(tenant_id)
}

impl Tenant {
    pub fn open(
        data_dir: &Path,
        record: TenantRecord,
        clock: Arc<dyn TrustedClock>,
        queue: usize,
    ) -> Result<Self, ApiError> {
        let dir = tenant_dir(data_dir, &record.tenant_id);
        std::fs::create_dir_all(&dir).map_err(|_| ApiError::internal())?;
        let engine = Engine::open_with_clock(dir.join(STORE_FILE), clock)?;
        let audit = AuditLog::open(&dir.join(AUDIT_FILE))?;
        Ok(Self {
            record,
            dir,
            state: Mutex::new(TenantState {
                engine,
                audit,
                quotas: Default::default(),
            }),
            queue: Semaphore::new(queue.max(1)),
            checkouts: AtomicU64::new(0),
        })
    }

    pub fn id(&self) -> &str {
        &self.record.tenant_id
    }

    /// How many requests have used this tenant's store. Tests use it to show
    /// that a request of one tenant never touches another tenant's store.
    pub fn checkouts(&self) -> u64 {
        self.checkouts.load(Ordering::SeqCst)
    }

    /// Run `work` on this tenant's engine and audit log on a blocking thread,
    /// after waiting at most `wait` for a place in the queue.
    pub async fn run<T, F>(self: &Arc<Self>, wait: Duration, work: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&mut TenantState) -> T + Send + 'static,
    {
        let permit = match tokio::time::timeout(wait, self.queue.acquire()).await {
            Ok(Ok(permit)) => permit,
            _ => {
                return Err(ApiError::new(
                    codes::BACKPRESSURE,
                    "the service is busy; retry later",
                ))
            }
        };
        let tenant = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut state = tenant.state.lock().map_err(|_| ApiError::internal())?;
            tenant.checkouts.fetch_add(1, Ordering::SeqCst);
            Ok(work(&mut state))
        })
        .await
        .map_err(|_| ApiError::internal())?;
        drop(permit);
        result
    }

    /// Run `work` synchronously, for trusted local administration.
    pub fn with_state<T>(&self, work: impl FnOnce(&mut TenantState) -> T) -> Result<T, ApiError> {
        let mut state = self.state.lock().map_err(|_| ApiError::internal())?;
        self.checkouts.fetch_add(1, Ordering::SeqCst);
        Ok(work(&mut state))
    }
}
