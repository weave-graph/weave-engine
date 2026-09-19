#![no_main]

use libfuzzer_sys::fuzz_target;

use weave_engine::{Capsule, Engine, HostContext};
fuzz_target!(|data: &[u8]| {
    if data.len() > 131_072 {
        return;
    }
    if let Ok(capsule) = serde_json::from_slice::<Capsule>(data) {
        let mut engine = Engine::memory().expect("in-memory test storage");
        let host = HostContext::new("fuzz", ["fuzz".into()]);
        let first = engine.receive_capsule(&capsule, &host);
        assert!(
            engine.events().unwrap().is_empty(),
            "quarantine import published an accepted event"
        );
        if first.is_ok() {
            assert_eq!(
                engine.receive_capsule(&capsule, &host).unwrap(),
                0,
                "duplicate capsule changed quarantine"
            );
            assert!(engine.events().unwrap().is_empty());
        }
    }
});
