//! Fixed trusted fixture bridge, not a public runtime SDK or remote authority endpoint.
use serde_json::{json, Value};
use std::ffi::{c_char, CString};
use std::sync::{Mutex, OnceLock};
use weave_contract::Program;
use weave_engine::{Engine, HostContext};

static ENGINE: OnceLock<Mutex<Option<Engine>>> = OnceLock::new();
const DB: &str = "/tmp/browser.sqlite";
const IMAGE: &str = "/tmp/browser.image";
fn reply(result: Result<Value, String>) -> *mut c_char {
    let value = match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(code) => json!({"ok":false,"code":code}),
    };
    CString::new(serde_json::to_vec(&value).expect("fixture JSON"))
        .expect("JSON escapes NUL")
        .into_raw()
}
fn with_engine(f: impl FnOnce(&mut Engine) -> Result<Value, String>) -> *mut c_char {
    reply((|| {
        let mut guard = ENGINE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "E_POISON".to_string())?;
        f(guard.as_mut().ok_or("E_NOT_OPEN")?)
    })())
}

#[no_mangle]
pub extern "C" fn weave_image_open(create: u32) -> *mut c_char {
    reply((|| {
        let mut guard = ENGINE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "E_POISON".to_string())?;
        if guard.is_some() || ![0, 1].contains(&create) {
            return Err("E_OPEN".into());
        }
        if std::path::Path::new(DB).exists() != (create == 0) {
            return Err("E_CREATE_INTENT".into());
        }
        *guard = Some(
            if create == 1 {
                Engine::open_single_owner_image(DB)
            } else {
                Engine::open_restored_single_owner_image(DB)
            }
            .map_err(|e| e.code)?,
        );
        Ok(json!({"handle":"singleton-fixture","schema":17}))
    })())
}

#[no_mangle]
pub extern "C" fn weave_image_step(operation: u32) -> *mut c_char {
    with_engine(|engine| {
        let host = HostContext::new("owner", ["browser".into()]);
        let value = match operation {
            0 => {
                json!({"version":"0.18.0","commands":[{"op":"query","query":{"graph_id":"browser"}}]})
            }
            1 | 2 | 4 | 5 => {
                let mut properties = json!({"step":operation,"exact":9_007_199_254_740_993_i64,"min":i64::MIN,"max":i64::MAX});
                if operation == 4 || operation == 5 {
                    properties["payload"] = Value::String("x".repeat(if operation == 4 {
                        9 * 1024 * 1024
                    } else {
                        7 * 1024 * 1024
                    }));
                }
                json!({"version":"0.18.0","commands":[{"op":"commit","graph_id":"browser","expected_head":engine.head("browser","main").map_err(|e|e.code)?,"data":{"nodes":[{"id":"n","entity_id":"E","space_id":"s","properties":properties}]}}]})
            }
            3 => json!({"version":"0.18.0","commands":[
                {"op":"commit","graph_id":"browser","expected_head":engine.head("browser","main").map_err(|e|e.code)?,"data":{"nodes":[]}},
                {"op":"commit","graph_id":"not-granted","data":{"nodes":[]}}
            ]}),
            _ => return Err("E_OPERATION".into()),
        };
        let program: Program =
            serde_json::from_value(value).map_err(|_| "E_FIXTURE".to_string())?;
        let result = engine.execute(&program, &host).map_err(|e| e.code)?;
        // A string preserves raw JSON i64 values across the JS test driver.
        Ok(
            json!({"result_json":serde_json::to_string(&result).map_err(|_|"E_FIXTURE".to_string())?,
            "head":engine.head("browser","main").map_err(|e|e.code)?,
            "events":engine.event_count().map_err(|e|e.code)?}),
        )
    })
}

#[no_mangle]
pub extern "C" fn weave_image_export() -> *mut c_char {
    with_engine(|engine| {
        let image = engine.export_single_owner_image().map_err(|e| e.code)?;
        std::fs::write(IMAGE, &image).map_err(|_| "E_IMAGE_STORAGE".to_string())?;
        Ok(json!({"bytes":image.len()}))
    })
}

/// # Safety
/// Pointer must be an outstanding fixture response, freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn weave_image_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}
