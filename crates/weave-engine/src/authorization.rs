//! One bounded proof traversal shared by nested authorization and protected callbacks.
//! The mutex only protects bookkeeping; no SQL or recursive authorization runs under it.
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
type Key = (u8, String, String, String, String);
struct Session {
    nesting: usize,
    remaining: usize,
    active: HashSet<Key>,
}
#[derive(Clone, Default)]
pub(crate) struct Authorization(Arc<Mutex<Option<Session>>>);
pub(crate) struct Scope(Authorization);
pub(crate) struct Proof(Authorization, Key);
impl Authorization {
    pub(crate) fn enter(&self) -> Scope {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let session = state.get_or_insert_with(|| Session {
            nesting: 0,
            remaining: 10_000,
            active: HashSet::new(),
        });
        session.nesting += 1;
        Scope(self.clone())
    }
    pub(crate) fn proof(
        &self,
        kind: u8,
        graph: &str,
        revision: &str,
        object: &str,
        principal: &str,
    ) -> Option<Proof> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let session = state.as_mut()?;
        if session.remaining == 0 || session.active.len() >= 32 {
            return None;
        }
        session.remaining -= 1;
        let key = (
            kind,
            graph.into(),
            revision.into(),
            object.into(),
            principal.into(),
        );
        if !session.active.insert(key.clone()) {
            return None;
        }
        Some(Proof(self.clone(), key))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let mut state = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = state.as_mut() {
            session.nesting -= 1;
            if session.nesting == 0 {
                *state = None;
            }
        }
    }
}
impl Drop for Proof {
    fn drop(&mut self) {
        if let Some(session) = self.0 .0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            session.active.remove(&self.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_keys_and_work_are_bounded_and_new_operation_recovers() {
        let state = Authorization::default();
        {
            let _scope = state.enter();
            let mut active = Vec::new();
            for n in 0..32 {
                active.push(state.proof(3, "g", &n.to_string(), "", "p").unwrap());
            }
            assert!(state.proof(3, "g", "extra", "", "p").is_none());
            assert!(state.proof(3, "g", "0", "", "p").is_none());
            drop(active);
            // Previously completed ancestors can be revisited; work does not reset.
            for _ in 32..10_000 {
                drop(state.proof(3, "g", "0", "", "p").unwrap());
            }
            assert!(state.proof(3, "g", "0", "", "p").is_none());
        }
        let _fresh = state.enter();
        assert!(state.proof(3, "g", "0", "", "p").is_some());
    }
    #[test]
    fn unwinding_drops_active_keys_and_operation_state() {
        let state = Authorization::default();
        let failure = std::panic::catch_unwind(|| {
            let _scope = state.enter();
            let _proof = state.proof(3, "g", "r", "", "p").unwrap();
            panic!("observer");
        });
        assert!(failure.is_err());
        let _fresh = state.enter();
        assert!(state.proof(3, "g", "r", "", "p").is_some());
    }
}
