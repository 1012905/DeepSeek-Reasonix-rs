import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const script = path.join(here, "publish-cli-pointer.sh");
const repo = "esengine/DeepSeek-Reasonix";
const bucket = "bkt";

const names = [
  "reasonix-darwin-amd64.tar.gz",
  "reasonix-darwin-arm64.tar.gz",
  "reasonix-linux-amd64.tar.gz",
  "reasonix-linux-arm64.tar.gz",
  "reasonix-windows-amd64.zip",
  "reasonix-windows-arm64.zip",
];

// A stand-in for the two CLIs the script drives. gh keeps releases in
// gh.json, aws keeps objects under r2/<bucket>/; both append to calls.log.
const ghStub = `#!/usr/bin/env node
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const dir = process.env.STUB_DIR;
const args = process.argv.slice(2);
fs.appendFileSync(path.join(dir, "calls.log"), "gh " + args.join(" ") + "\\n");
const db = path.join(dir, "gh.json");
const releases = fs.existsSync(db) ? JSON.parse(fs.readFileSync(db, "utf8")) : {};
const repo = ${JSON.stringify(repo)};
if (args[0] === "api") {
  const tag = args[1].split("/releases/tags/")[1];
  if (!releases[tag]) { console.error("gh: Not Found (HTTP 404)"); process.exit(1); }
  console.log(JSON.stringify(releases[tag].json));
} else if (args[0] === "release" && args[1] === "download") {
  const tag = args[2];
  fs.writeFileSync(args[args.indexOf("--output") + 1], releases[tag].sums);
} else if (args[0] === "release" && args[1] === "create") {
  if (process.env.STUB_GH_FAIL) { console.error("boom"); process.exit(1); }
  const tag = args[2];
  const files = args.slice(3).filter((a) => !a.startsWith("-") && fs.existsSync(a) && fs.statSync(a).isFile());
  const flags = args.filter((a) => a.startsWith("--"));
  releases[tag] = {
    sums: fs.readFileSync(files.find((f) => path.basename(f) === "SHA256SUMS"), "utf8"),
    flags,
    json: {
      tag_name: tag,
      draft: false,
      prerelease: flags.includes("--prerelease"),
      html_url: "https://github.com/" + repo + "/releases/tag/" + tag,
      assets: files.map((f) => ({
        name: path.basename(f),
        size: fs.statSync(f).size,
        state: "uploaded",
        digest: "sha256:" + crypto.createHash("sha256").update(fs.readFileSync(f)).digest("hex"),
        browser_download_url: "https://github.com/" + repo + "/releases/download/" + tag + "/" + path.basename(f),
      })),
    },
  };
  fs.writeFileSync(db, JSON.stringify(releases));
} else { console.error("unexpected gh call"); process.exit(2); }
`;

const awsStub = `#!/usr/bin/env node
const fs = require("fs");
const path = require("path");
const dir = process.env.STUB_DIR;
const args = process.argv.slice(2);
const [src, dst] = [args[2], args[3]];
const key = (u) => path.join(dir, "r2", u.slice("s3://".length));
if (src.startsWith("s3://")) {
  fs.appendFileSync(path.join(dir, "calls.log"), "aws get " + src + "\\n");
  if (!fs.existsSync(key(src))) { console.error("An error occurred (404) when calling the HeadObject operation: Key does not exist"); process.exit(1); }
  fs.copyFileSync(key(src), dst);
} else {
  fs.appendFileSync(path.join(dir, "calls.log"), "aws put " + dst + "\\n");
  if (process.env.STUB_R2_FAIL_PUT) { console.error("denied"); process.exit(1); }
  fs.mkdirSync(path.dirname(key(dst)), { recursive: true });
  fs.copyFileSync(src, key(dst));
}
`;

function world() {
  const dir = mkdtempSync(path.join(tmpdir(), "cli-pointer-"));
  const bin = path.join(dir, "bin");
  mkdirSync(bin);
  for (const [name, body] of [["gh", ghStub], ["aws", awsStub]]) {
    writeFileSync(path.join(bin, name), body);
    chmodSync(path.join(bin, name), 0o755);
  }
  const archives = path.join(dir, "archives");
  mkdirSync(archives);
  const sums = [];
  for (const name of names) {
    const body = Buffer.from(`archive:${name}`);
    writeFileSync(path.join(archives, name), body);
    sums.push(`${createHash("sha256").update(body).digest("hex")}  ${name}`);
  }
  writeFileSync(path.join(archives, "SHA256SUMS"), sums.join("\n") + "\n");
  const origin = path.join(dir, "origin.git");
  const work = path.join(dir, "work");
  const git = (cwd, ...args) => execFileSync("git", args, { cwd, encoding: "utf8" }).trim();
  mkdirSync(origin);
  git(origin, "init", "-q", "--bare");
  mkdirSync(work);
  git(work, "init", "-q");
  git(work, "remote", "add", "origin", origin);
  git(work, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "studio");
  const sha = git(work, "rev-parse", "HEAD");
  return { dir, bin, archives, work, sha, git: (...args) => git(work, ...args) };
}

function run(w, version, extra = {}) {
  const { tagged = true, ...env } = extra;
  if (tagged && !w.git("ls-remote", "--tags", "origin", `refs/tags/v${version.replace(/^v/, "")}`)) {
    w.git("tag", `v${version.replace(/^v/, "")}`, w.sha);
    w.git("push", "-q", "origin", `v${version.replace(/^v/, "")}`);
  }
  return spawnSync("bash", [script], {
    encoding: "utf8",
    cwd: w.work,
    env: {
      ...process.env,
      PATH: `${w.bin}:${process.env.PATH}`,
      STUB_DIR: w.dir,
      CLI_VERSION: version,
      STUDIO_TAG: `studio-${version}`,
      REPOSITORY: repo,
      ARCHIVES_DIR: w.archives,
      R2_BUCKET: bucket,
      R2_ENDPOINT: "https://r2.invalid",
      APPROVED_SHA: w.sha,
      CLI_PUBLISH_FROZEN: "true",
      CLI_TAG_WAIT_ATTEMPTS: "1",
      ...env,
    },
  });
}

const r2 = (w, key) => path.join(w.dir, "r2", bucket, key);
const calls = (w) => {
  const log = path.join(w.dir, "calls.log");
  return existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(Boolean) : [];
};
const puts = (w) => calls(w).filter((c) => c.startsWith("aws put ")).map((c) => c.slice("aws put s3://bkt/".length));
const created = (w) => calls(w).filter((c) => c.startsWith("gh release create"));

const legacyPointer = (tag) => ({
  tag_name: tag,
  prerelease: false,
  html_url: `https://github.com/${repo}/releases/tag/${tag}`,
  release_notes_url: null,
  assets: [...names, "SHA256SUMS"].map((name) => ({
    name,
    browser_download_url: `https://github.com/${repo}/releases/download/${tag}/${name}`,
    size: 42,
  })),
});

function seed(w, key, value) {
  mkdirSync(path.dirname(r2(w, key)), { recursive: true });
  writeFileSync(r2(w, key), JSON.stringify(value, null, 2) + "\n");
}

test("a stable version publishes the release, then the immutable record, then the pointer", () => {
  const w = world();
  seed(w, "cli/stable/latest.json", legacyPointer("v1.39.5"));
  const result = run(w, "v2.24.0");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(puts(w), ["cli/releases/v2.24.0/latest.json", "cli/stable/latest.json"]);
  const order = calls(w).map((c) => (c.startsWith("gh release create") ? "release" : c.startsWith("aws put") ? "put" : ""));
  assert.equal(order.indexOf("release") < order.indexOf("put"), true);
  assert.match(created(w)[0], /--latest=false/);
  assert.doesNotMatch(created(w)[0], /--prerelease/);
  assert.equal(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"), readFileSync(r2(w, "cli/releases/v2.24.0/latest.json"), "utf8"));
});

test("the pointer is what the 1.x updater reads: strict tag, GitHub URLs under that same tag", () => {
  const w = world();
  assert.equal(run(w, "v2.24.0").status, 0);
  const pointer = JSON.parse(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"));
  const golden = JSON.parse(readFileSync(path.join(here, "testdata", "cli-pointer-v2.24.0.json"), "utf8"));
  assert.deepEqual(pointer, golden);
  assert.deepEqual(Object.keys(pointer), ["tag_name", "prerelease", "html_url", "release_notes_url", "assets"]);
  assert.deepEqual(pointer.assets.map((a) => a.name), [...names, "SHA256SUMS"]);
  for (const asset of pointer.assets) {
    assert.equal(asset.browser_download_url, `https://github.com/${repo}/releases/download/v2.24.0/${asset.name}`);
  }
});

test("a preview version moves only the preview pointer", () => {
  const w = world();
  const stable = legacyPointer("v1.39.5");
  seed(w, "cli/stable/latest.json", stable);
  const before = readFileSync(r2(w, "cli/stable/latest.json"), "utf8");
  const result = run(w, "v2.25.0-preview.1");
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(puts(w), ["cli/releases/v2.25.0-preview.1/latest.json", "cli/preview/latest.json"]);
  assert.equal(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"), before);
  assert.match(created(w)[0], /--prerelease/);
});

test("any other prerelease publishes nothing", () => {
  for (const version of ["v2.25.0-rc.1", "v2.25.0-beta", "v2.25.0-preview", "v2.25.0-preview.01"]) {
    const w = world();
    seed(w, "cli/stable/latest.json", legacyPointer("v1.39.5"));
    const result = run(w, version);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(calls(w), [], version);
  }
});

test("a rerun changes nothing and creates no second release", () => {
  const w = world();
  assert.equal(run(w, "v2.24.0").status, 0);
  const pointer = readFileSync(r2(w, "cli/stable/latest.json"), "utf8");
  const before = puts(w).length;
  const result = run(w, "v2.24.0");
  assert.equal(result.status, 0, result.stderr);
  assert.equal(puts(w).length, before);
  assert.equal(created(w).length, 1);
  assert.equal(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"), pointer);
});

test("an older version never moves the pointer back", () => {
  const w = world();
  assert.equal(run(w, "v2.30.0").status, 0);
  const pointer = readFileSync(r2(w, "cli/stable/latest.json"), "utf8");
  assert.equal(run(w, "v2.24.0").status, 0);
  assert.equal(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"), pointer);
  assert.equal(existsSync(r2(w, "cli/releases/v2.24.0/latest.json")), true);
});

test("an immutable record with different content stops the run before the pointer", () => {
  const w = world();
  seed(w, "cli/stable/latest.json", legacyPointer("v1.39.5"));
  const other = legacyPointer("v2.24.0");
  other.assets[0].size = 1;
  seed(w, "cli/releases/v2.24.0/latest.json", other);
  const before = readFileSync(r2(w, "cli/stable/latest.json"), "utf8");
  const result = run(w, "v2.24.0");
  assert.notEqual(result.status, 0);
  assert.equal(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"), before);
});

test("a failed GitHub release writes nothing to R2", () => {
  const w = world();
  const result = run(w, "v2.24.0", { STUB_GH_FAIL: "1" });
  assert.notEqual(result.status, 0);
  assert.deepEqual(puts(w), []);
});

test("a failed immutable write leaves the pointer alone", () => {
  const w = world();
  seed(w, "cli/stable/latest.json", legacyPointer("v1.39.5"));
  const before = readFileSync(r2(w, "cli/stable/latest.json"), "utf8");
  const result = run(w, "v2.24.0", { STUB_R2_FAIL_PUT: "1" });
  assert.notEqual(result.status, 0);
  assert.equal(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"), before);
});

test("archives that disagree with SHA256SUMS are refused before anything is written", () => {
  const w = world();
  writeFileSync(path.join(w.archives, names[0]), "tampered");
  const result = run(w, "v2.24.0");
  assert.notEqual(result.status, 0);
  assert.deepEqual(calls(w), []);
});

test("a missing archive is refused before anything is written", () => {
  const w = world();
  writeFileSync(path.join(w.archives, "SHA256SUMS"), "");
  const result = run(w, "v2.24.0");
  assert.notEqual(result.status, 0);
  assert.deepEqual(calls(w), []);
});

test("a version whose v tag is missing publishes nothing", () => {
  const w = world();
  const result = run(w, "v2.24.0", { tagged: false });
  assert.notEqual(result.status, 0);
  assert.deepEqual(calls(w), []);
});

test("a v tag on another commit publishes nothing", () => {
  const w = world();
  const result = run(w, "v2.24.0", { APPROVED_SHA: "0".repeat(40) });
  assert.notEqual(result.status, 0);
  assert.deepEqual(calls(w), []);
});

test("an existing release is trusted on its own terms, not compared with a rebuild", () => {
  const w = world();
  assert.equal(run(w, "v2.24.0").status, 0);
  rmSync(r2(w, "cli/releases/v2.24.0/latest.json"));
  rmSync(r2(w, "cli/stable/latest.json"));
  const sums = [];
  for (const name of names) {
    const body = Buffer.from(`rebuilt:${name}`);
    writeFileSync(path.join(w.archives, name), body);
    sums.push(`${createHash("sha256").update(body).digest("hex")}  ${name}`);
  }
  writeFileSync(path.join(w.archives, "SHA256SUMS"), sums.join("\n") + "\n");
  const result = run(w, "v2.24.0");
  assert.equal(result.status, 0, result.stderr);
  assert.equal(created(w).length, 1);
  const pointer = JSON.parse(readFileSync(r2(w, "cli/stable/latest.json"), "utf8"));
  assert.equal(pointer.tag_name, "v2.24.0");
});

test("an existing release whose checksums disagree with its own assets is refused", () => {
  const w = world();
  assert.equal(run(w, "v2.24.0").status, 0);
  const db = path.join(w.dir, "gh.json");
  const releases = JSON.parse(readFileSync(db, "utf8"));
  releases["v2.24.0"].sums = releases["v2.24.0"].sums.replace(/^./, (c) => (c === "0" ? "1" : "0"));
  writeFileSync(db, JSON.stringify(releases));
  rmSync(r2(w, "cli/stable/latest.json"));
  assert.notEqual(run(w, "v2.24.0").status, 0);
  assert.equal(existsSync(r2(w, "cli/stable/latest.json")), false);
});

test("the pointer is not written while the 1.x line is not frozen", () => {
  const w = world();
  const result = run(w, "v2.24.0", { CLI_PUBLISH_FROZEN: "" });
  assert.notEqual(result.status, 0);
  assert.deepEqual(calls(w), []);
});
