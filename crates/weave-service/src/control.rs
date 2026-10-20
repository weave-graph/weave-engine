//! The control store (ADR-0007 section 6): tenant definitions and hashed API
//! tokens with their tenant binding. It holds no graph data and no registry
//! state; each tenant's registry, graphs and audit log live with the tenant.
//!
//! A token is `wvs_` followed by 64 hex digits (256 random bits). It is shown
//! once, when issued. The store keeps only its SHA-256 digest, a public token
//! ID used to list and revoke it, its tenant and principal, and its expiry.
use crate::error::ApiError;
use crate::ids::{hex, random, sha256_hex};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

/// Prefix of every API token.
pub const TOKEN_PREFIX: &str = "wvs_";
/// Length of a well-formed token.
pub const TOKEN_LEN: usize = TOKEN_PREFIX.len() + 64;
/// Longest token lifetime: 365 days.
pub const MAX_TOKEN_TTL_MS: i64 = 365 * 24 * 3600 * 1000;
/// Shortest token lifetime: one minute.
pub const MIN_TOKEN_TTL_MS: i64 = 60 * 1000;
/// Default token lifetime: 30 days.
pub const DEFAULT_TOKEN_TTL_MS: i64 = 30 * 24 * 3600 * 1000;
/// Most live (unexpired, unrevoked) tokens one principal may hold.
pub const MAX_TOKENS_PER_PRINCIPAL: i64 = 100;
/// Longest token label.
pub const MAX_LABEL_BYTES: usize = 256;

const SCHEMA_VERSION: i64 = 1;

/// A tenant as the control store defines it.
#[derive(Debug, Clone)]
pub struct TenantRecord {
    pub tenant_id: String,
    /// The tenant's signed-admission audience (ADR-0007 section 6).
    pub audience: String,
    /// Key for service-minted record IDs ([`crate::ids`]). It authenticates
    /// no one; see SERVICE.md.
    pub id_key: [u8; 32],
    pub created_at_ms: i64,
}

/// What a valid token authenticates.
#[derive(Debug, Clone)]
pub struct TokenBinding {
    pub token_id: String,
    pub tenant_id: String,
    pub principal: String,
    pub expires_at_ms: i64,
}

/// A newly issued token. `token` is the only copy of the secret.
#[derive(Debug, Clone, Serialize)]
pub struct IssuedToken {
    pub token_id: String,
    pub token: String,
    pub principal: String,
    pub expires_at_ms: i64,
}

/// A token as listings report it: never the secret or its digest.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TokenInfo {
    pub token_id: String,
    pub principal: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub issued_by: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at_ms: Option<i64>,
}

pub struct ControlStore {
    conn: Connection,
}

/// Tenant IDs are operator-chosen names: 1 to 63 bytes of `a-z`, `0-9`
/// and `-`, starting with a letter or digit. They name a directory.
pub fn valid_tenant_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 63
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

fn storage(error: rusqlite::Error) -> ApiError {
    let _ = error;
    ApiError::internal()
}

impl ControlStore {
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        let conn = Connection::open(path).map_err(storage)?;
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS control_meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS tenants(
               tenant_id TEXT PRIMARY KEY,
               audience TEXT NOT NULL UNIQUE,
               id_key BLOB NOT NULL,
               created_at_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS api_tokens(
               token_id TEXT PRIMARY KEY,
               token_sha256 TEXT NOT NULL UNIQUE,
               tenant_id TEXT NOT NULL REFERENCES tenants(tenant_id),
               principal TEXT NOT NULL,
               label TEXT,
               issued_by TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               expires_at_ms INTEGER NOT NULL,
               revoked_at_ms INTEGER);
             CREATE INDEX IF NOT EXISTS api_tokens_by_principal ON api_tokens(tenant_id, principal);",
        )
        .map_err(storage)?;
        let version: Option<i64> = conn
            .query_row(
                "SELECT value FROM control_meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        match version {
            None => {
                conn.execute(
                    "INSERT INTO control_meta VALUES ('schema_version', ?1)",
                    [SCHEMA_VERSION],
                )
                .map_err(storage)?;
            }
            Some(SCHEMA_VERSION) => {}
            Some(_) => {
                return Err(ApiError::new(
                    "E_STORAGE_VERSION",
                    "control store schema version is unsupported",
                ))
            }
        }
        Ok(Self { conn })
    }

    pub fn create_tenant(&self, tenant_id: &str, now_ms: i64) -> Result<TenantRecord, ApiError> {
        if !valid_tenant_id(tenant_id) {
            return Err(ApiError::input(
                "tenant IDs are 1 to 63 bytes of a-z, 0-9 and '-'",
            ));
        }
        let record = TenantRecord {
            tenant_id: tenant_id.into(),
            audience: format!("weave-service:{tenant_id}:{}", hex(&random::<16>())),
            id_key: random::<32>(),
            created_at_ms: now_ms,
        };
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO tenants VALUES (?1,?2,?3,?4)",
                params![
                    record.tenant_id,
                    record.audience,
                    record.id_key.as_slice(),
                    now_ms
                ],
            )
            .map_err(storage)?;
        if inserted == 0 {
            return Err(ApiError::new("E_CONFLICT", "the tenant already exists"));
        }
        Ok(record)
    }

    pub fn tenant(&self, tenant_id: &str) -> Result<Option<TenantRecord>, ApiError> {
        self.conn
            .query_row(
                "SELECT tenant_id,audience,id_key,created_at_ms FROM tenants WHERE tenant_id=?1",
                [tenant_id],
                |r| {
                    let key: Vec<u8> = r.get(2)?;
                    Ok((r.get(0)?, r.get(1)?, key, r.get(3)?))
                },
            )
            .optional()
            .map_err(storage)?
            .map(|(tenant_id, audience, key, created_at_ms)| {
                let id_key: [u8; 32] = key.try_into().map_err(|_| ApiError::internal())?;
                Ok(TenantRecord {
                    tenant_id,
                    audience,
                    id_key,
                    created_at_ms,
                })
            })
            .transpose()
    }

    pub fn tenant_ids(&self) -> Result<Vec<String>, ApiError> {
        let mut statement = self
            .conn
            .prepare("SELECT tenant_id FROM tenants ORDER BY tenant_id")
            .map_err(storage)?;
        let rows = statement
            .query_map([], |r| r.get(0))
            .map_err(storage)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// Issue a token. The caller has already checked that `principal` is an
    /// active principal of `tenant_id` that may hold one.
    pub fn issue_token(
        &self,
        tenant_id: &str,
        principal: &str,
        issued_by: &str,
        ttl_ms: i64,
        label: Option<&str>,
        now_ms: i64,
    ) -> Result<IssuedToken, ApiError> {
        if !(MIN_TOKEN_TTL_MS..=MAX_TOKEN_TTL_MS).contains(&ttl_ms) {
            return Err(ApiError::input(
                "token lifetimes are one minute to 365 days",
            ));
        }
        if label.is_some_and(|l| l.len() > MAX_LABEL_BYTES) {
            return Err(ApiError::input("token labels are at most 256 bytes"));
        }
        let live: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM api_tokens WHERE tenant_id=?1 AND principal=?2
                 AND revoked_at_ms IS NULL AND expires_at_ms>?3",
                params![tenant_id, principal, now_ms],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if live >= MAX_TOKENS_PER_PRINCIPAL {
            return Err(ApiError::new(
                "E_BUDGET",
                "a principal holds at most 100 live tokens",
            ));
        }
        let token = format!("{TOKEN_PREFIX}{}", hex(&random::<32>()));
        let issued = IssuedToken {
            token_id: format!("tok-{}", hex(&random::<16>())),
            principal: principal.into(),
            expires_at_ms: now_ms.saturating_add(ttl_ms),
            token,
        };
        self.conn
            .execute(
                "INSERT INTO api_tokens VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL)",
                params![
                    issued.token_id,
                    sha256_hex(issued.token.as_bytes()),
                    tenant_id,
                    principal,
                    label,
                    issued_by,
                    now_ms,
                    issued.expires_at_ms
                ],
            )
            .map_err(storage)?;
        Ok(issued)
    }

    /// The binding of a presented token, when it is well formed, known,
    /// unrevoked and unexpired at `now_ms`. Every other case is `None`, so
    /// callers cannot tell the reasons apart.
    pub fn authenticate(&self, token: &str, now_ms: i64) -> Result<Option<TokenBinding>, ApiError> {
        if token.len() != TOKEN_LEN
            || !token.starts_with(TOKEN_PREFIX)
            || !token[TOKEN_PREFIX.len()..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Ok(None);
        }
        self.conn
            .query_row(
                "SELECT token_id,tenant_id,principal,expires_at_ms FROM api_tokens
                 WHERE token_sha256=?1 AND revoked_at_ms IS NULL AND expires_at_ms>?2",
                params![sha256_hex(token.as_bytes()), now_ms],
                |r| {
                    Ok(TokenBinding {
                        token_id: r.get(0)?,
                        tenant_id: r.get(1)?,
                        principal: r.get(2)?,
                        expires_at_ms: r.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(storage)
    }

    /// Tokens of one tenant, optionally of one principal, newest first.
    pub fn tokens(
        &self,
        tenant_id: &str,
        principal: Option<&str>,
    ) -> Result<Vec<TokenInfo>, ApiError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT token_id,principal,label,issued_by,created_at_ms,expires_at_ms,revoked_at_ms
                 FROM api_tokens WHERE tenant_id=?1 AND (?2 IS NULL OR principal=?2)
                 ORDER BY created_at_ms DESC, token_id LIMIT 10000",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map(params![tenant_id, principal], |r| {
                Ok(TokenInfo {
                    token_id: r.get(0)?,
                    principal: r.get(1)?,
                    label: r.get(2)?,
                    issued_by: r.get(3)?,
                    created_at_ms: r.get(4)?,
                    expires_at_ms: r.get(5)?,
                    revoked_at_ms: r.get(6)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// The principal a token of this tenant is bound to, if it exists.
    pub fn token_principal(
        &self,
        tenant_id: &str,
        token_id: &str,
    ) -> Result<Option<String>, ApiError> {
        self.conn
            .query_row(
                "SELECT principal FROM api_tokens WHERE tenant_id=?1 AND token_id=?2",
                params![tenant_id, token_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
    }

    /// Revoke a token of this tenant. Returns false when the tenant has no
    /// such token. Revoking a revoked token keeps the first revocation time.
    pub fn revoke_token(
        &self,
        tenant_id: &str,
        token_id: &str,
        now_ms: i64,
    ) -> Result<bool, ApiError> {
        let found = self
            .conn
            .execute(
                "UPDATE api_tokens SET revoked_at_ms=COALESCE(revoked_at_ms, ?3)
                 WHERE tenant_id=?1 AND token_id=?2",
                params![tenant_id, token_id, now_ms],
            )
            .map_err(storage)?;
        Ok(found > 0)
    }

    /// Revoke a token wherever it is, for the local operator CLI.
    pub fn revoke_token_anywhere(&self, token_id: &str, now_ms: i64) -> Result<bool, ApiError> {
        let found = self
            .conn
            .execute(
                "UPDATE api_tokens SET revoked_at_ms=COALESCE(revoked_at_ms, ?2) WHERE token_id=?1",
                params![token_id, now_ms],
            )
            .map_err(storage)?;
        Ok(found > 0)
    }
}
