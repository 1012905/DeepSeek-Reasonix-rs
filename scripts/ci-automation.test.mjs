import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const workflow = (name) =>
  readFileSync(new URL(`../.github/workflows/${name}.yml`, import.meta.url), "utf8");
// A job's body is indented four spaces or more, so the next job-level key is
// the first line indented exactly two.
const job = (text, id) => {
  const start = text.indexOf(`\n  ${id}:`);
  assert.ok(start > 0, `workflow has no job ${id}`);
  const rest = text.slice(start + 1);
  const m = rest.slice(1).match(/\n {2}[a-zA-Z-]+:/);
  return m ? rest.slice(0, m.index + 1) : rest;
};

test("the PR automation gates every job to the event types each behavior owns", () => {
  const text = workflow("pr-gate-and-labels");
  assert.match(text, /pull_request_target:\n\s+types: \[opened, synchronize, reopened, edited\]/);
  // The gate closes pull requests: opened only, never a fork-authored push.
  assert.match(job(text, "gate"), /if: github\.event\.action == 'opened'/);
  assert.match(job(text, "gate"), /cancel-in-progress: false/);
  // The labelers run on open and update; the version label also follows edits.
  assert.match(job(text, "area-label"), /if: github\.event\.action != 'edited'/);
  assert.match(job(text, "version-label"), /if: github\.event\.action != 'synchronize'/);
});

test("the PR automation never executes pull-request code", () => {
  const text = workflow("pr-gate-and-labels");
  assert.doesNotMatch(text, /head\.(sha|ref)|refs\/pull/);
  const labelers = text.slice(text.indexOf("\n  area-label:"));
  assert.doesNotMatch(labelers, /actions\/checkout/);
  assert.match(text, /permissions:\n\s+contents: read\n\s+pull-requests: write/);
});

test("the PR version label names all three release lines, matching the issue forms", () => {
  const script = job(workflow("pr-gate-and-labels"), "version-label");
  assert.match(script, /v1: 'v1', studio: 'v3'/);
});

test("the issue automation splits the two labelers across the edited boundary", () => {
  const text = workflow("issue-labels");
  assert.match(text, /issues:\n\s+types: \[opened, reopened, edited\]/);
  assert.match(job(text, "classify"), /github\.event\.action == 'opened' \|\| github\.event\.action == 'reopened'/);
  assert.match(job(text, "version-label"), /github\.event\.action == 'opened' \|\| github\.event\.action == 'edited'/);
  assert.match(text, /permissions:\n\s+issues: write/);
  // The model credential is the classify job's alone.
  assert.match(job(text, "classify"), /DEEPSEEK_API_KEY/);
  assert.doesNotMatch(job(text, "version-label"), /DEEPSEEK_API_KEY/);
});
