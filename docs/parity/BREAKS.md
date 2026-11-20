# Known Reference Breaks and Quirks

Findings in the reference implementation that a Rust port must handle
deliberately. Default policy (AGENTS.md 3.1): keep behavior parity unless the
finding is a security vulnerability.

| ID | Finding | Impact | Decision |
|---|---|---|---|
| BREAK-001 | /plan-exec (session/control/turn_submit.go:535-536) and /prometheus (session/control/controller.go:697) guidance instructs the model to use parallel_tasks, which is not a registered tool (docs/contracts/tools.md 3.9) | A model following the guidance gets an unknown-tool failure instead of parallel sub-agents; the failure path itself is the observable behavior | Keep text parity: reproduce the guidance strings byte-for-byte in Rust and fail identically through the unknown-tool path; do not register parallel_tasks |
