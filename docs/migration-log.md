# Migration log

Chronological record of what each wave did, with the command output that proves
it. Newest wave last.

## Wave 1 — inventory and rulings (2026-11-16)

### Inventory

- Trunk `C:\Users\10129\Desktop\Projects\DeepSeek-Reasonix-rs`: Go kernel complete,
  Electron/React baseline complete, `desktop-tauri` Vue port at 196 `.vue` /
  736 `.ts` / 267 test files.
- A `C:\Users\10129\Downloads\DeepSeek-Reasonix-rs\DeepSeek-Reasonix-rs`: a Gitee
  mirror three commits behind trunk's Tauri work. Diff shows only
  `SidebarNav.tsx`, `wire.parity.test.ts`, `App.vue`, `stores/`,
  `port/kernel.ts` as A-only — all superseded on trunk.
- B `C:\Users\10129\Downloads\DeepSeek-Reasonix-studio-RS`: one commit, 438 lines
  of Rust, empty parity ledger past `CLI-004`. A Go→Rust kernel rewrite, which is
  the opposite of this migration's backend decision.

### Baseline verification

```
cd desktop-tauri && pnpm typecheck
$ vue-tsc -b --force
(no diagnostics, exit 0)

cd desktop-tauri && pnpm test
 Test Files  267 passed (267)
      Tests  1952 passed (1952)
   Duration  72.11s
```

### Rulings written

`docs/decisions.md` D1–D7: trunk, A absorption, B absorption (declined),
Tauri 2, Vue Composition API with no router and no Pinia, HTTP/SSE for the
kernel with Tauri commands only for shell capabilities, parallel subagents with
disjoint file ownership.

### Gap analysis

Compared 330 React component names against 196 Vue components. The measurable
losses, in the order a user meets them: settings sheet (placeholder div in
`App.vue`), workspace sidebar, chrome title bar, onboarding, six panels, and the
browser/remotes surfaces. Recorded in `migration-state.md`.
