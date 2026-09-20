//! Trusted embedding-host ABI. JSON plans never grant authority; each handle has fixed host grants.
pub mod artifacts;
pub mod host;
mod host_abi;
mod strict_json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::ffi::{c_char, CString};
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use weave_contract::Program;
use weave_engine::{Engine, HostContext};
const LIMIT: usize = 16 * 1024 * 1024;
// Share the engine precommit cumulative result bound; reserve ABI envelope/array bytes.
const OUTPUT_LIMIT: usize = weave_engine::MATERIALIZED_LIMIT + 4096;
struct Handle {
    engine: Engine,
    host: HostContext,
}
struct Registry {
    next: u64,
    handles: BTreeMap<u64, Handle>,
}
static HANDLES: OnceLock<Mutex<Registry>> = OnceLock::new();
fn registry() -> &'static Mutex<Registry> {
    HANDLES.get_or_init(|| {
        Mutex::new(Registry {
            next: 1,
            handles: BTreeMap::new(),
        })
    })
}
type Failure = (String, String);
fn failure(code: &str, message: &str) -> Failure {
    (code.into(), message.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
    principal: String,
    writable_graphs: Vec<String>,
}
fn id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control)
}
/// Caller must provide a valid readable byte range for the entire invocation.
unsafe fn bytes<'a>(pointer: *const u8, len: usize, limit: usize) -> Result<&'a [u8], Failure> {
    if len > limit {
        return Err(failure(
            "E_NATIVE_BUDGET",
            "input exceeds native boundary byte limit",
        ));
    }
    if pointer.is_null() {
        return Err(failure("E_NATIVE_INPUT", "null input pointer"));
    }
    // SAFETY: ABI callers promise a readable allocation of len bytes for this call.
    Ok(unsafe { std::slice::from_raw_parts(pointer, len) })
}
struct Output(Vec<u8>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(b.len()) > OUTPUT_LIMIT {
            return Err(std::io::Error::other("native output limit"));
        }
        self.0.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn boundary(f: impl FnOnce() -> Result<Value, Failure>) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(f));
    let value = match result {
        Ok(Ok(value)) => json!({"ok":true,"value":value}),
        Ok(Err((code, message))) => json!({"ok":false,"error":{"code":code,"message":message}}),
        Err(_) => {
            json!({"ok":false,"error":{"code":"E_NATIVE_PANIC","message":"native operation failed; restart the host"}})
        }
    };
    let mut output = Output(Vec::new());
    if serde_json::to_writer(&mut output, &value).is_err() {
        output.0=br#"{"ok":false,"error":{"code":"E_NATIVE_BUDGET","message":"native output exceeds byte limit"}}"#.to_vec();
    }
    // JSON escapes every embedded NUL. Ownership transfers to caller until weave_native_free.
    CString::new(output.0)
        .expect("JSON has no embedded NUL")
        .into_raw()
}
/// Opens a database under fixed authority supplied by a trusted embedding host.
/// # Safety
/// Both pointers must reference readable allocations of the specified lengths until return.
#[no_mangle]
pub unsafe extern "C" fn weave_native_open(
    path: *const u8,
    path_len: usize,
    host: *const u8,
    host_len: usize,
) -> *mut c_char {
    boundary(|| {
        let path = std::str::from_utf8(unsafe { bytes(path, path_len, 4096)? })
            .map_err(|_| failure("E_NATIVE_INPUT", "path must be UTF-8"))?;
        if path.is_empty() || path.contains('\0') {
            return Err(failure(
                "E_NATIVE_INPUT",
                "database path must be nonempty without NUL",
            ));
        }
        let host: Host = serde_json::from_slice(unsafe { bytes(host, host_len, 128 * 1024)? })
            .map_err(|_| failure("E_NATIVE_INPUT", "invalid host configuration"))?;
        if !id(&host.principal)
            || host.writable_graphs.len() > 128
            || host.writable_graphs.iter().any(|s| !id(s))
        {
            return Err(failure("E_NATIVE_INPUT", "invalid bounded host authority"));
        }
        let mut registry = registry()
            .lock()
            .map_err(|_| failure("E_NATIVE_PANIC", "native handle registry unavailable"))?;
        if registry.handles.len() >= 128 {
            return Err(failure("E_NATIVE_BUDGET", "native handle capacity reached"));
        }
        let next = registry
            .next
            .checked_add(1)
            .ok_or(failure("E_NATIVE_BUDGET", "handle identifiers exhausted"))?;
        let engine =
            Engine::open(path).map_err(|_| failure("E_NATIVE_STORAGE", "database unavailable"))?;
        let handle = registry.next;
        registry.next = next;
        registry.handles.insert(
            handle,
            Handle {
                engine,
                host: HostContext::new(host.principal, host.writable_graphs),
            },
        );
        Ok(
            json!({"handle":handle,"contract":weave_contract::VERSION,"input_byte_limit":LIMIT,"output_byte_limit":OUTPUT_LIMIT,"command_limit":16}),
        )
    })
}
/// Executes one atomic graph program. A handle's principal/grants cannot be changed by its JSON.
/// # Safety
/// The pointer must reference len readable bytes until return.
#[no_mangle]
pub unsafe extern "C" fn weave_native_execute(
    handle: u64,
    program: *const u8,
    len: usize,
) -> *mut c_char {
    boundary(|| {
        let program: Program = serde_json::from_slice(unsafe { bytes(program, len, LIMIT)? })
            .map_err(|_| failure("E_NATIVE_INPUT", "invalid graph program"))?;
        if program.commands.len() > 16 {
            return Err(failure(
                "E_NATIVE_BUDGET",
                "native programs permit at most sixteen commands",
            ));
        }
        let mut registry = registry()
            .lock()
            .map_err(|_| failure("E_NATIVE_PANIC", "native handle registry unavailable"))?;
        let handle = registry
            .handles
            .get_mut(&handle)
            .ok_or(failure("E_NATIVE_HANDLE", "native handle unavailable"))?;
        match handle.engine.execute(&program, &handle.host) {
            Ok(value) => Ok(json!({"results":value})),
            // Engine diagnostic codes are public; storage internals/messages stay inside host.
            Err(error) => Err((error.code, "graph program rejected".into())),
        }
    })
}
/// Closes an owned handle. Handles are never reused in a process.
#[no_mangle]
pub extern "C" fn weave_native_close(handle: u64) -> *mut c_char {
    boundary(|| {
        let mut registry = registry()
            .lock()
            .map_err(|_| failure("E_NATIVE_PANIC", "native handle registry unavailable"))?;
        if registry.handles.remove(&handle).is_none() {
            return Err(failure("E_NATIVE_HANDLE", "native handle unavailable"));
        }
        Ok(json!({"closed":true}))
    })
}
/// Releases a response exactly once; NULL is accepted.
/// # Safety
/// A non-null pointer must be an outstanding return value from this library, not already freed.
#[no_mangle]
pub unsafe extern "C" fn weave_native_free(response: *mut c_char) {
    if !response.is_null() {
        drop(unsafe { CString::from_raw(response) });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;
    unsafe fn take(p: *mut c_char) -> Value {
        let value = serde_json::from_slice(unsafe { CStr::from_ptr(p) }.to_bytes()).unwrap();
        unsafe { weave_native_free(p) };
        value
    }
    fn open(path: &str, write: bool) -> u64 {
        let host =
            json!({"principal":"alice","writable_graphs":if write{vec!["data"]}else{vec![]}})
                .to_string();
        let r = unsafe {
            take(weave_native_open(
                path.as_ptr(),
                path.len(),
                host.as_ptr(),
                host.len(),
            ))
        };
        assert_eq!(r["ok"], true);
        r["value"]["handle"].as_u64().unwrap()
    }
    fn run(h: u64, p: Value) -> Value {
        let p = p.to_string();
        unsafe { take(weave_native_execute(h, p.as_ptr(), p.len())) }
    }
    #[test]
    fn abi_reopen_persists_and_closed_handles_do_not_reopen_as_other_handles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let path = path.to_str().unwrap();
        let h = open(path, true);
        let p = json!({"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"data","data":{"nodes":[{"id":"a","entity_id":"A","space_id":"s"}]}}]});
        assert_eq!(run(h, p)["ok"], true);
        assert_eq!(unsafe { take(weave_native_close(h)) }["ok"], true);
        let reopened = open(path, false);
        assert_ne!(h, reopened);
        let q = json!({"version":weave_contract::VERSION,"commands":[{"op":"query","query":{"graph_id":"data"}}]});
        assert_eq!(
            run(reopened, q.clone())["value"]["results"][0]["result"]["graph"]["nodes"][0]
                ["entity_id"],
            "A"
        );
        assert_eq!(run(h, q)["error"]["code"], "E_NATIVE_HANDLE");
        unsafe { weave_native_free(weave_native_close(reopened)) };
    }
    #[test]
    fn abi_input_limits_and_fixed_authority_fail_closed() {
        let h = open(":memory:", false);
        let p = json!({"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"data","data":{"nodes":[]}}]});
        assert_eq!(run(h, p)["error"]["code"], "E_FORBIDDEN");
        assert_eq!(
            unsafe { take(weave_native_execute(h, std::ptr::null(), 10)) }["error"]["code"],
            "E_NATIVE_INPUT"
        );
        assert_eq!(
            unsafe { take(weave_native_execute(h, std::ptr::null(), LIMIT + 1)) }["error"]["code"],
            "E_NATIVE_BUDGET"
        );
        unsafe { weave_native_free(weave_native_close(h)) };
    }
    #[test]
    fn abi_large_success_is_returned_and_engine_budget_rejection_rolls_back() {
        let h = open(":memory:", true);
        let seed = json!({"version":weave_contract::VERSION,"commands":[{"op":"commit","graph_id":"data","data":{"nodes":[{"id":"a","entity_id":"A","space_id":"s","properties":{"payload":"x".repeat(3*1024*1024)}}]}}]});
        assert_eq!(run(h, seed)["ok"], true);
        let query = json!({"op":"query","query":{"graph_id":"data"}});
        let marker = |branch: &str| json!({"op":"commit","graph_id":"data","branch_id":branch,"data":{"nodes":[]}});
        let mut commands = vec![marker("success")];
        commands.extend(std::iter::repeat_n(query.clone(), 6));
        let large = run(
            h,
            json!({"version":weave_contract::VERSION,"commands":commands}),
        );
        assert_eq!(large["ok"], true);
        assert_eq!(large["value"]["results"].as_array().unwrap().len(), 7);
        assert!(serde_json::to_vec(&large).unwrap().len() > LIMIT);
        drop(large);
        let mut commands = vec![marker("failure")];
        commands.extend(std::iter::repeat_n(query, 11));
        assert_eq!(
            run(
                h,
                json!({"version":weave_contract::VERSION,"commands":commands})
            )["error"]["code"],
            "E_BUDGET"
        );
        assert_eq!(
            run(
                h,
                json!({"version":weave_contract::VERSION,"commands":[marker("failure")]})
            )["ok"],
            true
        );
        unsafe { weave_native_free(weave_native_close(h)) };
    }
}
