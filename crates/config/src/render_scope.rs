//! Render scope matrix and project delta (CONFIG-025, config.md §14; `render.go`).
//!
//! Three scopes gate which sections a save renders:
//! - `Full`     — the annotated reference template, all sections;
//! - `User`     — user/global file; project-only sections stay commented or out;
//! - `Project`  — `./reasonix.toml`; only fields that differ from built-in
//!   defaults, and only sections a project may set.
//!
//! The Rust `Config` models a subset of the reference sections. Sections the
//! schema has not carried yet (`[network]`, `[environment]`, `[cli]`,
//! `[tools.shell]`, `[lsp]`, `[browser]`, `[skills]`, `[memory]`,
//! `[tools.background_jobs]`, `[remote]`, OTLP remote etc.) are gated here by
//! scope decision only; their template bodies are a documented wip until the
//! section lands in `model.rs`.

use std::collections::BTreeMap;

use crate::agent::AgentSection;
use crate::model::Config;
use crate::plugin::PluginEntry;
use crate::provider::ProviderEntry;
use crate::sandbox::{PermissionsSection, SandboxSection};
use crate::toml_lex::{quote_toml_string, render_toml_key_part};
use crate::tools::ToolsSection;
use crate::ui::UiSection;

/// Which config file a render targets (`render.go:36-51`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderScope {
    /// Annotated reference template for a full save.
    #[default]
    Full,
    /// The user/global file.
    User,
    /// The project-local `reasonix.toml`.
    Project,
}

impl RenderScope {
    /// Parses the scope spelling; anything unrecognized falls back to Full,
    /// matching the reference's default.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "user" => Self::User,
            "project" => Self::Project,
            _ => Self::Full,
        }
    }

    /// The canonical scope spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// Shared gate: user/global scopes always render a section; project scopes do
/// so only when it differs from defaults.
fn section_differs(scope: RenderScope, differs: bool) -> bool {
    match scope {
        RenderScope::Project => differs,
        _ => true,
    }
}

/// `[ui]` gate (`render.go:699-704`): project compares the whole section to
/// defaults; non-project renders it.
pub fn should_render_ui(c: &Config, defaults: &Config, scope: RenderScope) -> bool {
    section_differs(scope, c.ui != defaults.ui)
}

/// `[[providers]]` gate (`render.go:744-749`): project compares the entry
/// slice to the shipped baseline; non-project renders it.
pub fn should_render_providers(c: &Config, defaults: &Config, scope: RenderScope) -> bool {
    section_differs(scope, c.providers != defaults.providers)
}

/// `[agent].system_prompt` gate (`render.go:785-790`): Full always; the other
/// scopes only when a non-blank prompt differs from the built-in default.
pub fn should_render_system_prompt(c: &Config, defaults: &Config, scope: RenderScope) -> bool {
    match scope {
        RenderScope::Full => true,
        _ => {
            !c.agent.system_prompt.trim().is_empty()
                && c.agent.system_prompt != defaults.agent.system_prompt
        }
    }
}

/// Gate for sections the Rust schema has not modeled yet: project renders
/// nothing until the section lands; non-project keeps the reference behaviour
/// of always rendering.
pub fn should_render_unmodeled_section(scope: RenderScope) -> bool {
    !matches!(scope, RenderScope::Project)
}

/// Keeps entries in their owning scope (`render.go:372-392`). The Rust
/// `PluginEntry` does not carry the source tag yet, so provenance filtering is
/// a wip: every entry passes through; tag-based exclusion attaches when the
/// source field lands.
pub fn plugins_for_scope(plugins: &[PluginEntry], scope: RenderScope) -> Vec<&PluginEntry> {
    match scope {
        RenderScope::Full => plugins.iter().collect(),
        RenderScope::User | RenderScope::Project => plugins.iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// Render primitives (`render.go:836-951`).
// ---------------------------------------------------------------------------

/// Renders a string array: `["a", "b"]` (`render.go:842-848`).
pub fn render_string_array(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| quote_toml_string(item)).collect();
    format!("[{}]", quoted.join(", "))
}

/// Renders a sorted string map inline: `{ a = "1", b = "2" }`
/// (`render.go:850-863`).
pub fn render_string_map(map: &BTreeMap<String, String>) -> String {
    if map.is_empty() {
        return "{}".to_owned();
    }
    let parts: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{} = {}", render_toml_key_part(k), quote_toml_string(v)))
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

/// Renders a sorted int map inline: `{ a = 1, b = 2 }` (`render.go:865-878`).
pub fn render_int_map(map: &BTreeMap<String, i64>) -> String {
    if map.is_empty() {
        return "{}".to_owned();
    }
    let parts: Vec<String> = map
        .iter()
        .filter(|(_, v)| **v > 0)
        .map(|(k, v)| format!("{} = {v}", render_toml_key_part(k)))
        .collect();
    if parts.is_empty() {
        return "{}".to_owned();
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Renders a rule list; rules are strings, quoted like any other array.
pub fn render_rule_list(rules: &[String]) -> String {
    render_string_array(rules)
}

/// Shortest round-trip float, mirroring Go `strconv.FormatFloat(v, 'g', -1, 64)`
/// (`render.go:814-820`).
pub fn format_float(value: f64) -> String {
    format!("{value}")
}

/// Writes `s` as a `"""` basic string that decodes back to `s`
/// (`render_agent.go:102-121`).
pub fn toml_multiline_basic_string(s: &str) -> String {
    let mut out = String::from("\"\"\"\n");
    let bytes = s.as_bytes();
    for (i, &ch) in bytes.iter().enumerate() {
        match ch {
            b'\\' => out.push_str("\\\\"),
            b'"' if i == bytes.len() - 1 || bytes[i + 1] == b'"' => out.push_str("\\\""),
            b'\n' | b'\t' => out.push(ch as char),
            0x20..=0x7e => out.push(ch as char),
            _ => out.push_str(&format!("\\u{ch:04X}")),
        }
    }
    out.push_str("\"\"\"");
    out
}

// ---------------------------------------------------------------------------
// Project delta (`render.go:398-650`).
// ---------------------------------------------------------------------------

/// Clean TOML holding only fields that differ from built-in defaults. Compare
/// against `Config::default()`, not against another loaded file; the delta
/// merges into a project file through the surgery layer.
pub fn render_toml_project_delta(c: &Config) -> String {
    let defaults = Config::default();
    let mut out = String::new();

    if c.config_version != defaults.config_version {
        out.push_str(&format!("config_version = {}\n", c.config_version));
    }
    if c.default_model != defaults.default_model {
        out.push_str(&format!(
            "default_model = {}\n",
            quote_toml_string(&c.default_model)
        ));
    }
    if c.language != defaults.language {
        out.push_str(&format!("language = {}\n", quote_toml_string(&c.language)));
    }

    render_ui_delta(&mut out, &c.ui, &defaults.ui);
    render_agent_delta(&mut out, &c.agent, &defaults.agent);
    render_providers_delta(&mut out, &c.providers, &defaults.providers);
    render_tools_delta(&mut out, &c.tools, &defaults.tools);
    render_permissions_delta(&mut out, &c.permissions, &defaults.permissions);
    render_sandbox_delta(&mut out, &c.sandbox, &defaults.sandbox);
    render_plugins_delta(&mut out, &c.plugins);

    out
}

fn push_str_if_neq(buf: &mut String, key: &str, value: &str, default: &str) {
    if value != default {
        buf.push_str(&format!("{key} = {value}\n"));
    }
}

fn render_ui_delta(buf: &mut String, c: &UiSection, d: &UiSection) {
    if c == d {
        return;
    }
    let mut section = String::new();
    push_str_if_neq(
        &mut section,
        "theme",
        &quote_toml_string(&c.theme),
        &quote_toml_string(&d.theme),
    );
    push_str_if_neq(
        &mut section,
        "theme_style",
        &quote_toml_string(&c.theme_style),
        &quote_toml_string(&d.theme_style),
    );
    push_str_if_neq(
        &mut section,
        "shortcut_layout",
        &quote_toml_string(&c.shortcut_layout),
        &quote_toml_string(&d.shortcut_layout),
    );
    push_str_if_neq(
        &mut section,
        "close_behavior",
        &quote_toml_string(&c.close_behavior),
        &quote_toml_string(&d.close_behavior),
    );
    push_str_if_neq(
        &mut section,
        "cursor_shape",
        &quote_toml_string(&c.cursor_shape),
        &quote_toml_string(&d.cursor_shape),
    );
    push_str_if_neq(
        &mut section,
        "command_mode",
        &quote_toml_string(&c.command_mode),
        &quote_toml_string(&d.command_mode),
    );
    if c.show_reasoning != d.show_reasoning {
        section.push_str(&format!("show_reasoning = {}\n", c.show_reasoning));
    }
    if c.show_turn_usage != d.show_turn_usage {
        section.push_str(&format!("show_turn_usage = {}\n", c.show_turn_usage));
    }
    if c.show_changed_files != d.show_changed_files {
        section.push_str(&format!("show_changed_files = {}\n", c.show_changed_files));
    }
    if c.feedback_level_notices != d.feedback_level_notices {
        section.push_str(&format!(
            "feedback_level_notices = {}\n",
            c.feedback_level_notices
        ));
    }
    if c.fold_turn_work != d.fold_turn_work {
        section.push_str(&format!("fold_turn_work = {}\n", c.fold_turn_work));
    }
    if c.show_receipt != d.show_receipt {
        section.push_str(&format!("show_receipt = {}\n", c.show_receipt));
    }
    if !section.is_empty() {
        buf.push_str("[ui]\n");
        buf.push_str(&section);
        buf.push('\n');
    }
}

fn render_agent_delta(buf: &mut String, c: &AgentSection, d: &AgentSection) {
    let mut section = String::new();
    if !c.system_prompt.trim().is_empty() && c.system_prompt != d.system_prompt {
        section.push_str(&format!(
            "system_prompt = {}\n",
            toml_multiline_basic_string(&c.system_prompt)
        ));
    }
    if c.system_prompt_file != d.system_prompt_file && !c.system_prompt_file.is_empty() {
        section.push_str(&format!(
            "system_prompt_file = {}\n",
            quote_toml_string(&c.system_prompt_file)
        ));
    }
    if c.temperature != d.temperature {
        section.push_str(&format!("temperature = {}\n", format_float(c.temperature)));
    }
    if c.recovery_model != d.recovery_model && !c.recovery_model.is_empty() {
        section.push_str(&format!(
            "recovery_model = {}\n",
            quote_toml_string(&c.recovery_model)
        ));
    }
    if c.compact_ratio != d.compact_ratio {
        section.push_str(&format!(
            "compact_ratio = {}\n",
            format_float(c.compact_ratio)
        ));
    }
    if !c.plan_mode_read_only_commands.is_empty()
        && c.plan_mode_read_only_commands != d.plan_mode_read_only_commands
    {
        section.push_str(&format!(
            "plan_mode_read_only_commands = {}\n",
            render_string_array(&c.plan_mode_read_only_commands)
        ));
    }
    if c.web_search_model != d.web_search_model && !c.web_search_model.is_empty() {
        section.push_str(&format!(
            "web_search_model = {}\n",
            quote_toml_string(&c.web_search_model)
        ));
    }
    if c.title_model != d.title_model && !c.title_model.is_empty() {
        section.push_str(&format!(
            "title_model = {}\n",
            quote_toml_string(&c.title_model)
        ));
    }
    if c.planner_model != d.planner_model && !c.planner_model.is_empty() {
        section.push_str(&format!(
            "planner_model = {}\n",
            quote_toml_string(&c.planner_model)
        ));
    }
    if c.subagent_model != d.subagent_model && !c.subagent_model.is_empty() {
        section.push_str(&format!(
            "subagent_model = {}\n",
            quote_toml_string(&c.subagent_model)
        ));
    }
    if !c.subagent_models.is_empty() && c.subagent_models != d.subagent_models {
        section.push_str(&format!(
            "subagent_models = {}\n",
            render_string_map(&c.subagent_models)
        ));
    }
    if c.subagent_effort != d.subagent_effort && !c.subagent_effort.is_empty() {
        section.push_str(&format!(
            "subagent_effort = {}\n",
            quote_toml_string(&c.subagent_effort)
        ));
    }
    if !c.subagent_efforts.is_empty() && c.subagent_efforts != d.subagent_efforts {
        section.push_str(&format!(
            "subagent_efforts = {}\n",
            render_string_map(&c.subagent_efforts)
        ));
    }
    if c.max_subagent_depth != d.max_subagent_depth {
        section.push_str(&format!("max_subagent_depth = {}\n", c.max_subagent_depth));
    }
    if c.output_style != d.output_style && !c.output_style.is_empty() {
        section.push_str(&format!(
            "output_style = {}\n",
            quote_toml_string(&c.output_style)
        ));
    }
    if !section.is_empty() {
        buf.push_str("[agent]\n");
        buf.push_str(&section);
        buf.push('\n');
    }
}

fn render_providers_delta(buf: &mut String, c: &[ProviderEntry], d: &[ProviderEntry]) {
    if c == d {
        return;
    }
    for provider in c {
        buf.push_str("[[providers]]\n");
        if !provider.name.is_empty() {
            buf.push_str(&format!("name = {}\n", quote_toml_string(&provider.name)));
        }
        if !provider.kind.is_empty() {
            buf.push_str(&format!("kind = {}\n", quote_toml_string(&provider.kind)));
        }
        if !provider.base_url.is_empty() {
            buf.push_str(&format!(
                "base_url = {}\n",
                quote_toml_string(&provider.base_url)
            ));
        }
        if !provider.model.is_empty() {
            buf.push_str(&format!("model = {}\n", quote_toml_string(&provider.model)));
        }
        if !provider.models.is_empty() {
            buf.push_str(&format!(
                "models = {}\n",
                render_string_array(&provider.models)
            ));
        }
        if !provider.api_key_env.is_empty() {
            buf.push_str(&format!(
                "api_key_env = {}\n",
                quote_toml_string(&provider.api_key_env)
            ));
        }
        if provider.idle_timeout_seconds > 0 {
            buf.push_str(&format!(
                "idle_timeout_seconds = {}\n",
                provider.idle_timeout_seconds
            ));
        }
        buf.push('\n');
    }
}

fn render_tools_delta(buf: &mut String, c: &ToolsSection, d: &ToolsSection) {
    let mut section = String::new();
    if !c.enabled.is_empty() && c.enabled != d.enabled {
        section.push_str(&format!("enabled = {}\n", render_string_array(&c.enabled)));
    }
    if !c.disabled.is_empty() && c.disabled != d.disabled {
        section.push_str(&format!(
            "disabled = {}\n",
            render_string_array(&c.disabled)
        ));
    }
    if let Some(seconds) = c.bash_timeout_seconds
        && seconds != 0
    {
        section.push_str(&format!("bash_timeout_seconds = {seconds}\n"));
    }
    if let Some(seconds) = c.mcp_startup_timeout_seconds
        && seconds > 0
    {
        section.push_str(&format!("mcp_startup_timeout_seconds = {seconds}\n"));
    }
    if let Some(seconds) = c.mcp_call_timeout_seconds
        && seconds > 0
    {
        section.push_str(&format!("mcp_call_timeout_seconds = {seconds}\n"));
    }
    if let Some(protect) = c.protect_changed_files {
        section.push_str(&format!("protect_changed_files = {protect}\n"));
    }
    if let Some(browser) = c.browser_tools {
        section.push_str(&format!("browser_tools = {browser}\n"));
    }
    if c.present_files != d.present_files {
        section.push_str(&format!("present_files = {}\n", c.present_files));
    }
    if !section.is_empty() {
        buf.push_str("[tools]\n");
        buf.push_str(&section);
        buf.push('\n');
    }
    if !c.mcp_load.is_empty() {
        buf.push_str("[tools.mcp_load]\n");
        for (name, mode) in &c.mcp_load {
            buf.push_str(&format!(
                "{} = {}\n",
                render_toml_key_part(name),
                quote_toml_string(mode)
            ));
        }
        buf.push('\n');
    }
}

fn render_permissions_delta(buf: &mut String, c: &PermissionsSection, d: &PermissionsSection) {
    if c == d {
        return;
    }
    let mut section = String::new();
    if c.mode != d.mode {
        section.push_str(&format!("mode = {}\n", quote_toml_string(c.mode())));
    }
    if !c.allow.is_empty() && c.allow != d.allow {
        section.push_str(&format!("allow = {}\n", render_rule_list(&c.allow)));
    }
    if !c.ask.is_empty() && c.ask != d.ask {
        section.push_str(&format!("ask = {}\n", render_rule_list(&c.ask)));
    }
    if !c.deny.is_empty() && c.deny != d.deny {
        section.push_str(&format!("deny = {}\n", render_rule_list(&c.deny)));
    }
    if !section.is_empty() {
        buf.push_str("[permissions]\n");
        buf.push_str(&section);
        buf.push('\n');
    }
}

fn render_sandbox_delta(buf: &mut String, c: &SandboxSection, d: &SandboxSection) {
    // User-global fields (windows_integrity, host_authorities, allow_devices)
    // never enter a project delta.
    if c.workspace_root == d.workspace_root
        && c.allow_write == d.allow_write
        && c.forbid_read == d.forbid_read
        && c.bash == d.bash
        && c.network == d.network
        && c.allowed_domains == d.allowed_domains
        && c.denied_domains == d.denied_domains
        && c.egress_unlisted == d.egress_unlisted
    {
        return;
    }
    let mut section = String::new();
    if c.workspace_root != d.workspace_root {
        section.push_str(&format!(
            "workspace_root = {}\n",
            quote_toml_string(&c.workspace_root)
        ));
    }
    if c.allow_write != d.allow_write && !c.allow_write.is_empty() {
        section.push_str(&format!(
            "allow_write = {}\n",
            render_string_array(&c.allow_write)
        ));
    }
    if c.forbid_read != d.forbid_read && !c.forbid_read.is_empty() {
        section.push_str(&format!(
            "forbid_read = {}\n",
            render_string_array(&c.forbid_read)
        ));
    }
    if c.bash != d.bash && !c.bash.is_empty() {
        section.push_str(&format!("bash = {}\n", quote_toml_string(&c.bash)));
    }
    if c.network != d.network {
        section.push_str(&format!("network = {}\n", c.network));
    }
    if c.allowed_domains != d.allowed_domains && !c.allowed_domains.is_empty() {
        section.push_str(&format!(
            "allowed_domains = {}\n",
            render_string_array(&c.allowed_domains)
        ));
    }
    if c.denied_domains != d.denied_domains && !c.denied_domains.is_empty() {
        section.push_str(&format!(
            "denied_domains = {}\n",
            render_string_array(&c.denied_domains)
        ));
    }
    if c.egress_unlisted != d.egress_unlisted && !c.egress_unlisted.is_empty() {
        section.push_str(&format!(
            "egress_unlisted = {}\n",
            quote_toml_string(&c.egress_unlisted)
        ));
    }
    if !section.is_empty() {
        buf.push_str("[sandbox]\n");
        buf.push_str(&section);
        buf.push('\n');
    }
}

fn render_plugins_delta(buf: &mut String, c: &[PluginEntry]) {
    for plugin in plugins_for_scope(c, RenderScope::Project) {
        buf.push_str("[[plugins]]\n");
        if !plugin.name.is_empty() {
            buf.push_str(&format!("name = {}\n", quote_toml_string(&plugin.name)));
        }
        if !plugin.kind.is_empty() {
            buf.push_str(&format!("kind = {}\n", quote_toml_string(&plugin.kind)));
        }
        if !plugin.command.is_empty() {
            buf.push_str(&format!(
                "command = {}\n",
                quote_toml_string(&plugin.command)
            ));
        }
        if !plugin.args.is_empty() {
            buf.push_str(&format!("args = {}\n", render_string_array(&plugin.args)));
        }
        if !plugin.url.is_empty() {
            buf.push_str(&format!("url = {}\n", quote_toml_string(&plugin.url)));
        }
        if !plugin.disabled_tools.is_empty() {
            buf.push_str(&format!(
                "disabled_tools = {}\n",
                render_string_array(&plugin.disabled_tools)
            ));
        }
        if !plugin.concurrency.is_empty() {
            buf.push_str(&format!(
                "concurrency = {}\n",
                quote_toml_string(&plugin.concurrency)
            ));
        }
        if let Some(auto_start) = plugin.auto_start {
            buf.push_str(&format!("auto_start = {auto_start}\n"));
        }
        if !plugin.load.is_empty() {
            buf.push_str(&format!("load = {}\n", quote_toml_string(&plugin.load)));
        }
        buf.push('\n');
    }
}

/// Renders the full annotated template for a scope. `user_config_display_path`
/// stands in for `processRoots().userConfigDisplayPath()`; wiring it to the
/// roots layer is a wip before the second header line can be finalized
/// (`render.go:2`).
pub fn render_toml_for_scope(
    c: &Config,
    scope: RenderScope,
    user_config_display_path: &str,
) -> String {
    let defaults = Config::default();
    let mut out = String::new();
    out.push_str("# Reasonix configuration\n");
    out.push_str(&format!("# user config: {user_config_display_path}\n"));

    out.push_str(&format!("config_version = {}\n", c.config_version));
    out.push_str(&format!(
        "default_model = {}\n",
        quote_toml_string(&c.default_model)
    ));
    if !c.language.is_empty() {
        out.push_str(&format!("language = {}\n", quote_toml_string(&c.language)));
    }
    out.push('\n');

    if should_render_ui(c, &defaults, scope) {
        out.push_str("[ui]\n");
        out.push_str(&format!("theme = {}\n", quote_toml_string(c.ui.theme())));
        if !c.ui.theme_style().is_empty() {
            out.push_str(&format!(
                "theme_style = {}\n",
                quote_toml_string(c.ui.theme_style())
            ));
        }
        out.push('\n');
    }

    if should_render_providers(c, &defaults, scope) {
        for provider in &c.providers {
            out.push_str("[[providers]]\n");
            if !provider.name.is_empty() {
                out.push_str(&format!("name = {}\n", quote_toml_string(&provider.name)));
            }
            out.push('\n');
        }
    }

    let delta = render_toml_project_delta(c);
    if !delta.is_empty() && scope == RenderScope::Project {
        out.push_str(&delta);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_parse_falls_back_to_full_for_unknown_spellings() {
        assert_eq!(RenderScope::parse("full"), RenderScope::Full);
        assert_eq!(RenderScope::parse("USER"), RenderScope::User);
        assert_eq!(RenderScope::parse("project"), RenderScope::Project);
        assert_eq!(RenderScope::parse("remote"), RenderScope::Full);
    }

    #[test]
    fn ui_gate_renders_everywhere_except_default_project_state() {
        let defaults = Config::default();
        let mut changed = Config::default();
        changed.ui.theme = "dark".to_owned();
        assert!(should_render_ui(&defaults, &defaults, RenderScope::Full));
        assert!(should_render_ui(&defaults, &defaults, RenderScope::User));
        assert!(!should_render_ui(
            &defaults,
            &defaults,
            RenderScope::Project
        ));
        assert!(should_render_ui(&changed, &defaults, RenderScope::Project));
    }

    #[test]
    fn providers_gate_compares_the_entry_slice() {
        let defaults = Config::default();
        assert!(!should_render_providers(
            &defaults,
            &defaults,
            RenderScope::Project
        ));
        assert!(should_render_providers(
            &defaults,
            &defaults,
            RenderScope::User
        ));
    }

    #[test]
    fn system_prompt_gate_requires_a_non_default_project_prompt() {
        let defaults = Config::default();
        let empty = Config::default();
        assert!(!should_render_system_prompt(
            &empty,
            &defaults,
            RenderScope::Project
        ));
        let mut custom = Config::default();
        custom.agent.system_prompt = "be terse".to_owned();
        assert!(should_render_system_prompt(
            &custom,
            &defaults,
            RenderScope::Project
        ));
        assert!(should_render_system_prompt(
            &empty,
            &defaults,
            RenderScope::Full
        ));
    }

    #[test]
    fn default_config_produces_an_empty_project_delta() {
        let delta = render_toml_project_delta(&Config::default());
        assert!(delta.trim().is_empty(), "delta = {delta:?}");
    }

    #[test]
    fn project_delta_emits_only_changed_model_and_agent_fields() {
        let c = Config {
            default_model: "gateway/model-x".to_owned(),
            agent: AgentSection {
                compact_ratio: 0.85,
                ..Default::default()
            },
            ..Config::default()
        };
        let delta = render_toml_project_delta(&c);
        assert!(delta.contains("default_model = \"gateway/model-x\""));
        assert!(delta.contains("compact_ratio = 0.85"));
        assert!(!delta.contains("theme"));
        assert!(!delta.contains("system_prompt"));
    }

    #[test]
    fn unchanged_system_prompt_is_absent_from_the_delta() {
        let c = Config::default();
        let delta = render_toml_project_delta(&c);
        assert!(!delta.contains("system_prompt"));
    }

    #[test]
    fn ui_delta_writes_only_differing_presentation_fields() {
        let mut c = Config::default();
        c.ui.show_receipt = true;
        let delta = render_toml_project_delta(&c);
        assert!(delta.contains("[ui]"));
        assert!(delta.contains("show_receipt = true"));
        assert!(!delta.contains("theme ="));
    }

    #[test]
    fn sandbox_delta_skips_user_global_authority_fields() {
        let mut c = Config::default();
        c.sandbox.network = false;
        let delta = render_toml_project_delta(&c);
        assert!(delta.contains("network = false"));
        assert!(!delta.contains("host_authorities"));
        assert!(!delta.contains("windows_integrity"));
    }

    #[test]
    fn primitives_render_sorted_arrays_maps_and_floats() {
        let map: BTreeMap<String, String> = BTreeMap::from([
            ("b".to_owned(), "2".to_owned()),
            ("a".to_owned(), "1".to_owned()),
        ]);
        assert_eq!(render_string_map(&map), "{ a = \"1\", b = \"2\" }");
        let ints: BTreeMap<String, i64> =
            BTreeMap::from([("x".to_owned(), 4), ("y".to_owned(), 0)]);
        assert_eq!(render_int_map(&ints), "{ x = 4 }");
        assert_eq!(render_string_array(&["a".to_owned()]), "[\"a\"]");
        assert_eq!(format_float(0.90), "0.9");
    }

    #[test]
    fn multiline_string_round_trips_a_newline_and_escapes_quotes() {
        let rendered = toml_multiline_basic_string("line one\nline two\"\"x");
        assert!(rendered.starts_with("\"\"\"\n"));
        assert!(rendered.ends_with("\"\"\""));
        assert!(rendered.contains("\\\"\""));
    }

    #[test]
    fn full_template_keeps_the_user_path_header_line() {
        let out = render_toml_for_scope(
            &Config::default(),
            RenderScope::Full,
            "/home/u/reasonix.toml",
        );
        assert!(out.contains("# user config: /home/u/reasonix.toml"));
        assert!(out.contains("config_version = 12"));
    }

    #[test]
    fn plugins_for_scope_preserves_order_until_source_tag_lands() {
        let plugins = vec![
            PluginEntry {
                name: "a".to_owned(),
                ..PluginEntry::default()
            },
            PluginEntry {
                name: "b".to_owned(),
                ..PluginEntry::default()
            },
        ];
        let kept = plugins_for_scope(&plugins, RenderScope::Project);
        assert_eq!(kept.len(), 2);
    }
}
