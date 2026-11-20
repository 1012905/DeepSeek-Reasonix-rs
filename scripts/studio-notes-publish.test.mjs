import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const indexScript = path.join(dir, "update-versions-index.sh");
const backfill = path.join(dir, "backfill-studio-notes.sh");

function index(existing, ...args) {
  const run = spawnSync("bash", [indexScript, existing, ...args], { encoding: "utf8" });
  return { status: run.status, stderr: run.stderr, json: run.status === 0 ? JSON.parse(run.stdout) : null };
}

const NOTES = "https://github.com/1012905/DeepSeek-Reasonix-rs/releases/download/studio-v2.32.0/notes.md";

test("an entry carries notes only when a notes address is given", () => {
  const withNotes = index("-", "v2.32.0", "studio-v2.32.0", "studio", "2026-10-09T00:00:00Z", "20", NOTES);
  assert.equal(withNotes.json.versions[0].notes, NOTES);
  const without = index("-", "v2.32.0", "studio-v2.32.0", "studio", "2026-10-09T00:00:00Z");
  assert.equal("notes" in without.json.versions[0], false);
});

test("a notes address outside the release notes assets is refused", () => {
  for (const bad of [
    "https://example.com/2.32.0.md",
    "https://dl.reasonix.io/studio/notes/2.32.0.md",
    "https://github.com/1012905/DeepSeek-Reasonix-rs/releases/download/studio-v2.32.0/latest.json",
    "https://github.com/evil/repo/releases/download/studio-v2.32.0/notes.md",
  ]) {
    const run = index("-", "v2.32.0", "studio-v2.32.0", "studio", "2026-10-09T00:00:00Z", "20", bad);
    assert.notEqual(run.status, 0);
  }
});

test("a rerun never rewrites an existing entry, notes included", () => {
  const first = index("-", "v2.32.0", "studio-v2.32.0", "studio", "2026-10-09T00:00:00Z");
  const dirPath = mkdtempSync(path.join(tmpdir(), "idx-"));
  const file = path.join(dirPath, "c.json");
  writeFileSync(file, JSON.stringify(first.json));
  const again = index(file, "v2.32.0", "studio-v2.32.0", "studio", "2026-10-10T00:00:00Z", "20", NOTES);
  assert.equal(again.json.versions.length, 1);
  assert.equal("notes" in again.json.versions[0], false);
});

test("website publication waits for its own deployment and propagates dispatch, API and deployment failures", () => {
  const root = mkdtempSync(path.join(tmpdir(), "website-publish-"));
  const calls = path.join(root, "calls");
  writeFileSync(path.join(root, "gh"), `#!/bin/sh
echo "$*" >> "$CALLS"
case "$1" in
  workflow) exit "$DISPATCH_STATUS" ;;
  api) [ "$API_STATUS" = 0 ] || exit "$API_STATUS"; printf '%s' "$RUN_ID" ;;
  run) exit "$WATCH_STATUS" ;;
  *) exit 99 ;;
esac
`);
  chmodSync(path.join(root, "gh"), 0o755);
  writeFileSync(path.join(root, "sleep"), "#!/bin/sh\nexit 0\n");
  chmodSync(path.join(root, "sleep"), 0o755);
  const run = (extra = {}) => spawnSync("bash", [path.join(dir, "publish-studio-changelog.sh")], {
    encoding: "utf8", env: { ...process.env, PATH: `${root}:${process.env.PATH}`, CALLS: calls,
      GITHUB_REPOSITORY: "example/reasonix", GITHUB_RUN_ID: "456", GITHUB_RUN_ATTEMPT: "2", VERSION: "v2.45.0",
      DISPATCH_STATUS: "0", API_STATUS: "0", WATCH_STATUS: "0", RUN_ID: "123", ...extra },
  });
  assert.equal(run().status, 0);
  const log = readFileSync(calls, "utf8");
  assert.match(log, /workflow run pages\.yml --repo example\/reasonix --ref website/);
  assert.match(log, /studio_version=2\.45\.0/);
  assert.match(log, /release_request=studio-456-2/);
  assert.match(log, /endswith\(" \[studio-456-2\]"\)/);
  assert.match(log, /run watch 123 .*--exit-status/);
  for (const extra of [{ DISPATCH_STATUS: "1" }, { API_STATUS: "1" }, { WATCH_STATUS: "1" }, { RUN_ID: "" }, { VERSION: "v2.46.0-preview.1" }]) {
    assert.notEqual(run(extra).status, 0, JSON.stringify(extra));
  }
});

test("the notes backfill is retired: the release workflow publishes notes as release assets", () => {
  const run = spawnSync("bash", [backfill], { encoding: "utf8" });
  assert.notEqual(run.status, 0);
  assert.match(run.stderr, /retired/);
  assert.match(run.stderr, /release-studio-tauri\.yml/);
});

test("the notes backfill refuses --apply as well, so no R2 write can be attempted", () => {
  const run = spawnSync("bash", [backfill, "--apply"], { encoding: "utf8" });
  assert.notEqual(run.status, 0);
  assert.match(run.stderr, /retired/);
});
