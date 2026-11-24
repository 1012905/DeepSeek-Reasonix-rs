//! The builtin tool slice: the read tools (read_file, glob, ls, grep) from
//! waves 17-19 and the write/edit family (write_file, edit_file,
//! multi_edit, move_file) with the workspace confinement machinery they
//! share (confine.go). Parity targets live in each module's docs.

#![forbid(unsafe_code)]

pub mod anchors;
pub mod ask;
pub mod bash;
pub mod bash_cmdline;
pub mod bash_env;
pub mod bash_nul;
pub mod bash_pipestatus;
pub mod bashrun;
pub mod codeindex;
pub mod confine;
pub mod editengine;
pub mod editfile;
pub mod editsource;
pub mod fileencode;
pub mod fuzzyedit;
pub mod gitignore;
pub mod glob;
pub mod globengine;
pub mod grep;
pub mod listdir;
pub mod movefile;
pub mod multiedit;
pub mod network;
pub mod pathgo;
pub mod readfile;
pub mod receipt;
pub mod secretpaths;
pub mod walk;
pub mod writeatomic;
pub mod writefile;
pub mod writepaths;

pub use anchors::{CODE_WRITE_OUTSIDE_SCOPE, ConfineError, confine_write_side_resolved};
pub use ask::{
    ASK_DESCRIPTION, ASK_DISMISSED, ASK_REASON_MISSING_VALUE, ASK_REASON_USER_DECISION,
    ASK_TOOL_NAME, ASK_UNRESOLVED, AskTool, ask_tool, schema_value as ask_schema_value,
};
pub use bash::{
    BASH_TOOL_STEER, BashError, BashTool, ExternalRefTokens, bash_error, confine_bash,
    descriptor_from_shell, unconfined_bash,
};
pub use bash_cmdline::{CODE_COMMAND_LINE_TOO_LONG, check_command_line, command_line_units};
pub use bash_env::{
    AGENT_CHILD_ENV, apply_env_overrides, command_env, env_key_equal, filter_env, process_env,
    set_env_value, shell_env_map, shell_env_overrides,
};
pub use bash_nul::{NulWatch, arm_nul_watch, nul_file_in};
pub use bash_pipestatus::{
    PipeStatusProbe, pipe_status_can_decide_verification, pipeline_for_status,
    set_verification_decider, terminal_pipelines,
};
pub use bashrun::{
    COMBINED_OUTPUT_MAX_BYTES, COMBINED_OUTPUT_TAIL_BYTES, COMBINED_OUTPUT_TRUNCATED, CancelToken,
    RunError, go_duration_text, run_foreground,
};
pub use codeindex::CodeIndexTool;
pub use confine::{
    CODE_READ_OUTSIDE_SCOPE, ManagedConfigHook, SessionDataGuardHook, SessionTempSource,
    WriteHooks, confine, confine_opened, confine_read, confine_resolved, confine_scope,
    confine_write, read_outside_scope, real_path, real_roots, under_session_temp, within,
    within_fold,
};
pub use editengine::{
    EditApplyResult, EditRange, EditReplacementReceipt, apply_old_string_edit, matched_range_sample,
};
pub use editfile::EditFileTool;
pub use editsource::{
    CODE_OVERWRITE_READ_FORBIDDEN, CODE_OVERWRITE_READ_OUTSIDE_SCOPE, CODE_READ_FORBIDDEN,
    EditReadPolicy, EditSource, FileOverlay, FileViewsHook, OVERWRITE_READ_ACCESS_DENIED_MESSAGE,
    WriteReceipt, check_overwrite_read_access, check_read_target_access, read_edit_source,
    source_for_write, write_file_encoded,
};
pub use fileencode::{ERR_UNENCODABLE, UnencodableError, encode, encode_bytes};
pub use fuzzyedit::{
    fuzzy_edit_ranges, multi_edit_failure, old_string_not_found_error, old_string_not_unique_error,
};
pub use glob::{
    GLOB_DEFAULT_TIMEOUT, GLOB_MAX_RESULTS, GLOB_MAX_TIMEOUT, GlobTool, glob_base, glob_walk_root,
};
pub use grep::{
    GREP_DEFAULT_TIMEOUT, GREP_MAX_MATCHES, GREP_MAX_TIMEOUT, GrepScope, GrepTool, confine_search,
    format_grep, grep_glob_matches, resolve_search, timeout as tool_timeout,
};
pub use listdir::ListDirTool;
pub use movefile::{MoveFileTool, RenameFn, is_cross_device, std_rename};
pub use multiedit::{EditStep, MultiEditTool};
pub use network::{
    CODE_NETWORK_PATH_OUTSIDE_SCOPE, ResolvedPath, is_network_path, refuse_network_path,
};
pub use readfile::{READ_FILE_DEFAULT_LIMIT, ReadFileTool};
pub use receipt::{
    MAX_CAPTURED_RECEIPT_SPAN_BYTES, MAX_POST_WRITE_RECEIPT_BYTES, POST_WRITE_RECEIPT_TRUNCATED,
    POST_WRITE_SPAN_TRUNCATED, with_actual_post_write_receipts,
};
pub use secretpaths::{protect_sensitive_files, set_protect_sensitive_files};
pub use writefile::WriteFileTool;
pub use writepaths::{
    ERR_WRITE_PATH_REQUIRED, SESSION_TEMP_ENV_KEYS, canonical_write_path, canonical_writer_path,
    resolve_session_temp, resolve_write_paths,
};

use std::sync::Arc;

use crate::abi::Tool;

/// confine.go `ConfineReaders`: the read/list/search built-ins bound to
/// forbidRoots — directories the agent may not read or list. Empty
/// forbidRoots yields unconfined readers. grep is handled separately by
/// [`confine_search`] so it can carry the sandbox spec for its ripgrep
/// subprocess.
pub fn confine_readers(forbid_roots: &[String]) -> Vec<Arc<dyn Tool>> {
    let rs = real_roots(forbid_roots);
    vec![
        Arc::new(ReadFileTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
        Arc::new(ListDirTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
        Arc::new(GlobTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
        Arc::new(CodeIndexTool {
            net_roots: Vec::new(),
            work_dir: String::new(),
            forbid_roots: rs.clone(),
            read_roots: Vec::new(),
        }),
    ]
}

/// confine.go `ConfineWriters` for the writer family this slice migrated:
/// the file-writing built-ins bound to roots — the only directories they
/// may modify. Empty roots yields unconfined writers. guard rejects
/// Reasonix's own session stores even inside the roots; managed names the
/// config files writable outside the roots after a fresh human approval.
/// The Go members notebook_edit, delete_range and delete_symbol land with
/// their own slices.
pub fn confine_writers(
    roots: &[String],
    guard: Option<Arc<dyn SessionDataGuardHook>>,
    managed: Option<Arc<dyn ManagedConfigHook>>,
) -> Vec<Arc<dyn Tool>> {
    let rs = real_roots(roots);
    let guard = guard.clone();
    let managed = managed.clone();
    vec![
        Arc::new(WriteFileTool {
            roots: rs.clone(),
            guard: guard.clone(),
            managed: managed.clone(),
            ..WriteFileTool::default()
        }),
        Arc::new(EditFileTool {
            roots: rs.clone(),
            guard: guard.clone(),
            managed: managed.clone(),
            ..EditFileTool::default()
        }),
        Arc::new(MultiEditTool {
            roots: rs.clone(),
            guard: guard.clone(),
            managed: managed.clone(),
            ..MultiEditTool::default()
        }),
        Arc::new(MoveFileTool {
            roots: rs,
            guard,
            managed,
            ..MoveFileTool::default()
        }),
    ]
}

/// The unconfined instances a composition root overrides, mirroring the Go
/// init-time registrations for this slice's tools.
pub fn unconfined_readers() -> Vec<Arc<dyn Tool>> {
    confine_readers(&[])
}

pub fn unconfined_writers() -> Vec<Arc<dyn Tool>> {
    confine_writers(&[], None, None)
}

/// Go `init`'s `tool.RegisterBuiltin(bash{})`: the unconfined shell tool,
/// bound to nothing. A composition root overrides it with [`confine_bash`].
pub fn unconfined_bash_tool() -> Arc<dyn Tool> {
    Arc::new(bash::unconfined_bash())
}

/// Go `addTurnExitTools`' `ask` registration: the decision that is the
/// user's to make. It stays registered every turn — the schema is
/// cache-stable and the host does not detect the condition first — and
/// reaches the user through the call context's asker.
pub fn turn_exit_tools() -> Vec<Arc<dyn Tool>> {
    vec![ask::ask_tool()]
}
