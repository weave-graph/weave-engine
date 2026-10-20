//! Per-principal quotas and rate limits (B4, #40).
//!
//! Two limits apply to each principal of a tenant:
//!
//! - **requests per minute**: every audited request (every route but
//!   `health`, every MCP message) counts once, in fixed one-minute windows of
//!   the trusted clock. A request over the limit fails `E_QUOTA` (`429`) and
//!   is not counted.
//! - **fact writes per hour**: every change (`assert`, `correct`,
//!   `retract`) of a committed `write` counts once, in fixed one-hour
//!   windows. A program whose changes would pass the limit fails `E_QUOTA`
//!   before the engine sees it, so nothing of it is written.
//!
//! The limits of a principal are, in order: its own quota row; for an
//! agent, the tenant's [`AGENTS`] row; for an agent, the service default
//! ([`crate::Limits`]); otherwise unlimited. Rows are read from the control
//! store on every request, so an operator's change applies to the next one.
//!
//! Usage lives in memory with the tenant's state. A tenant runs one request
//! at a time, so the check, the write and the count of one request are
//! atomic with respect to every other request of the tenant: concurrent
//! requests at the boundary cannot overshoot. A restart resets the windows.
use crate::error::{codes, ApiError};
use serde::Serialize;
use std::collections::HashMap;

/// The quota subject that sets a tenant's default for its agents. It is in
/// the reserved `weave:` namespace, so no principal can hold it.
pub const AGENTS: &str = "weave:agents";
/// Length of the request window.
pub const MINUTE_MS: i64 = 60_000;
/// Length of the write window.
pub const HOUR_MS: i64 = 3_600_000;
/// Most principals whose usage one tenant tracks; the stalest windows are
/// dropped beyond it, which only ever forgets usage of past windows.
pub const MAX_TRACKED: usize = 100_000;

/// One subject's limits. `None` is unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Quota {
    pub requests_per_minute: Option<u64>,
    pub writes_per_hour: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Usage {
    minute: i64,
    requests: u64,
    hour: i64,
    writes: u64,
}

impl Usage {
    fn roll(&mut self, now_ms: i64) {
        let minute = now_ms.div_euclid(MINUTE_MS);
        if self.minute != minute {
            self.minute = minute;
            self.requests = 0;
        }
        let hour = now_ms.div_euclid(HOUR_MS);
        if self.hour != hour {
            self.hour = hour;
            self.writes = 0;
        }
    }
}

/// What a principal used in its current windows, and its limits.
#[derive(Debug, Clone, Serialize)]
pub struct Standing {
    #[serde(flatten)]
    pub quota: Quota,
    pub requests_this_minute: u64,
    pub writes_this_hour: u64,
    pub minute_resets_at_ms: i64,
    pub hour_resets_at_ms: i64,
}

fn exceeded(message: String, retry_after_ms: i64) -> ApiError {
    let mut error = ApiError::new(codes::QUOTA, message);
    error.retry_after_s = Some(((retry_after_ms.max(0) + 999) / 1000).max(1) as u64);
    error
}

/// The usage of one tenant's principals.
#[derive(Debug, Default)]
pub struct Quotas {
    usage: HashMap<String, Usage>,
}

impl Quotas {
    fn entry(&mut self, principal: &str, now_ms: i64) -> &mut Usage {
        if !self.usage.contains_key(principal) && self.usage.len() >= MAX_TRACKED {
            let hour = now_ms.div_euclid(HOUR_MS);
            self.usage.retain(|_, u| u.hour == hour);
        }
        let usage = self.usage.entry(principal.to_string()).or_default();
        usage.roll(now_ms);
        usage
    }

    /// Count one request, or fail `E_QUOTA` without counting it.
    pub fn admit_request(
        &mut self,
        principal: &str,
        quota: &Quota,
        now_ms: i64,
    ) -> Result<(), ApiError> {
        let usage = self.entry(principal, now_ms);
        if let Some(limit) = quota.requests_per_minute {
            if usage.requests >= limit {
                let reset = (usage.minute + 1) * MINUTE_MS;
                return Err(exceeded(
                    format!("request quota of {limit} per minute reached"),
                    reset - now_ms,
                ));
            }
        }
        usage.requests += 1;
        Ok(())
    }

    /// Fail `E_QUOTA` when `changes` more fact writes would pass the limit.
    pub fn check_writes(
        &mut self,
        principal: &str,
        quota: &Quota,
        changes: u64,
        now_ms: i64,
    ) -> Result<(), ApiError> {
        let Some(limit) = quota.writes_per_hour else {
            return Ok(());
        };
        let usage = self.entry(principal, now_ms);
        if usage.writes.saturating_add(changes) > limit {
            let reset = (usage.hour + 1) * HOUR_MS;
            return Err(exceeded(
                format!(
                    "write quota of {limit} fact changes per hour: {} used, {changes} requested",
                    usage.writes
                ),
                reset - now_ms,
            ));
        }
        Ok(())
    }

    /// Count committed fact writes.
    pub fn record_writes(&mut self, principal: &str, changes: u64, now_ms: i64) {
        let usage = self.entry(principal, now_ms);
        usage.writes = usage.writes.saturating_add(changes);
    }

    /// The principal's current windows.
    pub fn standing(&mut self, principal: &str, quota: Quota, now_ms: i64) -> Standing {
        let usage = *self.entry(principal, now_ms);
        Standing {
            quota,
            requests_this_minute: usage.requests,
            writes_this_hour: usage.writes,
            minute_resets_at_ms: (usage.minute + 1) * MINUTE_MS,
            hour_resets_at_ms: (usage.hour + 1) * HOUR_MS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reset_and_rejections_are_not_counted() {
        let mut q = Quotas::default();
        let quota = Quota {
            requests_per_minute: Some(2),
            writes_per_hour: Some(5),
        };
        let t = 10 * HOUR_MS;
        assert!(q.admit_request("a", &quota, t).is_ok());
        assert!(q.admit_request("a", &quota, t + 1).is_ok());
        let error = q.admit_request("a", &quota, t + 2).unwrap_err();
        assert_eq!(error.code, codes::QUOTA);
        assert_eq!(error.retry_after_s, Some(60));
        assert!(q.admit_request("b", &quota, t + 2).is_ok());
        assert!(q.admit_request("a", &quota, t + MINUTE_MS).is_ok());
        assert!(q.check_writes("a", &quota, 5, t).is_ok());
        q.record_writes("a", 4, t);
        assert!(q.check_writes("a", &quota, 1, t).is_ok());
        assert!(q.check_writes("a", &quota, 2, t).is_err());
        assert!(q.check_writes("a", &quota, 2, t + HOUR_MS).is_ok());
        let unlimited = Quota::default();
        assert!(q.check_writes("a", &unlimited, u64::MAX, t).is_ok());
    }
}
