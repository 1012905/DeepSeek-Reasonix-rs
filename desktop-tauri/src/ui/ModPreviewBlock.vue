<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import type { ModPreview } from "../port/plugin";

// Keyed by the kernel's reason and kind. A key with no entry shows the key itself
// rather than nothing: an unlisted module must never read as a clean one.
const NOT_LISTED: Record<string, string> = {
  typescript: "ext.it_is_typescript_which_this",
  too_large: "ext.the_file_is_too_large",
  unparseable: "ext.it_could_not_be_parsed",
  timeout: "ext.parsing_timed_out",
  unsupported_module: "ext.it_is_not_a_module",
};

const FINDING: Record<string, string> = {
  eval: "ext.uses_eval",
  function_ctor: "ext.uses_new_function",
  computed_member: "ext.reaches_the_api_by_a",
  api_escapes: "ext.hands_the_api_object_elsewhere",
  on_escapes: "ext.hands_the_registering_function_elsewhere",
  destructured_api: "ext.destructures_the_api_object",
  no_register: "ext.no_exported_register_was_found",
};

// What a literal session call takes from the session (read) or puts in front of
// the model (submit). Two lines, so a module that only reads usage is not
// described as one that can queue prompts.
const SESSION_READ = new Set(["messages", "usage"]);

const IMPORT_KIND: Record<string, string> = { static: "import", require: "require", dynamic: "import()" };

const list = (xs: string[]) => xs.join("、");

/** What the install preview says about a Claude Code mod: it runs on this
 *  machine with the user's full trust, and the listing below is read from the
 *  module's syntax tree. It is a static list and not a guarantee, and the block
 *  says so in those words; a module that could not be read is "not previewed". */
const props = defineProps<{ mod: ModPreview }>();

const dynamic = computed(() => props.mod.events.filter((e) => e.dynamic).length);
const calls = computed(() => props.mod.api.map((c) => (c.dynamic ? "$.…" : c.method ? `$.${c.namespace}.${c.method}` : `$.${c.namespace}`)));
</script>

<template>
  <section class="modpreview" :aria-label="t('ext.claude_code_mod_preview')">
    <div class="risk" data-kind="shell">
      <span class="lb">{{ t("ext.claude_code_mod_full_trust") }}</span>
      <span class="why">{{ t("ext.its_code_runs_on_this") }}</span>
    </div>
    <p class="why">{{ t("ext.what_follows_is_a_static") }}</p>
    <div v-if="!mod.listed" class="risk" data-kind="secret">
      <span class="lb">{{ t("ext.not_previewed") }}</span>
      <span class="dt">{{ t(NOT_LISTED[mod.reason ?? ""] ?? mod.reason ?? "") }}</span>
      <span class="why">{{ t("ext.that_does_not_mean_it") }}</span>
    </div>
    <template v-else>
      <div class="risk"><span class="lb">{{ t("ext.events_it_hooks") }}</span><span class="dt">{{ mod.events.filter((e) => !e.dynamic).map((e) => e.name).join(", ") || t("ext.none_listed") }}</span></div>
      <div v-if="dynamic > 0" class="risk" data-kind="secret"><span class="lb">{{ t("ext.events_whose_names_cannot_be") }}</span><span class="dt">{{ dynamic }}</span></div>
      <div v-if="calls.length > 0" class="risk"><span class="lb">{{ t("ext.api_it_uses") }}</span><span class="dt">{{ calls.join(", ") }}</span></div>
      <div v-if="mod.session.some((m) => SESSION_READ.has(m))" class="risk"><span class="lb">{{ t("ext.reads_your_conversation") }}</span><span class="dt">{{ mod.session.filter((m) => SESSION_READ.has(m)).join(", ") }}</span></div>
      <div v-if="mod.session.includes('submit')" class="risk" data-kind="secret"><span class="lb">{{ t("ext.queues_prompts_in_front_of") }}</span><span class="dt">{{ t("ext.under_its_own_name_never") }}</span></div>
      <div v-if="mod.imports.length > 0" class="risk" data-kind="secret">
        <span class="lb">{{ t("ext.code_it_pulls_in_not") }}</span>
        <span class="dt">{{ mod.imports.map((i) => `${IMPORT_KIND[i.kind] ?? i.kind} ${i.spec}`).join(", ") }}</span>
      </div>
      <div v-if="mod.findings.length > 0" class="risk" data-kind="secret">
        <span class="lb">{{ t("ext.constructs_that_leave_the_listing") }}</span>
        <span class="dt">{{ list(mod.findings.map((f) => t(FINDING[f.kind] ?? f.kind))) }}</span>
      </div>
      <div v-if="mod.unserved.length > 0" class="risk"><span class="lb">{{ t("ext.events_this_app_never_raises") }}</span><span class="dt">{{ mod.unserved.join(", ") }}</span></div>
    </template>
    <div class="risk">
      <span class="lb">{{ t("ext.module") }}</span>
      <span class="dt" :title="mod.sha256">{{ mod.module }} · {{ mod.sha256.slice(0, 12) }}</span>
    </div>
  </section>
</template>