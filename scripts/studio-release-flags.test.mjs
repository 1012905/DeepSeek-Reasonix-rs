import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "studio-release-flags.sh");

function flags(tag, candidate, tags) {
  const out = spawnSync("bash", [script, tag, candidate], { encoding: "utf8", input: tags.join("\n") + "\n" });
  assert.equal(out.status, 0, out.stderr);
  return out.stdout.trim().split("\n");
}

test("the first release of a new major line takes latest past a retired line", () => {
  assert.deepEqual(flags("studio-v0.1.0", "false", ["studio-v2.61.0", "studio-v0.1.0"]), ["--latest"]);
});

test("a higher tag of the same line wins, an older tag does not take the badge back", () => {
  assert.deepEqual(flags("studio-v0.2.0", "false", ["studio-v0.1.0", "studio-v0.2.0"]), ["--latest"]);
  assert.deepEqual(flags("studio-v0.1.0", "false", ["studio-v0.1.0", "studio-v0.2.0"]), ["--latest=false"]);
});

test("a prerelease takes neither latest nor the catalog", () => {
  assert.deepEqual(flags("studio-v0.3.0-rc1", "true", ["studio-v0.2.0", "studio-v0.3.0-rc1"]), ["--prerelease", "--latest=false"]);
});
