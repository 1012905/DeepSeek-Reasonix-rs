//! TOML configuration: the schema, built-in defaults, layering, and the path
//! roots every other crate resolves through.
//!
//! Parity target: `internal/contract/config` plus `reasonix.example.toml` —
//! every key, default, precedence rule, and unknown-key behavior matches the
//! reference. The frozen inventory is `docs/contracts/config.md`.
//!
//! Scope of this slice: the config surface the host reads to build a runtime —
//! the section schema with its built-in defaults and normalization matrices
//! (CONFIG-003/004), layered user → project loading with the user-globals rule
//! (CONFIG-001/002/008), provider and plugin merge by name with positional
//! decode isolation (CONFIG-007), model resolution and validation (CONFIG-005),
//! atomic config writes with the symlink policy, and the storage roots with
//! their env pins and relocation (CONFIG-022).
//!
//! Vendor rates are never restated here: the shipped DeepSeek entries wait on
//! `reasonix_contract::pricing` being reachable from this crate, and no price
//! table lives in this tree.

#![forbid(unsafe_code)]

pub mod agent;
pub mod delta;
pub mod desktop;
pub mod language;
pub mod load;
pub mod merge;
pub mod model;
pub mod mutator;
pub mod paths;
pub mod plugin;
pub mod provider;
pub mod render_scope;
pub mod resolve;
pub mod roots;
pub mod sandbox;
pub mod secrets;
pub mod serve;
pub mod surgery;
pub mod toml_doc;
pub mod toml_lex;
pub mod tools;
pub mod ui;
pub mod write;

pub use agent::{
    AgentSection, DEFAULT_SYSTEM_PROMPT, default_plan_mode, default_system_prompt,
    normalize_reasoning_language,
};
pub use desktop::{
    DesktopSection, TelemetrySection, default_status_bar_items, normalize_cli_update_channel,
    normalize_desktop_update_channel, normalize_tool_approval_mode,
};
pub use language::normalize_language;
pub use load::ConfigError;
pub use merge::{
    DeclaredLists, EntrySource, MergedProviders, decode_declared_lists, isolate_providers,
    merge_plugins, merge_providers,
};
pub use model::{BillingSection, Config, FRESH_CONFIG_VERSION};
pub use mutator::MutatorError;
pub use paths::{
    auto_workspace_dir, cache_dir, delivery_worktree_dir, expand_vars, locks_dir, memory_lock_dir,
    reasonix_home_dir, reasonix_state_dir, repair_mutation_lock_dir, user_config_path,
    workspace_lease_dir,
};
pub use plugin::{PluginEntry, normalize_plugin_command_line};
pub use provider::{ProviderEntry, ProviderModelOverride, is_valid_credential_key};
pub use render_scope::RenderScope;
pub use resolve::{ModelError, ResolvedModel, unconfigured_entry, unconfigured_ref};
pub use roots::{
    LOCK_DIRS, ROOT_TABLE, RootEnv, RootId, STATE_ROOT_ENTRIES, StorageError, StorageRoot,
    state_root_entries_early_moves_left,
};
pub use sandbox::{PermissionsSection, SandboxSection};
pub use secrets::SecretsSection;
pub use serve::{AuthMode, ServeSection, host_serve_section};
pub use toml_doc::parse_toml_doc;
pub use tools::ToolsSection;
pub use ui::UiSection;
pub use write::{WriteError, config_file_perm, resolve_config_write_path, write_config_file};
