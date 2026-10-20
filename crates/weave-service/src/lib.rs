//! Authenticated multi-tenant Weave service (B3, #39; ADR-0007 sections 6-9).
//!
//! `weave-service` serves the core v1 JSON contract over HTTP/1.1 and TLS.
//! It is a thin adapter: every graph operation is one engine call under
//! [`weave_engine::HostContext`], with the authenticated principal as
//! `HostContext.principal`, and the registry of the caller's tenant
//! authorizes it.
//!
//! - **Authentication.** Every route, including unknown ones, requires a
//!   bearer API token. The token's tenant and principal come from the
//!   control store, never from the request, and the principal must be an
//!   active user of that tenant's registry on every request. Every failure
//!   is the same `401 E_UNAUTHENTICATED` response.
//! - **Tenants.** One store per tenant ([`tenant`]), with its own registry,
//!   graphs and audit log. A request can reach only its credential's tenant.
//! - **Audit.** Every authenticated request except `health` appends an entry
//!   to its tenant's audit log ([`audit`]) before the response is sent.
//! - **Limits.** Request bodies, queues, IDs and pages are bounded
//!   ([`Limits`]); the engine's own budgets apply inside each call.
//!
//! See `docs/SERVICE.md` for the API and its limits.
pub mod audit;
pub mod control;
pub mod error;
pub mod ids;
pub mod log;
pub mod tenant;
pub mod tls;

mod api;

use crate::control::{ControlStore, IssuedToken, DEFAULT_TOKEN_TTL_MS};
use crate::error::ApiError;
use crate::log::Logger;
use crate::tenant::{tenant_dir, Tenant};
use axum::http::Method;
use axum::Router;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use weave_engine::{HostContext, PrincipalKind, PrincipalStatus, TrustedClock};

/// Name of the control store inside the data directory.
pub const CONTROL_FILE: &str = "control.db";

/// Request limits of the service.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Largest request body.
    pub max_body_bytes: usize,
    /// Most requests queued or running per tenant.
    pub queue_per_tenant: usize,
    /// Longest wait for a place in a tenant's queue.
    pub queue_timeout: Duration,
    /// Most commands in one program.
    pub max_commands: usize,
    /// Most record IDs minted per request.
    pub max_ids: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: 16 * 1024 * 1024,
            queue_per_tenant: 256,
            queue_timeout: Duration::from_secs(10),
            max_commands: 64,
            max_ids: 1000,
        }
    }
}

struct Inner {
    data_dir: PathBuf,
    control: Mutex<ControlStore>,
    tenants: Mutex<HashMap<String, Arc<Tenant>>>,
    clock: Arc<dyn TrustedClock>,
    limits: Limits,
    log: Logger,
}

/// The service: a control store and the tenants it defines. Clones share
/// state.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

impl Service {
    /// Open (or create) the service data directory.
    pub fn open(
        data_dir: &Path,
        clock: Arc<dyn TrustedClock>,
        limits: Limits,
        log: Logger,
    ) -> Result<Self, ApiError> {
        std::fs::create_dir_all(data_dir.join("tenants")).map_err(|_| ApiError::internal())?;
        let control = ControlStore::open(&data_dir.join(CONTROL_FILE))?;
        Ok(Self(Arc::new(Inner {
            data_dir: data_dir.to_path_buf(),
            control: Mutex::new(control),
            tenants: Mutex::new(HashMap::new()),
            clock,
            limits,
            log,
        })))
    }

    pub fn limits(&self) -> &Limits {
        &self.0.limits
    }
    pub fn log(&self) -> &Logger {
        &self.0.log
    }
    pub fn data_dir(&self) -> &Path {
        &self.0.data_dir
    }

    /// The trusted clock's current time.
    pub fn now(&self) -> Result<i64, ApiError> {
        self.0.clock.unix_millis().map_err(ApiError::from)
    }

    pub(crate) fn control<T>(
        &self,
        work: impl FnOnce(&ControlStore) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let control = self.0.control.lock().map_err(|_| ApiError::internal())?;
        work(&control)
    }

    /// A tenant by ID, opened on first use. Only the control store names
    /// tenants; requests never do.
    pub fn tenant(&self, tenant_id: &str) -> Result<Arc<Tenant>, ApiError> {
        let mut tenants = self.0.tenants.lock().map_err(|_| ApiError::internal())?;
        if let Some(tenant) = tenants.get(tenant_id) {
            return Ok(tenant.clone());
        }
        let record = self
            .control(|c| c.tenant(tenant_id))?
            .ok_or_else(|| ApiError::new("E_UNAVAILABLE", "tenant unavailable"))?;
        let tenant = Arc::new(Tenant::open(
            &self.0.data_dir,
            record,
            self.0.clock.clone(),
            self.0.limits.queue_per_tenant,
        )?);
        tenants.insert(tenant_id.into(), tenant.clone());
        Ok(tenant)
    }

    /// Create a tenant with its own store and audit log, enable its registry
    /// with `admin` as the first tenant administrator, and issue `admin` a
    /// bootstrap token. Trusted local administration (the CLI); there is no
    /// HTTP route for it.
    pub fn create_tenant(&self, tenant_id: &str, admin: &str) -> Result<IssuedToken, ApiError> {
        let now = self.now()?;
        if tenant_dir(&self.0.data_dir, tenant_id).exists() {
            return Err(ApiError::new("E_CONFLICT", "the tenant already exists"));
        }
        self.control(|c| c.create_tenant(tenant_id, now))?;
        let tenant = self.tenant(tenant_id)?;
        tenant.with_state(|state| {
            state
                .engine
                .enable_registry(&HostContext::new(admin, Vec::<String>::new()))
        })??;
        self.issue_token(
            tenant_id,
            admin,
            admin,
            DEFAULT_TOKEN_TTL_MS,
            Some("bootstrap"),
        )
    }

    /// Issue an API token for an active user of a tenant. Trusted local
    /// administration; tenant administrators use `POST /v1/tokens`.
    pub fn issue_token(
        &self,
        tenant_id: &str,
        principal: &str,
        issued_by: &str,
        ttl_ms: i64,
        label: Option<&str>,
    ) -> Result<IssuedToken, ApiError> {
        let tenant = self.tenant(tenant_id)?;
        let active = tenant
            .with_state(|state| active_user(&state.engine, principal).map_err(ApiError::from))??;
        if !active {
            return Err(ApiError::new("E_UNAVAILABLE", "principal unavailable"));
        }
        let now = self.now()?;
        self.control(|c| c.issue_token(tenant_id, principal, issued_by, ttl_ms, label, now))
    }

    /// Revoke a token by ID, in any tenant. Trusted local administration.
    pub fn revoke_token(&self, token_id: &str) -> Result<bool, ApiError> {
        let now = self.now()?;
        self.control(|c| c.revoke_token_anywhere(token_id, now))
    }

    /// The HTTP router. Every route, and the fallback, authenticates first.
    pub fn router(&self) -> Router {
        api::router(self.clone())
    }

    /// Every route the router serves, as (method, path template).
    pub fn endpoints() -> Vec<(Method, &'static str)> {
        api::endpoints()
    }
}

/// True when `principal` is an active user of the engine's registry, read as
/// that principal (a principal can read its own registry record).
pub(crate) fn active_user(
    engine: &weave_engine::Engine,
    principal: &str,
) -> weave_engine::Result<bool> {
    match engine.registry_principal(
        principal,
        &HostContext::new(principal, Vec::<String>::new()),
    ) {
        Ok(p) => Ok(p.status == PrincipalStatus::Active && p.kind == PrincipalKind::User),
        Err(e) if e.code == "E_UNAVAILABLE" => Ok(false),
        Err(e) => Err(e),
    }
}
