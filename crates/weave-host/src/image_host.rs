//! Trusted experimental image embedding of the ordinary host session.
//! Exported bytes are not durable: the browser must fence the complete generation.
use crate::{
    artifacts::ArtifactBundle,
    host::{HostError, HostReply, HostSession},
    strict_json,
};
use serde::Deserialize;
use std::path::Path;
use weave_engine::{
    AdapterManifest, Engine, HandlerOutputBinding, HostContext, RecordedActorDefinition,
};

pub const CONFIG_LIMIT: usize = 128 * 1024;
pub const HANDLER_CONFIG_LIMIT: usize = 256 * 1024;
pub const ACTOR_CONFIG_LIMIT: usize = 2 * 1024 * 1024;
pub const IMAGE_LIMIT: usize = 8 * 1024 * 1024;

/// One owner, one live operation clock, and the same Engine semantics as native.
/// Initial configuration is trusted app state, never source-supplied authority.
pub struct ImageHost {
    session: HostSession,
}
impl ImageHost {
    pub fn open(path: impl AsRef<Path>, create: bool, config: &[u8]) -> Result<Self, HostError> {
        strict_json::check(config, CONFIG_LIMIT)
            .map_err(|c| HostError::new(c, "invalid image host configuration"))?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            principal: String,
            writable_graphs: Vec<String>,
        }
        let config: Config = serde_json::from_slice(config)
            .map_err(|_| HostError::new("E_HOST_CONFIG", "invalid image host configuration"))?;
        if config.writable_graphs.len() > 128 {
            return Err(HostError::new(
                "E_HOST_CONFIG",
                "too many configured output grants",
            ));
        }
        let authority = HostContext::new(config.principal, config.writable_graphs);
        // Validate before opening or initializing a file. Match HostSession's bounds.
        fn valid(s: &str) -> bool {
            !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control)
        }
        if !valid(&authority.principal)
            || authority.writable_graphs.len() > 128
            || authority.writable_graphs.iter().any(|g| !valid(g))
        {
            return Err(HostError::new(
                "E_HOST_CONFIG",
                "invalid bounded image authority",
            ));
        }
        let path = path.as_ref();
        if path.exists() == create {
            return Err(HostError::new(
                if create {
                    "E_ALREADY_CREATED"
                } else {
                    "E_STORE_MISSING"
                },
                "explicit image creation intent required",
            ));
        }
        if !create {
            let size = std::fs::metadata(path)
                .map_err(|_| HostError::new("E_IMAGE_STORAGE", "image unavailable"))?
                .len();
            if size > IMAGE_LIMIT as u64 {
                return Err(HostError::new(
                    "E_IMAGE_BUDGET",
                    "image exceeds profile capacity",
                ));
            }
        }
        let engine = if create {
            Engine::open_single_owner_image(path)
        } else {
            Engine::open_restored_single_owner_image(path)
        }
        .map_err(|e| HostError::new(&e.code, "image unavailable"))?;
        Ok(Self {
            session: HostSession::new(engine, authority)?,
        })
    }
    pub fn call(&mut self, bytes: &[u8]) -> HostReply {
        self.session.call(bytes)
    }
    pub fn poison(&mut self) {
        self.session.poison();
    }
    pub fn is_poisoned(&self) -> bool {
        self.session.is_poisoned()
    }
    pub fn export_image(&mut self) -> Result<Vec<u8>, HostError> {
        self.session.export_image()
    }
    /// Validate a complete trusted SDK inventory for retention in the embedding's
    /// journal. This installs no adapter and confers no authority. The original
    /// SDK bytes stay with the embedding, which must fence them before replying.
    pub fn retain_sdk(&mut self, sdk: &[u8]) -> HostReply {
        if self.is_poisoned() {
            return self
                .session
                .rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        let bundle = match ArtifactBundle::parse(sdk) {
            Ok(bundle) => bundle,
            Err(_) => {
                return self
                    .session
                    .rejected("E_HOST_ARTIFACT", "invalid complete SDK artifacts")
            }
        };
        self.session
            .invoke(|_, _| crate::host::encode_value(bundle.inventory()))
    }
    /// Trusted explicit SDK selection. Retain the complete original SDK buffer in
    /// the embedding's generation journal; no inventory member is implicitly installed.
    pub fn install_handler(&mut self, sdk: &[u8], config: &[u8]) -> HostReply {
        if self.is_poisoned() {
            return self
                .session
                .rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        if let Err(code) = strict_json::check(config, HANDLER_CONFIG_LIMIT) {
            return self
                .session
                .rejected(code, "invalid bounded handler configuration");
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            name: String,
            manifest: AdapterManifest,
            output: HandlerOutputBinding,
        }
        let config: Config = match serde_json::from_slice(config) {
            Ok(c) => c,
            Err(_) => {
                return self
                    .session
                    .rejected("E_HOST_CONFIG", "invalid handler configuration")
            }
        };
        let bundle = match ArtifactBundle::parse(sdk) {
            Ok(b) => b,
            Err(_) => {
                return self
                    .session
                    .rejected("E_HOST_ARTIFACT", "invalid complete SDK artifacts")
            }
        };
        self.session.install_compiled_handler(
            &bundle,
            &config.name,
            &config.manifest,
            &config.output,
        )
    }
    /// Privileged initial registration; this does not execute or attest an artifact.
    pub fn install_actor(&mut self, config: &[u8]) -> HostReply {
        if self.is_poisoned() {
            return self
                .session
                .rejected("E_HOST_POISONED", "host unavailable; reopen and inspect");
        }
        if let Err(code) = strict_json::check(config, ACTOR_CONFIG_LIMIT) {
            return self
                .session
                .rejected(code, "invalid bounded actor definition");
        }
        let definition: RecordedActorDefinition = match serde_json::from_slice(config) {
            Ok(d) => d,
            Err(_) => {
                return self
                    .session
                    .rejected("E_HOST_CONFIG", "invalid actor definition")
            }
        };
        self.session.install_recorded_actor(&definition)
    }
}
