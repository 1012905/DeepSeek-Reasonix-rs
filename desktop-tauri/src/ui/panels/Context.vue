<script lang="ts">
import type { AgentPort, ContextBreakdown } from "../../port/port";

/** What the Context panel needs from the rail. `context` is the reading the pane
 *  already polled and put in its report; the panel holds its own copy only
 *  because a window declared here answers on the same call that writes it. */
export interface ContextPanelProps {
  port: AgentPort;
  context?: ContextBreakdown | null;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import { pct, tokens } from "../../i18n/format";
import SaveNote from "../SaveNote.vue";

// The order the classes arrive in a prompt, not the order of their sizes: a
// class that grows is easier to spot when its neighbours stay put.
//
// Read at render rather than at import — t() answers out of a dictionary that
// boot() installs, and a module body runs before it, which is how a table built
// at import once froze five labels in the source language.
const PARTS = [
  ["system", "common.system_prompt", "panels.base_instructions_memory_the_skill"],
  ["tools", "common.tool_definitions", "panels.the_tool_list_sent_to"],
  ["user", "common.what_you_said", "panels.your_side_of_this_session"],
  ["reply", "common.model_replies", "panels.what_the_model_has_said"],
  ["output", "common.tool_output", "panels.what_commands_reads_and_searches"],
] as const;

const props = defineProps<ContextPanelProps>();

const ctx = ref<ContextBreakdown | null>(props.context ?? null);
watch(() => props.context, (next) => { ctx.value = next ?? null; });

const used = computed(() => ctx.value?.used ?? 0);
// A fold point past the window is how maintenance is retired: the ratio was put
// where usage cannot reach it. There is then no deadline to count down to, and
// capacity is the whole gauge rather than a footnote.
const folds = computed(() => {
  const c = ctx.value;
  return !!c && c.compact_at > 0 && c.compact_at <= c.window;
});
const denom = computed(() => {
  const c = ctx.value;
  return c ? (folds.value ? c.compact_at : c.window) : 0;
});
const share = computed(() => (denom.value > 0 ? Math.min((used.value / denom.value) * 100, 100) : 0));
// The kernel's own two rungs: it tells the model to work narrower at 75% of the
// trigger and to land what it knows at 92%. A panel inventing its own
// thresholds would report a pressure the session is not under, and maintenance
// is routine, so both read as accent and never as a fault.
const press = computed<"soon" | "near" | undefined>(() =>
  !folds.value ? undefined : share.value >= 92 ? "soon" : share.value >= 75 ? "near" : undefined);

// Only non-empty classes get a slice: an equal-width segment for a class that
// holds nothing claims a share no request ever carried.
const shown = computed(() => {
  const c = ctx.value;
  if (!c) return [];
  return PARTS.map(([key, label, why]) => ({ key, label, why, n: c[key] || 0 })).filter((p) => p.n > 0);
});
const sum = computed(() => shown.value.reduce((n, p) => n + p.n, 0) || 1);

const editing = ref(false);
const draft = ref("");
const busy = ref(false);
const note = ref<{ text: string; unapplied: boolean } | null>(null);

// Prefilled, because the common edit is to a figure that is already there: an
// empty box asks for the whole number again just to change one digit of it.
const editWindow = () => {
  draft.value = (ctx.value?.window ?? 0) > 0 ? String(ctx.value?.window) : "";
  note.value = null;
  editing.value = true;
};

const commit = async () => {
  const window = Number(draft.value);
  if (!window || busy.value) return;
  busy.value = true;
  note.value = null;
  try {
    // The rebuilt gauge comes back on this same call rather than on the next
    // poll, so the bar on screen is the one just declared.
    ctx.value = await props.port.setContextWindow(window);
    editing.value = false;
  } catch (e) {
    note.value = { text: reason(e), unapplied: false };
    props.onError(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div v-if="ctx" class="block" data-b="ctx">
    <div class="lbl">
      <span>{{ t("common.context") }}</span>
      <span v-if="denom > 0" class="n">{{ pct(share / 100) }}</span>
    </div>

    <div class="drow">
      <span class="k">{{ t("common.context") }}</span>
      <span class="v"><span class="ctxq">{{ tokens(Math.round(used)) }}</span></span>
    </div>

    <!-- A window nobody declared has no denominator to draw, but the number
         still matters — and a zero window is also what switches automatic
         compaction off, which vanishing said neither. -->
    <template v-if="!ctx.window">
      <p class="ctxnote">{{ t("panels.nobody_has_said_how_large") }}</p>
      <div v-if="!editing" class="ctxwin">
        <div class="r">
          <button type="button" class="ctxden" :aria-expanded="false" @click="editWindow">{{ t("panels.the_context_ceiling_from_the") }}</button>
        </div>
      </div>
      <div v-else class="ctxwin">
        <div class="r">
          <input inputmode="numeric" :value="draft" placeholder="131072" :disabled="busy"
            :aria-label="t('panels.context_window_tokens')"
            @input="draft = ($event.target as HTMLInputElement).value.replace(/\D/g, '')"
            @keydown.enter="commit" @keydown.esc="editing = false" />
          <button type="button" :disabled="busy || !draft" @click="commit">{{ t("common.save") }}</button>
        </div>
        <SaveNote :note="note" />
        <p class="ctxnote">{{ t('panels.the_context_ceiling_from_the') }}</p>
      </div>
    </template>

    <template v-else>
      <div v-if="!folds" class="drow">
        <span class="k">{{ t("common.model_capacity") }}</span>
        <span class="v">
          <span class="ctxq">
            <button v-if="!editing" type="button" class="ctxden" :aria-expanded="false"
              :title="t('panels.whoever_entered_this_window_may')" @click="editWindow">{{ tokens(ctx.window) }}</button>
            <span v-else>{{ tokens(ctx.window) }}</span>
            <em>{{ pct(used / ctx.window) }}</em>
          </span>
        </span>
      </div>
      <div v-if="editing" class="ctxwin">
        <div class="r">
          <input inputmode="numeric" :value="draft" placeholder="131072" :disabled="busy"
            :aria-label="t('panels.context_window_tokens')"
            @input="draft = ($event.target as HTMLInputElement).value.replace(/\D/g, '')"
            @keydown.enter="commit" @keydown.esc="editing = false" />
          <button type="button" :disabled="busy || !draft || Number(draft) === ctx.window" @click="commit">{{ t("common.save") }}</button>
        </div>
        <SaveNote :note="note" />
        <p class="ctxnote">{{ t('panels.applies_to_this_model_alone') }}</p>
      </div>

      <div class="ctxbar" :data-press="press" tabindex="0" role="group" :aria-label="t('panels.what_fills_the_context')">
        <i v-for="p in shown" :key="p.key" :data-p="p.key" :style="{ width: `${(p.n / sum) * share}%` }" :title="`${t(p.label)} · ${tokens(p.n)}`" />
      </div>
      <p v-if="press" class="ctxnote" :data-press="press">
        {{ press === "soon" ? t("panels.the_maintenance_point_is_close") : t("panels.approaching_the_maintenance_point_the") }}
      </p>

      <!-- Capacity is the diagnosis, not the deadline: it answers "is this model
           simply too small", and it is where a wrong window is corrected. -->
      <div v-if="folds" class="ctxcap">
        <div class="drow">
          <span class="k">{{ t("panels.next_maintenance") }}</span>
          <span class="v"><span class="ctxq">{{ tokens(ctx.compact_at) }}<em>{{ pct(used / ctx.compact_at) }}</em></span></span>
        </div>
        <div class="drow">
          <span class="k">{{ t("common.model_capacity") }}</span>
          <span class="v"><span class="ctxq">{{ tokens(ctx.window) }}<em>{{ pct(used / ctx.window) }}</em></span></span>
        </div>
        <!-- The fold point marked on the window it is a fraction of. Read as two
             numbers, 160k against 1M is arithmetic nobody does; read as a notch
             this far along, it is the whole answer at a glance. -->
        <div class="ctxcapbar">
          <i :style="{ width: `${Math.min((used / ctx.window) * 100, 100)}%` }" />
          <b :style="{ left: `${Math.min((ctx.compact_at / ctx.window) * 100, 100)}%` }" />
        </div>
      </div>

      <!-- The legend is always drawn rather than behind a hover bubble: the last
           one was a fixed-position card placed by measurement, which the rail's
           own scroller clipped. Five rows say the same thing and cannot. -->
      <div class="ctxlg">
        <div v-for="p in shown" :key="p.key" class="r" :title="t(p.why)">
          <i :data-p="p.key" />
          <span class="t">{{ t(p.label) }}</span>
          <em>{{ pct(p.n / sum) }}</em>
          <b>{{ tokens(p.n) }}</b>
        </div>
      </div>
      <p class="ctxnote">{{ t('panels.an_estimate_measured_the_same') }}</p>
    </template>
  </div>
</template>
