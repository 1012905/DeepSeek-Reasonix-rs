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
// wave38-a2: TOOL-025 bash decision satellite and TOOL-026 background-job
// tools. Visibility declarations only; registry wiring into the live session
// is added by the integration slice.
pub mod bash_tool;
pub mod bashrun;
pub mod bgjobs;
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
pub mod goalreport;
pub mod grep;
pub mod listdir;
pub mod movefile;
pub mod multiedit;
pub mod network;
pub mod pathgo;
pub mod peer;
pub mod project;
pub mod readfile;
pub mod recall;
pub mod receipt;
pub mod secretpaths;
pub mod settings;
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
pub use goalreport::{
    GoalReport, GoalTurnRecorder, UPDATE_GOAL_DESCRIPTION, UPDATE_GOAL_TOOL_NAME, UpdateGoalTool,
    completion_note, goal_recorder_from, schema_value as goal_schema_value, update_goal_tool,
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
pub use peer::{
    CODE_DISPATCH_UNAVAILABLE, CODE_PEER_DISABLED, CODE_PEER_EMPTY, DISABLED_MESSAGE,
    LIST_PEERS_DESCRIPTION, LIST_PEERS_NAME, ListPeersTool, PeerDirectory, PeerLink, PeerReceipt,
    PeerView, PrefixedError, SEND_PEER_MESSAGE_DESCRIPTION, SEND_PEER_MESSAGE_NAME,
    SendPeerMessageTool, peer_tools,
};
pub use project::{
    DISPATCH_DESCRIPTION, DISPATCH_TO_PROJECT_NAME, DispatchReceipt, DispatchToProjectTool,
    OPEN_PROJECTS_DESCRIPTION, OPEN_PROJECTS_NAME, OpenProjectsTool, OpenedProject,
    ProjectDispatcher, ProjectOpener, dispatch_tool, open_projects_tool,
};
pub use readfile::{READ_FILE_DEFAULT_LIMIT, ReadFileTool};
pub use recall::{
    ContextRecaller, MAX_RECALL_POSITIONS, RECALL_DESCRIPTION_FULL, RECALL_DESCRIPTION_SEARCH_OFF,
    RECALL_TOOL_NAME, RecallHit, RecallRequest, RecallResult, RecallTool, position_list,
    recall_tool, recall_without_search, recaller_from, render_recall, render_recall_search,
};
pub use receipt::{
    MAX_CAPTURED_RECEIPT_SPAN_BYTES, MAX_POST_WRITE_RECEIPT_BYTES, POST_WRITE_RECEIPT_TRUNCATED,
    POST_WRITE_SPAN_TRUNCATED, with_actual_post_write_receipts,
};
pub use secretpaths::{protect_sensitive_files, set_protect_sensitive_files};
pub use settings::{
    CODE_SETTING_INVALID_VALUE, CODE_SETTING_UNAVAILABLE, SETTING_DESCRIPTION, SETTING_LIST_MAX,
    SETTING_TOOL_NAME, SettingInfo, SettingReceipt, SettingTool, SettingsHost, SettingsLink,
    list_settings, setting_tool,
};
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

// wave37-tools-b: TOOL-014/015/016/017 seams plus the compress and
// context_budget built-ins. Visibility declarations only; the registry
// wiring into the live session is added by the integration slice.
pub mod contextbudget;
pub mod ctxfold;
pub mod elicit;
pub mod managedconfig;
pub mod paidsearch;

pub use contextbudget::{
    CONTEXT_BUDGET_DESCRIPTION, CONTEXT_BUDGET_TOOL_NAME, ContextBudget, ContextBudgetReporter,
    ContextBudgetTool, STATUS_OK, STATUS_UNMEASURED, context_budget_tool, reporter_from,
};
pub use ctxfold::{
    COMPRESS_DESCRIPTION, COMPRESS_TOOL_NAME, CompressRequest, CompressResult, CompressTool,
    ContextCompressor, MAX_COMPRESS_ANCHOR_BYTES, MAX_COMPRESS_FOCUS_BYTES, compress_tool,
    compressor_from,
};
pub use elicit::{ElicitField, ElicitReply, ElicitRequest, Elicitor, elicit_from};
pub use managedconfig::{
    CONFIG_WRITE_APPROVAL_TOOL, CONFIG_WRITE_DECLINED, CONFIG_WRITE_REASON,
    CONFIG_WRITE_SUBJECT_PREFIX, ConfigWriteApprover, ConfigWriteRequest,
    ConfigWriteSessionChecker, DECLINE_REASON_DEFAULT, ManagedConfigError, ManagedConfigPaths,
    config_write_approver_from, config_write_subject,
};
pub use paidsearch::{
    PAID_SEARCH_APPROVAL_TOOL, PAID_SERVICE_FETCH, PAID_SERVICE_SEARCH, PaidSearchApprover,
    PaidSearchRequest, paid_search_approver_from,
};

// wave38-a1: TOOL-024 catalog snapshot and TOOL-045 shared shellrun slice.
// Visibility declarations only; registry wiring is added by the integration slice.
pub mod catalog;
pub mod shellrun;

// wave38-a3: TOOL-046/047/048 background jobs manager slice. Visibility
// declarations only; the bash_output/kill_shell/wait wrapper registration is
// added by the integration slice.
pub mod jobs;

// wave39-b: TOOL-035 misc tools (present_files, system_one). Card and seam
// only; host resolution binds in the integration slice.
pub mod misc;

pub use misc::{
    CODE_SYSTEM_ONE_NOT_CONFIGURED, LOCAL_BACKEND, MAX_PRESENT_FILES, PRESENT_FILES_DESCRIPTION,
    PRESENT_FILES_TOOL_NAME, PresentFilesHost, PresentFilesTool, PresentResult, PresentedFile,
    SYSTEM_ONE_TOOL_NAME, SystemOneError, SystemOneEvaluator, SystemOneSpec, SystemOneTool,
    present_files_tool, system_one_description, system_one_not_configured, system_one_schema,
};

// wave39-a: TOOL-030/031/032/033/034 web/browser/computer/media/plan faces.
// Visibility declarations only; registry wiring into the live session is
// added by the integration slice.
pub mod browser;
pub mod chartspec;
pub mod computer;
pub mod media;
pub mod plan;
pub mod plan_evidence;
pub mod web;

pub use browser::{
    ActResult, BROWSER_ACT_DESCRIPTION, BROWSER_ACT_SCHEMA, BROWSER_ACT_TOOL_NAME,
    BROWSER_OPEN_DESCRIPTION, BROWSER_OPEN_TOOL_NAME, BROWSER_READ_DESCRIPTION,
    BROWSER_READ_TOOL_NAME, BrowserActTool, BrowserFailure, BrowserHost, BrowserOpenTool,
    BrowserReadTool, BrowserStep, Changes, Dialog, LogEntry, Request, Snapshot, TabInfo,
    browser_act_tool, browser_host_from, browser_open_tool, browser_read_tool, browser_tools,
    human_bytes, render_changes, render_found, render_logs, render_requests, render_snapshot,
    render_tabs, request_line, tab_line,
};
pub use chartspec::{
    CODE_COLUMN_UNKNOWN, CODE_LIMIT_EXCEEDED, CODE_SCHEMA_INVALID, CODE_SPEC_VERSION_UNSUPPORTED,
    CODE_TYPE_MISMATCH, ChartError, Mark, Spec, chart_id, parse as chartspec_parse,
};
pub use computer::{
    ActResult as ComputerActResult, App, COMPUTER_ACT_DESCRIPTION, COMPUTER_ACT_TOOL_NAME,
    COMPUTER_READ_DESCRIPTION, COMPUTER_READ_TOOL_NAME, ComputerActTool, ComputerHost,
    ComputerReadTool, ComputerStep, Effect, Modal, Rect, Snapshot as ComputerSnapshot, StepResult,
    Window, computer_act_tool, computer_host_from, computer_read_tool, computer_tools,
    pointer_steps, render_apps, render_computer_snapshot, render_effect, screenshot_caption,
};
pub use media::{
    READ_VIDEO_DESCRIPTION, READ_VIDEO_TOOL_NAME, RENDER_CHART_DESCRIPTION, RENDER_CHART_TOOL_NAME,
    ReadVideoTool, RenderChartTool, VideoHost, VideoInfo, media_tools, normalize_frame_count,
    read_video_tool, render_chart_tool, render_video_summary, sample_times, video_demuxer,
    video_host_from,
};
pub use plan::{
    CODE_PLAN_MODE_ACTIVE, COMPLETE_STEP_DESCRIPTION, COMPLETE_STEP_TOOL_NAME, CompleteStepTool,
    PlanHost, PlanModeReader, TODO_WRITE_DESCRIPTION, TODO_WRITE_TOOL_NAME, TodoWriteTool,
    complete_step_tool, plan_host_from, plan_mode_from, plan_tools, todo_write_tool,
};
pub use plan_evidence::{
    TodoItem, TodoStepMatch, advance_serial_todo, first_unfinished_sub_step, in_progress_todo,
    incomplete_todos, match_step, match_step_id, match_todo_identity, normalize_serial_todos,
    preserves_completed_todo_positions, same_todos, todo_citation, todo_step_ids,
    validate_serial_todos,
};
pub use web::{
    FetchPage, IMPORT_WEB_IMAGE_DESCRIPTION, IMPORT_WEB_IMAGE_TOOL_NAME, ImageImportArgs,
    ImageReceipt, ImportWebImageTool, WEB_FETCH_DESCRIPTION, WEB_FETCH_TOOL_NAME, WebFetchTool,
    WebHost, content_type_short, import_web_image_tool, public_by_construction, render_fetch_page,
    sha256_hex, validate_image_target, vet_fetch_url, web_fetch_tool, web_host_from, web_tools,
};

// wave40-c: TOOL-049/051/053/054/057 productdocs corpus, session-data guard,
// delete-extent classifier, file views and divergence review. Visibility
// declarations only; registry wiring into the live session is added by the
// integration slice.
pub mod divergences;
pub mod fileviews;
pub mod productdocs;
pub mod productdocs_retrieval;
pub mod sessiondata;

pub use divergences::{Divergence, DivergenceStatus, divergences, is_unregistered_delegator};
pub use fileviews::FileViews;
pub use productdocs::{Catalog, DocsTool, docs_tool, load_catalog};
pub use sessiondata::{CODE_SCHEDULE_STATE_WRITE, CODE_TRUSTED_STATE_WRITE, SessionDataGuard};

// wave40-a: TOOL-038 capability catalog/ledger/audit + routing/semantic router,
// TOOL-039 delegation tools, TOOL-040 skill tools and wrappers. Visibility
// declarations and seams only; registry wiring into the live session is added
// by the integration slice.
pub mod capability_audit;
pub mod capability_catalog;
pub mod capability_ledger;
pub mod capability_router;
pub mod delegate;
pub mod skill;
pub mod skill_tools;

// wave40-b: TOOL-041 extension runtime tools, TOOL-042 agent/state/platform
// cards, TOOL-043 conditional surface, TOOL-044 legacy delegation name table.
// Card/seam declarations only; registry wiring into the live session is added
// by the integration slice.
pub mod agentstate;
pub mod conditional;
pub mod conditional_slash;
pub mod extension;
pub mod legacy_names;

pub use agentstate::{
    AWAIT_USER_DESCRIPTION, AWAIT_USER_NEED_LIMIT, AWAIT_USER_TOOL_NAME, AwaitTarget,
    AwaitUserError, AwaitUserHost, AwaitUserTool, CODE_IMAGE_TOO_LARGE, CODE_IMAGE_UNFIT,
    CODE_IMAGE_UNSUPPORTED, ReadOnlyBashBlocker, ReadOnlyBashClassifier, ReadOnlyBashTool,
    VIEW_IMAGE_DESCRIPTION, VIEW_IMAGE_MAX_BYTES, VIEW_IMAGE_TOOL_NAME, ViewImageAsset,
    ViewImageHost, ViewImageTool, data_url, mime_supported, readonly_bash_description,
    readonly_block_reason, render_image, render_recorded, resolve_await_target, too_large_refusal,
    unfit_refusal, unsupported_refusal, validate_need,
};
pub use conditional::{
    BilledSink, CODE_FETCH_ALL_FAILED, CODE_FETCH_INTERNAL_ADDRESS, CODE_FETCH_PAID_NEEDS_CONSENT,
    ConditionalSurface, FETCH_SNIP_HINT, FallbackSink, FetchBackend, FetchChain, FetchChainError,
    FetchConsent, FetchFallback, FetchVet, MCP_CONNECT_PREFIX, MCP_TOOL_PREFIX, SearchResult,
    SnipHint, WEB_SEARCH_DESCRIPTION, WEB_SEARCH_QUERY_MAX, WEB_SEARCH_TOOL_NAME,
    WebSearchProvider, WebSearchTool, conditional_surfaces, mcp_connect_permission_name,
    mcp_model_tool_name, mcp_tool_prefix, normalize_name, render_page, validate_web_query,
};
pub use conditional_slash::{
    INSTALL_SOURCE_DESCRIPTION, INSTALL_SOURCE_TOOL_NAME, InstallApprover, InstallConnector,
    InstallError, InstallReceipt, InstallRequest, InstallSourceTool, NormalizedRequest,
    SlashCatalog, SlashCommandTool, SlashEntry, SlashGate, SlashRenderer, SlashVisibility,
    expand_frame, list_slash_commands, normalize_install_op, normalize_kind, normalize_mode,
    normalize_scope, normalize_slash_name, normalize_tier, normalize_transport, unticketed_apply,
    validate_install_request,
};
pub use extension::{
    CONNECT_EXTENSION_PREFIX, ExtensionCall, ExtensionDiscoveryTool, ExtensionError, ExtensionHost,
    ExtensionPackageMetadata, ExtensionTool, MAX_TOOL_NAME_LEN, MintedExtension, RuntimeTool,
    contract_matches, decl_digest, digest, discovery_description, discovery_tool_name,
    extension_tool_name, mint_declared, name_offered, overlong_warning,
};
pub use legacy_names::{
    FIXTURE_FLEET_ID, FIXTURE_PARALLEL_TASKS_ID, FIXTURE_READ_ONLY_TASK_ID,
    GUIDANCE_PLAN_EXEC_DISPATCH, GUIDANCE_PLAN_EXEC_RESEARCH, GUIDANCE_PROMETHEUS_PARALLEL,
    LEGACY_FLEET, LEGACY_PARALLEL_TASKS, LEGACY_READ_ONLY_TASK, LEGACY_TASK, NESTED_SINK_DOC,
    READ_SUBAGENT_RESULT_DESCRIPTION, SCHEDULER_COMMENT, claude_default_description,
    claude_spawning_tools, e2ebench_delegation_tools, is_background_task_call,
    is_builtin_or_meta_tool, is_legacy_delegation_name, is_review_task_receipt,
    joined_parallel_task_prompts, legacy_delegation_names, read_only_subagent_workflow_tools,
    subagent_always_hidden_tools, subagent_hooks_fire, subagent_job_tools, subagent_meta_tools,
    subagent_recursive_tools, task_profile_from_args,
};
