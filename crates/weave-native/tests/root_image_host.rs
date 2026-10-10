#![cfg(feature = "browser-image-experiment")]
use serde_json::{json, Value};
use weave_native::image_host::{ImageHost, IMAGE_LIMIT};

fn config() -> Vec<u8> {
    serde_json::to_vec(&json!({"principal":"owner","writable_graphs":["g"]})).unwrap()
}
fn call(host: &mut ImageHost, operation: Value) -> Value {
    let reply = host.call(
        &serde_json::to_vec(&json!({"format":"weave-host-request/2","operation":operation}))
            .unwrap(),
    );
    assert!(!reply.poisoned);
    assert!(reply.requires_fence);
    serde_json::from_slice(&reply.bytes).unwrap()
}
fn commit(expected: Option<&str>, value: i64) -> Value {
    json!({"kind":"execute","program":{"version":"0.21.0","commands":[{"op":"commit","graph_id":"g","expected_head":expected,"data":{"nodes":[{"id":"n","entity_id":"entity","space_id":"s","properties":{"value":value},"readers":["owner"]}]}}]}})
}
fn query() -> Value {
    json!({"kind":"execute","program":{"version":"0.21.0","commands":[{"op":"query","query":{"graph_id":"g"}}]}})
}

#[test]
fn actual_session_image_restore_preserves_exact_values_and_fences_abandoned_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut host = ImageHost::open(dir.path().join("live.sqlite"), true, &config()).unwrap();
    let first = call(&mut host, commit(None, 9_007_199_254_740_993));
    assert_eq!(first["ok"], true);
    let revision = first["value"][0]["revision"].as_str().unwrap();
    let original = call(&mut host, query());
    assert_eq!(
        original["value"][0]["result"]["graph"]["nodes"][0]["properties"]["value"],
        9_007_199_254_740_993_i64
    );
    let durable = host.export_image().unwrap();
    let second = call(&mut host, commit(Some(revision), i64::MAX));
    assert_eq!(second["ok"], true);
    let latest = host.export_image().unwrap();
    assert_ne!(latest, durable);
    // An unsuccessful outer durability fence invalidates the committed in-memory session.
    host.poison();
    let denied =
        host.call(br#"{"format":"weave-host-request/2","operation":{"kind":"capabilities"}}"#);
    assert!(denied.poisoned);
    assert_eq!(
        serde_json::from_slice::<Value>(&denied.bytes).unwrap()["error"]["code"],
        "E_HOST_POISONED"
    );
    assert_eq!(host.export_image().unwrap_err().code, "E_HOST_POISONED");
    let restored_path = dir.path().join("restored.sqlite");
    std::fs::write(&restored_path, durable).unwrap();
    let mut restored = ImageHost::open(&restored_path, false, &config()).unwrap();
    assert_eq!(call(&mut restored, query()), original);
    let newer_path = dir.path().join("newer.sqlite");
    std::fs::write(&newer_path, latest).unwrap();
    let mut newer = ImageHost::open(&newer_path, false, &config()).unwrap();
    assert_eq!(
        call(&mut newer, query())["value"][0]["result"]["graph"]["nodes"][0]["properties"]["value"],
        i64::MAX
    );
    let c = rusqlite::Connection::open(restored_path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn invalid_authority_duplicate_keys_and_creation_intent_do_not_create_or_replace_images() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.sqlite");
    let bad = br#"{"principal":"owner","princip\u0061l":"other","writable_graphs":[]}"#;
    assert_eq!(
        ImageHost::open(&path, true, bad).err().unwrap().code,
        "E_HOST_INPUT"
    );
    assert!(!path.exists());
    let repeated =
        serde_json::to_vec(&json!({"principal":"owner","writable_graphs":vec!["g";129]})).unwrap();
    assert_eq!(
        ImageHost::open(&path, true, &repeated).err().unwrap().code,
        "E_HOST_CONFIG"
    );
    assert!(!path.exists());
    assert_eq!(
        ImageHost::open(&path, false, &config()).err().unwrap().code,
        "E_STORE_MISSING"
    );
    let mut host = ImageHost::open(&path, true, &config()).unwrap();
    let before = host.export_image().unwrap();
    assert_eq!(
        ImageHost::open(&path, true, &config()).err().unwrap().code,
        "E_ALREADY_CREATED"
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn oversized_or_uninitialized_restores_fail_before_storage_initialization() {
    let dir = tempfile::tempdir().unwrap();
    let large = dir.path().join("large.sqlite");
    let file = std::fs::File::create(&large).unwrap();
    file.set_len((IMAGE_LIMIT + 1) as u64).unwrap();
    drop(file);
    assert_eq!(
        ImageHost::open(&large, false, &config())
            .err()
            .unwrap()
            .code,
        "E_IMAGE_BUDGET"
    );
    assert_eq!(
        std::fs::metadata(large).unwrap().len(),
        (IMAGE_LIMIT + 1) as u64
    );
    let zero = dir.path().join("zero.sqlite");
    let c = rusqlite::Connection::open(&zero).unwrap();
    c.execute_batch("VACUUM").unwrap();
    drop(c);
    let before = std::fs::read(&zero).unwrap();
    assert_eq!(
        ImageHost::open(&zero, false, &config()).err().unwrap().code,
        "E_IMAGE_UNINITIALIZED"
    );
    assert_eq!(std::fs::read(zero).unwrap(), before);
}
