//! The hub write side: the runtime open/publish machine (Go serve
//! `hub.go` `openBound` + `publish`). One open builds a runtime from
//! resolved construction inputs — provider client, tools registry, system
//! prompt — adopts the turn controller over them, and publishes it under a
//! minted id so `runtimes()` names it and the runtime routes can address
//! it.
//!
//! The construction inputs arrive typed ([`RuntimeSpec`]) because the
//! config→client edge lives in serve/CLI (ADR-0001 layering); the hub is
//! one layer below it. The broadcaster and the emit chain over it arrive
//! through [`RuntimeKitFactory`] for the same reason: those entities live
//! in serve and assembly.

use std::sync::Arc;

use reasonix_agent::config::AgentConfig;
use reasonix_agent::sink::Sink;
use reasonix_provider::config::{ClientConfig, build_client};
use reasonix_tools::Registry;

use crate::controller::BroadcasterHandle;
use crate::model::{HubRefusal, OpenRuntimeRequest, RuntimeId, RuntimeSummary};
use crate::paths::session_dir_for;
use crate::turns::{TurnController, TurnControllerParts};

use super::{Hub, RuntimeSlot, runtime_summary};

/// The resolved construction inputs one runtime is built from (Go
/// `boot.Options`' provider/prompt/workspace subset, handed over typed).
#[derive(Debug, Clone, Default)]
pub struct RuntimeSpec {
    /// Provider base endpoint, e.g. `https://api.deepseek.com/v1`.
    pub endpoint: String,
    /// Resolved API key; empty sends none (a local endpoint may need no
    /// key).
    pub api_key: String,
    /// The configured model; a request naming one overrides it.
    pub model: String,
    /// The cache-stable system prompt the transcript opens with.
    pub system_prompt: String,
    /// The workspace a request without a root falls back to.
    pub workspace_root: String,
}

/// What one open needs from the host beside the spec: the runtime's event
/// fan-out and the sink its turns emit through (Go: `bc := NewBroadcaster()`
/// plus `paneSink := h.decorateSink(bc)` — one per pane, never shared).
#[derive(Clone)]
pub struct RuntimeKit {
    pub broadcaster: Arc<dyn BroadcasterHandle>,
    pub sink: Arc<dyn Sink>,
}

/// Mints one [`RuntimeKit`] per open. The factory is the host's
/// collaborator (Go `HubOptions` carrying what a pane needs): session is
/// below the broadcaster's and the emit chain's owners, so it cannot build
/// them itself.
pub trait RuntimeKitFactory: Send + Sync {
    fn kit(&self) -> RuntimeKit;
}

/// The open machine installed on a hub (Go `HubOptions`' construction
/// subset).
pub struct HubOptions {
    pub spec: RuntimeSpec,
    pub kit: Arc<dyn RuntimeKitFactory>,
}

impl Hub {
    /// Go `Hub.Open`: builds a runtime for the request, or refuses with
    /// the typed identity of why it could not. A session a published
    /// runtime is already driving is refused with the holder's id — Go
    /// focuses it, and the pane-focus machine is not ported; returning the
    /// holder would claim an open that did not run.
    pub fn open_runtime(&self, req: OpenRuntimeRequest) -> Result<RuntimeSummary, HubRefusal> {
        let machine = match self.machine.as_ref() {
            Some(machine) => machine,
            None => return Err(HubRefusal::UnsupportedInThisBuild),
        };
        // One transcript, one writer: checked on the canonical path before
        // anything is built, so a refused open constructs nothing.
        let session_path = req.session_path.trim();
        if !session_path.is_empty()
            && let Some(id) = self.find_session(session_path)
        {
            return Err(HubRefusal::AlreadyOpen { id: id.0 });
        }
        let root = self.open_root(&req)?;
        let model = first_non_empty(&[req.model.as_str(), machine.spec.model.as_str()])
            .ok_or(HubRefusal::MissingField { field: "model" })?
            .to_owned();
        let endpoint = machine.spec.endpoint.trim();
        if endpoint.is_empty() {
            return Err(HubRefusal::MissingField { field: "endpoint" });
        }

        let mut client_cfg = ClientConfig::new("", endpoint, &model);
        client_cfg.api_key = machine.spec.api_key.clone();
        let client = build_client(&client_cfg).map_err(|err| HubRefusal::ProviderConfig {
            message: err.to_string(),
        })?;

        let kit = machine.kit.kit();
        // Go `SessionDirFor(root)`: the workspace's own transcript
        // directory when the hub has a state root, else the global store.
        // No state root keeps the transcript in memory until a path is
        // pinned — `session_dir_for` would name a relative `sessions`.
        let session_dir = if self.paths.state_root.as_os_str().is_empty() {
            String::new()
        } else {
            session_dir_for(&self.paths.state_root, &root)
                .to_string_lossy()
                .into_owned()
        };
        let ctrl = TurnController::new(TurnControllerParts {
            config: AgentConfig {
                model_ref: model.clone(),
                system_prompt: machine.spec.system_prompt.clone(),
                ..AgentConfig::default()
            },
            client,
            registry: open_registry(&root),
            sink: kit.sink,
            workspace_root: root.clone(),
            session_dir,
            label: model,
            approval: None,
        });
        // Go resumes into the named transcript; resume is a later wave, so
        // the fresh-pin half (Go `SetSessionPath`) pins the file the next
        // turn writes.
        if !session_path.is_empty() {
            ctrl.set_session_path(session_path);
        }

        let id = self.next_id();
        let summary = runtime_summary(&id, ctrl.as_ref());
        self.runtimes
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(RuntimeSlot {
                id,
                controller: ctrl.clone(),
                broadcaster: Some(kit.broadcaster),
                turn: Some(ctrl),
            });
        Ok(summary)
    }

    /// The machine-built turn group of one published runtime (Go
    /// `rt.Server.Controller()` — the concrete controller a runtime route
    /// drives). A host-adopted runtime carries its write side itself and
    /// answers `None`.
    pub fn turn_controller(&self, runtime: RuntimeId) -> Option<Arc<TurnController>> {
        self.runtimes
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|slot| slot.id == runtime)
            .and_then(|slot| slot.turn.clone())
    }

    /// Go `resolveRoot`'s ported chain: the request's root, the installed
    /// spec's workspace, then an open runtime's root. The remaining Go
    /// fallbacks (session-path parent, launch workspaces, auto workspace)
    /// are the workspace-resolution machine and stay unported.
    fn open_root(&self, req: &OpenRuntimeRequest) -> Result<String, HubRefusal> {
        let spec_root = self
            .machine
            .as_ref()
            .map(|machine| machine.spec.workspace_root.as_str())
            .unwrap_or_default();
        let root = first_non_empty(&[req.root.as_str(), spec_root])
            .map(str::to_owned)
            .or_else(|| {
                self.runtimes()
                    .first()
                    .map(|rt| rt.root.clone())
                    .filter(|root| !root.is_empty())
            })
            .ok_or(HubRefusal::NoWorkspace)?;
        // Go `resolveWorkspaceDir`: absolute, existing, a directory.
        let abs = std::path::absolute(root.trim())
            .map_err(|_| HubRefusal::WorkspaceMissing { path: root.clone() })?
            .to_string_lossy()
            .into_owned();
        if !std::path::Path::new(&abs).is_dir() {
            return Err(HubRefusal::WorkspaceMissing { path: abs });
        }
        Ok(abs)
    }
}

/// The first non-empty trimmed member, in order.
fn first_non_empty<'a>(values: &[&'a str]) -> Option<&'a str> {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

/// The registry one runtime drives: the read tools plus the write family,
/// the writers confined to the workspace (Go `ConfineReaders`/
/// `ConfineWriters` at boot). Search lands with grep's slice; the
/// session-store guard and managed-config hooks land with the machines
/// that own their path sets.
fn open_registry(root: &str) -> Arc<Registry> {
    let registry = Registry::new();
    for tool in reasonix_tools::builtin::confine_readers(&[]) {
        registry.add(tool);
    }
    for tool in reasonix_tools::builtin::confine_writers(&[root.to_owned()], None, None) {
        registry.add(tool);
    }
    Arc::new(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::HubPaths;

    #[test]
    fn ids_mint_in_publish_order() {
        let tmp = crate::test_support::tempdir();
        let hub = Hub::new(HubPaths {
            state_root: tmp.path().to_path_buf(),
        });
        assert_eq!(hub.next_id(), RuntimeId("r1".into()));
        assert_eq!(hub.next_id(), RuntimeId("r2".into()));
    }
}
