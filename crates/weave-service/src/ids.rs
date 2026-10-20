//! Randomness, hashing and service-minted record IDs.
//!
//! ADR-0007 open question 1: a write that asserts a client-chosen record ID
//! that a hidden live record already holds fails `E_CONFLICT`, revealing that
//! the ID is taken. The service closes that channel for non-administrators:
//! every record ID they assert must have been minted for them by
//! `POST /v1/ids`. A minted ID is `r-` + 32 hex digits of fresh randomness +
//! `-` + a 16-hex-digit tag, an HMAC-SHA-256 of the tenant, the principal
//! and the random part under the tenant's ID key. A principal can therefore
//! assert only IDs minted for it, which no hidden record can already hold,
//! and it cannot reuse an ID it saw on someone else's record.
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

/// Prefix of every minted record ID.
pub const RECORD_ID_PREFIX: &str = "r-";

/// `N` bytes from the operating system's secure random source.
pub fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    SystemRandom::new()
        .fill(&mut out)
        .expect("the operating system random source is available");
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn tag(key: &[u8; 32], tenant: &str, principal: &str, random_part: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    let mut context = hmac::Context::with_key(&key);
    for part in ["weave-service:record-id", tenant, principal, random_part] {
        context.update(&(part.len() as u64).to_be_bytes());
        context.update(part.as_bytes());
    }
    hex(&context.sign().as_ref()[..8])
}

/// A fresh record ID minted for `principal` of `tenant`.
pub fn mint_record_id(key: &[u8; 32], tenant: &str, principal: &str) -> String {
    let random_part = hex(&random::<16>());
    let tag = tag(key, tenant, principal, &random_part);
    format!("{RECORD_ID_PREFIX}{random_part}-{tag}")
}

/// True when `id` was minted for `principal` of `tenant` under `key`.
pub fn minted_for(key: &[u8; 32], tenant: &str, principal: &str, id: &str) -> bool {
    let Some(rest) = id.strip_prefix(RECORD_ID_PREFIX) else {
        return false;
    };
    let Some((random_part, given)) = rest.split_once('-') else {
        return false;
    };
    if random_part.len() != 32 || given.len() != 16 {
        return false;
    }
    let expected = tag(key, tenant, principal, random_part);
    // Compare the tags without an early exit.
    expected
        .bytes()
        .zip(given.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_ids_bind_tenant_principal_and_key() {
        let key = random::<32>();
        let id = mint_record_id(&key, "acme", "alice");
        assert!(minted_for(&key, "acme", "alice", &id));
        assert!(!minted_for(&key, "acme", "bob", &id));
        assert!(!minted_for(&key, "other", "alice", &id));
        assert!(!minted_for(&random::<32>(), "acme", "alice", &id));
        let mut forged = id.clone();
        forged.pop();
        forged.push(if id.ends_with('0') { '1' } else { '0' });
        assert!(!minted_for(&key, "acme", "alice", &forged));
        assert!(!minted_for(&key, "acme", "alice", "node-1"));
        assert_ne!(id, mint_record_id(&key, "acme", "alice"));
    }
}
