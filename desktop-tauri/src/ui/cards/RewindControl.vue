<script setup lang="ts">
import { computed, onScopeDispose, ref } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import type { Checkpoint, RewindPlan, RewindResult, RewindScope } from "../../port/session";
import { useDismiss } from "../dismiss";
import { pinToViewport, useFollow } from "../place";
import StudioIcon from "../StudioIcon.vue";

// Gap between the trigger and the menu: a portaled menu is placed by
// measurement, so no style rule owns the offset.
const GAP = 7;

// Only edit-tool writes are snapshotted. A turn that also ran shell commands
// has gaps the restore cannot reach, and the kernel refuses such a plan until
// it has been shown — so the second stage displays them from the plan.
const SCOPES: { value: RewindScope; label: string; files: boolean }[] = [
  { value: "both", label: "chat.code_and_conversation", files: true },
  { value: "code", label: "chat.code_only", files: true },
  { value: "conversation", label: "chat.conversation_only", files: false },
];

const GAP_REASONS: Record<string, string> = {
  bash_side_effect: "chat.changes_made_by_bash_commands",
};

type Stage =
  | { at: "closed" }
  | { at: "menu" }
  | { at: "working" }
  | { at: "confirm"; plan: RewindPlan }
  | { at: "failed"; why: string };

const props = withDefaults(defineProps<{
  cp: Checkpoint;
  compact?: boolean;
  onPrepare: (turn: number, scope: RewindScope) => Promise<RewindPlan>;
  onCommit: (planId: string) => Promise<RewindResult>;
}>(), { compact: false });

const stage = ref<Stage>({ at: "closed" });
const request = { id: 0, busy: false };
onScopeDispose(() => { request.id++; });
const close = () => {
  request.id++;
  request.busy = false;
  stage.value = { at: "closed" };
};
const wrap = ref<HTMLDivElement | null>(null);
const btn = ref<HTMLButtonElement | null>(null);
const menu = ref<HTMLDivElement | null>(null);
const open = computed(() => stage.value.at !== "closed");
const stageAt = computed(() => stage.value.at);
const plan = computed(() => (stage.value.at === "confirm" ? stage.value.plan : null));
const whyText = computed(() => (stage.value.at === "failed" ? stage.value.why : ""));
useDismiss(open, wrap, close, menu);

// Below the trigger, right edges flush, and above it when the bottom of the
// window is closer than the menu is tall.
const place = () => {
  const el = menu.value;
  const anchor = btn.value;
  if (!el || !anchor) return;
  const to = anchor.getBoundingClientRect();
  const box = el.getBoundingClientRect();
  const above = to.top - box.height - GAP;
  const fits = to.bottom + GAP + box.height <= innerHeight - 6;
  pinToViewport(el, to.right - box.width, fits || above < 6 ? to.bottom + GAP : above);
};

useFollow(open, place, () => stage.value, { anchor: btn, container: menu });

const work = async (prepare: () => Promise<RewindPlan>, commitNow: boolean) => {
  if (request.busy) return;
  const id = ++request.id;
  request.busy = true;
  stage.value = { at: "working" };
  try {
    const got = await prepare();
    if (request.id !== id) return;
    if (!commitNow && got.requiresConfirmation) stage.value = { at: "confirm", plan: got };
    else {
      await props.onCommit(got.planId);
      if (request.id === id) stage.value = { at: "closed" };
    }
  } catch (e) {
    if (request.id === id) stage.value = { at: "failed", why: reason(e) };
  } finally {
    if (request.id === id) request.busy = false;
  }
};
const pick = (scope: RewindScope) => void work(() => props.onPrepare(props.cp.turn, scope), false);
const commit = (held: RewindPlan) => void work(() => Promise.resolve(held), true);
const commitPlan = () => { if (plan.value) commit(plan.value); };
const openMenu = () => { stage.value = { at: "menu" }; };
</script>

<template>
  <div ref="wrap" class="stepctl rewind" :data-open="open ? '' : undefined">
    <div class="picker">
      <button
        ref="btn"
        aria-haspopup="menu"
        :aria-expanded="open"
        :title="t('chat.take_the_workspace_and_the')"
        :aria-label="compact ? t('chat.back_to_here') : undefined"
        @click="stageAt === 'closed' ? openMenu() : close()"
      >
        <StudioIcon name="rewind" />{{ !compact ? t("chat.back_to_here") : "" }}
      </button>
      <Teleport v-if="open" to="body">
        <!-- Out of the transcript entirely: the card it sits on is both a clip
             and a containing block for a fixed child, so the menu would be cut
             off at the card's edge and again at the scroller's. -->
        <div ref="menu" class="menu rewindmenu" role="menu">
          <template v-if="stageAt === 'menu'">
            <button
              v-for="s in SCOPES.filter((s) => cp.files > 0 || !s.files)"
              :key="s.value"
              class="mi"
              role="menuitem"
              data-action="rewind.prepare"
              @click="pick(s.value)"
            >
              <span class="dot" />
              <span class="tx"><span class="lb">{{ t(s.label) }}</span></span>
              <span v-if="s.files" class="rt">{{ t("common.n_files", { n: cp.files }) }}</span>
            </button>
            <div v-if="cp.files === 0" class="mi plain">
              <span class="dot" />
              <span class="tx">
                <span class="lb">{{ t(cp.forkCopied ? "chat.this_checkpoint_contains_no_file" : "chat.no_files_changed_in_this") }}</span>
              </span>
            </div>
          </template>
          <div v-if="stageAt === 'working'" class="mi plain">
            <span class="dot" />
            <span class="tx"><span class="lb">{{ t("common.restoring") }}</span></span>
          </div>
          <template v-if="stageAt === 'confirm' && plan">
            <div class="mi plain">
              <span class="dot" />
              <span class="tx">
                <span class="lb">{{ t("chat.some_of_this_turns_changes") }}</span>
                <span class="ds">{{ (plan.coverageGaps ?? []).map((g) => t(GAP_REASONS[g.reason] ?? g.detail)).join("；") || t("chat.some_of_the_changes_are") }}</span>
              </span>
            </div>
            <div class="div" />
            <button class="mi" role="menuitem" data-action="rewind.commit" @click="commitPlan">
              <span class="dot" />
              <span class="tx"><span class="lb">{{ t("chat.restore_the_rest_anyway") }}</span></span>
              <span class="rt">{{ t("common.n_files", { n: plan.fileCount }) }}</span>
            </button>
            <button class="mi plain" role="menuitem" @click="close">
              <span class="dot" />
              <span class="tx"><span class="lb">{{ t("common.cancel") }}</span></span>
            </button>
          </template>
          <div v-if="stageAt === 'failed'" class="mi plain">
            <span class="dot" />
            <span class="tx">
              <span class="lb">{{ t("chat.could_not_restore") }}</span>
              <span class="ds">{{ whyText }}</span>
            </span>
          </div>
        </div>
      </Teleport>
    </div>
  </div>
</template>
