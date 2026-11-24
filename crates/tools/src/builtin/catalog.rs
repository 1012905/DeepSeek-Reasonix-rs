//! TOOL-024: the frozen built-in catalog snapshot (docs/contracts/tools.md §2.1).
//!
//! The 33 built-in tools register through `RegisterBuiltin` (`present_files`
//! and `system_one` through `RegisterConditional`). The model-visible surface
//! is `Name()` + `Description()` + `Schema()` + `ReadOnly()`; this table is
//! that surface frozen from the Go source, so a boot-time contract test can
//! pin names, verbatim descriptions, schema shapes and the read-only
//! classification without instantiating every tool.
//!
//! Variants: `bash`, `grep` and `recall` carry two description/schema arms in
//! Go; the row records the default arm and names the other in [`CatalogEntry::notes`].
//! `system_one`'s description and backend enum are dynamic over the configured
//! backends; the row holds the zero-spec snapshot.

#![forbid(unsafe_code)]

use serde_json::Value;

/// One frozen catalog row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogEntry {
    /// The model-callable tool name (`Name()`).
    pub name: &'static str,
    /// The verbatim model-visible description (`Description()`).
    pub description: &'static str,
    /// The canonical JSON Schema (`Schema()`), serialized once at registration.
    pub schema: &'static str,
    /// No observable side effects (`ReadOnly()`). A batch parallelizes only when
    /// every call in it is read-only.
    pub read_only: bool,
    /// Registered conditionally (`RegisterConditional`) rather than unconditionally.
    pub conditional: bool,
    /// Variant / dynamic-arm caveats for the row; empty when none apply.
    pub notes: &'static str,
}

const SCHEMA_BASH: &str = r#"{"type":"object","properties":{"command":{"type":"string","description":"Shell command to execute"},"description":{"type":"string","description":"Optional short name for this call, in the user's language, shown in task lists."},"run_in_background":{"type":"boolean","description":"Run detached: returns a job id immediately and keeps running across turns (no foreground timeout). Read new output with bash_output, wait with wait, stop it with kill_shell. Use for long-running commands like servers, watchers, or builds you don't need to block on."},"preserve_background_processes":{"type":"boolean","description":"After the shell command exits normally, keep any process-group members it intentionally left behind. Use only for deliberate daemonization, browser/GUI/session launchers such as playwright-cli open, or nohup/disown/setsid; cancellation and timeouts still kill the process group."},"timeout_seconds":{"type":"integer","description":"Deadline for this foreground call. Only tightens the host cap — a larger value is clamped to it. Output collected before the deadline is still returned, so set it low when a command's cost is unpredictable (a broad filesystem walk, a network fetch) instead of paying the full cap to find out. For work that legitimately runs long, use run_in_background.","minimum":1},"verifies":{"type":"string","description":"Name what this call decides (e.g. \"balances.json keys are sorted\") to record it as a check. The exit status becomes the verdict, so the command must be one command or an && chain; a ';', a pipe, or '||' hands the status to something else and the call is refused."}},"required":["command"]}"#;

const SCHEMA_BASH_OUTPUT: &str = r#"{"type":"object","properties":{"job_id":{"type":"string","description":"The background job id (e.g. \"bash-1\") returned when it was started."},"filter":{"type":"string","description":"Optional regular expression; only matching lines of the new output are returned."}}}"#;

const SCHEMA_KILL_SHELL: &str = r#"{"type":"object","properties":{"job_id":{"type":"string","description":"The background job id to terminate (e.g. \"bash-1\")."}},"required":["job_id"]}"#;

const SCHEMA_WAIT: &str = r#"{"type":"object","properties":{"job_ids":{"type":"array","items":{"type":"string"},"description":"Background job ids to wait for. Omit to wait for every currently-running job."},"timeout_seconds":{"type":"integer","description":"Seconds to block before returning. Every wait is capped at 300, including when this is omitted. On timeout the result carries progress; read it, then wait again.","minimum":1,"maximum":300},"until_output_matches":{"type":"string","description":"Optional regular expression (RE2). Return early as soon as a job writes something matching it. Use it to wait once for the line you actually care about — a completion marker, or an error — instead of polling to check."}}}"#;

const SCHEMA_BROWSER_OPEN: &str = r#"{"type":"object","properties":{"url":{"type":"string"},"tab":{"type":"string","description":"t1, t2...; default active"},"new_tab":{"type":"boolean"},"close":{"type":"boolean"}}}"#;

const SCHEMA_BROWSER_READ: &str = r#"{"type":"object","properties":{"what":{"type":"string","enum":["snapshot","find","screenshot","logs","network","tabs"]},"tab":{"type":"string"},"ref":{"type":"string"},"text":{"type":"string"},"offset":{"type":"integer"},"limit":{"type":"integer"}}}"#;

const SCHEMA_BROWSER_ACT: &str = r#"{"type":"object","properties":{"tab":{"type":"string"},"steps":{"type":"array","items":{"type":"object","properties":{"action":{"type":"string","enum":["click","double_click","hover","fill","type","press","select","scroll","drag","upload","wait","wait_for","dialog","eval","resize","back","forward","reload"]},"ref":{"type":"string"},"text":{"type":"string"},"script":{"type":"string"},"key":{"type":"string"},"values":{"type":"array","items":{"type":"string"}},"x":{"type":"number"},"y":{"type":"number"},"delta_x":{"type":"number"},"delta_y":{"type":"number"},"to_ref":{"type":"string"},"to_x":{"type":"number"},"to_y":{"type":"number"},"files":{"type":"array","items":{"type":"string"}},"ms":{"type":"integer"},"width":{"type":"integer"},"height":{"type":"integer"},"accept":{"type":"boolean"}},"required":["action"]}}},"required":["steps"]}"#;

const SCHEMA_RENDER_CHART: &str = r#"{"type":"object","additionalProperties":false,"required":["spec_version","title","data","marks"],"properties":{"spec_version":{"type":"integer","minimum":1,"maximum":1},"title":{"type":"string"},"data":{"type":"object","required":["columns","rows"],"properties":{"columns":{"type":"array","items":{"type":"object","required":["name","type"],"properties":{"name":{"type":"string"},"type":{"type":"string","enum":["number","string","date"]}}}},"rows":{"type":"array","items":{"type":"array"}}}},"marks":{"type":"array","items":{"type":"object","required":["type","x","y"],"properties":{"type":{"type":"string","enum":["bar","line","pie"]},"x":{"type":"string"},"y":{"type":"array","items":{"type":"string"}},"color":{"type":"string"},"stacked":{"type":"boolean"},"donut":{"type":"boolean"}}}},"x_axis":{"type":"object"},"y_axis":{"type":"object"}}}"#;

const SCHEMA_CODE_INDEX: &str = r#"{
"type":"object",
"properties":{
  "action":{"type":"string","enum":["outline","search"],"description":"outline lists symbols under path; search finds symbol definition candidates by name."},
  "path":{"type":"string","description":"File or directory path to inspect (default \".\")."},
  "query":{"type":"string","description":"Symbol name or substring for action=search."},
  "kind":{"type":"string","description":"Optional symbol kind filter, such as func, method, class, type, interface, const, var, struct, enum, trait."},
  "limit":{"type":"integer","description":"Maximum symbols to return (default 100, max 200).","minimum":1}
},
"required":["action"]
}"#;

const SCHEMA_COMPLETE_STEP: &str = r#"{
"type":"object",
"properties":{
  "step_id":{"type":"string","description":"PREFERRED: the stable step_id of the task-list item this completes, e.g. \"plan_step_02\". Unlike a title or a number it survives retitles, insertions, and reordering, so cite it whenever the item has one."},
  "step":{"type":"string","description":"Which plan step this completes — its title or number, matching the task list. Use only when the item has no step_id."},
  "step_index":{"type":"integer","minimum":1,"description":"Optional 1-based task-list item number. Use only when the item has no step_id; an index goes stale the moment a step is inserted above it."},
  "result":{"type":"string","description":"What is now true or changed as a result of finishing this step."},
  "notes":{"type":"string","description":"Optional caveats, follow-ups, or anything deferred."}
},
"required":["result"]
}"#;

const SCHEMA_COMPRESS: &str = r#"{
"type":"object",
"additionalProperties":false,
"properties":{
  "direction":{"type":"string","enum":["before","after"],"description":"Which side of the anchor user turn to compress."},
  "anchor":{"type":"string","minLength":1,"maxLength":512,"description":"An exact, unique excerpt from one real user message in the current model-visible conversation."},
  "focus":{"type":"string","maxLength":2000,"description":"Optional guidance about facts or decisions the summary must preserve."}
},
"required":["direction","anchor"]
}"#;

const SCHEMA_COMPUTER_READ: &str = r#"{"type":"object","properties":{"window":{"type":"integer","minimum":1},"what":{"type":"string","enum":["apps","snapshot","screenshot"]},"app":{"type":"string"}},"required":["what"]}"#;

const SCHEMA_COMPUTER_ACT: &str = r#"{"type":"object","properties":{"app":{"type":"string"},"window":{"type":"integer","minimum":1},"steps":{"type":"array","maxItems":64,"items":{"type":"object","properties":{"action":{"type":"string","enum":["launch","click","right_click","set_value","type","paste","key","scroll","wait","pointer_click"]},"ref":{"type":"string"},"text":{"type":"string"},"key":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},"amount":{"type":"number"},"ms":{"type":"integer"},"to_x":{"type":"number"},"to_y":{"type":"number"},"button":{"type":"string","enum":["left","right","middle"]},"times":{"type":"integer"},"seconds":{"type":"number"}},"required":["action"]}}},"required":["app","steps"]}"#;

const SCHEMA_CONTEXT_BUDGET: &str =
    r#"{"type":"object","additionalProperties":false,"properties":{}}"#;

const SCHEMA_DELETE_RANGE: &str = r#"{
	"type":"object",
	"properties":{
		"path":{"type":"string","description":"File path"},
		"start_anchor":{"type":"string","description":"Exact text of the first line to delete (must be unique in the file)"},
		"end_anchor":{"type":"string","description":"Exact text of the last line to delete (must be unique in the file)"},
		"inclusive":{"type":"boolean","description":"Whether to include the anchor lines in the deletion (default true)"}
	},
	"required":["path","start_anchor","end_anchor"]
}"#;

const SCHEMA_DELETE_SYMBOL: &str = r#"{
	"type":"object",
	"properties":{
		"path":{"type":"string","description":"Path to the source file"},
		"name":{"type":"string","description":"Name of the symbol to delete"},
		"kind":{"type":"string","description":"Optional kind filter: func, method, type, interface, const, var"},
		"parent":{"type":"string","description":"Optional parent struct name for method disambiguation"}
	},
	"required":["path","name"]
}"#;

const SCHEMA_EDIT_FILE: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"File path"},"old_string":{"type":"string","description":"Exact text to replace (must be unique in the file)"},"new_string":{"type":"string","description":"Replacement text (may be empty to delete)"}},"required":["path","old_string","new_string"]}"#;

const SCHEMA_GLOB: &str = r#"{"type":"object","properties":{"pattern":{"type":"string","description":"Glob pattern (supports ** for recursive matching)"},"timeout_seconds":{"type":"integer","description":"Walk timeout in seconds (default 30, max 300); partial results are returned when it expires"}},"required":["pattern"]}"#;

const SCHEMA_GREP: &str = r#"{"type":"object","properties":{"pattern":{"type":"string","description":"Regular expression (RE2 syntax)"},"path":{"type":"string","description":"File or directory to search (default \".\")"},"glob":{"type":"string","description":"Only search files matching this glob. A glob with no slash matches the file name at any depth (\"*.go\"); one with a slash matches the path below the search root (\"internal/**/*_test.go\")."},"timeout_seconds":{"type":"integer","description":"Abort and return partial matches after this many seconds (default 30, max 300). Raise it for a large tree; lower it for a quick probe.","minimum":1}},"required":["pattern"]}"#;

const SCHEMA_LS: &str = r#"{"type":"object","properties":{"path":{"type":"string","description":"Directory path (default \".\")"},"recursive":{"type":"boolean","description":"When true, recursively list all nested files (default false)"}}}"#;

const SCHEMA_MOVE_FILE: &str = r#"{"type":"object","properties":{"source_path":{"type":"string","description":"Existing file path to move"},"destination_path":{"type":"string","description":"Destination file path; must not already exist"}},"required":["source_path","destination_path"]}"#;

const SCHEMA_MULTI_EDIT: &str = r#"{
"type":"object",
"properties":{
  "path":{"type":"string","description":"File path"},
  "edits":{
    "type":"array",
    "minItems":1,
    "description":"Ordered edits. Each step sees the file as left by the previous step.",
    "items":{
      "type":"object",
      "properties":{
        "old_string":{"type":"string","description":"Exact text to find. Without replace_all, must match exactly once."},
        "new_string":{"type":"string","description":"Replacement text (empty deletes)."},
        "replace_all":{"type":"boolean","description":"Replace every occurrence instead of requiring uniqueness."}
      },
      "required":["old_string","new_string"]
    }
  }
},
"required":["path","edits"]
}"#;

const SCHEMA_NOTEBOOK_EDIT: &str = r#"{
	"type": "object",
	"properties": {
		"path": {"type": "string", "description": "Path to the .ipynb notebook."},
		"cell_number": {"type": "integer", "description": "0-based index of the target cell. For insert, the new cell goes after this one (-1 prepends)."},
		"cell_id": {"type": "string", "description": "Target the cell by its id instead of cell_number (replace/delete)."},
		"new_source": {"type": "string", "description": "The cell's new source text (replace/insert)."},
		"cell_type": {"type": "string", "enum": ["code", "markdown"], "description": "Cell type for insert (and optional retype on replace)."},
		"edit_mode": {"type": "string", "enum": ["replace", "insert", "delete"], "description": "replace (default), insert, or delete."}
	},
	"required": ["path"]
}"#;

const SCHEMA_PRESENT_FILES: &str = r#"{"type":"object","properties":{"files":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"object","properties":{"path":{"type":"string","maxLength":4096},"description":{"type":"string","maxLength":256}},"required":["path"],"additionalProperties":false}}},"required":["files"],"additionalProperties":false}"#;

const SCHEMA_READ_FILE: &str = r#"{
"type":"object",
"properties":{
  "path":{"type":"string","description":"File path"},
  "offset":{"type":"integer","description":"0-based line offset to start reading from (default 0)","minimum":0},
  "limit":{"type":"integer","description":"Maximum lines to return (default 2000)","minimum":1}
},
"required":["path"]
}"#;

const SCHEMA_READ_VIDEO: &str = r#"{"type":"object","properties":{"path":{"type":"string"},"url":{"type":"string","description":"public https address the user wrote"},"mode":{"type":"string","enum":["auto","frames","native"]},"frames":{"type":"integer","description":"max 12"},"start":{"type":"number","description":"seconds"},"end":{"type":"number","description":"seconds"}},"additionalProperties":false}"#;

const SCHEMA_RECALL: &str = r#"{
"type":"object",
"additionalProperties":false,
"properties":{
  "positions":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"integer","minimum":0},"description":"Read these #n addresses, from an index line or a search hit, without the '#'."},
  "query":{"type":"string","description":"Search for these words instead. A tool result matches on its output and is addressed by its call."},
  "limit":{"type":"integer","minimum":1,"maximum":20,"description":"Search hits (default 8)."}
}
}"#;

const SCHEMA_SYSTEM_ONE: &str = r#"{"type":"object","properties":{"backend":{"type":"string","enum":[],"description":"Configured decision backend; optional only when exactly one is available"},"state":{"description":"Text or structured JSON state to evaluate"},"questions":{"type":"object","description":"Named questions; answer keys match these ids","additionalProperties":{"type":"object","properties":{"type":{"type":"string","enum":["noul","choice","score"]},"instructions":{"description":"Question text or structured instructions"},"criteria":{"description":"Noul: optional true/false object; Choice: 2-255 option map; Score: ordered array of 2-10 levels"}},"required":["type","instructions"],"additionalProperties":false}}},"required":["state","questions"],"additionalProperties":false}"#;

const SCHEMA_TODO_WRITE: &str = r#"{
"type":"object",
"properties":{
  "todos":{
    "type":"array",
    "description":"The complete task list, in order. Replaces any previous list.",
    "items":{
      "type":"object",
      "properties":{
        "content":{"type":"string","description":"Imperative description of the task."},
        "status":{"type":"string","enum":["pending","in_progress","completed"],"description":"Task state. Keep at most one in_progress."},
        "activeForm":{"type":"string","description":"Present-continuous form shown while the task is in progress (e.g. \"Running tests\")."},
        "level":{"type":"integer","minimum":0,"maximum":1,"description":"0 = phase/milestone, 1 = a sub-step of the phase above it. Omit for a flat list."},
        "step_id":{"type":"string","description":"Stable identity for this item, e.g. \"plan_step_02\". Copy it verbatim from the item's previous entry so completions stay attached across retitles, insertions, and reordering; use a fresh unique id for a genuinely new item."}
      },
      "required":["content","status"]
    }
  }
},
"required":["todos"]
}"#;

const SCHEMA_UPDATE_GOAL: &str = r#"{
"type":"object",
"properties":{
  "status":{"type":"string","enum":["continue","complete","blocked"],"description":"continue = keep working autonomously; complete = the goal is fully done and verified; blocked = only the user can unblock."},
  "reason":{"type":"string","description":"Short explanation. REQUIRED for continue and blocked; optional for complete."},
  "next_action":{"type":"string","description":"Optional concrete next step. Recommended for continue so the host can guide the next turn."},
  "completion":{
    "type":"object",
    "description":"Your own account of the finished work.",
    "properties":{
      "verified":{"type":"array","items":{"type":"string"},"description":"Commands you ran as proof, as they actually ran. One that never ran, failed, or predates your latest change is recorded as an unbacked claim."},
      "unverified":{"type":"array","items":{"type":"string"},"description":"What you did NOT verify. The host cannot infer what you skipped, so stating it is the only way it is known — and it never blocks completion."},
      "risks":{"type":"array","items":{"type":"string"},"description":"Known risks to carry forward."}
    }
  }
},
"required":["status"]
}"#;

const SCHEMA_WEB_FETCH: &str = r#"{
"type":"object",
"properties":{
  "url":{"type":"string"}
},
"required":["url"]
}"#;

const SCHEMA_IMPORT_WEB_IMAGE: &str = r#"{"type":"object","additionalProperties":false,"required":["url","image_url","path","alt"],"properties":{"url":{"type":"string"},"image_url":{"type":"string"},"path":{"type":"string"},"alt":{"type":"string"}}}"#;

const SCHEMA_WRITE_FILE: &str = r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#;

/// The 33 built-in tools in §2.1 source order.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "bash",
        description: "Execute a command in the shell and return combined stdout/stderr. Use for builds, tests, git, package managers, etc. To read or modify a file use read_file / edit_file / write_file — never Get-Content, cat, sed, head/tail or a shell redirect. read_file returns up to 2000 lines per call, each prefixed with its line number, and those numbers are what edit_file targets; paging a file through the shell instead costs one round trip per chunk and yields no line anchors. To search, call tool:grep (contents) or tool:glob (names) through use_capability — not grep, rg or find in the shell; the built-ins honor .gitignore and cap their output. A pipeline over another command's output (git log | grep) is the exception.",
        schema: SCHEMA_BASH,
        read_only: false,
        conditional: false,
        notes: "two-variant description: a PowerShell host substitutes the Windows PowerShell / pwsh variant; this row holds the POSIX default.",
    },
    CatalogEntry {
        name: "bash_output",
        description: "Read new output from a background job started with bash(run_in_background=true) or task(run_in_background=true). Returns the output produced since the last bash_output call for that job, plus its status (running/done/failed/killed). Does not block. Omit job_id to list the jobs still running, with their kind, label and elapsed time.",
        schema: SCHEMA_BASH_OUTPUT,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "kill_shell",
        description: "Terminate a running background job (bash or task) started with run_in_background. A no-op if the job has already finished or the id is unknown.",
        schema: SCHEMA_KILL_SHELL,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "wait",
        description: "Block until background jobs finish, then return each job's status and final output/answer. Use to collect the result of a task(run_in_background) or bash(run_in_background) before continuing. Omit job_ids to wait for every running job.",
        schema: SCHEMA_WAIT,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "browser_open",
        description: "Load a page and return its snapshot, one element per line with [eN] refs. url: http(s), about:blank or a workspace file; no url switches to tab; close closes. The user sees tabs: web_fetch reads static text, close tabs when done.",
        schema: SCHEMA_BROWSER_OPEN,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "browser_read",
        description: "Read a browser page: snapshot (default; page with offset/limit, or ref for one subtree), find (lines carrying text, with their refs), screenshot (browser_act x/y use its pixels, and the person is shown it too), logs (errors since you last read them), network (what it requested and what came back; text narrows by URL), tabs.",
        schema: SCHEMA_BROWSER_READ,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "browser_act",
        description: "Operate a page with real input. Ordered steps stop at the first failure; target by ref, or screenshot x/y. Returns results, page errors and changes. fill replaces, type inserts; press takes keys like Enter or Control+a; select sets a <select>; scroll takes delta_y or delta_x; drag ends at to_ref or to_x/to_y; upload gives a file input workspace files; dialog answers prompts; eval executes arbitrary page JavaScript from script and awaits promises; resize sets the viewport to width×height; back, forward and reload move this tab.",
        schema: SCHEMA_BROWSER_ACT,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "render_chart",
        description: "Show the user a chart inline. Rows hold only numbers, strings and nulls; no URLs, markup or colours. Returns a chart_id and a text summary, not the rows.",
        schema: SCHEMA_RENDER_CHART,
        read_only: true,
        conditional: false,
        notes: "Name resolves through chartspec.ToolName (ChartToolName).",
    },
    CatalogEntry {
        name: "code_index",
        description: "Lightweight built-in code symbol index. Prefer lsp_* for language semantics and installed code graph MCP tools for call graph, impact, and architecture relationships; use this as the local fallback for file outlines and symbol definition candidates, then verify with read_file or grep.",
        schema: SCHEMA_CODE_INDEX,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "complete_step",
        description: "Mark ONE step of an approved plan as done. Call it as you finish each step. The host advances the list for you: it marks this step completed and moves the next to in_progress, so a todo_write that marks such an item completed on its own is REJECTED. Sign off one step per tool-call ROUND, not one per call: the sign-off promotes the next item, and a second sign-off sent beside it signs against the list as it stood before.",
        schema: SCHEMA_COMPLETE_STEP,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "compress",
        description: "Compress a selected part of the current model-visible conversation without deleting visible history. Use only when the user explicitly asks for context compression. Choose `before` to summarize everything before the uniquely matched user turn while keeping that turn and later context, or `after` to summarize from that turn through the last completed turn while keeping the active turn. The anchor must be an exact, unique excerpt from a real user message; use a longer excerpt if the tool reports multiple matches.",
        schema: SCHEMA_COMPRESS,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "computer_read",
        description: "See an approved application listed by what=apps. Installed entries without a targetable window provide an exact launch identity for a separate computer_act launch step. Select window by id when it has multiple windows. snapshot returns session-bound element refs in brackets for computer_act. screenshot captures that exact window; computer_act x/y use pixels of this latest displayed image. Old tokens and captures expire on refresh or reconnect.",
        schema: SCHEMA_COMPUTER_READ,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "computer_act",
        description: "Operate an approved application through CUA Driver. launch must be one separate step using an exact installed identity from computer_read what=apps; no window selector or extra arguments. Select window when the app has multiple windows. Use refs from its latest snapshot or coordinates from its latest screenshot. Background clicks, set_value, type/paste, key and scroll never silently escalate. pointer_click explicitly takes foreground control and requires separate approval. wait is bounded. Inspect fresh state after an unverified effect; never assume an action landed. Returns a snapshot afterwards except launch, which returns the verified running identity for a separately approved read.",
        schema: SCHEMA_COMPUTER_ACT,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "context_budget",
        description: "Report how much context room is left before this conversation is automatically compacted. Nothing reports it unprompted: check it at the start of a long task, before a large change, and before output you may not finish reading (a broad search, a large file, a long build log), so you can narrow the command. `tokens_remaining` counts down to the compaction trigger, not the physical window.",
        schema: SCHEMA_CONTEXT_BUDGET,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "delete_range",
        description: "Delete a contiguous text range from a file using exact start/end text anchors. Each anchor must match exactly one line. Returns unified diff on success. Use for large deletions — smaller changes should use edit_file.",
        schema: SCHEMA_DELETE_RANGE,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "delete_symbol",
        description: "Delete a named symbol (function, method, type, interface, const, var) from a Go source file using AST parsing. For non-Go files, use delete_range with manual anchors.",
        schema: SCHEMA_DELETE_SYMBOL,
        read_only: false,
        conditional: false,
        notes: "AST parsing is Go only.",
    },
    CatalogEntry {
        name: "edit_file",
        description: "Replace an exact string in a file with another. old_string must occur exactly once; add surrounding context to disambiguate. Use for targeted edits instead of rewriting the whole file.",
        schema: SCHEMA_EDIT_FILE,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "glob",
        description: "Find files matching a glob pattern (e.g. \"*.go\", \"internal/*/*.go\", \"**/*.test.ts\"). Supports shell metacharacters * ? [] and the recursive ** pattern.",
        schema: SCHEMA_GLOB,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "grep",
        description: "Search for a regular expression in a file, or recursively under a directory (skips hidden files and files matched by .gitignore). Returns matching lines as path:line:text, capped at 200 matches. Ignored paths are searched only when nothing else matched; such matches say so, so an empty answer means absent rather than filtered.",
        schema: SCHEMA_GREP,
        read_only: true,
        conditional: false,
        notes: "two-variant description: the ripgrep-backed arm honors .gitignore outright; this row holds the built-in fallback arm.",
    },
    CatalogEntry {
        name: "ls",
        description: "List the entries of a directory. Directories are shown with a trailing slash; files show their byte size. Set recursive=true to list all nested files depth-first (skips .git/node_modules).",
        schema: SCHEMA_LS,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "move_file",
        description: "Move or rename a file from source_path to destination_path. Creates the destination parent directory as needed. Use instead of shell mv, Move-Item, or ren for file moves so workspace confinement and file-edit permissions apply.",
        schema: SCHEMA_MOVE_FILE,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "multi_edit",
        description: "Apply a list of edits to a single file atomically: each edit runs against the result of the previous one, all in memory; the file is rewritten only if every edit succeeds. Cheaper and safer than chaining edit_file calls — a failure in step 3 leaves the file untouched instead of half-edited.",
        schema: SCHEMA_MULTI_EDIT,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "notebook_edit",
        description: "Edit one cell of a Jupyter notebook (.ipynb). Target a cell by 0-based cell_number (or cell_id). edit_mode: \"replace\" (default) swaps the cell's source; \"insert\" adds a new cell after cell_number (use -1 to prepend at the top), taking cell_type and new_source; \"delete\" removes the cell. cell_type is \"code\" or \"markdown\" (required for insert). Editing a code cell clears its outputs. Prefer this over edit_file for notebooks — it keeps the JSON valid.",
        schema: SCHEMA_NOTEBOOK_EDIT,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "present_files",
        description: "Present existing final files as clickable cards, including shell-created documents. Paths must be in the workspace or authorized folders. Opens current files, not snapshots; does not certify correctness.",
        schema: SCHEMA_PRESENT_FILES,
        read_only: true,
        conditional: true,
        notes: "Conditional registration; name resolves through delivery.ToolName.",
    },
    CatalogEntry {
        name: "read_file",
        description: "Read a text file with optional line offset/limit. Output prefixes each line with its 1-based number (e.g. `   42→...`) so subsequent edit_file calls can target exact lines. Use `offset` and `limit` to page through large files; the tool reports total length and pagination hints in a trailer.",
        schema: SCHEMA_READ_FILE,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "read_video",
        description: "Look at a video: native if the model watches video, else ffmpeg frames.",
        schema: SCHEMA_READ_VIDEO,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "recall",
        description: "Bring folded conversation back: read the #n positions listed under \"Folded work index\" in a compaction summary, or search the whole folded region by query when no index line names what you need. Prefer re-reading a file over recalling it — a current copy beats a folded one. One budget per compaction covers both, and a request past it is refused whole, not truncated.",
        schema: SCHEMA_RECALL,
        read_only: true,
        conditional: false,
        notes: "two schema/description variants: the searchOff arm reads positions only; this row holds the full-search variant.",
    },
    CatalogEntry {
        name: "system_one",
        description: "Evaluate state with the configured System One decision backend (). Ask typed Noul, Choice, or Score questions and receive complete probabilities, confidence, routing metadata, and usage.",
        schema: SCHEMA_SYSTEM_ONE,
        read_only: true,
        conditional: true,
        notes: "Conditional registration; description parenthetical and backend enum are dynamic over configured backends; this row holds the zero-spec snapshot.",
    },
    CatalogEntry {
        name: "todo_write",
        description: "Record and update a structured task list for the current work. Send the COMPLETE list every call — it replaces the previous one. Use it to plan multi-step work and show progress: keep exactly one item in_progress at a time, and flip an item to completed the moment it's done (don't batch completions). Skip it for trivial single-step tasks. The list is two-level: a `level` 0 item is a PHASE (a milestone) and the `level` 1 items after it are its concrete sub-steps. COPY `step_id` VERBATIM for every item that already has one: it is how a completion stays attached to its step when you retitle it, insert a step above it, or reorder the list. Give a new item a fresh unique id (e.g. \"plan_step_07\"); never reuse or renumber an existing one.",
        schema: SCHEMA_TODO_WRITE,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "update_goal",
        description: "Report this turn's disposition for the active goal. Call it at the end of every goal turn instead of using prose markers: `continue` (work is ongoing — give a concrete next_action), `complete` (the request is fully done, output format and constraints satisfied, and verification was attempted or reported unavailable), or `blocked` (only the user can unblock: missing user-only information, an irreversible/externally visible operation, or changed scope). The host validates your claim against Delivery acceptance criteria and decides whether to continue automatically.",
        schema: SCHEMA_UPDATE_GOAL,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "web_fetch",
        description: "Read HTTP(S) text. Reduces HTML to text and image candidates for import_web_image; returns JSON/plain text verbatim.",
        schema: SCHEMA_WEB_FETCH,
        read_only: true,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "import_web_image",
        description: "Import web_fetch image_url from source url into new path. Validates PNG/JPEG/GIF/WebP. Returns image {path,alt,source} for render_surface. Match extension.",
        schema: SCHEMA_IMPORT_WEB_IMAGE,
        read_only: false,
        conditional: false,
        notes: "",
    },
    CatalogEntry {
        name: "write_file",
        description: "Write full file content, overwriting and creating parent directories. Prefer edit_file/multi_edit for partial changes.",
        schema: SCHEMA_WRITE_FILE,
        read_only: false,
        conditional: false,
        notes: "",
    },
];

/// The typed catalog validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// The table does not carry the frozen 33 built-ins.
    #[error("catalog has {count} entries, want 33")]
    CountMismatch { count: usize },
    /// A tool name appears more than once (Go boots on `"tool: duplicate built-in <name>"`).
    #[error("duplicate tool name {0:?}")]
    DuplicateName(&'static str),
    /// The model-visible description is blank.
    #[error("tool {0:?} has an empty description")]
    EmptyDescription(&'static str),
    /// The schema does not parse as a JSON object.
    #[error("tool {0:?} schema is not a JSON object: {1}")]
    InvalidSchema(&'static str, String),
    /// The name is not a lowercase `[a-z0-9_]` identifier.
    #[error("tool name {0:?} is not a lowercase identifier")]
    InvalidName(&'static str),
}

/// The frozen catalog rows.
pub fn entries() -> &'static [CatalogEntry] {
    CATALOG
}

/// Looks up a row by tool name, case-sensitive.
pub fn by_name(name: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|entry| entry.name == name)
}

/// Parses the row's schema once, for contract reads and schema assertions.
pub fn schema_value(entry: &CatalogEntry) -> Value {
    serde_json::from_str(entry.schema).expect("catalog schema parses; validate() checks it")
}

/// Validates the frozen table: 33 unique names, non-empty descriptions, every
/// schema a JSON object, and every name a lowercase identifier.
pub fn validate() -> Result<(), CatalogError> {
    let count = CATALOG.len();
    if count != 33 {
        return Err(CatalogError::CountMismatch { count });
    }
    for entry in CATALOG {
        if !is_lower_identifier(entry.name) {
            return Err(CatalogError::InvalidName(entry.name));
        }
        if entry.description.trim().is_empty() {
            return Err(CatalogError::EmptyDescription(entry.name));
        }
        match serde_json::from_str::<Value>(entry.schema) {
            Ok(Value::Object(_)) => {}
            Ok(other) => {
                return Err(CatalogError::InvalidSchema(
                    entry.name,
                    format!("schema root is {other}"),
                ));
            }
            Err(err) => return Err(CatalogError::InvalidSchema(entry.name, err.to_string())),
        }
    }
    for (i, entry) in CATALOG.iter().enumerate() {
        if CATALOG[..i].iter().any(|other| other.name == entry.name) {
            return Err(CatalogError::DuplicateName(entry.name));
        }
    }
    Ok(())
}

fn is_lower_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_table_passes_contract_validation() {
        validate().expect("the frozen catalog is internally consistent");
    }

    #[test]
    fn catalog_holds_exactly_the_thirty_three_builtins() {
        assert_eq!(CATALOG.len(), 33);
        let names: Vec<&str> = CATALOG.iter().map(|e| e.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len(), "names are unique");
    }

    #[test]
    fn read_only_classification_matches_the_go_table() {
        let expected_read_only: &[(&str, bool)] = &[
            ("bash", false),
            ("bash_output", true),
            ("kill_shell", false),
            ("wait", true),
            ("browser_open", false),
            ("browser_read", true),
            ("browser_act", false),
            ("render_chart", true),
            ("code_index", true),
            ("complete_step", true),
            ("compress", true),
            ("computer_read", false),
            ("computer_act", false),
            ("context_budget", true),
            ("delete_range", false),
            ("delete_symbol", false),
            ("edit_file", false),
            ("glob", true),
            ("grep", true),
            ("ls", true),
            ("move_file", false),
            ("multi_edit", false),
            ("notebook_edit", false),
            ("present_files", true),
            ("read_file", true),
            ("read_video", true),
            ("recall", true),
            ("system_one", true),
            ("todo_write", true),
            ("update_goal", true),
            ("web_fetch", true),
            ("import_web_image", false),
            ("write_file", false),
        ];
        assert_eq!(expected_read_only.len(), 33);
        for (name, want) in expected_read_only {
            let entry = by_name(name).expect("catalog row exists");
            assert_eq!(entry.read_only, *want, "{name}.ReadOnly()");
        }
    }
}
