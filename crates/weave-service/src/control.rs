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

/// Control store schema: 1 (B3 part 1: tenants and API tokens), 2 (part 2:
/// OIDC issuers and bindings, ed25519 key bindings, signed-request nonces).
/// Version 1 upgrades in place by adding tables; a version-1 binary refuses
/// version 2.
const SCHEMA_VERSION: i64 = 2;
/// Most unexpired signed-request nonces one tenant keeps.
pub const MAX_NONCES_PER_TENANT: i64 = 1_000_000;
/// Most OIDC issuers per tenant, bindings per tenant, and keys per principal.
pub const MAX_ISSUERS_PER_TENANT: i64 = 16;
pub const MAX_OIDC_BINDINGS_PER_TENANT: i64 = 1_000_000;
pub const MAX_KEYS_PER_PRINCIPAL: i64 = 16;

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
             CREATE INDEX IF NOT EXISTS api_tokens_by_principal ON api_tokens(tenant_id, principal);
             CREATE TABLE IF NOT EXISTS oidc_issuers(
               tenant_id TEXT NOT NULL REFERENCES tenants(tenant_id),
               issuer TEXT NOT NULL,
               audience TEXT NOT NULL,
               jwks TEXT NOT NULL,
               updated_at_ms INTEGER NOT NULL,
               PRIMARY KEY(tenant_id, issuer),
               UNIQUE(issuer, audience));
             CREATE TABLE IF NOT EXISTS oidc_bindings(
               tenant_id TEXT NOT NULL REFERENCES tenants(tenant_id),
               issuer TEXT NOT NULL,
               subject TEXT NOT NULL,
               principal TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               PRIMARY KEY(tenant_id, issuer, subject));
             CREATE TABLE IF NOT EXISTS key_bindings(
               tenant_id TEXT NOT NULL REFERENCES tenants(tenant_id),
               public_key TEXT NOT NULL,
               principal TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               revoked_at_ms INTEGER,
               PRIMARY KEY(tenant_id, public_key));
             CREATE TABLE IF NOT EXISTS signed_nonces(
               tenant_id TEXT NOT NULL,
               replay_id TEXT NOT NULL,
               expires_at_ms INTEGER NOT NULL,
               PRIMARY KEY(tenant_id, replay_id));",
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
            // Version 2 only added tables, created above.
            Some(1) => {
                conn.execute(
                    "UPDATE control_meta SET value=?1 WHERE key='schema_version'",
                    [SCHEMA_VERSION],
                )
                .map_err(storage)?;
            }
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

/// One OIDC issuer of a tenant, as the service verifies tokens against it.
#[derive(Debug, Clone)]
pub struct IssuerConfig {
    pub tenant_id: String,
    pub issuer: String,
    pub audience: String,
    pub keys: crate::oidc::KeySet,
}

/// A key binding as listings report it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KeyInfo {
    pub public_key: String,
    pub principal: String,
    pub created_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at_ms: Option<i64>,
}

/// An OIDC binding as listings report it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OidcBinding {
    pub issuer: String,
    pub subject: String,
    pub principal: String,
    pub created_at_ms: i64,
}

impl ControlStore {
    /// Configure (or replace the keys of) an OIDC issuer for a tenant.
    /// Trusted local administration. An `(issuer, audience)` pair names at
    /// most one tenant, so a token's tenant is never ambiguous.
    pub fn set_oidc_issuer(
        &self,
        tenant_id: &str,
        issuer: &str,
        audience: &str,
        jwks: &str,
        now_ms: i64,
    ) -> Result<(), ApiError> {
        crate::oidc::KeySet::parse(jwks.as_bytes()).map_err(ApiError::input)?;
        if issuer.is_empty() || issuer.len() > 512 || audience.is_empty() || audience.len() > 512 {
            return Err(ApiError::input("issuers and audiences are 1 to 512 bytes"));
        }
        if self.tenant(tenant_id)?.is_none() {
            return Err(ApiError::new("E_UNAVAILABLE", "tenant unavailable"));
        }
        let taken: Option<String> = self
            .conn
            .query_row(
                "SELECT tenant_id FROM oidc_issuers WHERE issuer=?1 AND audience=?2",
                params![issuer, audience],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if taken.is_some_and(|t| t != tenant_id) {
            return Err(ApiError::new(
                "E_CONFLICT",
                "this issuer and audience already belong to another tenant",
            ));
        }
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM oidc_issuers WHERE tenant_id=?1 AND issuer<>?2",
                params![tenant_id, issuer],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if count >= MAX_ISSUERS_PER_TENANT {
            return Err(ApiError::new(
                "E_BUDGET",
                "a tenant has at most 16 OIDC issuers",
            ));
        }
        self.conn
            .execute(
                "INSERT INTO oidc_issuers VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(tenant_id, issuer) DO UPDATE SET audience=excluded.audience,
                 jwks=excluded.jwks, updated_at_ms=excluded.updated_at_ms",
                params![tenant_id, issuer, audience, jwks, now_ms],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Remove an issuer and every binding to it. Its tokens fail on the
    /// next request.
    pub fn remove_oidc_issuer(&self, tenant_id: &str, issuer: &str) -> Result<bool, ApiError> {
        self.conn
            .execute(
                "DELETE FROM oidc_bindings WHERE tenant_id=?1 AND issuer=?2",
                params![tenant_id, issuer],
            )
            .map_err(storage)?;
        let removed = self
            .conn
            .execute(
                "DELETE FROM oidc_issuers WHERE tenant_id=?1 AND issuer=?2",
                params![tenant_id, issuer],
            )
            .map_err(storage)?;
        Ok(removed > 0)
    }

    /// The issuer configurations that could verify a token claiming `issuer`
    /// and one of `audiences`.
    pub fn oidc_candidates(
        &self,
        issuer: &str,
        audiences: &[String],
    ) -> Result<Vec<IssuerConfig>, ApiError> {
        let mut out = Vec::new();
        for audience in audiences.iter().take(16) {
            let row: Option<(String, String)> = self
                .conn
                .query_row(
                    "SELECT tenant_id, jwks FROM oidc_issuers WHERE issuer=?1 AND audience=?2",
                    params![issuer, audience],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(storage)?;
            if let Some((tenant_id, jwks)) = row {
                let keys = crate::oidc::KeySet::parse(jwks.as_bytes())
                    .map_err(|_| ApiError::internal())?;
                out.push(IssuerConfig {
                    tenant_id,
                    issuer: issuer.into(),
                    audience: audience.clone(),
                    keys,
                });
            }
        }
        Ok(out)
    }

    pub fn oidc_issuer_configured(&self, tenant_id: &str, issuer: &str) -> Result<bool, ApiError> {
        self.conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM oidc_issuers WHERE tenant_id=?1 AND issuer=?2)",
                params![tenant_id, issuer],
                |r| r.get(0),
            )
            .map_err(storage)
    }

    /// Bind `(issuer, subject)` to a principal. The caller has checked the
    /// principal. Replaces an earlier binding of the same subject.
    pub fn bind_oidc(
        &self,
        tenant_id: &str,
        issuer: &str,
        subject: &str,
        principal: &str,
        now_ms: i64,
    ) -> Result<(), ApiError> {
        if subject.is_empty() || subject.len() > 255 {
            return Err(ApiError::input("OIDC subjects are 1 to 255 bytes"));
        }
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM oidc_bindings WHERE tenant_id=?1",
                [tenant_id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if count >= MAX_OIDC_BINDINGS_PER_TENANT {
            return Err(ApiError::new(
                "E_BUDGET",
                "a tenant holds at most 1000000 OIDC bindings",
            ));
        }
        self.conn
            .execute(
                "INSERT INTO oidc_bindings VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(tenant_id, issuer, subject) DO UPDATE SET principal=excluded.principal,
                 created_at_ms=excluded.created_at_ms",
                params![tenant_id, issuer, subject, principal, now_ms],
            )
            .map_err(storage)?;
        Ok(())
    }

    pub fn unbind_oidc(
        &self,
        tenant_id: &str,
        issuer: &str,
        subject: &str,
    ) -> Result<bool, ApiError> {
        let removed = self
            .conn
            .execute(
                "DELETE FROM oidc_bindings WHERE tenant_id=?1 AND issuer=?2 AND subject=?3",
                params![tenant_id, issuer, subject],
            )
            .map_err(storage)?;
        Ok(removed > 0)
    }

    pub fn oidc_principal(
        &self,
        tenant_id: &str,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<String>, ApiError> {
        self.conn
            .query_row(
                "SELECT principal FROM oidc_bindings WHERE tenant_id=?1 AND issuer=?2 AND subject=?3",
                params![tenant_id, issuer, subject],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
    }

    /// Bindings of one tenant, optionally of one issuer.
    pub fn oidc_bindings(
        &self,
        tenant_id: &str,
        issuer: Option<&str>,
    ) -> Result<Vec<OidcBinding>, ApiError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT issuer, subject, principal, created_at_ms FROM oidc_bindings
                 WHERE tenant_id=?1 AND (?2 IS NULL OR issuer=?2) ORDER BY issuer, subject",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map(params![tenant_id, issuer], |r| {
                Ok(OidcBinding {
                    issuer: r.get(0)?,
                    subject: r.get(1)?,
                    principal: r.get(2)?,
                    created_at_ms: r.get(3)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// Bind an ed25519 public key (`ed25519:` and 64 hex digits) to a
    /// principal. A key is bound at most once per tenant, ever: a revoked key
    /// is never rebound.
    pub fn bind_key(
        &self,
        tenant_id: &str,
        public_key: &str,
        principal: &str,
        now_ms: i64,
    ) -> Result<(), ApiError> {
        let well_formed = public_key.strip_prefix("ed25519:").is_some_and(|h| {
            h.len() == 64
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        if !well_formed {
            return Err(ApiError::input(
                "keys are ed25519: and 64 lowercase hex digits",
            ));
        }
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM key_bindings WHERE tenant_id=?1 AND principal=?2 AND revoked_at_ms IS NULL",
                params![tenant_id, principal],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if count >= MAX_KEYS_PER_PRINCIPAL {
            return Err(ApiError::new(
                "E_BUDGET",
                "a principal holds at most 16 live keys",
            ));
        }
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO key_bindings VALUES (?1,?2,?3,?4,NULL)",
                params![tenant_id, public_key, principal, now_ms],
            )
            .map_err(storage)?;
        if inserted == 0 {
            return Err(ApiError::new("E_CONFLICT", "the key is already bound"));
        }
        Ok(())
    }

    pub fn revoke_key(
        &self,
        tenant_id: &str,
        public_key: &str,
        now_ms: i64,
    ) -> Result<bool, ApiError> {
        let found = self
            .conn
            .execute(
                "UPDATE key_bindings SET revoked_at_ms=COALESCE(revoked_at_ms, ?3)
                 WHERE tenant_id=?1 AND public_key=?2",
                params![tenant_id, public_key, now_ms],
            )
            .map_err(storage)?;
        Ok(found > 0)
    }

    /// The principal a live (unrevoked) key is bound to.
    pub fn key_principal(
        &self,
        tenant_id: &str,
        public_key: &str,
    ) -> Result<Option<String>, ApiError> {
        self.conn
            .query_row(
                "SELECT principal FROM key_bindings WHERE tenant_id=?1 AND public_key=?2 AND revoked_at_ms IS NULL",
                params![tenant_id, public_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
    }

    /// The principal of a key binding, revoked or not.
    pub fn key_owner(&self, tenant_id: &str, public_key: &str) -> Result<Option<String>, ApiError> {
        self.conn
            .query_row(
                "SELECT principal FROM key_bindings WHERE tenant_id=?1 AND public_key=?2",
                params![tenant_id, public_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
    }

    pub fn keys(&self, tenant_id: &str, principal: Option<&str>) -> Result<Vec<KeyInfo>, ApiError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT public_key, principal, created_at_ms, revoked_at_ms FROM key_bindings
                 WHERE tenant_id=?1 AND (?2 IS NULL OR principal=?2) ORDER BY created_at_ms, public_key LIMIT 10000",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map(params![tenant_id, principal], |r| {
                Ok(KeyInfo {
                    public_key: r.get(0)?,
                    principal: r.get(1)?,
                    created_at_ms: r.get(2)?,
                    revoked_at_ms: r.get(3)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// Every revoked key of a tenant.
    pub fn revoked_keys(
        &self,
        tenant_id: &str,
    ) -> Result<std::collections::BTreeSet<String>, ApiError> {
        let mut statement = self
            .conn
            .prepare("SELECT public_key FROM key_bindings WHERE tenant_id=?1 AND revoked_at_ms IS NOT NULL")
            .map_err(storage)?;
        let rows = statement
            .query_map([tenant_id], |r| r.get(0))
            .map_err(storage)?
            .collect::<Result<_, _>>()
            .map_err(storage)?;
        Ok(rows)
    }

    /// The tenant whose signed-admission audience this is.
    pub fn tenant_by_audience(&self, audience: &str) -> Result<Option<String>, ApiError> {
        self.conn
            .query_row(
                "SELECT tenant_id FROM tenants WHERE audience=?1",
                [audience],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)
    }

    /// Reserve a signed request's replay ID until it expires. False when it
    /// was already used. Expired reservations are pruned first.
    pub fn reserve_nonce(
        &self,
        tenant_id: &str,
        replay_id: &str,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<bool, ApiError> {
        self.conn
            .execute(
                "DELETE FROM signed_nonces WHERE tenant_id=?1 AND expires_at_ms<=?2",
                params![tenant_id, now_ms],
            )
            .map_err(storage)?;
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM signed_nonces WHERE tenant_id=?1",
                [tenant_id],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if count >= MAX_NONCES_PER_TENANT {
            return Err(ApiError::new(
                crate::error::codes::BACKPRESSURE,
                "too many signed requests in flight; retry later",
            ));
        }
        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO signed_nonces VALUES (?1,?2,?3)",
                params![tenant_id, replay_id, expires_at_ms],
            )
            .map_err(storage)?;
        Ok(inserted > 0)
    }
}
