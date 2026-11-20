# Reasonix — Rust kernel

Rust rewrite of the Go kernel behind DeepSeek-Reasonix-studio: same CLI,
HTTP/SSE, ACP, MCP, configuration, SQLite data, and extension protocol. The
React/Electron desktop frontend and the Cloudflare Workers are kept as-is;
reference/DeepSeek-Reasonix-studio is the read-only behavioral oracle.

Status: **M0 — workspace bootstrap complete.** Next milestone is M1, the
contract inventory freeze (AGENTS.md §5.1). Track behavior parity in
docs/parity/LEDGER.md; architecture decisions live under docs/adr/.

## Layout

| Path                        | Purpose                                        |
|-----------------------------|------------------------------------------------|
| apps/reasonix               | reasonix binary (replaces cmd/reasonix)        |
| apps/reasonix-studio-host   | Studio loopback host (cmd/reasonix-studio-host)|
| crates/…                    | kernel libraries, layered per AGENTS.md §4.2   |
| xtask                       | repo automation: parity + dependency gates     |
| docs/parity/LEDGER.md       | behavior parity ledger (xtask parity --check)  |
| docs/adr                    | architecture decision records                  |
| reference/                  | read-only Go oracle (gitignored, not shipped)  |

## Build

    cargo build --workspace --release

## Quality gates (AGENTS.md §8.6)

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo xtask parity --check
    cargo xtask deps --check

cargo xtask is a workspace alias (see .cargo/config.toml) that runs the
automation crate in release mode.

## Conventions

- Behavior parity with the reference implementation rules every change; see
  AGENTS.md at the repository root.
- Every crate forbids unsafe code; exceptions will come later only in
  reasonix-platform and reasonix-safety, under review.
- The workspace currently has zero external dependencies by design
  (ADR-0001): no supply-chain surface until the M1 contract freeze.

License: GPL-3.0-only; see the repository LICENSE and NOTICE.
