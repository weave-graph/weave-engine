//! OIDC ID and access tokens (ADR-0007 section 7).
//!
//! A tenant administrator's operator configures an issuer for a tenant with
//! its expected audience (the client ID) and its JSON Web Key Set. The
//! service never fetches keys: a key rotation is a new `oidc set` from the
//! operator, so no request can make the service open a network connection.
//!
//! A bearer JWT authenticates when its signature verifies under a configured
//! key (RS256, ES256 or EdDSA over Ed25519), its `iss` and `aud` match one
//! issuer configuration, it is inside its `nbf`..`exp` window (with 60 s of
//! leeway), and its `(iss, sub)` is bound to a principal of that tenant. The
//! token's tenant is the configuration's tenant, never anything the request
//! names. Claims other than `iss`, `sub`, `aud`, `exp` and `nbf` are ignored:
//! in particular IdP group claims are never live Weave groups (ADR-0007
//! section 11, alternative 6). They reach the registry only through the
//! audited group import.
use ring::signature::{self, RsaPublicKeyComponents, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest bearer JWT the service parses.
pub const MAX_JWT_BYTES: usize = 8 * 1024;
/// Most keys one issuer's key set may hold.
pub const MAX_JWKS_KEYS: usize = 64;
/// Largest key set document.
pub const MAX_JWKS_BYTES: usize = 256 * 1024;
/// Clock leeway for `exp` and `nbf`, in seconds.
pub const LEEWAY_SECONDS: i64 = 60;

/// Decode unpadded (or padded) base64url.
pub fn base64url(input: &str) -> Option<Vec<u8>> {
    let input = input.trim_end_matches('=');
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    // A dangling 6-bit group cannot come from whole bytes.
    if bits >= 6 || buffer != 0 {
        return None;
    }
    Some(out)
}

/// Encode unpadded base64url (tests and clients).
pub fn base64url_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        let count = chunk.len() + 1;
        for i in 0..count {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// One verification key of an issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    P256 { point: Vec<u8> },
    Ed25519 { public: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Jwk {
    kid: Option<String>,
    key: Key,
}

/// A parsed, validated JSON Web Key Set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySet(Vec<Jwk>);

fn member<'a>(object: &'a serde_json::Map<String, Value>, name: &str) -> Option<&'a str> {
    object.get(name).and_then(Value::as_str)
}

impl KeySet {
    /// Parse a JWKS document (`{"keys": [...]}`). Keys with `use` other than
    /// `sig`, unsupported types and curves are skipped; a set with no usable
    /// key is an error.
    pub fn parse(document: &[u8]) -> Result<Self, String> {
        if document.len() > MAX_JWKS_BYTES {
            return Err("the key set exceeds 256 KiB".into());
        }
        let value: Value =
            serde_json::from_slice(document).map_err(|_| "the key set is not JSON".to_string())?;
        let keys = value
            .get("keys")
            .and_then(Value::as_array)
            .ok_or("the key set has no keys array")?;
        if keys.len() > MAX_JWKS_KEYS {
            return Err("the key set holds more than 64 keys".into());
        }
        let mut out = Vec::new();
        for key in keys {
            let Some(object) = key.as_object() else {
                continue;
            };
            if member(object, "use").is_some_and(|u| u != "sig") {
                continue;
            }
            let bytes = |name: &str| member(object, name).and_then(base64url);
            let parsed = match (member(object, "kty"), member(object, "crv")) {
                (Some("RSA"), _) => match (bytes("n"), bytes("e")) {
                    (Some(n), Some(e)) if n.len() >= 256 => Some(Key::Rsa { n, e }),
                    _ => None,
                },
                (Some("EC"), Some("P-256")) => match (bytes("x"), bytes("y")) {
                    (Some(x), Some(y)) if x.len() == 32 && y.len() == 32 => {
                        let mut point = vec![4u8];
                        point.extend(x);
                        point.extend(y);
                        Some(Key::P256 { point })
                    }
                    _ => None,
                },
                (Some("OKP"), Some("Ed25519")) => match bytes("x") {
                    Some(x) if x.len() == 32 => Some(Key::Ed25519 { public: x }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(key) = parsed {
                out.push(Jwk {
                    kid: member(object, "kid").map(String::from),
                    key,
                });
            }
        }
        if out.is_empty() {
            return Err("the key set has no supported signing key".into());
        }
        Ok(KeySet(out))
    }

    fn verify(&self, alg: &str, kid: Option<&str>, message: &[u8], signature: &[u8]) -> bool {
        self.0
            .iter()
            .filter(|k| kid.is_none() || k.kid.as_deref() == kid)
            .any(|k| match (&k.key, alg) {
                (Key::Rsa { n, e }, "RS256") => RsaPublicKeyComponents { n, e }
                    .verify(&signature::RSA_PKCS1_2048_8192_SHA256, message, signature)
                    .is_ok(),
                (Key::P256 { point }, "ES256") => {
                    UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, point)
                        .verify(message, signature)
                        .is_ok()
                }
                (Key::Ed25519 { public }, "EdDSA") => {
                    UnparsedPublicKey::new(&signature::ED25519, public)
                        .verify(message, signature)
                        .is_ok()
                }
                _ => false,
            })
    }
}

/// The unverified `iss` and `aud` of a JWT, used only to choose which
/// configured key set verifies it.
pub fn peek_issuer(token: &str) -> Option<(String, Vec<String>)> {
    if token.len() > MAX_JWT_BYTES {
        return None;
    }
    let mut parts = token.split('.');
    let (_, payload, _) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let claims: Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    let issuer = claims.get("iss")?.as_str()?.to_string();
    Some((issuer, audiences(&claims)))
}

fn audiences(claims: &Value) -> Vec<String> {
    match claims.get("aud") {
        Some(Value::String(a)) => vec![a.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .take(16)
            .collect(),
        _ => vec![],
    }
}

/// What a verified token asserts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    pub issuer: String,
    pub subject: String,
    /// `exp`, in Unix milliseconds.
    pub expires_at_ms: i64,
}

/// Verify a compact JWS against one issuer configuration at `now_ms`.
pub fn verify(
    token: &str,
    issuer: &str,
    audience: &str,
    keys: &KeySet,
    now_ms: i64,
) -> Option<Verified> {
    if token.len() > MAX_JWT_BYTES {
        return None;
    }
    let mut parts = token.split('.');
    let (header_b64, payload_b64, signature_b64) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let header: Value = serde_json::from_slice(&base64url(header_b64)?).ok()?;
    let alg = header.get("alg")?.as_str()?;
    if header.get("crit").is_some() {
        return None;
    }
    let kid = header.get("kid").and_then(Value::as_str);
    let signature = base64url(signature_b64)?;
    let message = format!("{header_b64}.{payload_b64}");
    if !keys.verify(alg, kid, message.as_bytes(), &signature) {
        return None;
    }
    let claims: Value = serde_json::from_slice(&base64url(payload_b64)?).ok()?;
    if claims.get("iss")?.as_str()? != issuer || !audiences(&claims).iter().any(|a| a == audience) {
        return None;
    }
    let now = now_ms.div_euclid(1000);
    let exp = claims.get("exp")?.as_i64()?;
    if now >= exp.saturating_add(LEEWAY_SECONDS) {
        return None;
    }
    if let Some(nbf) = claims.get("nbf") {
        if now.saturating_add(LEEWAY_SECONDS) < nbf.as_i64()? {
            return None;
        }
    }
    let subject = claims.get("sub")?.as_str()?;
    if subject.is_empty() || subject.len() > 255 {
        return None;
    }
    Some(Verified {
        issuer: issuer.into(),
        subject: subject.into(),
        expires_at_ms: exp.saturating_add(LEEWAY_SECONDS).saturating_mul(1000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_round_trips_and_rejects_junk() {
        for bytes in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            &[255, 0, 254],
        ] {
            assert_eq!(base64url(&base64url_encode(bytes)).unwrap(), bytes);
        }
        assert!(base64url("a+b/").is_none());
        assert!(base64url("A").is_none());
    }
}
