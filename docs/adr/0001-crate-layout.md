# ADR 0001 — Crate layout and dependency policy

- Status: accepted
- Date: 2026-11-13

## Context

AGENTS.md §4.2 fixes the dependency direction for the rewrite and forbids
cycles, and §1.3 states the Rust tree does not need to mirror the Go package
layout. M0 bootstraps the workspace, so crate boundaries, layering, and
enforcement have to be frozen before any business code lands. The Go tree
under reference/DeepSeek-Reasonix-studio is the behavioral oracle, not a
structural template.

## Decision

The workspace contains 19 packages plus the xtask automation crate:

| Package                    | Maps to reference (Go)                    | Layer |
|----------------------------|-------------------------------------------|-------|
| reasonix-core              | internal/base                             | 0     |
| reasonix-contract          | internal/contract (minus config)          | 1     |
| reasonix-platform          | internal/platform                         | 2     |
| reasonix-config            | internal/contract/config                  | 2     |
| reasonix-safety            | internal/safety                           | 3     |
| reasonix-state             | internal/state                            | 3     |
| reasonix-provider          | internal/model                            | 4     |
| reasonix-tools             | internal/tools                            | 4     |
| reasonix-ext               | internal/ext                              | 4     |
| reasonix-agent             | internal/runtime                          | 5     |
| reasonix-session           | internal/session                          | 6     |
| reasonix-cli               | internal/frontend/cli                     | 7     |
| reasonix-tui               | internal/frontend/tui, termrender         | 7     |
| reasonix-serve             | internal/frontend/serve                   | 7     |
| reasonix-acp               | internal/frontend/acp                     | 7     |
| reasonix-assembly          | internal/assembly                         | 8     |
| reasonix-testkit           | (new; test-only helpers)                  | n/a   |
| reasonix (apps/)           | cmd/reasonix                              | 9     |
| reasonix-studio-host (apps/) | cmd/reasonix-studio-host                | 9     |

Layer graph, top to bottom (identical to AGENTS.md §4.2):

    apps
      -> assembly
          -> cli / tui / serve / acp
          -> session
          -> agent
          -> ext
          -> platform

    session -> agent -> {tools, provider, state, safety, contract}
    cli -> {config, tui, serve, acp, session, agent, ext, ...}
    tools / provider -> {safety, contract, platform}
    state / safety / config -> {contract, platform}
    platform -> contract -> core

Key choices:

1. assembly is the composition root: the two binaries in apps/ depend on
   reasonix-assembly only, mirroring how the reference cmd/ entry points
   wire the kernel.
2. cli dispatches into the sibling frontends tui/serve/acp, mirroring the
   verb dispatch in internal/frontend/cli/cli.go. Frontends never depend on
   each other otherwise.
3. config is its own crate (reference internal/contract/config) so every
   frontend can read configuration directly; assembly does not depend on
   config, it reaches it through the frontend crates.
4. Naming follows the domain, not the Go tree: provider maps to
   internal/model, agent maps to internal/runtime, core maps to
   internal/base.
5. Version injection mirrors Go ldflags -X: compile-time environment
   variables REASONIX_VERSION, REASONIX_GIT_COMMIT, REASONIX_BUILD_TIME_UTC,
   REASONIX_BUILD_TARGET with dev defaults, consumed by
   reasonix_cli::BuildInfo::from_build().
6. M0 policy: zero external dependencies anywhere in the workspace,
   including xtask. The supply-chain surface stays empty until the M1
   contract inventory is frozen; every later dependency needs justification
   per AGENTS.md §10.6.
7. forbid(unsafe_code) is the default in every crate. The planned exceptions
   (reasonix-platform, reasonix-safety) will switch to
   deny(unsafe_op_in_unsafe_fn) with per-block SAFETY comments when platform
   code lands (AGENTS.md §3.4).

Enforcement:

- cargo xtask deps --check walks every member manifest with a fail-closed
  TOML subset parser: unknown packages, unregistered names, and malformed
  manifests are errors, never warnings.
- The allowed graph lives in xtask/src/deps.rs (policy) and must stay in
  sync with this ADR. Adding a crate means updating this ADR, the REGISTERED
  list, policy(), and the workspace members list.
- reasonix-testkit is test-only: exempt from direction rules, allowed solely
  as a dev-dependency or build-dependency.

## Consequences

- The package count (19) is larger than a flat design would need; the payoff
  is that the AGENTS.md §4.2 layering is mechanically enforced rather than
  aspirational.
- The TOML subset parser rejects multi-line inline tables on purpose:
  manifests must stay boring and greppable.
- Stub crates carry empty libs today; their lib.rs doc headers record the
  reference mapping so M1+ work can proceed one vertical slice at a time.
