---
owner: @SivanCola
backup: @esengine
status: active
reviewed: 2026-11-13
---

# Rust + Vue + Tauri rewrite

The standing plan for rewriting Reasonix from Go + Electron + React to Rust +
Vue + Tauri. It records the decisions the rewrite runs on and the layer mapping
the port follows. It is not a schedule; the phases live in the task list, this
file holds what must stay true while they execute.

> **Implementation adopted (D14, 2026-11-18):** the concrete Rust kernel is
> upstream B (Gitee `rikkarikka/reasonix-studio-rs`, M2), merged into `crates/`
> + `apps/` + `xtask/`. Its crate layout (`docs/adr/0001`) refines the mapping
> below; its frozen wire contracts live in `docs/contracts` and its acceptance
> ledger in `docs/parity/LEDGER.md`. The Go kernel stays the live backend until
> parity sign-off (decision 5). The earlier A-lineage skeleton is superseded;
> its `contract` eventwire/pricing golden slices are preserved in `crates/contract`.

## Decisions

### 1. The kernel stays a separate process

The Tauri shell spawns the Rust kernel binary as a sidecar, exactly as the
Electron main process spawns `cmd/reasonix-studio-host` today. The kernel is
not linked into the shell process.

What this preserves without re-derivation:

- Process isolation: a kernel crash takes down no window, and the shell can
  restart or report it the way `hostprobe` and `crashreport` do now.
- The loopback gate: `tcp4 127.0.0.1:0`, never a wildcard.
- The loopback gate: a credential minted per launch from a CSPRNG, never
  read from config and never persisted.
- The loopback gate: an HttpOnly SameSite=Strict host-only cookie set by the
  shell before anything loads.
- The loopback gate: exact `Host`, and exact `Origin` on anything that
  changes state, checks outermost first.
- The webview is still untrusted code; the socket still carries the boundary
  the host owns.
- `reasonix-kernel` strips `[serve]` authentication from its own config, for
  the same reason `reasonix-studio-host` does: Studio's boundary is the
  launch credential, and a configured auth mode cannot share one cookie with
  it.
- The lease and health-check protocol between shell and kernel.

### 2. Wire compatibility is proven against Go

The HTTP/SSE surface is the contract. Rust answers the same routes with the
same bytes for the recorded cases.

Both directions of every wire type are held by a parity gate: a field one
side sends and the other cannot read fails, and so does one it reads that
nothing sends. The Go↔TS gate (`tools/repolint/wireparity.go`) is replaced
by a Rust↔TS one; the TS side is unchanged and stays the reference.

The recorded cases are golden fixtures: representative requests and responses
plus SSE stream samples captured against the Go kernel, replayed against Rust
in CI. Rust is not done with a layer until its slice of the fixtures matches.

### 3. Sessions stay JSONL; the SQLite projections stay rebuildable

The session store is append-only JSONL — a transcript plus sidecars such as
`.events.jsonl` and `.guardian.jsonl` — and `internal/state/store` is the
single authority for every path on disk.

The three SQLite databases (history search, usage catalog, projections) are
disposable: `projectiondb` owns their lifecycle on a declared contract that
business data stays authoritative outside them, and any of them can be
discarded and rebuilt.

The Rust port keeps both halves:

- It reads and writes the same JSONL files and JSON sidecars, carrying over
  the format versions `sessionstore/migrate.go` migrates.
- It treats the SQLite projections as rebuildable: a database the Go build
  wrote may be discarded and rebuilt rather than opened in place.

The compatibility test opens a session directory produced by the Go build
before any Rust `state` work lands. `store`'s layout must be reproduced byte
for byte: a layout the Rust side spells differently strands the user's
history under a path nothing reads.

### 4. Frontends port in a fixed order

`serve` first (everything else sits on it), then the Tauri shell, then the
chat CLI and TUI (`clap` + `ratatui`), then ACP (editor integration).

The Go development gates (`tools/repolint`, `covergate` and siblings) are not
ported: they are development tooling, not delivery surface. Their replacement
is `clippy` plus the smallest self-written checks the new invariants need,
grown when an invariant earns a gate, not up front.

### 5. Both stacks coexist until parity sign-off

The new stack lives in `crates/` (kernel workspace) and `desktop-tauri/`
(shell + frontend). The Go kernel, the Electron shell and `frontend-next`
stay runnable the whole time; nothing is deleted before the parity checklist
passes and a real end-to-end task has been driven through the new stack.

## Golden fixtures

`scripts/record-golden.mjs` records them: it builds `cmd/reasonix-studio-host`,
spawns it under an isolated state home, reads the launch handshake from stdout
(`{version, origin, token}`, version 1), and replays each case through the
loopback gate.

Fixtures live in `crates/serve/fixtures/golden/manifest.json`, one object per
case: the request (method, path, headers, body) and the response (status,
headers, body), or the first SSE frames.

Three placeholders keep a fixture replayable against any launch:
`__CREDENTIAL__` for the cookie value, `http://127.0.0.1:__PORT__` for the
origin, nothing else — a fixture that needs a fourth placeholder is a case
that needs rethinking.

The first cases are the gate itself, because a refusal is contract too:
`loopback.host_rejected`, `loopback.origin_rejected` and
`loopback.unauthorized` each arrive as a status plus a dotted code, and the
Rust gate must produce the same bytes for the same bad request.

Business cases (`/runtimes`, `/events`, an unknown route) start the wider
set, which grows per layer.

## Process topology

```
┌───────────────────────────────────────────────────────────────┐
│  desktop-tauri renderer (Vue 3 + Naive UI, Vite)              │
│    fetch("/…") · EventSource("/events")                       │
└───────────────▲────────────────────────────┬──────────────────┘
                │ HTTP + SSE over 127.0.0.1  │ Tauri IPC:
┌───────────────┴────────────────────────────▼──────────────────┐
│  Tauri 2 shell (Rust)                                         │
│    windows, tray, menus, dialogs, platform opener,            │
│    cookie injection, sidecar spawn + lease                    │
│    owns OS capability and no business state                   │
└───────────────▲───────────────────────────────────────────────┘
                │ spawns, holds the lease
┌───────────────┴───────────────────────────────────────────────┐
│  reasonix-studio-host (Rust bin, apps/reasonix-studio-host)   │
│    assembly::Build → session::Controller                      │
│    (same assembly shape the Go CLI uses)                      │
│    + serve (HTTP/SSE, loopback gate)                          │
└───────────────────────────────────────────────────────────────┘
```

The renderer never learns which shell it is in, and business state has exactly
one writer — the kernel — as before.

## Layer mapping

Upstream B's crate layout (`docs/adr/0001`); `crates/testkit` is test-only.

| Go | Rust |
| --- | --- |
| `internal/base/*` | `crates/core` |
| `internal/contract/*` | `crates/contract` |
| `internal/state/*` | `crates/state` |
| `internal/safety/*` | `crates/safety` |
| `internal/model/*` | `crates/provider` |
| `internal/tools/*` | `crates/tools` |
| `internal/runtime/*` | `crates/agent` |
| `internal/session/control` | `crates/session` |
| `internal/frontend/serve` | `crates/serve` |
| `internal/frontend/cli` | `crates/cli` |
| `internal/frontend/tui` | `crates/tui` |
| `internal/frontend/acp` | `crates/acp` |
| `internal/assembly/boot` | `crates/assembly` |
| `internal/ext/*` | `crates/ext` |
| `internal/platform/*` | `crates/platform` |
| config schema | `crates/config` |
| `cmd/reasonix` | `apps/reasonix` |
| `cmd/reasonix-studio-host` | `apps/reasonix-studio-host` |
| `desktop/electron` | `desktop-tauri` (Tauri 2, Rust) |
| `desktop/frontend-next` | `desktop-tauri` (Vue 3) |

Workspace-wide dependencies are pinned once in the root `Cargo.toml`: one
edition (2024), one MSRV (1.99.1). The frozen contract inventory has entered
product deps (`serde`, `serde_json`, `sha2`, `thiserror`, `chrono`,
`encoding_rs`, `fs4`, `regex`, `getrandom`, `hostname`, `caseless`,
`unicode-*`, `tracing`); the async/HTTP/SQLite stack (`tokio`, `axum`,
`rusqlite`, `reqwest` with rustls) lands with the `serve` slice (M4), not yet.
`core` imports no other crate; only frontends and hosts reach `session`; that
is the Go layering carried over, held by `cargo xtask deps --check`.
