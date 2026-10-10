//! Safe local embedding facade. Configuration is trusted; request JSON cannot grant authority.
use crate::{artifacts::ArtifactBundle, strict_json};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use weave_contract::Program;
use weave_engine::{AdapterManifest, Engine, HandlerOutputBinding, HostContext};

pub const REQUEST_LIMIT: usize = 16 * 1024 * 1024;
pub const RESPONSE_LIMIT: usize = weave_engine::MATERIALIZED_LIMIT + 4096;
#[derive(Debug, Serialize)]
pub struct HostError {
    pub code: String,
    pub message: String,
}
impl HostError {
    pub(crate) fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for HostError {}
/// Bytes are an operation outcome, NOT a browser durability acknowledgment.
/// The embedding image host must fence every `requires_fence` reply before releasing it.
pub struct HostReply {
    pub bytes: Vec<u8>,
    pub requires_fence: bool,
    pub poisoned: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    format: String,
    operation: Operation,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Execute {
        program: Program,
    },
    Poll {
        adapter: String,
    },
    Prepare {
        adapter: String,
        event: String,
        lease: String,
    },
    Complete {
        adapter: String,
        event: String,
        lease: String,
        preparation: String,
    },
    Capabilities,
    Lifecycle {
        adapter: String,
        state: String,
    },
    Lag {
        adapter: String,
    },
    CausalPolicy {
        adapter: String,
        policy: weave_engine::CausalDispatchPolicy,
    },
    ProjectionInputs {
        adapter: String,
    },
    ProjectionState {
        adapter: String,
    },
    ProjectionMigrationInputs {
        adapter: String,
    },
    CompiledMigrationInputs {
        adapter: String,
    },
    CompiledRebuildInputs {
        adapter: String,
    },
    ActorInputs {
        adapter: String,
    },
    ActorState {
        adapter: String,
    },
    ActorMigrationInputs {
        adapter: String,
    },
    CancelHandler {
        request: Box<weave_engine::DeliveryCancellationRequest>,
    },
    ProjectionRebase {
        request: Box<weave_engine::ProjectionRebaseRequest>,
    },
    ProjectionComplete {
        request: Box<weave_engine::ProjectionCompletionRequest>,
    },
    ProjectionMigrate {
        request: Box<weave_engine::ProjectionMigrationRequest>,
    },
    CompiledMigrate {
        request: Box<weave_engine::CompiledMigrationRequest>,
    },
    CompiledRebuild {
        request: Box<weave_engine::CompiledRebuildRequest>,
    },
    ActorBootstrap {
        request: Box<weave_engine::RecordedActorBootstrap>,
    },
    ActorComplete {
        request: Box<weave_engine::RecordedActorCompletion>,
    },
    ActorMigrate {
        request: Box<weave_engine::RecordedActorMigration>,
    },
    ActorObserve {
        request: Box<weave_engine::RecordedActorObservation>,
    },
    ActorCancel {
        request: Box<weave_engine::DeliveryCancellationRequest>,
    },
    ActorRunInputs {
        adapter: String,
        event: String,
        lease: String,
    },
    ActorDeliveryMode {
        adapter: String,
        event: String,
        lease: String,
    },
    ActorReceipt {
        adapter: String,
        event: String,
    },
    ActorObservation {
        adapter: String,
        event: String,
    },
}
/// Owns one Engine and immutable authority. It never runs source code. Initial installation stays in trusted embedding configuration; owner migrations preserve actual installed scopes.
pub struct HostSession {
    pub(crate) engine: Engine,
    pub(crate) authority: HostContext,
    poisoned: bool,
}
fn valid(s: &str) -> bool {
    !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control)
}
impl HostSession {
    pub fn new(engine: Engine, authority: HostContext) -> Result<Self, HostError> {
        if !valid(&authority.principal)
            || authority.writable_graphs.len() > 128
            || authority.writable_graphs.iter().any(|g| !valid(g))
        {
            return Err(HostError::new(
                "E_HOST_CONFIG",
                "invalid bounded host configuration",
            ));
        }
        Ok(Self {
            engine,
            authority,
            poisoned: false,
        })
    }
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }
    /// The embedding host must invalidate this session after an uncertain durability fence.
    pub fn poison(&mut self) {
        self.poisoned = true;
    }
    /// No success/error from an invoked operation is safely replayable merely from its transport status.
    pub fn call(&mut self, bytes: &[u8]) -> HostReply {
        if self.poisoned {
            return self.rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        if let Err(code) = strict_json::check(bytes, REQUEST_LIMIT) {
            return self.rejected(code, "invalid bounded host request");
        }
        let request: Request = match serde_json::from_slice(bytes) {
            Ok(r) => r,
            Err(_) => return self.rejected("E_HOST_INPUT", "invalid host request"),
        };
        let legacy = matches!(
            &request.operation,
            Operation::Execute { .. }
                | Operation::Poll { .. }
                | Operation::Prepare { .. }
                | Operation::Complete { .. }
        );
        if request.format != "weave-host-request/2"
            && (request.format != "weave-host-request/1" || !legacy)
        {
            return self.rejected("E_HOST_VERSION", "unsupported local host format");
        }
        let command_count = match &request.operation {
            Operation::Execute { program } => program.commands.len(),
            Operation::ProjectionComplete { request } => request.program.commands.len(),
            Operation::ActorComplete { request } => request.program.commands.len(),
            _ => 0,
        };
        if command_count > 16 {
            return self.rejected("E_HOST_BUDGET", "program command limit exceeded");
        }
        self.invoke(|engine, host| match request.operation {
            Operation::Capabilities => encode_value(&serde_json::json!({
                "format":"weave-host-capabilities/1", "request_formats":["weave-host-request/1","weave-host-request/2"],
                "protocol":weave_contract::VERSION, "store_marker":weave_engine::STORAGE_VERSION,
                "operations":["execute","poll","prepare","complete","lifecycle","lag","causal_policy","projection_inputs","projection_state","projection_migration_inputs","compiled_migration_inputs","compiled_rebuild_inputs","actor_inputs","actor_state","actor_migration_inputs","cancel_handler","projection_rebase","projection_complete","projection_migrate","compiled_migrate","compiled_rebuild","actor_bootstrap","actor_complete","actor_migrate","actor_observe","actor_cancel","actor_run_inputs","actor_delivery_mode","actor_receipt","actor_observation","capabilities"],
                "initial_installation":"trusted_embedding_host", "broker_reconciliation":"trusted_embedding_host",
                "request_limit":REQUEST_LIMIT,"response_limit":RESPONSE_LIMIT,"program_commands":16,
                "execution_trust":"native_host", "durability":"embedding_host_fence"
            })),
            Operation::Lifecycle { adapter, state } => { engine.set_adapter_state_for(&adapter, &state, host)?; encode_value(&true) },
            Operation::Lag { adapter } => { encode_value(&engine.adapter_lag_status_for(&adapter, host)?) },
            Operation::CausalPolicy { adapter, policy } => { engine.set_causal_dispatch_policy_for(&adapter, &policy, host)?; encode_value(&true) },
            Operation::ProjectionInputs { adapter } => { encode_value(&engine.projection_rebase_inputs_for(&adapter, host)?) },
            Operation::ProjectionState { adapter } => { encode_value(&engine.projection_state_for(&adapter, host)?) },
            Operation::ProjectionMigrationInputs { adapter } => { encode_value(&engine.projection_migration_inputs_for(&adapter, host)?) },
            Operation::CompiledMigrationInputs { adapter } => { encode_value(&engine.compiled_migration_inputs_for(&adapter, host)?) },
            Operation::CompiledRebuildInputs { adapter } => { encode_value(&engine.compiled_rebuild_inputs_for(&adapter, host)?) },
            Operation::ActorInputs { adapter } => { encode_value(&engine.recorded_actor_inputs_for(&adapter, host)?) },
            Operation::ActorState { adapter } => { encode_value(&engine.recorded_actor_state_for(&adapter, host)?) },
            Operation::ActorMigrationInputs { adapter } => { encode_value(&engine.recorded_actor_migration_inputs_for(&adapter, host)?) },
            Operation::CancelHandler { request } => { encode_value(&engine.cancel_handler_delivery_for(&request, host)?) },
            Operation::ProjectionRebase { request } => { encode_value(&engine.rebase_projection_for(&request, host)?) },
            Operation::ProjectionComplete { request } => { encode_value(&engine.complete_projection_for(&request, host)?) },
            Operation::ProjectionMigrate { request } => { encode_value(&engine.migrate_projection_for(&request, host)?) },
            Operation::CompiledMigrate { request } => { encode_value(&engine.migrate_compiled_handler_for(&request, host)?) },
            Operation::CompiledRebuild { request } => { encode_value(&engine.rebuild_compiled_handler_for(&request, host)?) },
            Operation::ActorBootstrap { request } => { encode_value(&engine.bootstrap_recorded_actor_for(&request, host)?) },
            Operation::ActorComplete { request } => { encode_value(&engine.complete_recorded_actor_for(&request, host)?) },
            Operation::ActorMigrate { request } => { encode_value(&engine.migrate_recorded_actor_for(&request, host)?) },
            Operation::ActorObserve { request } => { encode_value(&engine.observe_recorded_actor_for(&request, host)?) },
            Operation::ActorCancel { request } => { encode_value(&engine.cancel_recorded_actor_delivery_for(&request, host)?) },
            Operation::ActorRunInputs { adapter, event, lease } => { encode_value(&engine.recorded_actor_run_inputs_for(&adapter, &event, &lease, host)?) },
            Operation::ActorDeliveryMode { adapter, event, lease } => { encode_value(&engine.recorded_actor_delivery_mode_for(&adapter, &event, &lease, host)?) },
            Operation::ActorReceipt { adapter, event } => { encode_value(&engine.recorded_actor_receipt_for(&adapter, &event, host)?) },
            Operation::ActorObservation { adapter, event } => { encode_value(&engine.recorded_actor_observation_for(&adapter, &event, host)?) },
            Operation::Execute { program } => encode_value(&engine.execute(&program, host)?),
            Operation::Poll { adapter } => encode_value(&engine.poll_adapter_for(&adapter, host)?),
            Operation::Prepare {
                adapter,
                event,
                lease,
            } => {
                encode_value(&engine.prepare_compiled_handler_for(&adapter, &event, &lease, host)?)
            }
            Operation::Complete {
                adapter,
                event,
                lease,
                preparation,
            } => encode_value(&engine.complete_prepared_handler_for(
                &adapter,
                &event,
                &lease,
                &preparation,
                host,
            )?),
        })
    }
    /// Privileged typed installation. No installation opcode exists in operational requests.
    /// Artifact scalar values and all other templates remain in `bundle` unchanged.
    pub fn install_compiled_handler(
        &mut self,
        bundle: &ArtifactBundle,
        name: &str,
        manifest: &AdapterManifest,
        output: &HandlerOutputBinding,
    ) -> HostReply {
        if self.poisoned {
            return self.rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        let Some(bytes) = bundle.handler_bytes(name) else {
            return self.rejected("E_HOST_ARTIFACT", "handler template unavailable");
        };
        let template = match serde_json::from_slice(bytes) {
            Ok(t) => t,
            Err(_) => return self.rejected("E_HOST_ARTIFACT", "handler template unavailable"),
        };
        self.invoke(|engine, host| {
            engine.install_compiled_handler(manifest, &template, output, host)?;
            encode_value(&true)
        })
    }
    /// Initial actor registration is trusted embedding configuration, never an operational opcode.
    /// Artifact hashing binds bytes; this method does not run a tool or attest its execution.
    pub fn install_recorded_actor(
        &mut self,
        definition: &weave_engine::RecordedActorDefinition,
    ) -> HostReply {
        self.invoke(|engine, host| {
            engine.install_recorded_actor_for(definition, host)?;
            encode_value(&true)
        })
    }
    /// Privileged trusted host cluster adapter installation; no operational install opcode.
    pub fn install_cluster_adapter(&mut self, manifest: &AdapterManifest) -> HostReply {
        if manifest.output_graphs.len() != 1 || !manifest.effect_destinations.is_empty() {
            return self.rejected("E_HOST_CONFIG", "invalid cluster adapter configuration");
        }
        self.invoke(|engine, host| {
            engine.install_adapter(manifest, host)?;
            encode_value(&true)
        })
    }
    /// Recovery harness only: the same principal checks as ordinary prepared operations.
    #[cfg(feature = "recovery-testing")]
    pub fn prepare_test_before_commit(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        before: impl FnOnce(),
    ) -> HostReply {
        self.invoke(|engine, host| {
            encode_value(&engine.prepare_compiled_handler_test_for_before_commit(
                adapter, event, lease, host, before,
            )?)
        })
    }
    #[cfg(feature = "recovery-testing")]
    pub fn complete_test_before_commit(
        &mut self,
        adapter: &str,
        event: &str,
        lease: &str,
        preparation: &str,
        before: impl FnOnce(),
    ) -> HostReply {
        self.invoke(|engine, host| {
            encode_value(&engine.complete_prepared_handler_test_for_before_commit(
                adapter,
                event,
                lease,
                preparation,
                host,
                before,
            )?)
        })
    }
    /// Privileged lifecycle management, still constrained by this session's durable ownership.
    pub fn set_adapter_state(&mut self, adapter: &str, state: &str) -> HostReply {
        if self.poisoned {
            return self.rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        self.invoke(|engine, host| {
            engine.set_adapter_state_for(adapter, state, host)?;
            encode_value(&true)
        })
    }
    #[cfg(feature = "browser-image-experiment")]
    pub fn export_image(&mut self) -> Result<Vec<u8>, HostError> {
        if self.poisoned {
            return Err(HostError::new(
                "E_HOST_POISONED",
                "host unavailable; reopen and inspect",
            ));
        }
        match self.engine.export_single_owner_image() {
            Ok(image) => Ok(image),
            Err(_) => {
                self.poisoned = true;
                Err(HostError::new(
                    "E_HOST_UNCERTAIN",
                    "image export failed; reopen and inspect",
                ))
            }
        }
    }
    pub(crate) fn invoke(
        &mut self,
        f: impl FnOnce(&mut Engine, &HostContext) -> weave_engine::Result<Vec<u8>>,
    ) -> HostReply {
        if self.poisoned {
            return self.rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        match catch_unwind(AssertUnwindSafe(|| f(&mut self.engine, &self.authority))) {
            Ok(Ok(value)) => self.response(true, &value, true),
            Ok(Err(e))
                if e.code != "E_HOST_ENCODE"
                    && e.code != "E_STORAGE"
                    && e.code != "E_HOST_UNCERTAIN" =>
            {
                let error = serde_json::to_vec(&HostError::new(&e.code, "host operation rejected"))
                    .expect("small error");
                self.response(false, &error, true)
            }
            _ => {
                self.poisoned = true;
                let error = serde_json::to_vec(&HostError::new(
                    "E_HOST_UNCERTAIN",
                    "operation outcome unavailable; reopen and inspect",
                ))
                .expect("small error");
                self.response(false, &error, true)
            }
        }
    }
    pub(crate) fn rejected(&self, code: &str, message: &str) -> HostReply {
        let error = serde_json::to_vec(&HostError::new(code, message)).expect("small error");
        self.response(false, &error, false)
    }
    fn response(&self, ok: bool, payload: &[u8], requires_fence: bool) -> HostReply {
        let mut bytes = Vec::with_capacity(payload.len() + 128);
        bytes.extend_from_slice(if ok {
            b"{\"format\":\"weave-host-response/1\",\"ok\":true,\"value\":"
        } else {
            b"{\"format\":\"weave-host-response/1\",\"ok\":false,\"error\":"
        });
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(if requires_fence {
            b",\"requires_fence\":true"
        } else {
            b",\"requires_fence\":false"
        });
        bytes.extend_from_slice(if self.poisoned {
            b",\"poisoned\":true}"
        } else {
            b",\"poisoned\":false}"
        });
        HostReply {
            bytes,
            requires_fence,
            poisoned: self.poisoned,
        }
    }
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > RESPONSE_LIMIT - 512 {
            return Err(std::io::Error::other("host response limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(crate) fn encode_value(value: &impl Serialize) -> weave_engine::Result<Vec<u8>> {
    let mut out = Bounded(Vec::new());
    serde_json::to_writer(&mut out, value).map_err(|_| weave_engine::Error {
        code: "E_HOST_ENCODE".into(),
        message: "host response unavailable".into(),
    })?;
    Ok(out.0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn post_mutation_encoding_failure_poison_is_not_a_safe_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        let mut session = HostSession::new(
            Engine::open(&path).unwrap(),
            HostContext::new("owner", ["data".into()]),
        )
        .unwrap();
        let program: Program =
            serde_json::from_value(serde_json::json!({"version":weave_contract::VERSION,
            "commands":[{"op":"commit","graph_id":"data","data":{}}]}))
            .unwrap();
        let oversized_reply = "x".repeat(RESPONSE_LIMIT);
        let response = session.invoke(|engine, host| {
            engine.execute(&program, host)?;
            encode_value(&oversized_reply)
        });
        assert!(response.poisoned && response.requires_fence);
        let value: serde_json::Value = serde_json::from_slice(&response.bytes).unwrap();
        assert_eq!(value["error"]["code"], "E_HOST_UNCERTAIN");
        assert_eq!(value["requires_fence"], true);
        assert!(session.call(br#"{}"#).poisoned);
        drop(session);
        let engine = Engine::open(&path).unwrap();
        assert_eq!(engine.event_count().unwrap(), 1);
        assert!(engine.head("data", "main").unwrap().is_some());
    }
    #[cfg(feature = "browser-image-experiment")]
    #[test]
    fn image_export_failure_invalidates_session_without_caller_action() {
        let dir = tempfile::tempdir().unwrap();
        // Ordinary WAL engine is deliberately incompatible with image export.
        let mut session = HostSession::new(
            Engine::open(dir.path().join("state.db")).unwrap(),
            HostContext::new("owner", []),
        )
        .unwrap();
        assert_eq!(session.export_image().unwrap_err().code, "E_HOST_UNCERTAIN");
        assert!(session.is_poisoned());
        assert!(session.call(br#"{}"#).poisoned);
    }
}
