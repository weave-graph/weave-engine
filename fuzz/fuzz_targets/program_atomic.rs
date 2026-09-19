#![no_main]
use libfuzzer_sys::fuzz_target;
use weave_contract::{Command, GraphData, Program, VERSION};
use weave_engine::{Engine, HostContext};

fuzz_target!(|data: &[u8]| {
    if data.len() > 131_072 {
        return;
    }
    if let Ok(program) = serde_json::from_slice::<Program>(data) {
        let mut engine = Engine::memory().expect("in-memory test storage");
        let host = HostContext::new("fuzz", ["fuzz".into()]);
        let before = engine.events().unwrap();
        if engine.execute(&program, &host).is_err() {
            assert_eq!(
                engine.events().unwrap(),
                before,
                "failed program published an event"
            );
            // A failed transaction cannot leave a head that rejects a fresh CAS(None) commit.
            let clean = Program {
                version: VERSION.into(),
                source_revisions: vec![],
                commands: vec![Command::Commit {
                    graph_id: "fuzz".into(),
                    branch_id: "main".into(),
                    expected_head: None,
                    data: GraphData::default(),
                }],
            };
            engine
                .execute(&clean, &host)
                .expect("failed program left persistent fuzz/main head");
        }
    }
});
