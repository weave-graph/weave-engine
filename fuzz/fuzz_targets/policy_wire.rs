#![no_main]

use libfuzzer_sys::fuzz_target;

use std::collections::BTreeSet;
use weave_policy::*;
fuzz_target!(|data: &[u8]| {
    if let Ok(proof) = decode_proof(data) {
        let context = AdmissionContext {
            audience: "fuzz".into(),
            now_ms: 100,
            policy_epoch: "epoch-1".into(),
            roots: vec![],
            revoked_capabilities: BTreeSet::new(),
            revoked_keys: BTreeSet::new(),
            consumed_nonces: BTreeSet::new(),
        };
        let operation = Operation {
            action: Action::Read,
            graph_id: "fuzz".into(),
            branch_id: "main".into(),
        };
        assert!(
            verify_request(&proof, b"{}", &operation, &context).is_err(),
            "wire input minted a trust root"
        );
    }
});
