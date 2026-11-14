#!/usr/bin/env node
// The Windows pull-request suite. One manifest replaces the smoke step and
// the per-topic steps: every package in the smoke list runs whole, every
// other package runs under the union of the patterns of the topics that
// named it, so each package runs exactly once. Over the combined JSONL every
// topic's required tests must pass, and a test a topic selected may not skip.

import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const GROUPS = [
  {
    name: "composition links",
    packages: ["./internal/state/agentpreset", "./internal/ext/skill", "./internal/assembly/boot"],
    pattern:
      "^Test(CaptureDirectoryMaterializesContainedLinks|CaptureDirectoryRefusesUnsafeLinks|CaptureHookFileFlattensContainedLink|MaterializeStillRejectsLinkedRetainedFilesAndRoot|WindowsCompositionJunctions|WindowsLiveSkillJunctions|CompositionRetainsLinkedSkillAcrossRestart|CompositionLinkedSkillRejectsLinkedResources|CompositionLinkedExtensionResources|CompositionLinkedHookResources)$",
    required: [
      "TestCaptureDirectoryMaterializesContainedLinks",
      "TestCaptureDirectoryRefusesUnsafeLinks",
      "TestCaptureHookFileFlattensContainedLink",
      "TestMaterializeStillRejectsLinkedRetainedFilesAndRoot",
      "TestWindowsCompositionJunctions",
      "TestWindowsLiveSkillJunctions",
      "TestCompositionRetainsLinkedSkillAcrossRestart",
      "TestCompositionLinkedSkillRejectsLinkedResources",
      "TestCompositionLinkedExtensionResources",
      "TestCompositionLinkedHookResources",
    ],
  },
  {
    name: "isolation owners",
    fullPackages: ["./internal/platform/worktree", "./internal/state/mutation"],
    required: [
      "TestHeldPathsCoverOnlyTheSameFileAcrossCaseSpellings",
      "TestLegacyWaitDoesNotConsumeExecutionCapacity",
      "TestWriteChangesRejectsRedirectedDeletion",
      "TestWriteChangesUnlinksLeafDirectoryLink",
    ],
  },
  {
    name: "delegated Goal usage scope",
    packages: ["./internal/contract/event", "./internal/session/control", "./internal/assembly/boot"],
    pattern: "^Test(UsageScope.*)$",
    required: ["TestUsageScopeKeepsEarlierEventOwner", "TestUsageScopeWithoutCapability"],
  },
  {
    name: "Controller routing root budget",
    packages: ["./internal/runtime/capability"],
    pattern: "^Test(SemanticRootUsage)$",
    required: [
      "TestSemanticRootUsage",
      "TestSemanticRootUsage/billed_error",
      "TestSemanticRootUsage/cache",
      "TestSemanticRootUsage/estimated",
      "TestSemanticRootUsage/invalid_answer",
      "TestSemanticRootUsage/missing",
      "TestSemanticRootUsage/nil_sink",
      "TestSemanticRootUsage/no_observer",
      "TestSemanticRootUsage/reported",
    ],
  },
  {
    name: "Coordinator root budget",
    packages: ["./internal/runtime/agent", "./internal/runtime/coordinator"],
    pattern: "^Test(PreparedRootBudget|CoordinatorRootBudget|EffectCoordinatorRoot)",
    required: [
      "TestCoordinatorRootBudgetRoutes",
      "TestCoordinatorRootBudgetRoutes/raw",
      "TestCoordinatorRootBudgetRoutes/tool_planner",
      "TestCoordinatorRootBudgetRoutes/failure_fallback",
      "TestCoordinatorRootBudgetRoutes/plan_only_continuation",
      "TestCoordinatorRootBudgetRoutes/approval_declined_continuation",
      "TestCoordinatorRootBudgetRoutes/approval_allowed",
    ],
  },
  {
    name: "resumed Goal planner budget",
    packages: ["./internal/session/control", "./internal/assembly/boot"],
    pattern:
      "^Test(GoalResumeRenewsPlannerBudget|GoalNonBudgetResumeKeepsPlannerBudget|GoalTokenBudgetPausesAndResumes|EffectGoalResumeRenewsPlannerBudget)$",
    required: [
      "TestGoalResumeRenewsPlannerBudget",
      "TestGoalNonBudgetResumeKeepsPlannerBudget",
      "TestGoalNonBudgetResumeKeepsPlannerBudget/manual_pause",
      "TestGoalNonBudgetResumeKeepsPlannerBudget/not_resumable",
      "TestGoalTokenBudgetPausesAndResumes",
      "TestEffectGoalResumeRenewsPlannerBudget",
    ],
  },
  {
    name: "root wall budget",
    packages: ["./internal/runtime/agent", "./internal/assembly/boot"],
    pattern: "^Test(RootWall|EffectRootWall)",
    required: [
      "TestRootWallBudgetReachesDelegatedAgents",
      "TestRootWallBudgetReachesDelegatedAgents/foreground",
      "TestRootWallBudgetReachesDelegatedAgents/nested",
      "TestRootWallBudgetReachesDelegatedAgents/siblings",
      "TestRootWallBudgetReachesDelegatedAgents/context",
      "TestRootWallBudgetReachesDelegatedAgents/unconfigured",
      "TestRootWallBudgetReachesDelegatedAgents/explicit_zero",
      "TestRootWallBudgetContinuationAndReset",
      "TestRootWallBudgetContinuationAndReset/continuation",
      "TestRootWallBudgetContinuationAndReset/new_task",
      "TestRootWallBudgetContinuationAndReset/reset",
      "TestRootWallBudgetContinuationAndReset/override",
    ],
  },
  {
    name: "root token budget",
    packages: ["./internal/runtime/agent"],
    pattern: "^Test(RootToken|EffectRootToken|BudgetCountsKnownReportedTokens|ReportedToken)",
    required: [
      "TestRootTokenBudgetIncludesChildProviderUsage",
      "TestRootTokenBudgetIncludesChildProviderUsage/configured",
      "TestRootTokenBudgetIncludesChildProviderUsage/context",
      "TestRootTokenBudgetIncludesChildProviderUsage/unconfigured",
      "TestRootTokenBudgetIncludesChildProviderUsage/explicit_zero",
      "TestRootTokenBudgetSurvivesNestedAndConcurrentChildren",
      "TestRootTokenBudgetSurvivesNestedAndConcurrentChildren/nested",
      "TestRootTokenBudgetSurvivesNestedAndConcurrentChildren/siblings",
      "TestRootTokenBudgetSurvivesNestedAndConcurrentChildren/child_override",
      "TestRootTokenBudgetContinuationAndReset",
      "TestRootTokenBudgetContinuationAndReset/continuation",
      "TestRootTokenBudgetContinuationAndReset/new_task",
      "TestRootTokenBudgetContinuationAndReset/reset",
      "TestBudgetCountsKnownReportedTokens",
      "TestBudgetCountsKnownReportedTokens/split",
      "TestBudgetCountsKnownReportedTokens/total_only",
      "TestBudgetCountsKnownReportedTokens/partial_split",
      "TestBudgetCountsKnownReportedTokens/lower_total",
      "TestBudgetCountsKnownReportedTokens/missing_total",
      "TestBudgetCountsKnownReportedTokens/negative_split",
      "TestBudgetCountsKnownReportedTokens/zero",
      "TestBudgetCountsKnownReportedTokens/negative",
      "TestReportedTokenCountersSaturate",
    ],
  },
  {
    name: "budget usage completeness",
    packages: ["./internal/contract/provider", "./internal/runtime/agent", "./internal/assembly/boot"],
    pattern:
      "^Test(RequestOnlyUsage|RunBudgetMissingUsage|TaskBudgetKnownCost|RunBudgetExplicitZero|TaskBudgetUsageCompleteness|TaskBudgetCostUses|EffectBudgetCompleteness|TaskBudgetGateIgnoresCostWhenUnpriced)",
    required: [
      "TestRequestOnlyUsageDoesNotCertifyTokenAccounting",
      "TestRunBudgetMissingUsageKeepsCostIncomplete",
      "TestRunBudgetMissingUsageKeepsCostIncomplete/complete",
      "TestRunBudgetMissingUsageKeepsCostIncomplete/missing_first",
      "TestRunBudgetMissingUsageKeepsCostIncomplete/missing_last",
      "TestRunBudgetMissingUsageKeepsCostIncomplete/all_missing",
      "TestTaskBudgetKnownCostStillLandsAfterMissingUsage",
      "TestRunBudgetExplicitZeroUsageIsPriced",
      "TestTaskBudgetUsageCompletenessFollowsItsLifetime",
      "TestTaskBudgetUsageCompletenessFollowsItsLifetime/continuation",
      "TestTaskBudgetUsageCompletenessFollowsItsLifetime/new_task",
      "TestTaskBudgetUsageCompletenessFollowsItsLifetime/explicit_reset",
      "TestTaskBudgetCostUsesOnlyTheReportedSubtotal",
      "TestTaskBudgetGateIgnoresCostWhenUnpriced",
      "TestEffectBudgetCompletenessFromOpenAIStream",
      "TestEffectBudgetCompletenessFromOpenAIStream/missing_0",
      "TestEffectBudgetCompletenessFromOpenAIStream/missing_1",
      "TestEffectBudgetCompletenessFromOpenAIStream/missing_2",
    ],
  },
  {
    name: "Git storage environment",
    packages: ["./internal/platform/gitcmd", "./internal/platform/worktree", "./internal/assembly/boot"],
    pattern:
      "^Test(HostStorageEnvironment|HostObjectStorage|HostExplicitStorageControls|DetachedStorageEnvironment)$",
    required: [
      "TestDetachedStorageEnvironment",
      "TestDetachedStorageEnvironment/explicit_env",
      "TestDetachedStorageEnvironment/inherited_env",
      "TestHostExplicitStorageControls",
      "TestHostExplicitStorageControls/explicit_object_directory",
      "TestHostExplicitStorageControls/private_index",
      "TestHostExplicitStorageControls/repository_declared_alternate",
      "TestHostObjectStorage",
      "TestHostObjectStorage/ambient_alternate_not_read",
      "TestHostObjectStorage/writes_stay_in_repository",
      "TestHostStorageEnvironment",
      "TestHostStorageEnvironment/GIT_ALTERNATE_OBJECT_DIRECTORIES",
      "TestHostStorageEnvironment/GIT_INDEX_FILE",
      "TestHostStorageEnvironment/GIT_OBJECT_DIRECTORY",
    ],
  },
  {
    name: "Anthropic interrupted usage",
    packages: ["./internal/model/anthropic", "./internal/assembly/boot"],
    pattern: "^Test(InterruptedUsage|EffectAnthropicPartialUsageRecovery)",
    required: [
      "TestInterruptedUsageTerminal",
      "TestInterruptedUsageTerminal/upstream_error",
      "TestInterruptedUsageTerminal/payload_error",
      "TestInterruptedUsageTerminal/premature_eof",
      "TestInterruptedUsageTerminal/decode_error",
      "TestInterruptedUsageTerminal/reported_zero",
      "TestInterruptedUsageTerminal/missing_control",
      "TestInterruptedUsageTerminal/complete_control",
      "TestInterruptedUsageBoundedConsumer",
      "TestInterruptedUsageReadErrorAndCancellation",
      "TestInterruptedUsageReadErrorAndCancellation/read_error",
      "TestInterruptedUsageReadErrorAndCancellation/cancelled_buffered_reader",
      "TestInterruptedUsageAbandonedConsumer",
      "TestInterruptedUsageIdleTimeout",
      "TestEffectAnthropicPartialUsageRecovery",
      "TestEffectAnthropicPartialUsageRecovery/upstream_error",
      "TestEffectAnthropicPartialUsageRecovery/premature_eof",
      "TestEffectAnthropicPartialUsageRecovery/complete_control",
    ],
  },
  {
    name: "root websearch budget",
    packages: ["./internal/tools/websearch"],
    pattern: "^Test(RootWebSearch|EffectRootWebSearch)",
    required: [
      "TestRootWebSearchUsageTerminal",
      "TestRootWebSearchUsageTerminal/reported",
      "TestRootWebSearchUsageTerminal/no_reporter",
      "TestRootWebSearchUsageTerminal/missing",
      "TestRootWebSearchUsageTerminal/no_observer",
      "TestRootWebSearchUsageTerminal/input_refused",
      "TestRootWebSearchUsageTerminal/no_factory",
      "TestRootWebSearchUsageTerminal/factory_error",
      "TestRootWebSearchUsageTerminal/billed_failure",
      "TestRootWebSearchUsageTerminal/cancel_after_usage",
      "TestRootWebSearchUsageTerminal/cancel_before_start",
      "TestRootWebSearchUsageTerminal/stream_error",
    ],
  },
  {
    name: "explicit skill root budget",
    packages: ["./internal/session/control"],
    pattern: "^(TestProfileRootBudgetLifetime|TestProfileBudgetPreparationUsesTurnAdmission|TestEffectExplicitSkillRootBudget)$",
    required: ["TestProfileBudgetPreparationUsesTurnAdmission"],
  },
  {
    name: "root triage budget",
    packages: ["./internal/runtime/agent"],
    pattern: "^Test(RootTriage|EffectRootTriage)",
    required: [
      "TestRootTriageTokenBudget",
      "TestRootTriageTokenBudget/classifier",
      "TestRootTriageTokenBudget/screen",
      "TestRootTriageTokenBudget/billed_failure",
      "TestRootTriageTokenBudget/missing",
      "TestRootTriageTokenBudget/disabled",
      "TestRootTriageTokenBudget/no_sink",
      "TestRootTriageTokenBudget/counts_without_total",
      "TestRootTriageOutsideRunDoesNotCreateScope",
    ],
  },
  {
    name: "local cost quote",
    packages: ["./internal/contract/event", "./internal/runtime/agent"],
    pattern: "^Test(LocalBudgetUsesOccurrenceQuote|LocalCost.*|UsageQuote.*)$",
    required: [
      "TestLocalBudgetUsesOccurrenceQuote",
      "TestLocalBudgetUsesOccurrenceQuote/peak",
      "TestLocalBudgetUsesOccurrenceQuote/offpeak",
      "TestLocalCostKeepsOriginalCurrencyBuckets",
      "TestLocalCostFixedPointThresholdAndUnknown",
      "TestUsageQuoteUsesOneOwnerAndOccurrence",
      "TestUsageQuoteLegacyAndNilSinks",
    ],
  },
  {
    name: "estimated usage quotes",
    packages: [
      "./internal/contract/pricing",
      "./internal/contract/event",
      "./internal/contract/config",
      "./internal/assembly/boot",
      "./internal/contract/eventwire",
      "./internal/frontend/acp",
      "./internal/state/stats",
    ],
    pattern:
      "^Test(EstimatedUsage|EffectEstimatedUsage|ToWireUsagePayloadJSON$|StatusExtensionTracksMultipleSessionsAndUsage$|CacheSplitProvenanceIsRecorded$|RecordedPlainCallIsUnchanged$)",
    required: [
      "TestEstimatedUsageQuotePreservesAmountsWithoutCompleteFacts",
      "TestEstimatedUsageQuotePreservesAmountsWithoutCompleteFacts/CNY",
      "TestEstimatedUsageQuotePreservesAmountsWithoutCompleteFacts/USD",
      "TestEstimatedUsageLedgerRetainsPartialCoverage",
      "TestEstimatedUsageLedgerRetainsPartialCoverage/separate_keys",
      "TestEstimatedUsageLedgerRetainsPartialCoverage/same_key",
      "TestEstimatedUsageSavedQuoteReader",
      "TestEstimatedUsageQuoteSinkCoversTurnAndGuardian",
      "TestEstimatedUsageQuoteSinkCoversTurnAndGuardian/turn",
      "TestEstimatedUsageQuoteSinkCoversTurnAndGuardian/guardian",
      "TestEstimatedUsageConfiguredQuote",
      "TestEffectEstimatedUsageQuoteAfterStreamRecovery",
      "TestEffectEstimatedUsageQuoteAfterStreamRecovery/reported_control",
      "TestEffectEstimatedUsageQuoteAfterStreamRecovery/interrupted",
      "TestToWireUsagePayloadJSON",
      "TestStatusExtensionTracksMultipleSessionsAndUsage",
      "TestCacheSplitProvenanceIsRecorded",
      "TestRecordedPlainCallIsUnchanged",
    ],
  },
  {
    name: "bounded root budget",
    packages: ["./internal/contract/provider", "./internal/model/boundedllm", "./internal/runtime/agent"],
    pattern: "^Test(AuxiliaryUsageObserver|BoundedUsageObserver|RootBounded|EffectRootBounded)",
    required: [
      "TestAuxiliaryUsageObserverReplacesAndCopies",
      "TestBoundedUsageObserverOwnsOneTerminal",
      "TestBoundedUsageObserverOwnsOneTerminal/reported",
      "TestBoundedUsageObserverOwnsOneTerminal/billed_failure",
      "TestBoundedUsageObserverOwnsOneTerminal/without_sink",
      "TestBoundedUsageObserverOwnsOneTerminal/missing",
      "TestBoundedUsageObserverOwnsOneTerminal/input_refused",
    ],
  },
  {
    name: "session temp and shell execution contract",
    packages: ["./internal/tools/builtin"],
    pattern:
      "^TestBashSharesSessionTempAcrossCalls$|^TestBashPowerShellExecuteDetailedContract$|^TestBashPowerShell51PreflightRejectsAndAndDetailed$|^TestBashPowerShellOutputIsUTF8$|^TestBashPowerShellSurfacesNonZeroExit$|^TestBashPowerShellRejectsChaining$",
    required: [
      "TestBashSharesSessionTempAcrossCalls",
      "TestBashPowerShellExecuteDetailedContract",
      "TestBashPowerShell51PreflightRejectsAndAndDetailed",
      "TestBashPowerShellOutputIsUTF8",
      "TestBashPowerShellSurfacesNonZeroExit",
      "TestBashPowerShellRejectsChaining",
    ],
  },
  {
    name: "windows smoke",
    // The packages that actually carry *_windows.go code, the startup and
    // checkpoint packages with portable filesystem contracts, plus cmd/. The
    // whole-package run is the smoke suite the topics above ride inside; a
    // package named here never gets its own patterned command. The full ./...
    // sweep stays on pushes to main-v2 (ci.yml test-windows-shards).
    fullPackages: [
      "./internal/runtime/agent/...",
      "./internal/platform/appidentity/...",
      "./internal/assembly/boot/...",
      "./internal/state/checkpoint/...",
      "./internal/frontend/cli/...",
      "./internal/session/control/...",
      "./internal/base/filelock/...",
      "./internal/base/fileutil/...",
      "./internal/ext/hook/...",
      "./internal/state/instruction/...",
      "./internal/ext/mcplaunch/...",
      "./internal/platform/notify/...",
      "./internal/base/proc/...",
      "./internal/platform/remote/...",
      "./internal/platform/repair/...",
      "./internal/safety/sandbox/...",
      "./internal/base/sysproxy/...",
      "./internal/state/workspacelease/...",
      "./cmd/...",
    ],
    checks: false,
  },
];

// A package named by a whole-package group never gets a patterned command:
// its group patterns' tests already run inside the whole-package run.
export function coveredByFull(pkg, full) {
  return full.some((f) => f === pkg || f.startsWith(`${pkg}/`));
}

// Each package's pattern keeps its per-group anchors: a group's regex is
// embedded verbatim, so an intentionally unanchored pattern stays that way.
export function buildCommands(groups) {
  const full = groups.flatMap((g) => g.fullPackages ?? []);
  const perPackage = new Map();
  for (const group of groups) {
    if (group.fullPackages) continue;
    for (const pkg of group.packages) {
      if (coveredByFull(pkg, full)) continue;
      if (!perPackage.has(pkg)) perPackage.set(pkg, new Set());
      perPackage.get(pkg).add(group.pattern);
    }
  }
  const commands = [];
  if (full.length > 0) {
    commands.push({ args: ["-count=1", "-json", "-p", "4", "-timeout=18m"], packages: full });
  }
  for (const pkg of [...perPackage.keys()].sort()) {
    const merged = [...perPackage.get(pkg)]
      .sort()
      .map((p) => `(?:${p})`)
      .join("|");
    commands.push({ args: ["-count=1", "-json", "-run", merged, "-timeout=3m"], packages: [pkg] });
  }
  return commands;
}

// A whole-package run legitimately carries skipped tests, so a skip only
// fails a group whose own selection could have run it: a patterned group's
// pattern, over the group's packages. Whole-package groups keep the old
// isolation step's contract — packages pass once and the required tests pass,
// with no opinion about the rest. Failures fail regardless — go test already
// exits non-zero for them.
export function verify(events, groups, importPaths) {
  const errors = [];
  const passed = new Set();
  const packagePasses = new Map();
  const skips = [];
  for (const e of events) {
    if (e.Action === "fail") {
      errors.push(`fail: ${e.Package ?? ""} ${e.Test ?? "(package)"}`.trim());
    }
    if (e.Action === "pass" && e.Test != null) passed.add(e.Test);
    if (e.Action === "pass" && e.Test == null) {
      packagePasses.set(e.Package, (packagePasses.get(e.Package) ?? 0) + 1);
    }
    if (e.Action === "skip" && e.Test != null) skips.push(e);
  }
  for (const group of groups) {
    if (group.checks === false) continue;
    const owned = new Set(
      (group.packages ?? []).map((p) => importPaths.get(p)).filter(Boolean),
    );
    for (const pkg of group.packages ?? group.fullPackages) {
      const importPath = importPaths.get(pkg);
      const n = importPath ? packagePasses.get(importPath) ?? 0 : 0;
      if (n !== 1) errors.push(`${group.name}: ${pkg} has ${n} passing package results, want exactly 1`);
    }
    for (const test of group.required) {
      if (!passed.has(test)) errors.push(`${group.name}: ${test} has no pass event`);
    }
    if (!group.pattern) continue;
    for (const e of skips) {
      if (!owned.has(e.Package)) continue;
      const root = e.Test.split("/")[0];
      if (new RegExp(group.pattern).test(root)) {
        errors.push(`${group.name}: ${e.Test} skipped`);
      }
    }
  }
  return errors;
}

function resolveImportPaths(packages) {
  const r = spawnSync("go", ["list", ...packages], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(r.stderr || "go list failed");
  const lines = r.stdout.trim().split(/\r?\n/);
  return new Map(packages.map((pkg, i) => [pkg, lines[i]]));
}

async function run(command, jsonlPath, events) {
  process.stdout.write(`\n== go test ${command.packages.join(" ")}\n`);
  const child = spawn("go", ["test", ...command.args, ...command.packages]);
  let buffer = "";
  const collect = (chunk) => {
    process.stdout.write(chunk);
    appendFileSync(jsonlPath, chunk);
    buffer += chunk.toString();
  };
  child.stdout.on("data", collect);
  child.stderr.on("data", (chunk) => process.stderr.write(chunk));
  const code = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  for (const line of buffer.split("\n")) {
    if (!line.trim()) continue;
    try {
      events.push(JSON.parse(line));
    } catch {
      // Build failures print plain text; it is echoed above either way.
    }
  }
  return code;
}

async function main() {
  const jsonlIndex = process.argv.indexOf("--jsonl");
  const jsonlPath = jsonlIndex >= 0 ? process.argv[jsonlIndex + 1] : "windows-targeted.jsonl";
  writeFileSync(jsonlPath, "");
  const commands = buildCommands(GROUPS);
  // Only checked groups' packages resolve to import paths, and they are all
  // single directories, so one line per query maps back by index. Globs ride
  // the whole-package command and are never looked up.
  const checked = GROUPS.filter((g) => g.checks !== false);
  const importPaths = resolveImportPaths([...new Set(checked.flatMap((g) => g.packages ?? g.fullPackages))]);
  const events = [];
  let failed = false;
  for (const command of commands) {
    const code = await run(command, jsonlPath, events);
    if (code !== 0) {
      failed = true;
      process.stderr.write(`go test ${command.packages.join(" ")} exited ${code}\n`);
    }
  }
  const errors = verify(events, GROUPS, importPaths);
  for (const e of errors) process.stderr.write(`::error::${e}\n`);
  if (failed || errors.length > 0) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
