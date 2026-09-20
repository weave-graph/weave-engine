//! Thin C/WASM-callable wrappers. These outcomes still require the browser image fence.
use crate::{artifacts::ArtifactBundle, host::HostSession, strict_json};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::{c_char, CString};
use std::sync::{Mutex, OnceLock};
use weave_engine::{Engine, HostContext};

struct Sessions {
    next: u64,
    sessions: BTreeMap<u64, HostSession>,
}
static SESSIONS: OnceLock<Mutex<Sessions>> = OnceLock::new();
fn sessions() -> &'static Mutex<Sessions> {
    SESSIONS.get_or_init(|| {
        Mutex::new(Sessions {
            next: 1,
            sessions: BTreeMap::new(),
        })
    })
}
fn owned(bytes: Vec<u8>) -> *mut c_char {
    CString::new(bytes).expect("JSON escapes NUL").into_raw()
}
fn failure(code: &str) -> Vec<u8> {
    serde_json::to_vec(
        &serde_json::json!({"format":"weave-host-response/1","ok":false,
        "error":{"code":code,"message":"host boundary rejected; no automatic replay"},
        "requires_fence":code == "E_HOST_UNCERTAIN","poisoned":code == "E_HOST_UNCERTAIN"}),
    )
    .expect("small error")
}
fn boundary(f: impl FnOnce() -> Result<Vec<u8>, &'static str>) -> *mut c_char {
    owned(match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(code)) => failure(code),
        Err(_) => serde_json::to_vec(&serde_json::json!({"format":"weave-host-response/1","ok":false,
            "error":{"code":"E_HOST_UNCERTAIN","message":"host trapped; discard instance and inspect"},
            "requires_fence":true,"poisoned":true})).expect("small error"),
    })
}
unsafe fn input<'a>(ptr: *const u8, len: usize, limit: usize) -> Result<&'a [u8], &'static str> {
    if ptr.is_null() {
        return Err("E_HOST_INPUT");
    }
    if len > limit {
        return Err("E_HOST_BUDGET");
    }
    // SAFETY: C ABI contract requires a readable range for the duration of this invocation.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}
fn handle(bytes: &[u8]) -> Result<u64, &'static str> {
    let text = std::str::from_utf8(bytes).map_err(|_| "E_HOST_HANDLE")?;
    let digits = text.strip_prefix("host:").ok_or("E_HOST_HANDLE")?;
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err("E_HOST_HANDLE");
    }
    digits.parse().map_err(|_| "E_HOST_HANDLE")
}
/// Opens ordinary native SQLite with fixed trusted host config; never an operational opcode.
/// # Safety
/// Each pointer must name a readable allocation of its stated length until return.
#[no_mangle]
pub unsafe extern "C" fn weave_host_open(
    path: *const u8,
    path_len: usize,
    config: *const u8,
    config_len: usize,
) -> *mut c_char {
    boundary(|| {
        let path = std::str::from_utf8(unsafe { input(path, path_len, 4096)? })
            .map_err(|_| "E_HOST_INPUT")?;
        if path.is_empty() || path.contains('\0') {
            return Err("E_HOST_INPUT");
        }
        let config = unsafe { input(config, config_len, 128 * 1024)? };
        strict_json::check(config, 128 * 1024)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            principal: String,
            writable_graphs: Vec<String>,
        }
        let config: Config = serde_json::from_slice(config).map_err(|_| "E_HOST_CONFIG")?;
        if config.writable_graphs.len() > 128
            || !crate::id(&config.principal)
            || config.writable_graphs.iter().any(|g| !crate::id(g))
        {
            return Err("E_HOST_CONFIG");
        }
        let mut all = sessions().lock().map_err(|_| "E_HOST_UNCERTAIN")?;
        if all.sessions.len() >= 128 {
            return Err("E_HOST_BUDGET");
        }
        let next = all.next.checked_add(1).ok_or("E_HOST_BUDGET")?;
        let engine = Engine::open(path).map_err(|_| "E_HOST_UNCERTAIN")?;
        let session = HostSession::new(
            engine,
            HostContext::new(config.principal, config.writable_graphs),
        )
        .map_err(|_| "E_HOST_CONFIG")?;
        let id = all.next;
        all.sessions.insert(id, session);
        all.next = next;
        Ok(serde_json::to_vec(&serde_json::json!({"format":"weave-host-response/1","ok":true,
            "value":{"handle":format!("host:{id}"),"request_limit":crate::host::REQUEST_LIMIT,
            "response_limit":crate::host::RESPONSE_LIMIT,"sdk_response_limit":crate::artifacts::SDK_RESPONSE_LIMIT},
            "requires_fence":true,"poisoned":false})).expect("small open response"))
    })
}
/// Calls the safe facade. Handle is an opaque ASCII token, never a JSON number.
/// # Safety
/// Each pointer must name a readable allocation of its stated length until return.
#[no_mangle]
pub unsafe extern "C" fn weave_host_call(
    token: *const u8,
    token_len: usize,
    request: *const u8,
    len: usize,
) -> *mut c_char {
    boundary(|| {
        let id = handle(unsafe { input(token, token_len, 32)? })?;
        let request = unsafe { input(request, len, crate::host::REQUEST_LIMIT)? };
        let mut all = sessions().lock().map_err(|_| "E_HOST_UNCERTAIN")?;
        let session = all.sessions.get_mut(&id).ok_or("E_HOST_HANDLE")?;
        Ok(session.call(request).bytes)
    })
}
/// Closes an owned session. Tokens are never reused within the process.
/// # Safety
/// Token must reference token_len readable bytes until return.
#[no_mangle]
pub unsafe extern "C" fn weave_host_close(token: *const u8, token_len: usize) -> *mut c_char {
    boundary(|| {
        let id = handle(unsafe { input(token, token_len, 32)? })?;
        let mut all = sessions().lock().map_err(|_| "E_HOST_UNCERTAIN")?;
        if all.sessions.remove(&id).is_none() {
            return Err("E_HOST_HANDLE");
        }
        Ok(br#"{"format":"weave-host-response/1","ok":true,"value":{"closed":true},"requires_fence":false,"poisoned":false}"#.to_vec())
    })
}
/// Strict pure SDK artifact selection. Caller retains the complete original SDK response.
/// # Safety
/// Each pointer must name a readable allocation of its stated length until return.
#[no_mangle]
pub unsafe extern "C" fn weave_host_artifact_select(
    bytes: *const u8,
    len: usize,
    selection: *const u8,
    selection_len: usize,
) -> *mut c_char {
    boundary(|| {
        let bundle = ArtifactBundle::parse(unsafe {
            input(bytes, len, crate::artifacts::SDK_RESPONSE_LIMIT)?
        })
        .map_err(|_| "E_HOST_ARTIFACT")?;
        let selection = unsafe { input(selection, selection_len, 1024)? };
        strict_json::check(selection, 1024)?;
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Select {
            Program {},
            View { name: String },
            Handler { name: String },
            Original {},
        }
        let selection: Select = serde_json::from_slice(selection).map_err(|_| "E_HOST_INPUT")?;
        let selected = match &selection {
            Select::Program {} => Some(bundle.program_bytes()),
            Select::View { name } => bundle.view_bytes(name),
            Select::Handler { name } => bundle.handler_bytes(name),
            Select::Original {} => Some(bundle.original_bytes()),
        }
        .ok_or("E_HOST_ARTIFACT")?;
        let mut reply =
            br#"{"format":"weave-host-response/1","ok":true,"value":{"inventory":"#.to_vec();
        serde_json::to_writer(&mut reply, bundle.inventory()).map_err(|_| "E_HOST_BUDGET")?;
        reply.extend_from_slice(b",\"selected\":");
        reply.extend_from_slice(selected);
        reply.extend_from_slice(b"},\"requires_fence\":false,\"poisoned\":false}");
        if reply.len() > crate::host::RESPONSE_LIMIT {
            return Err("E_HOST_BUDGET");
        }
        Ok(reply)
    })
}
/// Privileged explicit installation, separate from operational request JSON.
/// # Safety
/// Each pointer must name a readable allocation of its stated length until return.
#[no_mangle]
pub unsafe extern "C" fn weave_host_install_handler(
    token: *const u8,
    token_len: usize,
    sdk: *const u8,
    sdk_len: usize,
    config: *const u8,
    config_len: usize,
) -> *mut c_char {
    boundary(|| {
        let id = handle(unsafe { input(token, token_len, 32)? })?;
        let sdk = unsafe { input(sdk, sdk_len, crate::artifacts::SDK_RESPONSE_LIMIT)? };
        let bundle = ArtifactBundle::parse(sdk).map_err(|_| "E_HOST_ARTIFACT")?;
        let config = unsafe { input(config, config_len, 256 * 1024)? };
        strict_json::check(config, 256 * 1024)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            name: String,
            manifest: weave_engine::AdapterManifest,
            output: Output,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Output {
            slot: String,
            graph_id: String,
            branch_id: String,
        }
        let config: Config = serde_json::from_slice(config).map_err(|_| "E_HOST_CONFIG")?;
        let mut all = sessions().lock().map_err(|_| "E_HOST_UNCERTAIN")?;
        let session = all.sessions.get_mut(&id).ok_or("E_HOST_HANDLE")?;
        Ok(session
            .install_compiled_handler(
                &bundle,
                &config.name,
                &config.manifest,
                &weave_engine::HandlerOutputBinding {
                    slot: config.output.slot,
                    graph_id: config.output.graph_id,
                    branch_id: config.output.branch_id,
                },
            )
            .bytes)
    })
}
/// Privileged explicit lifecycle control, constrained by session ownership on every call.
/// # Safety
/// Each pointer must name a readable allocation of its stated length until return.
#[no_mangle]
pub unsafe extern "C" fn weave_host_set_adapter_state(
    token: *const u8,
    token_len: usize,
    config: *const u8,
    config_len: usize,
) -> *mut c_char {
    boundary(|| {
        let id = handle(unsafe { input(token, token_len, 32)? })?;
        let config = unsafe { input(config, config_len, 2048)? };
        strict_json::check(config, 2048)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            adapter: String,
            state: String,
        }
        let config: Config = serde_json::from_slice(config).map_err(|_| "E_HOST_CONFIG")?;
        let mut all = sessions().lock().map_err(|_| "E_HOST_UNCERTAIN")?;
        let session = all.sessions.get_mut(&id).ok_or("E_HOST_HANDLE")?;
        Ok(session
            .set_adapter_state(&config.adapter, &config.state)
            .bytes)
    })
}
