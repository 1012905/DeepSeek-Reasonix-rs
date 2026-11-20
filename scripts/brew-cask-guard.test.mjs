import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { caskDecision } from "./brew-cask-guard.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const cask = (v) => `cask "reasonix" do\n  version "${v}"\nend\n`;

test("a candidate equal to or newer than the tap's cask is written", () => {
  assert.equal(caskDecision(cask("2.27.0"), "2.27.0"), "write");
  assert.equal(caskDecision(cask("1.39.7"), "2.28.0"), "write");
  assert.equal(caskDecision(cask("2.9.0"), "2.10.0"), "write");
});

test("an older candidate never moves the cask backwards", () => {
  assert.equal(caskDecision(cask("2.28.0"), "2.27.0"), "skip");
  assert.equal(caskDecision(cask("2.10.0"), "2.9.0"), "skip");
  assert.equal(caskDecision(cask("2.28.0"), "2.28.0-preview.1"), "skip");
});

test("a cask without a readable version fails closed", () => {
  assert.throws(() => caskDecision("cask \"reasonix\" do\nend\n", "2.28.0"), /no readable version/);
});

test("the command prints the decision and fails with an error annotation when unreadable", () => {
  const script = path.join(here, "brew-cask-guard.mjs");
  const dir = mkdtempSync(path.join(tmpdir(), "cask-"));
  const file = path.join(dir, "reasonix.rb");
  writeFileSync(file, cask("2.28.0"));
  const run = (f, v) => spawnSync("node", [script, f, v], { encoding: "utf8" });
  assert.equal(run(file, "2.27.0").stdout.trim(), "skip");
  const missing = run(path.join(dir, "absent.rb"), "2.27.0");
  assert.notEqual(missing.status, 0);
  assert.match(missing.stderr, /::error::/);
});

test("the rendered cask's postflight_steps use the install-steps DSL, not cask methods", () => {
  const dir = mkdtempSync(path.join(tmpdir(), "cask-render-"));
  const sums = path.join(dir, "SHA256SUMS");
  const names = ["darwin-arm64.tar.gz", "darwin-amd64.tar.gz", "linux-arm64.tar.gz", "linux-amd64.tar.gz"];
  writeFileSync(sums, names.map((n) => `${"a".repeat(64)}  reasonix-${n}`).join("\n") + "\n");
  const out = spawnSync("bash", [path.join(here, "render-brew-cask.sh"), "2.27.0", "v2.27.0", sums], { encoding: "utf8" });
  assert.equal(out.status, 0, out.stderr);
  const block = out.stdout.match(/postflight_steps do\n([\s\S]*?)\n  end\n/)?.[1];
  assert.ok(block, "no postflight_steps block");
  assert.doesNotMatch(block, /system_command|#\{/);
  assert.match(block, /run "\/usr\/bin\/xattr".*\{\{staged_path\}\}\/reasonix/);
});
