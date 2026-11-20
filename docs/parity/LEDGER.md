# Parity Ledger

One row per acceptance-testable behavior (AGENTS.md 5.2). Statuses: todo,
wip, done, blocked, intentional-break. done requires evidence (a test name
or a reproducing command); blocked requires a reason; intentional-break
requires an ADR link. The table below is validated by
cargo xtask parity --check. Do not add other tables to this file: every
pipe-leading line must be a six-cell ledger row.

| ID | Area | Reference | Rust | Status | Evidence |
|---|---|---|---|---|---|
| INFRA-001 | Infra | Go kernel builds via go build | Cargo workspace | done | cargo build --workspace --release exit 0 (2026-11-13) |
| INFRA-002 | Infra | Six-target release pipeline | CI build matrix | wip | .github/workflows/ci.yml authored; first CI run pending |
| INFRA-003 | Infra | gofmt clean tree | rustfmt gate | done | cargo fmt --all --check exit 0 (2026-11-13) |
| INFRA-004 | Infra | go vet and staticcheck | clippy gate | done | cargo clippy --workspace --all-targets --all-features -- -D warnings exit 0 (2026-11-13) |
| INFRA-005 | Infra | go test on the module | workspace tests | done | cargo test --workspace --all-features: 45 passed, 0 failed (2026-11-13) |
| INFRA-006 | Infra | Parity ledger discipline | xtask parity | done | cargo xtask parity --check exit 0 (2026-11-13) |
| INFRA-007 | Infra | Dependency direction (AGENTS.md 4.2) | xtask deps | done | cargo xtask deps --check exit 0 (2026-11-13) |
| INFRA-008 | Infra | Dependency and license policy | cargo-deny gate | done | cargo deny check exit 0 (2026-11-13) |
| INFRA-009 | Infra | Crate layout rationale | ADR-0001 | done | docs/adr/0001-crate-layout.md accepted (2026-11-13) |
| INFRA-010 | Infra | Atomic write/replace family (internal/base/fileutil/atomicwrite.go) | reasonix-core | done | crates/core/src/fileutil.rs (unit tests) (2026-11-16) |
| INFRA-011 | Infra | Cross-process file lock (internal/base/filelock) | reasonix-core | done | crates/core/src/filelock.rs (unit tests: acquire/try-acquire round trip, waiter timeout, waiter wake, canonical path) (2026-11-16) |
| INFRA-012 | Infra | Text encoding detection and decode (internal/base/fileutil/encoding) | reasonix-core | done | crates/core/src/fileencoding.rs + crates/core/tests/fileencoding_golden.rs (43-case Go oracle corpus, byte-exact) (2026-11-16) |
| CLI-001 | CLI | reasonix --version | apps/reasonix | done | apps/reasonix/tests/cli.rs::version_flag_prints_single_line (2026-11-13) |
| CLI-002 | CLI | reasonix -v | apps/reasonix | done | apps/reasonix/tests/cli.rs::short_version_flag_matches (2026-11-13) |
| CLI-003 | CLI | reasonix version | apps/reasonix | done | apps/reasonix/tests/cli.rs::version_subcommand_prints_single_line (2026-11-13) |
| CLI-004 | CLI | reasonix version --verbose | reasonix-cli | todo | M1: verbose body needs usage() |
| CLI-005 | CLI | reasonix version --json | reasonix-cli | todo | M1: go runtime field decision for BREAKS.md |
| CLI-006 | CLI | reasonix version --help | reasonix-cli | todo | M1 |
| CLI-007 | CLI | reasonix help and --help and -h | reasonix-cli | todo | M1: usage() body |
| CLI-008 | CLI | Bare invocation outside TTY | reasonix-cli | todo | M1: depends on usage() |
| CLI-009 | CLI | Full command and flag matrix | reasonix-cli | todo | M1: see docs/contracts/cli.md; details in CLI-010..CLI-050 |
| CLI-010 | CLI | Bare invocation TTY rules + bareUsage exits (bare 0, session flags 2) + mintty hint | reasonix-cli | todo | M1: docs/contracts/cli.md §2 |
| CLI-011 | CLI | --acp shortcut routes to acp command | reasonix-cli | todo | M1: docs/contracts/cli.md §2 |
| CLI-012 | CLI | Leading session flags move into run when TUI+run parsers agree | reasonix-cli | todo | M1: docs/contracts/cli.md §2.1 |
| CLI-013 | CLI | Leading -p/--print routes to run --print | reasonix-cli | todo | M1: docs/contracts/cli.md §2 |
| CLI-014 | CLI | First-token interactive flag means bare TUI (isDefaultInteractiveFlag set) | reasonix-cli | todo | M1: docs/contracts/cli.md §2 |
| CLI-015 | CLI | Unknown command: stderr message + usage + exit 2 | reasonix-cli | todo | M1: docs/contracts/cli.md §3 |
| CLI-016 | CLI | Hidden schedule verbs with human-confirmation exits (77) | reasonix-cli | todo | M1: docs/contracts/cli.md §7.25 |
| CLI-017 | CLI | Hidden sandbox list/revoke | reasonix-cli | todo | M1: docs/contracts/cli.md §7.24 |
| CLI-018 | CLI | tui/chat/code aliases; TUI flag set incl. --inline and optional --resume; no --auto | reasonix-cli | todo | M1: docs/contracts/cli.md §7.2 |
| CLI-019 | CLI | run flag set and exit-code map | reasonix-cli | todo | M1: docs/contracts/cli.md §7.1 |
| CLI-020 | CLI | Permission-mode parsing matrix and error text | reasonix-cli | todo | M1: docs/contracts/cli.md §4.1 |
| CLI-021 | CLI | Preset/profile parsing matrix and error text | reasonix-cli | todo | M1: docs/contracts/cli.md §4.2 |
| CLI-022 | CLI | Approval flag conflicts (--auto/--yolo/--permission-mode) | reasonix-cli | todo | M1: docs/contracts/cli.md §4.3 |
| CLI-023 | CLI | Session query resolution: exact/partial/machine-id/ambiguous | reasonix-cli | todo | M1: docs/contracts/cli.md §4.4 |
| CLI-024 | CLI | --copy semantics and copy notice stream | reasonix-cli | todo | M1: docs/contracts/cli.md §4.4 |
| CLI-025 | CLI | serve/web flag set, auth defaults, broker flags | reasonix-cli | todo | M1: docs/contracts/cli.md §7.6 |
| CLI-026 | CLI | acp flag set and PostureCeiling | reasonix-cli | todo | M1: docs/contracts/cli.md §7.5 |
| CLI-027 | CLI | mcp verb matrix, usage text, browse limits | reasonix-cli | todo | M1: docs/contracts/cli.md §7.8 |
| CLI-028 | CLI | plugin verb matrix | reasonix-cli | todo | M1: docs/contracts/cli.md §7.7 |
| CLI-029 | CLI | remote verb matrix and agent-child refusal (77) | reasonix-cli | todo | M1: docs/contracts/cli.md §7.10 |
| CLI-030 | CLI | login/whoami/logout device flow; exit 130 on cancel | reasonix-cli | todo | M1: docs/contracts/cli.md §7.11 |
| CLI-031 | CLI | subagent verb matrix and usage | reasonix-cli | todo | M1: docs/contracts/cli.md §7.12 |
| CLI-032 | CLI | task verb matrix and machine list/show redaction | reasonix-cli | todo | M1: docs/contracts/cli.md §7.20 |
| CLI-033 | CLI | session machine interface and error codes | reasonix-cli | todo | M1: docs/contracts/cli.md §7.17 |
| CLI-034 | CLI | hook machine interface | reasonix-cli | todo | M1: docs/contracts/cli.md §7.18 |
| CLI-035 | CLI | sessions 1.x maintenance verbs (reindex/diagnose/cleanup) | reasonix-cli | todo | M1: docs/contracts/cli.md §7.17 |
| CLI-036 | CLI | catalogs reindex dispatch | reasonix-cli | todo | M1: docs/contracts/cli.md §7.19 |
| CLI-037 | CLI | trust full behavior incl. --allow-providers and --revoke | reasonix-cli | todo | M1: docs/contracts/cli.md §7.13 |
| CLI-038 | CLI | doctor verb matrix | reasonix-cli | todo | M1: docs/contracts/cli.md §7.15 |
| CLI-039 | CLI | config verb matrix (auto-plan/reasoning-language/compact-ratio/currency/telemetry) | reasonix-cli | todo | M1: docs/contracts/cli.md §7.9 |
| CLI-040 | CLI | report verb matrix | reasonix-cli | todo | M1: docs/contracts/cli.md §7.16 |
| CLI-041 | CLI | completion bash/zsh/fish and hidden __complete | reasonix-cli | todo | M1: docs/contracts/cli.md §7.22 |
| CLI-042 | CLI | docs-manifest flags and manifest output | reasonix-cli | todo | M1: docs/contracts/cli.md §7.23 |
| CLI-043 | CLI | upgrade/update syntax and channel compatibility | reasonix-cli | todo | M1: docs/contracts/cli.md §7.3 |
| CLI-044 | CLI | review flags; empty diff exits 0 | reasonix-cli | todo | M1: docs/contracts/cli.md §7.4 |
| CLI-045 | CLI | setup targets and overwrite refusal | reasonix-cli | todo | M1: docs/contracts/cli.md §7.14 |
| CLI-046 | CLI | init hint, exit 0 | reasonix-cli | todo | M1: docs/contracts/cli.md §3 |
| CLI-047 | CLI | Top-level usage text byte parity (en) | reasonix-cli | todo | M1: docs/contracts/cli.md §8.1 |
| CLI-048 | CLI | Exit-code matrix incl. 4, 77, 130 | reasonix-cli | todo | M1: docs/contracts/cli.md §5 |
| CLI-049 | CLI | REASONIX_DNS and env handling at process start | reasonix-cli | todo | M1: docs/contracts/cli.md §6 |
| CLI-050 | CLI | Legacy config migration trigger set | reasonix-cli | todo | M1: docs/contracts/cli.md §2.2 |
| CONFIG-001 | Config | Layered resolution: flag > project reasonix.toml > user config.toml > defaults | reasonix-config | wip | user/project/defaults layered in crates/config/src/load.rs (layered precedence + userGlobals tests); the flag layer belongs to the web command, not ported (2026-11-20) |
| CONFIG-002 | Config | Top-level TOML key inventory and unknown-field passthrough | reasonix-config | wip | unknown-key passthrough exact (unknown_keys_pass_through_silently); key inventory subset serve/desktop/language/paths (2026-11-20) |
| CONFIG-003 | Config | Built-in defaults Default(), incl. DefaultSystemPrompt and agent defaults | reasonix-config | wip | defaults for the boot subset (serve/desktop/paths/language) tested; system-prompt and agent defaults outstanding (2026-11-20) |
| CONFIG-004 | Config | Normalization matrices: theme, language, currency, modes, telemetry | reasonix-config | wip | language + telemetry/metrics/tray matrices done (desktop.rs, language.rs tests); theme/currency/modes outstanding (2026-11-20) |
| CONFIG-005 | Config | ResolveModel 3 forms, Validate error texts, atomic WriteFile | reasonix-config | todo | M1: docs/contracts/config.md §5 |
| CONFIG-006 | Config | Load pipeline: 18-step loadForRoot, LKG recovery, bad TOML never fails load | reasonix-config | todo | M1: docs/contracts/config.md §9 |
| CONFIG-007 | Config | Provider/plugin merge by name + BurntSushi positional decode isolation | reasonix-config | todo | M1: docs/contracts/config.md §9 |
| CONFIG-008 | Config | Project config only narrows; userGlobals never set by a workspace | reasonix-config | wip | userGlobals for [serve]/[storage] done (load.rs tests); full §10 narrow-only matrix outstanding (2026-11-20) |
| CONFIG-009 | Config | Project programs: project-programs.json approval gates + digest matching | reasonix-config | todo | M1: docs/contracts/config.md §10.17 |
| CONFIG-010 | Config | Workspace grants and workspace trust states | reasonix-config | todo | M1: docs/contracts/config.md §10.18-§10.19 |
| CONFIG-011 | Config | Claude ~/.claude.json import incl. CLAUDE_CONFIG_DIR | reasonix-config | todo | M1: docs/contracts/config.md §10.20 |
| CONFIG-012 | Config | .mcp.json load/merge/upsert/remove preserving unknown fields | reasonix-config | todo | M1: docs/contracts/config.md §10.21 |
| CONFIG-013 | Config | Plugin command-line normalization (runner list, path preservation) | reasonix-config | todo | M1: docs/contracts/config.md §10.22 |
| CONFIG-014 | Config | Unparsed-file identity + parse-error repair (256-round escape doubling) | reasonix-config | todo | M1: docs/contracts/config.md §10.25 |
| CONFIG-015 | Config | Retired-key stripping: 5 key sets, boot-time, locked | reasonix-config | todo | M1: docs/contracts/config.md §10.26 |
| CONFIG-016 | Config | Legacy v0.x JSON and v1-era TOML import + support-data moves | reasonix-config | todo | M1: docs/contracts/config.md §10.27 |
| CONFIG-017 | Config | Config version upgrades 3..6, freshConfigVersion 12, startup upgrades | reasonix-config | todo | M1: docs/contracts/config.md §10.28 |
| CONFIG-018 | Config | ExpandVars: process env only, workspace-root overrides | reasonix-config | todo | M1: docs/contracts/config.md §10.29 |
| CONFIG-019 | Config | dotenv: project .env never enters process env, control-key filter | reasonix-config | todo | M1: docs/contracts/config.md §10.30 |
| CONFIG-020 | Config | Credential store: env/dotenv/keyring resolution, tombstones, revision | reasonix-config | todo | M1: docs/contracts/config.md §10.31 |
| CONFIG-021 | Config | API key env naming: custom provider slots, free slots | reasonix-config | todo | M1: docs/contracts/config.md §10.32 |
| CONFIG-022 | Config | Paths + storage roots: home/state/cache/locks, env pins, relocatability | reasonix-config | wip | home/state root chain + ${VAR} expansion + Clean done (paths.rs tests); cache/locks/relocatability outstanding (2026-11-20) |
| CONFIG-023 | Config | Line-level TOML document index semantics | reasonix-config | todo | M1: docs/contracts/config.md §12 |
| CONFIG-024 | Config | Effort/reasoning-protocol ladders and NormalizeEffort errors | reasonix-config | todo | M1: docs/contracts/config.md §13 |
| CONFIG-025 | Config | RenderTOMLForScope scope matrix + RenderTOMLProjectDelta | reasonix-config | todo | M1: docs/contracts/config.md §14 |
| CONFIG-026 | Config | Mutator surface: Set*/Upsert*/Remove* with exact error texts | reasonix-config | todo | M1: docs/contracts/config.md §15.1-§15.2 |
| CONFIG-027 | Config | MCP source routing and install/remove across sources | reasonix-config | todo | M1: docs/contracts/config.md §15.1 |
| CONFIG-028 | Config | TOML text-surgery primitives: section replace/upsert/remove | reasonix-config | todo | M1: docs/contracts/config.md §15.4 |
| CONFIG-029 | Config | Save paths: SaveTo/SaveForRoot, scope resolution, symlink policy | reasonix-config | todo | M1: docs/contracts/config.md §15.3 |
| CONFIG-030 | Config | User config save-by-patch: patch-then-verify, backups, NaN skip | reasonix-config | todo | M1: docs/contracts/config.md §15.5 |
| CONFIG-031 | Config | Edit locking: per-file advisory locks, registry, owner checks | reasonix-config | todo | M1: docs/contracts/config.md §15.6 |
| CONFIG-032 | Config | Desktop posture release marker | reasonix-config | todo | M1: docs/contracts/config.md §15.7 |
| CONFIG-033 | Config | Sub-struct schemas §8.1-§8.24 (UI, CLI, Desktop, Billing, Persona, Memory, LSP, Browser, Attachments, Environment, Peers, Remote, Observability, Mods, ProgressWatch, AutoArchive, Plugin, Shell, SystemOne, BackgroundJobs, WebAccess, NetworkDNS, AskTimeout) | reasonix-config | todo | M1: docs/contracts/config.md §8 |
| CONFIG-034 | Config | Example TOML documented schema parity | reasonix-config | todo | M1: docs/contracts/config.md §6 |
| CONFIG-035 | Config | Open divergences to verify at implementation slices: compact_ratio default 0.90 vs example 0.85; serve auth_mode comment vs token default | reasonix-config | wip | auth_mode divergence resolved to the Go source (config layer default empty, the gate normalizes; test notes carry the §7.2 record); compact_ratio outstanding (2026-11-20) |
| ROUTE-001 | HTTP | Route matrix: 420 registrations (330 session, 74 desktop, 16 hostMux), methods, paths | reasonix-serve | wip | hub-level subset registered with per-route 405/404 envelope tests (router.rs); per-runtime families land with the runtime slice ({D}) |
| ROUTE-002 | HTTP | Server/Hub/Runtime topology and middleware order | reasonix-serve | wip | middleware order frozen and tested (log→auth→hostGuard→withPage→csrf→mutation→watchTree, gate outermost, panic catch); HubOptions conditional families land with their slices (2026-11-20) |
| ROUTE-003 | HTTP | /rt/{id} prefix, routeRuntime 404, routeDefault fallback + 503 hub.no_runtime_open | reasonix-serve | done | router tests + smoke: unknown /rt/{id} 404 request.not_found {kind:runtime}, no-runtime paths 503 hub.no_runtime_open (2026-11-20) |
| ROUTE-004 | HTTP | hostOnly split: devices reach mux, never hostMux (404 device.host_only) | reasonix-serve | todo | M1: docs/contracts/http.md §0, §2.8 |
| ROUTE-005 | HTTP | watchTreeWrites tree-change trigger set | reasonix-serve | done | tree_write() port of hub_sync.go:71-90 + 2xx trigger tests (router.rs::2026-11-20) |
| ROUTE-006 | HTTP | withPage file-first serving + fixed content-type map | reasonix-serve | done | page.rs file-first + content-type table tests; 304/If-Modified-Since outstanding (2026-11-20) |
| ROUTE-007 | HTTP | InProcessClient + shell.unavailable_over_http over sockets | reasonix-serve | todo | M1: docs/contracts/http.md §0, §7 |
| ROUTE-008 | HTTP | Auth modes none/token/password/invalid + plain-text deny 401 | reasonix-serve | wip | none/token/invalid done (auth.rs tests, deny 401 text); password constructs deny-all until its machine lands (2026-11-20) |
| ROUTE-009 | HTTP | Token mode: cookie/bearer/query credential + redirect cleanup | reasonix-serve | done | auth.rs cookie/bearer/query→302+cookie with safe_redirect_target tests (2026-11-20) |
| ROUTE-010 | HTTP | POST /auth/token bootstrap (constant-time, 204, cookie 30d) | reasonix-serve | done | auth.rs bootstrap tests (≤8KiB, one JSON value, constant-time, cookie) (2026-11-20) |
| ROUTE-011 | HTTP | Password mode: login form, bcrypt 12, HMAC session 30d, rate limit 429 | reasonix-serve | todo | M1: docs/contracts/http.md §2.3 |
| ROUTE-012 | HTTP | hostGuard DNS-rebound allowlist (421 serve.host_rejected) | reasonix-serve | wip | loopback-shape hostGuard done (tests); wildcard-bind interface expansion fails closed pending interface enumeration (2026-11-20) |
| ROUTE-013 | HTTP | mutationGate launch token (403 auth.launch_token_required) | reasonix-serve | done | auth.rs mutationGate tests; host-gated mark rides the loopback gate (2026-11-20) |
| ROUTE-014 | HTTP | Loopback gate: tcp4 127.0.0.1:0, host/origin/token checks | reasonix-serve | done | gate.rs tests + golden fixtures byte-exact vs recorded Go host + release smoke (403 codes ×3, GET no-origin passes) (2026-11-20) |
| ROUTE-015 | HTTP | Device gate: /pair, self routes, public paths, 401 device.unauthorized | reasonix-serve | todo | M1: docs/contracts/http.md §2.7 |
| ROUTE-016 | HTTP | hostOnlyRoutes 27-route list + triple enforcement | reasonix-serve | todo | M1: docs/contracts/http.md §2.8 |
| ROUTE-017 | HTTP | csrfGuard content types + binary upload exceptions (415) | reasonix-serve | done | auth.rs csrf tests (json-only POST, octet-stream exceptions) (2026-11-20) |
| ROUTE-018 | HTTP | No CORS / same-origin policy | reasonix-serve | done | no CORS surface emitted; same-origin only (router contract tests + golden replay) (2026-11-20) |
| ROUTE-019 | HTTP | OpenBindRefusal + PlainHTTPAuthWarning | reasonix-serve | todo | M1: docs/contracts/http.md §2.10 |
| ROUTE-020 | HTTP | Binding: Run/RunGraceful timeouts, 10s drain | reasonix-serve | done | listen.rs tests (shutdown ok, in-flight completes within drain, header 10s, idle 120s watchdog) (2026-11-20) |
| ROUTE-021 | HTTP | FindPage resolution order | reasonix-serve | done | page.rs tests (explicit dir requires index.html; exe-side; ../desktop-tauri/dist — BREAK-002 records the contract-doc path correction) (2026-11-20) |
| ROUTE-022 | HTTP | Reason envelope {code, error, params} | reasonix-serve | done | refuse.rs Go field order + params key ordering tests (2026-11-20) |
| ROUTE-023 | HTTP | writeErr mapping: coded/memory/config/answers | reasonix-serve | wip | coded/config.unparsed/answers branches + http.Error fallback tested; memory read-error types arrive with the memory machine (2026-11-20) |
| ROUTE-024 | HTTP | Shared refusal codes + decodeBody 64KiB/DisallowUnknownFields | reasonix-serve | done | refuse_common.go 19 codes as constants; decode_body stream-level truncation + unknown-fields tests (2026-11-20) |
| ROUTE-025 | HTTP | Request logging levels (routine -> Debug) | reasonix-serve | wip | one INFO line per request in the log middleware; routine/Debug split outstanding (2026-11-20) |
| ROUTE-026 | HTTP | Wire log kinds, 8MiB cap, witness, /trajectory availability | reasonix-serve | todo | M1: docs/contracts/http.md §4.5 |
| ROUTE-027 | HTTP | SSE framing: id/data/keepalive/watermark | reasonix-serve | done | eventstream tests (id only on numbered frames, 15s keepalive ': ping'+watermark real frame, ': connected' first-byte flush; golden SSE frames byte-exact) (2026-11-20) |
| ROUTE-028 | HTTP | Last-Event-ID resume + 4 subscription modes | reasonix-serve | done | broadcaster tests ported from broadcaster_test.go/eventstream_test.go (header+query cursor, From/InitialNotices/Attached/Resume) (2026-11-20) |
| ROUTE-029 | HTTP | /events/replay shape + events mux + metrics | reasonix-serve | wip | /events/replay {frames,complete,watermark} done (null→[] tests); events mux family lands with its wave (2026-11-20) |
| ROUTE-030 | HTTP | GET /status shape (fixed + conditional keys) | reasonix-serve | todo | M1: docs/contracts/http.md §7 |
| ROUTE-031 | HTTP | POST /submit contract (202, busy 409, format, shell refusal 403) | reasonix-serve | todo | M1: docs/contracts/http.md §7 |
| ROUTE-032 | HTTP | /sessions, /todos, /balance, /context, /new, /cancel, /approve shapes | reasonix-serve | wip | /sessions implemented per-runtime (routes.go:91 slot; hub-level falls to routeDefault 503 like Go — review M-1 fixed); /todos//balance//context//new//cancel//approve need the write-side slices (2026-11-20) |
| ROUTE-033 | HTTP | vite ROUTES cross-check + stale prefixes | reasonix-serve | todo | M1: docs/contracts/http.md §6 |
| SSE-001 | SSE | 36-kind taxonomy: names, order, payload fields, numbered/droppable | reasonix-core | wip | kind names/order from contract::eventwire_manifest.json (golden parity test); numbered/droppable in serve broadcaster; payload-field completeness tracks eventwire coverage (2026-11-20) |
| SSE-002 | SSE | Transport-synthetic kinds stream_watermark / stream_gap | reasonix-serve | done | broadcaster tests (gap frame seq = first trustworthy, attached watermark unnumbered) (2026-11-20) |
| SSE-003 | SSE | Fixed vocabularies: phases, attempts, retry scopes, levels, audiences, issuers, sub-agent channels | reasonix-core | todo | M1: docs/contracts/sse.md §1 |
| SSE-004 | SSE | eventwire.Event envelope: field set, omitempty, externalizable marks | reasonix-core | done | crates/contract/src/eventwire/event.rs held by the manifest golden parity suite (2026-11-20) |
| SSE-005 | SSE | droppable() set + seq assignment + never-blocking Emit | reasonix-serve | done | broadcaster tests ported from Go (stalled subscriber never blocks emit; droppable shed first; 4MiB budget) (2026-11-20) |
| SSE-006 | SSE | Transport-scoped seq: survives /new; ResetSession drops replay tail | reasonix-serve | done | broadcaster tests (reset keeps counting, drops tail) (2026-11-20) |
| SSE-007 | SSE | Replay log, gap frame, watermark, Replay(after) | reasonix-serve | done | broadcaster tests (8MiB budget eviction, since() completeness, watermark) (2026-11-20) |
| SSE-008 | SSE | Five subscription modes incl. initial-notices unnumbered replay | reasonix-serve | done | broadcaster tests (From/InitialNotices once-per-session unnumbered/Attached watermark-first/Resume(0) owes the log) (2026-11-20) |
| SSE-009 | SSE | Coalesce: 16ms window, 16KiB cap, first-delta immediate, order preserved | reasonix-core | todo | M1: docs/contracts/sse.md §5 |
| SSE-010 | SSE | Sink capability registry + AuditForwarder completeness | reasonix-core | todo | M1: docs/contracts/sse.md §5 |
| SSE-011 | SSE | CostQuoteSink quoting rules + no_price fallback | reasonix-core | todo | M1: docs/contracts/sse.md §5 |
| SSE-012 | SSE | NoticeLedger dedup across rebuilds | reasonix-core | todo | M1: docs/contracts/sse.md §5 |
| SSE-013 | SSE | MarkEmbeddedDiffs tagging rules | reasonix-core | todo | M1: docs/contracts/sse.md §5 |
| SSE-014 | SSE | Notice code registry (~100 codes) + localization fallback | reasonix-core | todo | M1: docs/contracts/sse.md §6 |
| SSE-015 | SSE | Notice Detail payload schemas + DetailIsPayload set + decode guards | reasonix-core | todo | M1: docs/contracts/sse.md §7 |
| SSE-016 | SSE | Tool wire struct incl. Bound, truncated compat, Profile, Execution | reasonix-core | todo | M1: docs/contracts/sse.md §8 |
| SSE-017 | SSE | Usage wire struct + usage source constants + CostQuote aliases | reasonix-core | todo | M1: docs/contracts/sse.md §8 |
| SSE-018 | SSE | Approval + Ask payloads (reasons, origin kinds, surfaces) | reasonix-core | todo | M1: docs/contracts/sse.md §8 |
| SSE-019 | SSE | Extension surface payload (6 kinds, one sub-struct set) | reasonix-core | todo | M1: docs/contracts/sse.md §8 |
| SSE-020 | SSE | CompletionSummary + CompletionReceipt + severities | reasonix-core | todo | M1: docs/contracts/sse.md §8 |
| SSE-021 | SSE | Compaction + ContextMaintenance + TodoProgress + ProgressWatch + WorkspaceLease + Guardian + WorkspaceChanged + DecisionReceipt | reasonix-core | todo | M1: docs/contracts/sse.md §8 |
| SSE-022 | SSE | Divergences: snake_case guardian, recovery_paused legacy, order guarantee | reasonix-core | todo | M1: docs/contracts/sse.md §9 |
| ACP-001 | ACP | NDJSON framing + 32MiB line cap (server.go) | reasonix-acp | todo | M1: docs/contracts/acp.md §0 |
| ACP-002 | ACP | Error code table + cancelled rewrite (server.go, protocol.go) | reasonix-acp | todo | M1: docs/contracts/acp.md §1 |
| ACP-003 | ACP | initialize capabilities JSON + _meta capability schema | reasonix-acp | todo | M1: docs/contracts/acp.md §2 |
| ACP-004 | ACP | authenticate (reasonix-setup only, else -32602) | reasonix-acp | todo | M1: docs/contracts/acp.md §2 |
| ACP-005 | ACP | Inbound wire types round-trip (SessionNew..SessionDelete params/results) | reasonix-acp | todo | M1: docs/contracts/acp.md §3 |
| ACP-006 | ACP | fs/read_text_file + fs/write_text_file degradation asymmetry | reasonix-acp | todo | M1: docs/contracts/acp.md §4 |
| ACP-007 | ACP | terminal create/wait_for_exit/output/release + 1MiB cap | reasonix-acp | todo | M1: docs/contracts/acp.md §4 |
| ACP-008 | ACP | session/update variants (messageChunk..currentModeUpdate) | reasonix-acp | todo | M1: docs/contracts/acp.md §5 |
| ACP-009 | ACP | Permission request round-trip + option kinds | reasonix-acp | todo | M1: docs/contracts/acp.md §5 |
| ACP-010 | ACP | Session lifecycle, lease codes, controls, permission ceiling | reasonix-acp | todo | M1: docs/contracts/acp.md §6 |
| ACP-011 | ACP | Vendor extensions opt-in (steer/inbox/goal/status/…) | reasonix-acp | todo | M1: docs/contracts/acp.md §7 |
| ACP-012 | ACP | Event→update dispatch mapping + flightSet cancel | reasonix-acp | todo | M1: docs/contracts/acp.md §8 |
| ACP-013 | ACP | Tool call/update wire shapes + message ids | reasonix-acp | todo | M1: docs/contracts/acp.md §8 |
| ACP-014 | ACP | Permission gating, argv fail-closed, toolResultText clip | reasonix-acp | todo | M1: docs/contracts/acp.md §8 |
| ACP-015 | ACP | session/status method + ReasonixSessionStatus v1 | reasonix-acp | todo | M1: docs/contracts/acp.md §9 |
| ACP-016 | ACP | Usage accounting + finishTurn usage event | reasonix-acp | todo | M1: docs/contracts/acp.md §9 |
| ACP-017 | ACP | session/info + session/mcpServers | reasonix-acp | todo | M1: docs/contracts/acp.md §10.1 |
| ACP-018 | ACP | MCP control reconnect/disconnect/submitForm states | reasonix-acp | todo | M1: docs/contracts/acp.md §10.2 |
| ACP-019 | ACP | goal/resume + goal/pause semantics | reasonix-acp | todo | M1: docs/contracts/acp.md §10.3 |
| ACP-020 | ACP | .acp.json sidecar schema + write rules | reasonix-acp | todo | M1: docs/contracts/acp.md §10.4 |
| ACP-021 | ACP | elicitation/create + form/url limits + capability gating | reasonix-acp | todo | M1: docs/contracts/acp.md §11 |
| ACP-022 | ACP | Serve registration table (27 Handle + session/cancel notify) | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-023 | ACP | acpSession state machine: begin/finish/maintenance/retired leases | reasonix-acp | todo | M1: docs/contracts/acp.md §10.6 |
| ACP-024 | ACP | session/new full flow + lease + afterResponse | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-025 | ACP | session/load + session/resume replay semantics | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-026 | ACP | session/prompt turn flow + inbox drain + stop mapping | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-027 | ACP | session/steer durable + fallback paths | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-028 | ACP | session/cancel silent notification | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-029 | ACP | session/close idempotent teardown + closeAll | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-030 | ACP | session/list merge + sort + cursor rejection | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-031 | ACP | session/delete + cleanup-pending + twin deletion | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-032 | ACP | session/set_mode incl. legacy default/auto | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-033 | ACP | session/set_config_option dispatch + aliases | reasonix-acp | todo | M1: docs/contracts/acp.md §10.8 |
| ACP-034 | ACP | Config rebuild pipeline ordering invariants | reasonix-acp | todo | M1: docs/contracts/acp.md §10.8 |
| ACP-035 | ACP | tool_approval option: 5 choices + ceiling filtering | reasonix-acp | todo | M1: docs/contracts/acp.md §10.8 |
| ACP-036 | ACP | Inbox methods + refusal map + enqueue intents | reasonix-acp | todo | M1: docs/contracts/acp.md §10.9 |
| ACP-037 | ACP | agent_composition option + switch + error-session | reasonix-acp | todo | M1: docs/contracts/acp.md §10.10 |
| ACP-038 | ACP | Write authority binding + recovery handler + redirect sidecar | reasonix-acp | todo | M1: docs/contracts/acp.md §10.6 |
| ACP-039 | ACP | additionalDirectories vetting + refusal codes | reasonix-acp | todo | M1: docs/contracts/acp.md §10.11 |
| ACP-040 | ACP | Warning notices wire shape | reasonix-acp | todo | M1: docs/contracts/acp.md §10.12 |
| ACP-041 | ACP | session/reloadExtensions coalescing + rebuild | reasonix-acp | todo | M1: docs/contracts/acp.md §10.7 |
| ACP-042 | ACP | Session id validation + UUID v4 generation | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-043 | ACP | MCP spec conversion + transport normalization | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-044 | ACP | availableCommands + slash resolution order | reasonix-acp | todo | M1: docs/contracts/acp.md §10.5 |
| ACP-045 | ACP | Mode/approval drift reconciliation ordering | reasonix-acp | todo | M1: docs/contracts/acp.md §10.6 |
| ACP-046 | ACP | Divergences/open decisions review | reasonix-acp | todo | M1: docs/contracts/acp.md §13 |
| MCP-001 | MCP | internal/ext/plugin (namespacing mcp__) | reasonix-ext | todo | M1: docs/contracts/mcp.md §0 |
| MCP-002 | MCP | protocol versions/eras | reasonix-ext | todo | M1: docs/contracts/mcp.md §1 |
| MCP-003 | MCP | modern era negotiation + era codes | reasonix-ext | todo | M1: docs/contracts/mcp.md §1 |
| MCP-004 | MCP | Spec/Host lifecycle | reasonix-ext | todo | M1: docs/contracts/mcp.md §2 |
| MCP-005 | MCP | StartPolicy/start phases | reasonix-ext | todo | M1: docs/contracts/mcp.md §2 |
| MCP-006 | MCP | tool naming/normalization | reasonix-ext | todo | M1: docs/contracts/mcp.md §3 |
| MCP-007 | MCP | rpcRequest/response wire | reasonix-ext | todo | M1: docs/contracts/mcp.md §3 |
| MCP-008 | MCP | stdio transport | reasonix-ext | todo | M1: docs/contracts/mcp.md §4 |
| MCP-009 | MCP | stdio env/PATH handling | reasonix-ext | todo | M1: docs/contracts/mcp.md §4 |
| MCP-010 | MCP | streamable HTTP transport | reasonix-ext | todo | M1: docs/contracts/mcp.md §5 |
| MCP-011 | MCP | HTTP headers/x-mcp-header | reasonix-ext | todo | M1: docs/contracts/mcp.md §5 |
| MCP-012 | MCP | legacy SSE transport | reasonix-ext | todo | M1: docs/contracts/mcp.md §6 |
| MCP-013 | MCP | reconnecting wrapper | reasonix-ext | todo | M1: docs/contracts/mcp.md §7 |
| MCP-014 | MCP | handshake schema cache | reasonix-ext | todo | M1: docs/contracts/mcp.md §9 |
| MCP-015 | MCP | client session dispatch/initialize | reasonix-ext | todo | M1: docs/contracts/mcp.md §10 |
| MCP-016 | MCP | lazy toolset pinning | reasonix-ext | todo | M1: docs/contracts/mcp.md §11 |
| MCP-017 | MCP | deferred spawn/startup | reasonix-ext | todo | M1: docs/contracts/mcp.md §12 |
| MCP-018 | MCP | OAuth authorize flow | reasonix-ext | todo | M1: docs/contracts/mcp.md §13 |
| MCP-019 | MCP | OAuth refresh/state | reasonix-ext | todo | M1: docs/contracts/mcp.md §14 |
| MCP-020 | MCP | OAuth helpers/consent/auth-required | reasonix-ext | todo | M1: docs/contracts/mcp.md §15 |
| MCP-021 | MCP | install | reasonix-ext | todo | M1: docs/contracts/mcp.md §16 |
| MCP-022 | MCP | failures/queries/diagnostics/status | reasonix-ext | todo | M1: docs/contracts/mcp.md §17 |
| MCP-023 | MCP | host _meta | reasonix-ext | todo | M1: docs/contracts/mcp.md §18 |
| MCP-024 | MCP | tool pins | reasonix-ext | todo | M1: docs/contracts/mcp.md §19 |
| MCP-025 | MCP | registration scope | reasonix-ext | todo | M1: docs/contracts/mcp.md §20 |
| MCP-026 | MCP | scope challenge | reasonix-ext | todo | M1: docs/contracts/mcp.md §21 |
| MCP-027 | MCP | identity/safety reconciliation | reasonix-ext | todo | M1: docs/contracts/mcp.md §22 |
| MCP-028 | MCP | catalog refresh | reasonix-ext | todo | M1: docs/contracts/mcp.md §23 |
| MCP-029 | MCP | MCP elicitation | reasonix-ext | todo | M1: docs/contracts/mcp.md §24 |
| MCP-030 | MCP | prompts/resources | reasonix-ext | todo | M1: docs/contracts/mcp.md §25 |
| MCP-031 | MCP | tool result parsing/images | reasonix-ext | todo | M1: docs/contracts/mcp.md §26 |
| MCP-032 | MCP | tasks extension | reasonix-ext | todo | M1: docs/contracts/mcp.md §27 |
| MCP-033 | MCP | in-flight cancellation | reasonix-ext | todo | M1: docs/contracts/mcp.md §28 |
| MCP-034 | MCP | mutable launcher locks | reasonix-ext | todo | M1: docs/contracts/mcp.md §29 |
| MCP-035 | MCP | stable server proxy | reasonix-ext | todo | M1: docs/contracts/mcp.md §30 |
| MCP-036 | MCP | composition guards | reasonix-ext | todo | M1: docs/contracts/mcp.md §31 |
| MCP-037 | MCP | credential projection | reasonix-ext | todo | M1: docs/contracts/mcp.md §32 |
| MCP-038 | MCP | divergences §33 | reasonix-ext | todo | M1: docs/contracts/mcp.md §33 |
| EXT-001 | EXT | transport framing §1 | reasonix-ext | todo | M1: docs/contracts/ext.md §1 |
| EXT-002 | EXT | lifecycle handshake §2 | reasonix-ext | todo | M1: docs/contracts/ext.md §2 |
| EXT-003 | EXT | content references §3 | reasonix-ext | todo | M1: docs/contracts/ext.md §3 |
| EXT-004 | EXT | interception contract §4 | reasonix-ext | todo | M1: docs/contracts/ext.md §4 |
| EXT-005 | EXT | streaming providers §5 | reasonix-ext | todo | M1: docs/contracts/ext.md §5 |
| EXT-006 | EXT | structured UI §6 | reasonix-ext | todo | M1: docs/contracts/ext.md §6 |
| EXT-007 | EXT | extension tools §7 | reasonix-ext | todo | M1: docs/contracts/ext.md §7 |
| EXT-008 | EXT | error table §8 | reasonix-ext | todo | M1: docs/contracts/ext.md §8 |
| EXT-009 | EXT | stability contract §9 | reasonix-ext | todo | M1: docs/contracts/ext.md §9 |
| EXT-010 | EXT | security model §10 | reasonix-ext | todo | M1: docs/contracts/ext.md §10 |
| EXT-011 | EXT | runtime v2 model §11 | reasonix-ext | todo | M1: docs/contracts/ext.md §11 |
| EXT-012 | EXT | protocol package §12 | reasonix-ext | todo | M1: docs/contracts/ext.md §12 |
| EXT-013 | EXT | rpcwire framing §13 | reasonix-ext | todo | M1: docs/contracts/ext.md §13 |
| EXT-014 | EXT | sidecar lifecycle §14 | reasonix-ext | todo | M1: docs/contracts/ext.md §14 |
| EXT-015 | EXT | dispatch chain §15 | reasonix-ext | todo | M1: docs/contracts/ext.md §15 |
| EXT-016 | EXT | providerext/providerconv §16 | reasonix-ext | todo | M1: docs/contracts/ext.md §16 |
| EXT-017 | EXT | uihub §17 | reasonix-ext | todo | M1: docs/contracts/ext.md §17 |
| EXT-018 | EXT | builder §18.1 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.1 |
| EXT-019 | EXT | publish gate §18.2 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.2 |
| EXT-020 | EXT | runtime plan §18.3 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.3 |
| EXT-021 | EXT | dependency graph §18.4 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.4 |
| EXT-022 | EXT | runtime snapshot §18.5 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.5 |
| EXT-023 | EXT | catalog/conflicts §18.6 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.6 |
| EXT-024 | EXT | session state store §18.7 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.7 |
| EXT-025 | EXT | receipt store §18.8 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.8 |
| EXT-026 | EXT | runtime owner §18.9 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.9 |
| EXT-027 | EXT | effect scope §18.10 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.10 |
| EXT-028 | EXT | replacement slots §18.11 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.11 |
| EXT-029 | EXT | scope resources §18.12 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.12 |
| EXT-030 | EXT | interceptor ordering §18.13 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.13 |
| EXT-031 | EXT | lifecycle registry §18.14 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.14 |
| EXT-032 | EXT | MCP proxy §18.15 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.15 |
| EXT-033 | EXT | host stream registry §18.16 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.16 |
| EXT-034 | EXT | file priors §18.17 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.17 |
| EXT-035 | EXT | scope tiers §18.18 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.18 |
| EXT-036 | EXT | stable proxy §18.19 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.19 |
| EXT-037 | EXT | discovery adapters §18.20 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.20 |
| EXT-038 | EXT | lifecycle metrics §18.21 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.21 |
| EXT-039 | EXT | message dedup §18.22 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.22 |
| EXT-040 | EXT | recovery policy §18.23 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.23 |
| EXT-041 | EXT | runtime set §18.24 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.24 |
| EXT-042 | EXT | status documents §18.25 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.25 |
| EXT-043 | EXT | contributor §18.26 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.26 |
| EXT-044 | EXT | stale generation §18.27 | reasonix-ext | todo | M1: docs/contracts/ext.md §18.27 |
| EXT-045 | EXT | protocolgen artifacts §19 | reasonix-ext | todo | M1: docs/contracts/ext.md §19 |
| EXT-046 | EXT | divergences §20 | reasonix-ext | todo | M1: docs/contracts/ext.md §20 |
| STATE-001 | State | disposable-projection philosophy; authoritative data outside DB | reasonix-state | todo | M1: docs/contracts/state.md §0 |
| STATE-002 | State | SQLite driver surface + file locations | reasonix-state | todo | M1: docs/contracts/state.md §0 |
| STATE-003 | State | projectiondb open modes, DSNs, pragmas, permissions | reasonix-state | todo | M1: docs/contracts/state.md §1.2-§1.3 |
| STATE-004 | State | projectiondb migrations, gap error, FutureSchema degraded read | reasonix-state | todo | M1: docs/contracts/state.md §1.4 |
| STATE-005 | State | corruption triage + quarantine rename | reasonix-state | todo | M1: docs/contracts/state.md §1.2, §1.5 |
| STATE-006 | State | OpenReadOnly + Inspect (no repair) | reasonix-state | todo | M1: docs/contracts/state.md §1.6 |
| STATE-007 | State | projectiondb Rebuild: lock, temp, integrity, swap, rollback | reasonix-state | todo | M1: docs/contracts/state.md §1.7 |
| STATE-008 | State | historycatalog schema v1 DDL | reasonix-state | todo | M1: docs/contracts/state.md §2.2 |
| STATE-009 | State | tokenizer version 4 wipe on open | reasonix-state | todo | M1: docs/contracts/state.md §2.3 |
| STATE-010 | State | indexing pipeline: queue coalescing, worker, revision bump | reasonix-state | todo | M1: docs/contracts/state.md §2.3 |
| STATE-011 | State | append fast path (tryAppendPath) + fingerprint skip | reasonix-state | todo | M1: docs/contracts/state.md §2.4 |
| STATE-012 | State | root scan: signature skip, generations, adopt/sweep, grace | reasonix-state | todo | M1: docs/contracts/state.md §2.6 |
| STATE-013 | State | document kinds + tokenization + digests | reasonix-state | todo | M1: docs/contracts/state.md §2.5 |
| STATE-014 | State | search: BM25 ordering, keyset pagination, filters, score | reasonix-state | todo | M1: docs/contracts/state.md §2.7 |
| STATE-015 | State | purge + record-read-failure + publish reasons | reasonix-state | todo | M1: docs/contracts/state.md §2.3 |
| STATE-016 | State | historycatalog rebuild/inspect/status counts | reasonix-state | todo | M1: docs/contracts/state.md §2.8 |
| STATE-017 | State | replay budgets + degraded modes + diagnostics redaction | reasonix-state | todo | M1: docs/contracts/state.md §2.9 |
| STATE-018 | State | usagecatalog schema v8 + older-version retirement | reasonix-state | todo | M1: docs/contracts/state.md §3.1-§3.2 |
| STATE-019 | State | daily stats JSONL line shape (rawRecord) | reasonix-state | todo | M1: docs/contracts/state.md §3.3 |
| STATE-020 | State | append receipts: idempotency, line hash, offset frontier | reasonix-state | todo | M1: docs/contracts/state.md §3.4 |
| STATE-021 | State | rollup/cost projection + rederiveDays | reasonix-state | todo | M1: docs/contracts/state.md §3.5 |
| STATE-022 | State | ReconcileDir/File + forgetOutsideRoot + underRoot | reasonix-state | todo | M1: docs/contracts/state.md §3.6 |
| STATE-023 | State | Ready semantics (vanish, same-size rewrite, frontier) | reasonix-state | todo | M1: docs/contracts/state.md §3.6 |
| STATE-024 | State | cost views, estimate mode, unpriced vocabulary | reasonix-state | todo | M1: docs/contracts/state.md §3.7 |
| STATE-025 | State | Query/attachCosts/UnpricedInRange | reasonix-state | todo | M1: docs/contracts/state.md §3.8 |
| STATE-026 | State | usagecatalog status/rebuild/inspect + perf datum | reasonix-state | todo | M1: docs/contracts/state.md §3.8 |
| STATE-027 | State | cc-switch import reader (external DB, read-only) | reasonix-config | todo | M1: docs/contracts/state.md §4 |
| STATE-028 | State | authoritative state formats survey | reasonix-state | todo | M1: docs/contracts/state.md §5 |
| STATE-029 | State | session-format contract follow-up gate (M2) | reasonix-state | done | M2: docs/contracts/session-format.md §0-§22 frozen (2026-11-15) |
| STATE-030 | State | divergences: catalog inventory, version drift, engine ADR | reasonix-state | todo | M1: docs/contracts/state.md §6 |
| TOOL-001 | Tools | contract/tool Tool interface + optional capabilities | reasonix-tools | done | crates/tools tests: preview_change_cases, provider_schemas_puts_every_contextual_tool_after_every_stable_one, contextual_tools_explain_themselves; CapabilitySet TypeId-downcast expresses Go's optional interfaces (2026-11-20) |
| TOOL-002 | Tools | per-run Registry, restriction, replace-prefix, withdrawn | reasonix-tools | done | crates/tools tests/registry*.rs: registry_resolves_portable_mcp_references_only_when_unique, replace_prefix_never_shows_an_empty_namespace, restrict_drops_what_is_held_and_refuses_what_comes_later, withdraw_keeps_the_schema_and_fails_the_call, disable_tools_removes_from_every_surface_and_blocks_later_adds (2026-11-20) |
| TOOL-003 | Tools | ContractEntry snapshot + provenance | reasonix-tools | done | crates/tools tests: contract_entries_does_not_hold_registry_lock_across_tool_callbacks, provenance_of_reads_the_tools_type, sink_keeps_the_first_external_origin_and_ignores_the_workspace (2026-11-20) |
| TOOL-004 | Tools | argument envelope unwrapping | reasonix-tools | done | crates/tools tests: unwrap_arguments_cases (Go arguments_test 20 cases), unwrap_arguments_requires_unambiguous_schema; Draft-7 subset validator, unknown combinators refuse (2026-11-20) |
| TOOL-005 | Tools | ShellExecution metadata | reasonix-tools | done | crates/tools/src/shell.rs serde shapes (camelCase, typed enums, OUTPUT_TAIL_MAX_BYTES); no direct Go test exists, behavior pinned by types+doc (2026-11-20) |
| TOOL-006 | Tools | MCP binding, aliases, disabled/held, annotations, scope refusals | reasonix-tools | wip | binding/aliases/portable/disabled/held done (mcp_binding_for_reads_declared_identity, replace_disabled_mcp_drops_stale_aliases etc.); annotations + runtime bind anchored (2026-11-20) |
| TOOL-007 | Tools | Roots, reach, target access, declared write paths | reasonix-tools | wip | roots.rs done (missing_root_names_the_first_gone_directory_by_its_display_path etc.); reach constants + declarer in registry; target access anchored (2026-11-20) |
| TOOL-008 | Tools | Invoker + nested calls | reasonix-tools | done | crates/tools tests: nested_call_id_names_its_caller (Go invoke_test cases); depth ceiling is the agent's policy, not contract/tool's (2026-11-20) |
| TOOL-009 | Tools | Goal reporting | reasonix-tools | todo | M1: docs/contracts/tools.md §1.14 |
| TOOL-010 | Tools | Recall | reasonix-tools | todo | M1: docs/contracts/tools.md §1.15 |
| TOOL-011 | Tools | Refusal identity/remedy + blocked errors | reasonix-tools | done | crates/tools tests: refusal_renders_the_setting_that_would_lift_it, a_remedy_that_only_opens_carries_no_value, blocked_error_carries_identity_not_words, held_mcp_refusal_is_typed_and_server_qualified (2026-11-20) |
| TOOL-012 | Tools | Peer and project interfaces | reasonix-tools | todo | M1: docs/contracts/tools.md §1.17 |
| TOOL-013 | Tools | Settings | reasonix-tools | todo | M1: docs/contracts/tools.md §1.18 |
| TOOL-014 | Tools | Elicitation | reasonix-tools | todo | M1: docs/contracts/tools.md §1.19 |
| TOOL-015 | Tools | Paid search approval | reasonix-tools | todo | M1: docs/contracts/tools.md §1.20 |
| TOOL-016 | Tools | Managed config write approval | reasonix-tools | todo | M1: docs/contracts/tools.md §1.21 |
| TOOL-017 | Tools | Context fold + context budget | reasonix-tools | todo | M1: docs/contracts/tools.md §1.22, §1.36 |
| TOOL-018 | Tools | Native video | reasonix-tools | todo | M1: docs/contracts/tools.md §1.23 |
| TOOL-019 | Tools | Transcript reader | reasonix-tools | todo | M1: docs/contracts/tools.md §1.26 |
| TOOL-020 | Tools | Background starter | reasonix-tools | todo | M1: docs/contracts/tools.md §1.27 |
| TOOL-021 | Tools | Approval scope + permission projection | reasonix-tools | todo | M1: docs/contracts/tools.md §1.28, §1.31 |
| TOOL-022 | Tools | Progress | reasonix-tools | todo | M1: docs/contracts/tools.md §1.30 |
| TOOL-023 | Tools | Resolved calls and proxy tools | reasonix-tools | todo | M1: docs/contracts/tools.md §1.37 |
| TOOL-024 | Tools | builtin catalog: 33 registrations (Name/Description/Schema/ReadOnly) | reasonix-tools | todo | M1: docs/contracts/tools.md §2.1 |
| TOOL-025 | Tools | bash shell tool | reasonix-tools | todo | M1: docs/contracts/tools.md §2.2 |
| TOOL-026 | Tools | background-job tools (bgjobs.go) | reasonix-tools | todo | M1: docs/contracts/tools.md §2.3 |
| TOOL-027 | Tools | grep two-variant description | reasonix-tools | wip | schema/description contract landed verbatim (grep.rs Tool impl); the ripgrep variant awaits the subprocess engine slice (2026-11-20) |
| TOOL-028 | Tools | file read/search tools (read_file, glob, ls, code_index) | reasonix-tools | wip | read_file/glob/ls done with Go test tables (tests/builtin_readfile.rs::test_read_file_nested_returns_the_text, tests/builtin_grep.rs::test_grep_glob_matches_file_name_at_any_depth, tests/builtin_glob_ls.rs::test_ls_recursive_skips_noise_dirs); code_index still todo (2026-11-20) |
| TOOL-029 | Tools | file write tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.6 |
| TOOL-030 | Tools | web tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.7 |
| TOOL-031 | Tools | browser tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.8 |
| TOOL-032 | Tools | computer (CUA) tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.9 |
| TOOL-033 | Tools | media tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.10 |
| TOOL-034 | Tools | planning, goal, context tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.11 |
| TOOL-035 | Tools | misc tools | reasonix-tools | todo | M1: docs/contracts/tools.md §2.12 |
| TOOL-036 | Tools | assembly wiring order (16 steps) | reasonix-tools | todo | M1: docs/contracts/tools.md §3.0 |
| TOOL-037 | Tools | use_capability resolution order + deferred index floor | reasonix-tools | todo | M1: docs/contracts/tools.md §3.1 |
| TOOL-038 | Tools | capability catalog/ledger/audit + routing/semantic router | reasonix-tools | todo | M1: docs/contracts/tools.md §3.2-§3.3 |
| TOOL-039 | Tools | delegation tools (spawn_agents, read_subagent_result) | reasonix-tools | todo | M1: docs/contracts/tools.md §3.4 |
| TOOL-040 | Tools | skill tools + wrappers | reasonix-tools | todo | M1: docs/contracts/tools.md §3.5 |
| TOOL-041 | Tools | extension runtime tools (ext__ naming, digest recheck) | reasonix-tools | todo | M1: docs/contracts/tools.md §3.6 |
| TOOL-042 | Tools | agent/state/platform tools (incl. view_image) | reasonix-tools | todo | M1: docs/contracts/tools.md §3.7 |
| TOOL-043 | Tools | assembly-conditional surface (web_search, fetchchain, slash_command, MCP connect, install_source) | reasonix-tools | todo | M1: docs/contracts/tools.md §3.8 |
| TOOL-044 | Tools | legacy delegation names + eight name-recognition surfaces | reasonix-tools | todo | M1: docs/contracts/tools.md §3.9 |
| TOOL-045 | Tools | shellrun foreground runner + login env + decode | reasonix-tools | todo | M1: docs/contracts/tools.md §4 |
| TOOL-046 | Tools | jobs Manager/Job lifecycle + validation + recovery | reasonix-tools | todo | M1: docs/contracts/tools.md §5.1 |
| TOOL-047 | Tools | jobs wait/watch/echo/peek/views/progress | reasonix-tools | todo | M1: docs/contracts/tools.md §5.2 |
| TOOL-048 | Tools | jobs completion, finalization, exit_error | reasonix-tools | todo | M1: docs/contracts/tools.md §5.3 |
| TOOL-049 | Tools | productdocs docs tool (search/read/list + corpus) | reasonix-tools | todo | M1: docs/contracts/tools.md §6 |
| TOOL-050 | Tools | workspace confinement (confine.go) | reasonix-tools | wip | read half + refusal texts done (tests/builtin_confine.rs::test_confine_read_blocks_read_file, test_confine_write_side_refusal_carries_workspace_code, test_confine_rejects_symlink_escape); write-side guard/managed/sessiontemp anchored for the write-tools slice (2026-11-20) |
| TOOL-051 | Tools | session-data guard | reasonix-tools | todo | M1: docs/contracts/tools.md §7.2 |
| TOOL-052 | Tools | admission check + network-path refusals | reasonix-tools | todo | M1: docs/contracts/tools.md §7.3 |
| TOOL-053 | Tools | destructive-delete guard + command line ceiling | reasonix-tools | todo | M1: docs/contracts/tools.md §7.4 |
| TOOL-054 | Tools | FileViews, receipts, setting, write-path identity, roots | reasonix-tools | todo | M1: docs/contracts/tools.md §7.5 |
| TOOL-055 | Tools | previews + edit engine + encoding round-trip | reasonix-tools | todo | M1: docs/contracts/tools.md §7.6 |
| TOOL-056 | Tools | Workspace binding + PathResolver + walk pruning | reasonix-tools | wip | walk pruning done (vendorDirs/skipWalkDir/skipForbidDir/WalkConfine in walk.rs tests); PathResolver/ForkFileViews outstanding (2026-11-20) |
| TOOL-057 | Tools | divergences (parallel_tasks guidance, preview asymmetry, V2 shadow) | reasonix-tools | todo | M1: docs/contracts/tools.md §8 |
| HOST-001 | Host | main() argv precedence (mac handoff, tree handoff, schedule exec before flags) | apps/reasonix-studio-host | wip | schedule-exec slot + flag precedence done (cli tests); mac/tree handoff machines land with the update slice (2026-11-20) |
| HOST-002 | Host | Host flag set (-page, -instance-id, -studio-*, -cua-driver, -computer-*-socket, -crash-dir, -strip-package-grants) + modhost embedder | apps/reasonix-studio-host | wip | full flag set parsed with Go flag semantics (args.rs tests); -instance-id answers the real hash; modhost embedder lands with the platform slice (2026-11-20) |
| HOST-003 | Host | Exit codes: 0 clean, 1 every startup failure | apps/reasonix-studio-host | done | cli tests (undefined flag 2, schedule child 1, lease EOF 0) (2026-11-20) |
| HOST-004 | Host | Startup phase log lines (startup phase=%s elapsed_ms=%d; boot.* phases; 12-phase order) | apps/reasonix-studio-host | done | release smoke: 12 phase lines in the frozen order with elapsed_ms; boot.* forwarding lands with the runtime slice (2026-11-20) |
| HOST-005 | Host | Parent lease: stdin named pipe or socket only; terminal/file/dev-null unsupervised | apps/reasonix-studio-host | wip | terminal = unsupervised (IsTerminal); pipe/file distinction needs FFI this tree forbids (OPEN_QUESTIONS §7); shell spawn always pipes (2026-11-20) |
| HOST-006 | Host | Parent EOF on the lease ends serving cleanly (exit 0, port released) | apps/reasonix-studio-host | done | integration test: handshake first line, lease close → exit 0 (2026-11-20) |
| HOST-007 | Host | Crash/fatal log: InstallFatalLog(crash-dir); released on 0, recorded with last line otherwise | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §1.6 |
| HOST-008 | Host | Handshake line: one JSON line (version 1, origin, token), field order version/origin/token | apps/reasonix-studio-host | done | handshake.rs field-order test + release smoke first line ({"version":1,...}) (2026-11-20) |
| HOST-009 | Host | Fresh credential per launch: 32 random bytes hex, never persisted, never from config | apps/reasonix-studio-host | done | credential.rs tests (64 hex, differs per mint) (2026-11-20) |
| HOST-010 | Host | Loopback bind 127.0.0.1:0 (tcp4); origin http://127.0.0.1:<port> | apps/reasonix-studio-host | done | serve gate tests + smoke origin shape (2026-11-20) |
| HOST-011 | Host | Announce ordering: after bind + provider-setup enable, before serve loop | apps/reasonix-studio-host | done | prepare() marks bind/provider-setup then the binary announces before serve_until; phase log order in smoke (2026-11-20) |
| HOST-012 | Host | host.js acceptance rules (version, token >= 32 chars, origin URL, http:, 127.0.0.1, port; 4096-byte limit; 60 s timeout; codes timeout/exited/spawn/overflow/malformed) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.2-2.3 |
| HOST-013 | Host | Shell retry contract: only exited/spawn within 10 s, one retry | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §2.4 |
| HOST-014 | Host | Acts: relaunch/quit/show, one JSON line each, mutex-serialized | apps/reasonix-studio-host | done | acts.rs tests (own-line, linux-only relaunch, quit on end); live triggers land with notify (2026-11-20) |
| HOST-015 | Host | Relaunch asked on Linux only; quit on EndApplication; PrepareForUpdate writes nothing | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §3 |
| HOST-016 | Host | Toast click asks show; notify: toast clicked / could-not-reach logs | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §3 |
| HOST-017 | Host | Loopback gate wiring with the launch credential (gate checks per http.md §2.6) | apps/reasonix-studio-host | done | release smoke: 403 unauthorized/origin_rejected/host_rejected each from the real process (2026-11-20) |
| HOST-018 | Host | hostServeConfig zeroes AuthMode/Token/PasswordHash/BehindProxy (no config contribution) | apps/reasonix-studio-host | done | config host_serve_section zeroing tests; the router's gate takes only the minted credential, user serve auth never reaches it (2026-11-20) |
| HOST-019 | Host | Provider setup enabled on the loopback listener only | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §4.3 |
| HOST-020 | Host | Graceful drain: RunGracefulHandler; hub.Shutdown after socket drain | apps/reasonix-studio-host | done | listen.rs drain tests + lease-EOF integration exit 0 (2026-11-20) |
| HOST-021 | Host | -instance-id: io.reasonix.studio.<16 hex> (canonicalized home sha256[:8]) | apps/reasonix-studio-host | done | assembly host tests (marker canonicalization, prefix pinned); canonicalize narrowing recorded in OPEN_QUESTIONS §8 (2026-11-20) |
| HOST-022 | Host | -strip-package-grants: exit 2/1/0 semantics; one-line JSON report with stripped/refused/unread always present | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §5.2 |
| HOST-023 | Host | schedule exec child: os.Args[3:] through schedulerun.Exec | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §5.3 |
| HOST-024 | Host | Schedule ticker eligibility (supervised && (iconLive or panes > 0)) + machine-wide scheduler.lock | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.5 |
| HOST-025 | Host | Launch workspace: userChoseDir rules, remembered fallback, auto created last, never home | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.1 |
| HOST-026 | Host | serve-workspaces.json: launch project wins over remembered order | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.1 |
| HOST-027 | Host | Kernel language: top-level language + REASONIX_SYSTEM_LANG matrix; /appearance live-switch | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.2 |
| HOST-028 | Host | Telemetry consent: strict user-config re-read per ping; project config cannot enable; AllowMarketInstallReport release-only | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.3 |
| HOST-029 | Host | Tray: IconLive re-read (DesktopTray != off); fold from tracker; ApplyTrayPrefs noop | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.4 |
| HOST-030 | Host | Notifications wrap every runtime live; disk watch says a low volume once, honours both switches, 5 min cadence | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.5 |
| HOST-031 | Host | Install/update typed-nil guards: nil unless version+exe+pid stated; layout follows stated application | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.6 |
| HOST-032 | Host | No built page: kernel-only serve + log line | apps/reasonix-studio-host | done | find_page None path + on_log line in prepare (2026-11-20) |
| HOST-033 | Host | Host must not depend on the CLI crate (schedule child via schedulerun) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.8 |
| HOST-034 | Host | Hub wiring keys (Tray/Asks/Remote/DecorateSink/Grant) + background cleanup reconciler + OpenOnFallbackModel | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §6.8 |
| HOST-035 | Host | Credential cookie contract: reasonix_token, httpOnly, sameSite strict, armed before load, restored on removal/refusal | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.2 |
| HOST-036 | Host | Shell single instance scoped by -instance-id answer (profile + lock) | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.4 |
| HOST-037 | Host | Quit drains through the lease (child stdin end); window-all-closed quits only after handshake | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.5 |
| HOST-038 | Host | Host exit marker + notice (unexpected-host-exit.json) for post-handshake death | apps/reasonix-studio-host | todo | M1: docs/contracts/host.md §7.3 |
| PATHS-001 | Paths | Storage roots, env pins, precedence (home/state/cache/worktrees/locks) | reasonix-config | wip | home/state precedence + env pins in crates/config/src/paths.rs and crates/session/src/paths.rs tests; cache/worktrees/locks outstanding (2026-11-20) |
| PATHS-002 | Paths | StateRootEntries (29) + first-move subset | reasonix-config | todo | M1: docs/contracts/paths.md §1 |
| PATHS-003 | Paths | Derived path functions (config, credentials, sessions, leases, worktrees) | reasonix-session | wip | SessionDirFor/workspace_slug derived in crates/session/src/paths.rs tests; config/credentials/leases/worktrees outstanding (2026-11-20) |
| PATHS-004 | Paths | Default dirs incl. locks-dir convergence across isolated instances | reasonix-config | todo | M1: docs/contracts/paths.md §1 |
| PATHS-005 | Paths | User config selection chain (primary → legacy OS-support → legacy XDG) | reasonix-config | wip | primary + legacy OS-support + pinned-home skip done (paths.rs tests); XDG legacy candidates unix-only; isUserConfigPath/managed scoping outstanding (2026-11-20) |
| PATHS-006 | Paths | Project config paths; fallback only on IsNotExist | reasonix-config | todo | M1: docs/contracts/paths.md §2 |
| PATHS-007 | Paths | userConfigCandidatePaths / isUserConfigPath / managed config paths | reasonix-config | todo | M1: docs/contracts/paths.md §2 |
| PATHS-008 | Paths | Config edit locks (aliasing, 5s timeout, unsafe-dir and owner checks) | reasonix-config | todo | M1: docs/contracts/paths.md §3 |
| PATHS-009 | Paths | Lock registry dirs (isolated home vs production, identity, unix tmp fallback) | reasonix-config | todo | M1: docs/contracts/paths.md §3 |
| PATHS-010 | Paths | Credential store file format (0600/0700, tombstones, revision, CAS) | reasonix-config | todo | M1: docs/contracts/paths.md §4 |
| PATHS-011 | Paths | API key env naming (APIKeyEnvFor, private slots, CUSTOM_ forms) | reasonix-config | todo | M1: docs/contracts/paths.md §4 |
| PATHS-012 | Paths | Legacy keyring migration + marker (1s budget, four-state) | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-013 | Paths | v1.8.1 legacy migration (TOML then config.json, BOM tolerated, support data) | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-014 | Paths | MigrationResult notice format | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-015 | Paths | v1.9.1 MCP backfill + one-time marker | reasonix-config | todo | M1: docs/contracts/paths.md §5 |
| PATHS-016 | Paths | State relocation adoption (.adopted aside, MergeOnce, no overwrite) | reasonix-state | todo | M1: docs/contracts/paths.md §5 |
| PATHS-017 | Paths | /migrate rescue semantics | reasonix-cli | todo | M1: docs/contracts/paths.md §5 |
| PATHS-018 | Paths | Credential path deny list (exact paths, ~/.ssh rules, snapshot) | reasonix-core | todo | M1: docs/contracts/paths.md §6 |
| PATHS-019 | Paths | Sensitive file names + dirs + ForbiddenReadPaths | reasonix-core | todo | M1: docs/contracts/paths.md §6 |
| PATHS-020 | Paths | Host secret path registration | reasonix-core | todo | M1: docs/contracts/paths.md §6 |
| PATHS-021 | Paths | Host-writable dirs per platform (hostwrite) | reasonix-core | todo | M1: docs/contracts/paths.md §7 |
| PATHS-022 | Paths | Subprocess env filtering (ProcessEnv, FilterEnv, toggles) | reasonix-core | todo | M1: docs/contracts/paths.md §8 |
| PATHS-023 | Paths | Credential env key registration (process-lifetime union) | reasonix-core | todo | M1: docs/contracts/paths.md §8 |
| PATHS-024 | Paths | REASONIX_AGENT_CHILD marker | reasonix-core | todo | M1: docs/contracts/paths.md §8 |
| PATHS-025 | Paths | Env redaction patterns + mask rules | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-026 | Paths | CredentialKey algorithm | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-027 | Paths | Endpoint projection (RedactEndpoint, RedactConfigValue, RedactArgs) | reasonix-core | done | crates/core/src/secrets/endpoint.rs tests + tests/secrets_projection_golden.rs |
| PATHS-028 | Paths | Diagnostic errors (identity kept, prose omitted) | reasonix-core | todo | M1: docs/contracts/paths.md §9 |
| PATHS-029 | Paths | Environment probes (default set, error taxonomy, 2s timeout) | reasonix-platform | todo | M1: docs/contracts/paths.md §10 |
| PATHS-030 | Paths | Probe caching (5min in-process, singleflight, 24h snapshot v2, fingerprint) | reasonix-platform | todo | M1: docs/contracts/paths.md §10 |
| PATHS-031 | Paths | FormatSection exact output | reasonix-platform | todo | M1: docs/contracts/paths.md §10 |
| PATHS-032 | Paths | Workspace VCS detection (jj/git/hg/svn probe order) | reasonix-core | todo | M1: docs/contracts/paths.md §10 |
| PATHS-033 | Paths | Language detection chain + normalization | reasonix-core | todo | M1: docs/contracts/paths.md §11 |
| PATHS-034 | Paths | Test isolation (IsolateUserState, testscope) | reasonix-testkit | todo | M1: docs/contracts/paths.md §12 |
| PATHS-035 | Paths | REASONIX_* env var inventory | reasonix-core | todo | M1: docs/contracts/paths.md §13 |
| PATHS-036 | Paths | Studio logs + kernel caches layout | reasonix-platform | todo | M1: docs/contracts/paths.md §14 |
| SESS-001 | Session | Store as sole session-path authority; event log authoritative; budgets refuse, never truncate | reasonix-state | todo | M2: docs/contracts/session-format.md §0 |
| SESS-002 | Session | Sidecar naming inventory; full-vs-stem accessors; transcript-name predicates and inverters | reasonix-state | done | crates/state/src/store/naming.rs::tests::{stem_accessors_append_to_the_stripped_stem, transcript_predicate_accepts_plain_transcripts_only, inverters_recover_session_paths} (2026-11-15) |
| SESS-003 | Session | RemoveSessionArtifacts: exact 13-file set + outputs/blobs; tolerated survivors (guardian, lock/lease, ckpt/jobs/inbox) | reasonix-state | done | crates/state/src/store/artifacts.rs::tests::removes_transcript_sidecars_and_output_dirs_but_not_tolerated_survivors (2026-11-15) |
| SESS-004 | Session | sessionSidecarMoves 10-pair rename order (blobs dir before event log) | reasonix-state | done | crates/state/src/store/naming.rs::tests::sidecar_moves_are_the_frozen_ten_pairs_in_order (2026-11-15) |
| SESS-005 | Session | Session filename layout (UTC 20060102-150405.000000000-model.jsonl) + sanitizer; version snapshots | reasonix-state | done | crates/state/src/sessionstore/path.rs (unit tests; naming; 2026-11-15); crates/state/src/sessionstore/versions.rs (unit tests) (2026-11-17) |
| SESS-006 | Session | Event-log record shape | reasonix-state | done | crates/state/src/sessionstore/eventlog/record.rs::tests (wire decode round trip, null word, created_at grammar; 2026-11-16) |
| SESS-007 | Session | Event-log schema versions 1-5; reader/writer matrix | reasonix-state | done | crates/state/src/sessionstore/eventlog/writer.rs::tests (golden byte line, zero-field omission, append/replace/append-event matrix, replay round trip; 2026-11-16) |
| SESS-008 | Session | Replay budgets (512MiB / 100K records / 100K messages / 4KiB probe); refuse, never truncate | reasonix-state | done | crates/state/src/sessionstore/eventlog/{replay,reader,preflight}.rs::tests (budget refusals encoded_bytes/records/messages/payload; 2026-11-16) |
| SESS-009 | Session | Tolerant replay: torn-record stop, damaged-tail semantics | reasonix-state | done | crates/state/src/sessionstore/eventlog/{replay,reader}.rs::tests (torn tail, index mismatch, undecodable payload; 2026-11-16) |
| SESS-010 | Session | Tail repair and salvage; .events.jsonl.damaged header fields | reasonix-state | done | crates/state/src/sessionstore/eventlog/repair.rs::tests (repair, preserve, index read; 2026-11-16) |
| SESS-011 | Session | Event-index sidecar (schema_version=1) | reasonix-state | done | crates/state/src/sessionstore/eventlog/index.rs (unit tests: write, read-back, stale removal) (2026-11-16) |
| SESS-012 | Session | Fold rules (>128MiB AND drop-half or >max(256KiB, 4x content)); NativeArchive v2/v3 192MiB cap | reasonix-state | done | crates/state/src/sessionstore/eventlog/writer.rs::tests::session_event_log_oversized (fold rules; 2026-11-16); crates/state/src/sessionstore/composition_archive.rs (unit tests) (2026-11-17) |
| SESS-013 | Session | Event-log probe: 4KiB sniff; native/foreign/dag/future classification | reasonix-state | done | crates/state/src/sessionstore/eventlog/probe.rs::tests (4KiB sniff, minimal decode fallback, native/dag/future; 2026-11-16) |
| SESS-014 | Session | Image blobs: sha256 addressing, 64MiB cap, .atomic-*.tmp sweep, refuseDegradedCheckpointRewrite | reasonix-state | done | crates/state/src/sessionstore/eventlog/images.rs::tests (externalize/write-blob/checkpoint-scan/backfill/refuse-guard/sweep; 2026-11-16) |
| SESS-015 | Session | Session model: fields, version semantics, HasUnsavedChanges, baseline/read methods | reasonix-state | done | crates/state/src/sessionstore/session/ (unit tests: session model, digest, storage equality, canonical paths, persistence baseline, observer) (2026-11-16) |
| SESS-016 | Session | Save entry points; locks and path identity | reasonix-state | done | crates/state/src/sessionstore/session/save.rs (unit tests) (2026-11-17) |
| SESS-017 | Session | saveLocked 14-step order | reasonix-state | done | crates/state/src/sessionstore/session/save.rs (unit tests) (2026-11-17) |
| SESS-018 | Session | checkSnapshotWrite CAS rules; snapshot fast path; disk baseline | reasonix-state | done | crates/state/src/sessionstore/session/save.rs (unit tests) (2026-11-17) |
| SESS-019 | Session | ownsPersistedState; conflicts and supersede | reasonix-state | done | crates/state/src/sessionstore/session/save.rs (unit tests) (2026-11-17) |
| SESS-020 | Session | Recovery branches (depth cap 3, 'session recovery chain depth exceeded') | reasonix-state | done | crates/state/src/sessionstore/session/recovery.rs (unit tests) (2026-11-17) |
| SESS-021 | Session | Revision ledger RMW in branch-meta sidecar | reasonix-state | done | crates/state/src/sessionstore/branch.rs (unit tests) (2026-11-17) |
| SESS-022 | Session | SessionWriterID (host-pid-8byte-hex-nonce) and digest | reasonix-state | done | crates/state/src/sessionstore/eventlog/writer_id.rs::tests (2026-11-16, committed dfd287f) |
| SESS-023 | Session | Load path normalizedDirty; cleanup-pending markers; listing helpers | reasonix-state | todo | M2: docs/contracts/session-format.md §7.12-§7.14 |
| SESS-024 | Session | BranchMeta json fields; InFlightTurnMeta; backoffs [20,50,100]ms; load/save; mutators | reasonix-state | done | crates/state/src/sessionstore/branch.rs (unit tests) (2026-11-17) |
| SESS-025 | Session | Session lease (cross-process single writer) + lease info json | reasonix-state | done | crates/state/src/sessionstore/session_lease.rs (unit tests) (2026-11-17) |
| SESS-026 | Session | Lock files: unix flock / Windows LockFileEx 1-byte FAIL_IMMEDIATELY | reasonix-state | done | crates/state/src/sessionstore/session_lock.rs (unit tests) (2026-11-17) |
| SESS-027 | Session | Write authority: generation-bound permit | reasonix-state | done | crates/state/src/sessionstore/session_lease.rs + session/save.rs (unit tests) (2026-11-18) |
| SESS-028 | Session | Display-index sidecar (schema_version=1, 16MiB line cap) + bounded recovery read model | reasonix-state | todo | M2: docs/contracts/session-format.md §10.1-§10.2 |
| SESS-029 | Session | Range reads | reasonix-state | todo | M2: docs/contracts/session-format.md §10.3 |
| SESS-030 | Session | Listing order + .list-index.json format 3 | reasonix-state | done | crates/state/src/sessionstore/session_order.rs (unit tests) (2026-11-17) |
| SESS-031 | Session | Turn classification; TransientUserBlockTags; SyntheticUserPrefixes; preview derivation | reasonix-state | todo | M2: docs/contracts/session-format.md §10.5 |
| SESS-032 | Session | Compaction .context.json: schemas V1..3 (1x V4); receipt statuses; load/save fingerprints | reasonix-state | done | crates/state/src/sessionstore/compaction_state.rs (unit tests) (2026-11-17) |
| SESS-033 | Session | Foreign 1.x schema-2 DAG: shape, head selection, save refusal + unchanged fast path | reasonix-state | todo | M2: docs/contracts/session-format.md §12.1-§12.3 |
| SESS-034 | Session | Shared record framing (session_replay_reader); unknown record type is a hard error | reasonix-state | todo | M2: docs/contracts/session-format.md §12.4 |
| SESS-035 | Session | Legacy v0.x import: one-time markers, passes, copy mechanics, flat-import + stranded re-home, skip taxonomy | reasonix-state | todo | M2: docs/contracts/session-format.md §13 |
| SESS-036 | Session | sessionv4 format: manifest, RX4F zstd frames (8MiB max), content pool | reasonix-state | todo | M2: docs/contracts/session-format.md §14.1 |
| SESS-037 | Session | sessionv4 import into the native store | reasonix-state | todo | M2: docs/contracts/session-format.md §14.2 |
| SESS-038 | Session | LegacyListings scan; v4ListingCache flush/backoff; .v4-removed marker | reasonix-state | todo | M2: docs/contracts/session-format.md §14.3-§14.4 |
| SESS-039 | Session | Recovery coverage and reclaimability | reasonix-state | done | crates/state/src/sessionstore/recovery_gc.rs (unit tests) (2026-11-18) |
| SESS-040 | Session | Recovery trash: staging, pending-marker-before-move, single-rename publish, trash meta | reasonix-state | done | crates/state/src/sessionstore/recovery_trash.rs (unit tests) (2026-11-18) |
| SESS-041 | Session | Lineage redundancy tie-break + lineage archive (all-meta-locked rollback) | reasonix-state | done | crates/state/src/sessionstore/recovery_lineage.rs (unit tests) (2026-11-17) |
| SESS-042 | Session | Isolated recovery lanes (8 attempts; collision error text) | reasonix-state | done | crates/state/src/sessionstore/session/recovery.rs (unit tests) (2026-11-17) |
| SESS-043 | Session | SessionRemovalGuard / TryConvertToRemovalGuard lease interplay | reasonix-state | done | crates/state/src/sessionstore/session_lease.rs (unit tests) (2026-11-17) |
| SESS-044 | Session | Empty-session GC (5m grace); fork cleanup; orphan outputs (1h grace) | reasonix-state | done | crates/state/src/sessionstore/cleanup_pending.rs (unit tests) (2026-11-18) |
| SESS-045 | Session | Checkpoint on-disk layout (turn files, blobs fan-out, transactions, temps) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.1 |
| SESS-046 | Session | Checkpoint schema types (retain 100, blob quota 1GiB, max file 32MiB, Coverage/CoverageGap/RewindConflict) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.2 |
| SESS-047 | Session | Checkpoint store lifecycle + GC (expire-then-quota-then-prune; inline content kept alongside blob refs) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.3 |
| SESS-048 | Session | Rewind plan (ConversationApplier methods; 10-minute plan expiry; precheckFiles) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.4 |
| SESS-049 | Session | Transaction protocol: publish-before-rename; compensation recovery copy; idempotent startup retry | reasonix-state | todo | M2: docs/contracts/session-format.md §16.5 |
| SESS-050 | Session | Undo manifests (inverse kind, undoFileConflicts) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.6 |
| SESS-051 | Session | Crash recovery (undoneParents pass; deferred conversation recovery) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.7 |
| SESS-052 | Session | Single-file revert (ConflictResolution keep_current / overwrite_checkpoint) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.8 |
| SESS-053 | Session | Conversation fork (CopyConversationPrefixTo) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.9 |
| SESS-054 | Session | Checkpoint blob store (validBlobRef = 64 lowercase hex) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.10 |
| SESS-055 | Session | Capture and fingerprints (CompareIdentity) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.11 |
| SESS-056 | Session | Mutation barrier (generation bumps on exclusive release and ExitWrite) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.12 |
| SESS-057 | Session | Mutation observer and writer admission | reasonix-state | todo | M2: docs/contracts/session-format.md §16.13 |
| SESS-058 | Session | Ownership bookkeeping (syncRestoredOwnership / syncForwardOwnership) | reasonix-state | todo | M2: docs/contracts/session-format.md §16.14 |
| SESS-059 | Session | Unreadable turn reservation | reasonix-state | todo | M2: docs/contracts/session-format.md §16.15 |
| SESS-060 | Session | Path validation, secure open (O_NOFOLLOW walk / Windows validate-then-plain), atomic JSON | reasonix-state | todo | M2: docs/contracts/session-format.md §16.16 |
| SESS-061 | Session | Inbox package scope and layout (manifest + blobs; admission via control.Controller) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.0 |
| SESS-062 | Session | Inbox types and wire shapes (states/dispositions, 13-error table, PromptEnvelope, InboxSnapshot) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.1 |
| SESS-063 | Session | Inbox manifest format + validation (512 receipts / 7d TTL / 1024-byte keys; 12 validation errors) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.2 |
| SESS-064 | Session | Inbox disk protocol (filelock 5s; loadOrInit branches incl. salvage, migration backfill, foreign RunID; commit refuses under unrecovered) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.3 |
| SESS-065 | Session | Inbox operations (enqueue order + rollback, delete/cancel scoping, claim/ack/retry/next-queued semantics) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.4 |
| SESS-066 | Session | Inbox held duplicates + idempotency receipts (alias = original client request) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.5 |
| SESS-067 | Session | Inbox reference freeze (no-symlink root, 100-entry cap, legacy v1 apply) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.6 |
| SESS-068 | Session | Inbox run identity, block codes, recovery (steer_unapplied retry-safe; orphan recovery via lock-free callback) | reasonix-state | todo | M2: docs/contracts/session-format.md §17.7 |
| SESS-069 | Session | Inbox process classification, metrics, PreviewText | reasonix-state | todo | M2: docs/contracts/session-format.md §17.8 |
| SESS-070 | Session | Memory store layout and resolution (project + global dirs) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.1 |
| SESS-071 | Session | Memory fact file format (frontmatter render/parse) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.2 |
| SESS-072 | Session | Memory enums and derived values (activation, importance, strength) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.3 |
| SESS-073 | Session | Memory save pipeline SaveWithOptions (revision CAS, staged/retired rename, pinned budget) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.4 |
| SESS-074 | Session | Memory crash recovery (interrupted rename sweep, restore staging) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.5 |
| SESS-075 | Session | Memory archive, restore, revisions (.archive, .revisions snapshots) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.6 |
| SESS-076 | Session | Memory migration v2 + legacy backup (O_EXCL + Link, never replace) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.7 |
| SESS-077 | Session | MEMORY.md index protocol (managed lines, hand-written preservation, rebuild) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.8 |
| SESS-078 | Session | Memory cross-process locking (2s wait, sha256-named lock files) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.9 |
| SESS-079 | Session | Memory usage sidecar .usage.json + retention math | reasonix-state | todo | M2: docs/contracts/session-format.md §18.10 |
| SESS-080 | Session | Memory freshness classification windows | reasonix-state | todo | M2: docs/contracts/session-format.md §18.11 |
| SESS-081 | Session | Automatic recall (defaults, suppression codes, BM25 weights, memory-recall block, local-home path) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.12 |
| SESS-082 | Session | Memory tools: output formats of memory / remember / forget / quick-add (complements TOOL rows; schemas live there) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.13 |
| SESS-083 | Session | Memory queue, suggester, auto-write claim context keys | reasonix-state | todo | M2: docs/contracts/session-format.md §18.14 |
| SESS-084 | Session | Memory prompt integration (Set/Compose/BackgroundBlock/memoryProtocol/PrefixCost) | reasonix-state | todo | M2: docs/contracts/session-format.md §18.15 |
| SESS-085 | Session | workspacelist serve-workspaces.json (legacy bare-array read, filelock 5s, .bak repair, MergeOnce) | reasonix-state | todo | M2: docs/contracts/session-format.md §19.1 |
| SESS-086 | Session | Serve workspace-list surface (capacity 409 workspace.limit_reached, ordering, add/keep/forget/move/place) | reasonix-serve | todo | M2: docs/contracts/session-format.md §19.2 |
| SESS-087 | Session | desktop-projects.json retired derived state (sole reader cli/catalogs.go; projects repair target) | reasonix-cli | todo | M2: docs/contracts/session-format.md §19.3 |
| SESS-088 | Session | Storage roots + StateRootEntries (25) + first-move subset [sessions, archive, stats, projects] | reasonix-config | todo | M2: docs/contracts/session-format.md §19.4 |
| SESS-089 | Session | Relocation adoption (AdoptRelocatedStateEntries; .adopted; event texts) | reasonix-state | todo | M2: docs/contracts/session-format.md §19.5 |
| SESS-090 | Session | Repair derived-state reset (tabs/projects/window/zoom; quarantine naming) | reasonix-platform | todo | M2: docs/contracts/session-format.md §19.6 |
| SESS-091 | Session | Scheduler folder source (Folders/Match; WorkspaceTrusted grants) | reasonix-agent | todo | M2: docs/contracts/session-format.md §19.7 |
| SESS-092 | Session | Wire log, trajectory replay, surface history (8MiB drop cap, 23 persisted kinds, witness merge, availability) | reasonix-serve | todo | M2: docs/contracts/session-format.md §20.2 |
| SESS-093 | Session | Adjudication journal (barrier records, Sync-before-observable, fold, inherited interruptions, context block) | reasonix-session | todo | M2: docs/contracts/session-format.md §20.3 |
| SESS-094 | Session | Execution journal format/ops/fold (open/queued/started; liveSet; settled reserved) | reasonix-state | todo | M2: docs/contracts/session-format.md §20.4 |
| SESS-095 | Session | Interrupted-execution context + ExecutionGraphSnapshot (journal-to-subagent join) | reasonix-session | todo | M2: docs/contracts/session-format.md §20.4 |
| SESS-096 | Session | Goal-state sidecar (goalState json, unknown-field round-trip, FSM transitions, migration, report errors) | reasonix-session | todo | M2: docs/contracts/session-format.md §20.5 |
| SESS-097 | Session | Snapshot conflict diagnostic (.conflicts.jsonl line schema, dedup, silent append) | reasonix-session | todo | M2: docs/contracts/session-format.md §20.6 |
| SESS-098 | Session | Subagent records: subagents/ layout, meta json, ListSubagentsByParent, InheritedContext/ContextCapsule | reasonix-state | todo | M2: docs/contracts/session-format.md §20.7 |
| SESS-099 | Session | History paging: historyReadIndex, ErrHistorySnapshotChanged/ErrHistoryPageRange, ReadHistoryToolOutput | reasonix-state | todo | M2: docs/contracts/session-format.md §20.7 |
| SESS-100 | Session | Sessionstore misc: steer/marker constants, session tail (16MiB), unread, grants (256/512), TurnWorkMs, load-context, normalize, auto-archive | reasonix-state | todo | M2: docs/contracts/session-format.md §20.7 |
| PMSG-001 | Provider | provider.Message wire shape (JSON keys, omitempty set) | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::message_full_field_order, message_omitempty_drops_zero_scalars_and_empty_slices, minimal_user_serializes_role_only, default_message_serializes_empty_role (2026-11-16) |
| PMSG-002 | Provider | Message decode semantics (tolerant, type errors fail record) | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::decode_type_mismatch_fails, decode_ignores_unknown_fields_and_missing_keys, decode_null_tolerates_every_field_class, decode_null_slice_elements_become_zero_values, decode_null_struct_fields_become_defaults, responses_items_keep_raw_bytes (2026-11-16) |
| PMSG-003 | Provider | Roles + local-only sentinel messages | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::role_const_partial_eq_and_roundtrip (2026-11-16) |
| PMSG-004 | Provider | ToolCall + ArgumentNormalization + ExecutionArguments | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::tool_call_shape_and_execution_arguments (2026-11-16) |
| PMSG-005 | Provider | Local metadata types (Via, DecisionReceipt, ToolExecution, ToolFailure, StreamFailureCause, interrupted_turn) | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::local_metadata_wire_shapes, stream_failure_cause_and_sentinels, decision_receipts_preserve_null_elements, diagnostic_error_display_and_source (2026-11-16) |
| PMSG-006 | Provider | VideoRef wire shape + video notes | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::video_ref_shape_notes_and_safe_name (2026-11-16) |
| PMSG-007 | Provider | agentcomposition wire shapes (Persona..Binding) | reasonix-contract | done | crates/contract/tests/provider_message_wire.rs::composition_wire_shapes, module_parameters_null_vs_absent, resource_data_defaults_to_null, binding_summary_projection, marshal_html_escapes_go_style (2026-11-16) |
| PMSG-008 | Provider | Digest / Revision / Seal | reasonix-contract | done | crates/contract/tests/agentcomposition.rs::revision_canonicalizes_parameter_objects, seal_excludes_started, builtin_revisions_match_kernel (2026-11-16)
| PMSG-009 | Provider | Live() + resource format validation | reasonix-contract | done | crates/contract/tests/agentcomposition.rs::binding_resource_formats, selection_allows (2026-11-16)
| PMSG-010 | Provider | Binding.Validate | reasonix-contract | done | crates/contract/tests/agentcomposition.rs::optional_tools_binding_round_trip, absent_optional_preserves_revision_and_binding_digest, retained_resource_limit_is_enforced, resource_without_data_is_invalid (2026-11-16)
| PMSG-011 | Provider | Definition.Validate | reasonix-contract | done | crates/contract/tests/agentcomposition.rs::definitions_validate_identity_and_selections, minimal_template_has_no_automation_or_delegation, optional_modules_decode_and_validate, module_parameters_group_rules (2026-11-16)
| PMSG-012 | Provider | Composition decode strictness | reasonix-contract | done | crates/contract/tests/agentcomposition.rs::decode_rejects_unknown_fields_and_arrays, decode_rejects_nested_unknown_fields, decode_rejects_trailing_data, decode_round_trips_builtins (2026-11-16)
| PMSG-013 | Provider | ParseParameters + module parameter rules + execution authority | reasonix-contract | done | crates/contract/tests/agentcomposition.rs::parse_parameters_enforces_reference_discipline, definition_authority_by_source_and_module (2026-11-16)
| PMSG-014 | Provider | NormalizeMessages (provider wire-safe) | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::sanitize_backfills_dangling_call, sanitize_keeps_call_order_and_multiple, sanitize_drops_orphan_tool_message, sanitize_leaves_well_formed_unchanged, sanitize_drops_local_only_interrupted_output, local_only_sentinel_is_safe_for_legacy_reader, sanitize_closes_truncated_args, sanitize_backfills_empty_name, sanitize_backfills_missing_tool_result_name, attached_decision_receipt_positional_pairing, duplicate_tool_call_ids_keep_each_result, empty_tool_call_ids_keep_each_result (2026-11-16)
| PMSG-015 | Provider | NormalizeSessionMessages (session view) | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::session_normalization_preserves_local_only_display, session_normalization_migrates_interleaved_decision_receipt, session_normalization_preserves_standalone_tool_message, session_normalization_preserves_extra_tool_result, session_normalization_keeps_empty_arguments, fast_path_returns_borrowed_only_when_untouched (2026-11-16)
| PMSG-016 | Provider | attachStandaloneDecisionReceipts | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::attach_standalone_decision_receipts_fast_path_borrows, attach_standalone_decision_receipts_folds_and_resets, session_normalization_migrates_interleaved_decision_receipt (2026-11-16)
| PMSG-017 | Provider | pairToolResults / sessionToolResults / idDistinct | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::pair_tool_results_by_id_and_placeholder, pair_tool_results_positional_overwrites_identity, session_tool_results_appends_placeholders_only, well_formed_and_id_distinct_primitives, sanitize_backfills_missing_tool_result_name, duplicate_tool_call_ids_keep_each_result, empty_tool_call_ids_keep_each_result (2026-11-16)
| PMSG-018 | Provider | backfillToolCallNames | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::backfill_by_id, backfill_positional, backfill_unpaired_stays_empty, backfill_no_empty_returns_input, sanitize_backfills_empty_name (2026-11-16)
| PMSG-019 | Provider | repairToolCallArgs / closeTruncatedJSON | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::close_truncated_json_repairs_streams, args_need_repair_modes, repair_tool_call_args_copy_on_write, sanitize_closes_truncated_args, session_normalization_keeps_empty_arguments (2026-11-16)
| PMSG-020 | Provider | tryNormalizeFastPath (zero-alloc passthrough) | reasonix-contract | done | crates/contract/tests/provider_normalization.rs::fast_path_returns_borrowed_only_when_untouched, sanitize_leaves_well_formed_unchanged, session_normalization_preserves_standalone_tool_message, attach_standalone_decision_receipts_fast_path_borrows, backfill_no_empty_returns_input (2026-11-16)

## Status summary

Counts: done 69, wip 1, todo 493 (563 rows).

The CLI contract is frozen in docs/contracts/cli.md (rows CLI-001..CLI-050).
The config contract is frozen in docs/contracts/config.md (rows
CONFIG-001..CONFIG-035). The HTTP contract is frozen in
docs/contracts/http.md (rows ROUTE-001..ROUTE-033). The SSE event
contract is frozen in docs/contracts/sse.md (rows SSE-001..SSE-022). The
ACP contract is frozen in docs/contracts/acp.md (rows ACP-001..ACP-046).
The MCP client contract is frozen in docs/contracts/mcp.md (rows
MCP-001..MCP-038). The extension v2 contract is
frozen in docs/contracts/ext.md (rows EXT-001..EXT-046). The state/SQLite
contract is frozen in docs/contracts/state.md (rows STATE-001..STATE-030).
The tools contract is frozen in docs/contracts/tools.md (rows
TOOL-001..TOOL-057). The host handshake contract is frozen in
docs/contracts/host.md (rows HOST-001..HOST-038). The paths and
environment contract is frozen in docs/contracts/paths.md (rows
PATHS-001..PATHS-036). All M1 contract inventories are now frozen.

## Conventions

- Evidence for done rows must name a test or a command a reviewer can
  re-run; keep the date so stale evidence stays visible.
- Never edit expected outputs to make a gate green: explain the difference
  in docs/parity/BREAKS.md and adjust the row status accordingly.
- Do not mark rows done from reading the reference source alone; run the
  reference in an isolated REASONIX_HOME when behavior is uncertain.
| SAFETY-001 | Safety | permrule syntax: three parse forms, malformed rejected | reasonix-safety | done | crates/safety/src/permission/permrule.rs::tests::parse_rule_forms (Go TestParseRule 17 cases) (2026-11-20) |
| SAFETY-002 | Safety | Decision tri-state + ParseDecision conservative default + subject extraction | reasonix-safety | done | crates/safety/src/permission/decision.rs tests (parse_decision_table, match_glob_table, subject_extraction, subjects_move_file_checks_both_endpoints, message_and_resource_subjects, dispatch_subject_is_project_and_whole_text) (2026-11-20) |
| SAFETY-003 | Safety | Policy pure rule evaluation (precedence, session-allow, PowerShell bare names, literal rules, provenance) | reasonix-safety | done | crates/safety/src/permission/policy.rs tests (policy_decide_table, session_allow_precedence, exact_and_literal_rules_match_exactly, bash_command_prefix_* etc.) + vocabulary.rs tests (2026-11-20) |
| SAFETY-004 | Safety | Gate: Approver resolution, denial naming, read-only refusal, unattended posture | reasonix-safety | done | crates/safety/src/permission/gate.rs tests (headless_gate_resolves_ask_but_not_deny, interactive_gate_remember_and_decline, verdict_names_who_refused, widening_is_refused_when_there_is_nobody_to_ask etc.) (2026-11-20) |
| SAFETY-005 | Safety | Write fence: subject-scoped grants, install plan, egress/signin, bare-grant narrowing | reasonix-safety | done | crates/safety/src/permission/write_fence.rs tests (subject_scoped_grants_name_their_subject, bare_grants_no_longer_answer_for_every_subject etc.) (2026-11-20) |
| SAFETY-006 | Safety | Session grant validity (host-recorded rules only) | reasonix-safety | done | crates/safety/src/permission/session_grant_valid.rs tests (every_rule_the_host_records_is_a_valid_grant, rules_the_host_could_not_have_written_are_rejected) (2026-11-20) |
| SAFETY-007 | Safety | Inline-code argv table + interpreter set + indirect-execution blocker (pure argv half) | reasonix-safety | done | crates/safety/src/permission/inline_code.rs tests + bash.rs tests (indirect_execution_table, environment_assignments, interpreter_set) (2026-11-20) |
| SAFETY-008 | Safety | Bash approval/readonly/network at AST level (wrapper peeling, compound leaves, redirect normalization) | reasonix-safety | done | bash_anchors unignored: wrapped_program_matches_deny_rule, policy_decide_compound_bash, bash_subject_requires_explicit_approval, is_read_only_bash_subject, bash_read_only_proof_names_what_stopped_the_command, gate_reports_matched_permission_rule, normalize_bash_safe_redirects_for_match all green over the AST engine (2026-11-20) |
| SAFETY-009 | Safety | Network-path operand spelling (//host, backslash-host, device-path forms) | reasonix-safety | done | network_path_shell_commands_ask_in_default_mode green on Windows (word skeletons, provider prefixes, brace alternatives); off-Windows unchanged test preserved (2026-11-20) |
