//! Generic trusted transport of raw host requests. IndexedDB fencing is external.
use serde::Serialize;
use serde_json::json;
use std::ffi::{c_char, CString};
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use weave_host::{
    artifacts::SDK_RESPONSE_LIMIT,
    host::{HostReply, REQUEST_LIMIT},
    image_host::{ImageHost, ACTOR_CONFIG_LIMIT, CONFIG_LIMIT, HANDLER_CONFIG_LIMIT},
};

static HOST: OnceLock<Mutex<Option<ImageHost>>> = OnceLock::new();
const DB: &str = "/tmp/runtime.sqlite";
const IMAGE: &str = "/tmp/runtime.image";
fn host() -> &'static Mutex<Option<ImageHost>> {
    HOST.get_or_init(|| Mutex::new(None))
}
fn failure(code: &str, poisoned: bool) -> HostReply {
    HostReply { bytes:serde_json::to_vec(&json!({"format":"weave-host-response/1","ok":false,"error":{"code":code,"message":"browser host unavailable; no automatic replay"},"requires_fence":poisoned,"poisoned":poisoned})).unwrap(), requires_fence:poisoned, poisoned }
}
fn owned(reply: HostReply) -> *mut c_char {
    #[derive(Serialize)]
    struct Frame {
        format: &'static str,
        ok: bool,
        response_json: String,
        requires_fence: bool,
        poisoned: bool,
    }
    let frame = Frame {
        format: "weave-browser-outcome/1",
        ok: reply
            .bytes
            .starts_with(br#"{"format":"weave-host-response/1","ok":true,"value":"#),
        response_json: String::from_utf8(reply.bytes).expect("host emits UTF-8 JSON"),
        requires_fence: reply.requires_fence,
        poisoned: reply.poisoned,
    };
    // Opaque graph/artifact numbers remain inside this string, never JS Number.
    CString::new(serde_json::to_vec(&frame).expect("browser frame JSON"))
        .expect("JSON escapes NUL")
        .into_raw()
}
fn bounded_file(path: &str, limit: usize) -> Result<Vec<u8>, &'static str> {
    let file = std::fs::File::open(path).map_err(|_| "E_HOST_INPUT")?;
    if file.metadata().map_err(|_| "E_HOST_INPUT")?.len() > limit as u64 {
        return Err("E_HOST_BUDGET");
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "E_HOST_INPUT")?;
    if bytes.len() > limit {
        return Err("E_HOST_BUDGET");
    }
    Ok(bytes)
}
#[no_mangle]
pub extern "C" fn weave_browser_open(create: u32) -> *mut c_char {
    let outcome = (|| {
        if create > 1 {
            return Err(failure("E_HOST_INPUT", false));
        }
        let config =
            bounded_file("/tmp/authority.json", CONFIG_LIMIT).map_err(|c| failure(c, false))?;
        let mut guard = host()
            .lock()
            .map_err(|_| failure("E_HOST_UNCERTAIN", true))?;
        if guard.is_some() {
            return Err(failure("E_ALREADY_OPEN", false));
        }
        let mut session =
            ImageHost::open(DB, create == 1, &config).map_err(|e| failure(&e.code, true))?;
        let reply = session
            .call(br#"{"format":"weave-host-request/2","operation":{"kind":"capabilities"}}"#);
        *guard = Some(session);
        Ok(reply)
    })();
    owned(outcome.unwrap_or_else(|e| e))
}
#[no_mangle]
pub extern "C" fn weave_browser_operation(operation: u32) -> *mut c_char {
    let outcome = (|| {
        let mut guard = host()
            .lock()
            .map_err(|_| failure("E_HOST_UNCERTAIN", true))?;
        let session = guard.as_mut().ok_or_else(|| failure("E_NOT_OPEN", false))?;
        let reply = match operation {
            0 => {
                let request = bounded_file("/tmp/request.json", REQUEST_LIMIT)
                    .map_err(|c| failure(c, false))?;
                session.call(&request)
            }
            1 => {
                let sdk = bounded_file("/tmp/sdk.json", SDK_RESPONSE_LIMIT)
                    .map_err(|c| failure(c, false))?;
                let config = bounded_file("/tmp/install.json", HANDLER_CONFIG_LIMIT)
                    .map_err(|c| failure(c, false))?;
                session.install_handler(&sdk, &config)
            }
            2 => {
                let config = bounded_file("/tmp/install.json", ACTOR_CONFIG_LIMIT)
                    .map_err(|c| failure(c, false))?;
                session.install_actor(&config)
            }
            3 => {
                let sdk = bounded_file("/tmp/sdk.json", SDK_RESPONSE_LIMIT)
                    .map_err(|c| failure(c, false))?;
                session.retain_sdk(&sdk)
            }
            _ => return Err(failure("E_HOST_INPUT", false)),
        };
        Ok(reply)
    })();
    owned(outcome.unwrap_or_else(|e| e))
}
#[no_mangle]
pub extern "C" fn weave_browser_export() -> *mut c_char {
    let outcome = (|| {
        let mut guard = host()
            .lock()
            .map_err(|_| failure("E_HOST_UNCERTAIN", true))?;
        let session = guard.as_mut().ok_or_else(|| failure("E_NOT_OPEN", false))?;
        let bytes = session.export_image().map_err(|e| failure(&e.code, true))?;
        if std::fs::write(IMAGE, &bytes).is_err() {
            session.poison();
            return Err(failure("E_IMAGE_STORAGE", true));
        }
        // Export metadata is separate from an operational payload and contains only
        // the bounded byte count. This is not a durable generation acknowledgment.
        let raw=format!("{{\"format\":\"weave-host-response/1\",\"ok\":true,\"value\":{{\"bytes\":{}}},\"requires_fence\":true,\"poisoned\":false}}",bytes.len()).into_bytes();
        Ok(HostReply {
            bytes: raw,
            requires_fence: true,
            poisoned: false,
        })
    })();
    owned(outcome.unwrap_or_else(|e| e))
}
#[no_mangle]
pub extern "C" fn weave_browser_poison() {
    if let Ok(mut guard) = host().lock() {
        if let Some(session) = guard.as_mut() {
            session.poison();
        }
    }
}
/// Releases one outstanding response returned by this module.
/// # Safety
/// The pointer must name a response allocated by this module and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn weave_browser_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(unsafe { CString::from_raw(pointer) });
    }
}
fn main() {
    println!("browser host transport; initialize explicitly under an exclusive durable owner");
}
