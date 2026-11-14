import assert from "node:assert/strict";
import test from "node:test";
import { buildCommands, verify, GROUPS, coveredByFull } from "./ci-windows-targeted.mjs";

const packages = (group) => group.packages ?? group.fullPackages ?? [];
const full = GROUPS.flatMap((g) => g.fullPackages ?? []);
const patterned = (commands, pkg) =>
  commands.find((c) => c.packages.includes(pkg) && c.args.includes("-run"));

test("every package runs exactly once: either whole or under one patterned command", () => {
  const commands = buildCommands(GROUPS);
  for (const pkg of new Set(GROUPS.flatMap(packages))) {
    const runs =
      commands.filter((c) => c.packages.includes(pkg) && c.args.includes("-run")).length +
      full.filter((f) => coveredByFull(pkg, [f])).length;
    assert.equal(runs, 1, `${pkg} is selected by ${runs} commands`);
  }
});

test("every command bypasses the test result cache", () => {
  for (const command of buildCommands(GROUPS)) {
    assert.ok(command.args.includes("-count=1"));
    assert.ok(command.args.includes("-json"));
  }
});

test("the whole-package command runs the smoke list and isolation owners without a pattern", () => {
  const command = buildCommands(GROUPS).find((c) =>
    c.packages.includes("./internal/platform/worktree"),
  );
  assert.ok(command, "no command runs the isolation owners");
  assert.ok(command.packages.includes("./internal/state/mutation"));
  assert.ok(command.packages.includes("./internal/runtime/agent/..."));
  assert.ok(!command.args.includes("-run"));
});

test("a topic package covered by a whole-package run gets no patterned command", () => {
  const commands = buildCommands(GROUPS);
  for (const group of GROUPS) {
    if (group.fullPackages) continue;
    for (const pkg of group.packages) {
      assert.equal(
        patterned(commands, pkg) === undefined,
        coveredByFull(pkg, full),
        `${group.name}: ${pkg} patterned-command presence disagrees with whole-package coverage`,
      );
    }
  }
});

test("every uncovered patterned command carries its group pattern verbatim", () => {
  const commands = buildCommands(GROUPS);
  for (const group of GROUPS) {
    if (!group.pattern) continue;
    for (const pkg of group.packages ?? []) {
      if (coveredByFull(pkg, full)) continue;
      const command = patterned(commands, pkg);
      assert.ok(command, `${pkg} has no patterned command`);
      const i = command.args.indexOf("-run");
      assert.ok(command.args[i + 1].includes(group.pattern), `${group.name} pattern missing for ${pkg}`);
    }
  }
});

test("every required test matches its group's pattern", () => {
  for (const group of GROUPS) {
    if (!group.pattern) continue;
    const re = new RegExp(group.pattern);
    for (const name of group.required) {
      assert.match(name.split("/")[0], re, `${group.name}: ${name}`);
    }
  }
});

test("verify accepts a clean run and flags failures, missing inventories, and owned skips", () => {
  const groups = [
    { name: "topic", packages: ["./p"], pattern: "^TestA", required: ["TestA", "TestB"] },
    { name: "whole", fullPackages: ["./q"], required: ["TestQ"] },
    { name: "smoke", fullPackages: ["./r"], checks: false },
  ];
  const importPaths = new Map([
    ["./p", "reasonix/p"],
    ["./q", "reasonix/q"],
    ["./r", "reasonix/r"],
  ]);
  const event = (o) => ({ Action: "pass", Package: "reasonix/p", Test: null, ...o });
  const clean = [
    event({ Test: "TestA" }),
    event({ Test: "TestA/sub" }),
    event({ Test: "TestB" }),
    event({ Package: "reasonix/q", Test: "TestQ" }),
    event({}),
    event({ Package: "reasonix/q" }),
    event({ Package: "reasonix/r" }),
    event({ Package: "reasonix/r", Test: "TestRAllowedToSkip", Action: "skip" }),
  ];
  assert.deepEqual(verify(clean, groups, importPaths), []);

  assert.ok(
    verify(clean.filter((_, i) => i !== 3), groups, importPaths).some((e) => e.includes("TestQ")),
  );
  assert.ok(verify(clean, [{ ...groups[0], required: ["TestA", "TestB", "TestMissing"] }], importPaths).some(
    (e) => e.includes("TestMissing"),
  ));

  // A skip of a pattern-matched test fails the topic that selected it, but
  // only inside the topic's own packages; elsewhere it is not the topic's.
  assert.ok(verify([...clean, event({ Test: "TestA/x", Action: "skip" })], groups, importPaths).some(
    (e) => e.includes("topic"),
  ));
  assert.deepEqual(
    verify([...clean, event({ Package: "reasonix/z", Test: "TestA/z", Action: "skip" })], groups, importPaths),
    [],
  );
  // A subtest whose root does not match the pattern is not the topic's skip.
  assert.deepEqual(
    verify([...clean, event({ Test: "TestOther/skip", Action: "skip" })], groups, importPaths),
    [],
  );
  // A whole-package group keeps the old isolation step's contract: package
  // results and required tests only — a skip in its packages is nobody's.
  assert.deepEqual(
    verify([...clean, event({ Package: "reasonix/q", Test: "TestQ/sub", Action: "skip" })], groups, importPaths),
    [],
  );
  // A checks:false group's skips and package results are nobody's inventory.
  assert.deepEqual(
    verify([...clean, event({ Package: "reasonix/r", Test: "TestR/x", Action: "skip" })], groups, importPaths),
    [],
  );
  assert.ok(verify([...clean, event({ Package: "reasonix/p", Action: "pass" })], groups, importPaths).some(
    (e) => e.includes("exactly 1"),
  ));
  assert.ok(verify([...clean, event({ Test: "TestC", Action: "fail" })], groups, importPaths).some(
    (e) => e.startsWith("fail:"),
  ));
});
