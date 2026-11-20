<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { t } from "../../i18n";
import RMark from "../RMark.vue";
import StudioIcon from "../StudioIcon.vue";

// A seven-second loop of what a session looks like: a request, the model
// working, a read, an edit, a run, and the result. It pauses under the pointer,
// a click shows the last frame, and reduced motion starts there.
const CYCLE_MS = 7000;

const calm = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

// The per-line delay is a custom property the sheet reads, not a style Vue
// knows about, so it is handed over untyped.
const at = (n: number) => ({ "--n": n }) as unknown as Record<string, string>;

const final = ref(calm());
const held = ref(false);
const round = ref(0);
let timer: number | undefined;

const stop = () => {
  if (timer !== undefined) window.clearTimeout(timer);
  timer = undefined;
};

// One timer at a time: a cycle that restarts arms the next one itself, so the
// loop never depends on a watcher firing.
const arm = () => {
  stop();
  if (final.value || held.value) return;
  timer = window.setTimeout(() => {
    round.value += 1;
    arm();
  }, CYCLE_MS);
};

onMounted(arm);
onBeforeUnmount(stop);
watch([final, held], arm);
</script>

<template>
  <div
    class="onb-demo"
    aria-hidden="true"
    :data-final="final ? '' : undefined"
    :data-held="held ? '' : undefined"
    data-action-mouseenter="onboarding.demo"
    data-action-mouseleave="onboarding.demo"
    data-action-click="onboarding.demo"
    @mouseenter="held = true"
    @mouseleave="held = false"
    @click="final = true"
  >
    <div :key="round" class="demo-run">
      <p class="d me" :style="at(0)">{{ t("onboarding.demo_ask") }}</p>
      <p class="d think" :style="at(1)"><RMark class="rmark demo-spin" />{{ t("onboarding.demo_thinking") }}</p>
      <p class="d tool" :style="at(3)"><b>{{ t("onboarding.demo_read") }}</b><code>scripts/export.py</code><small>48</small></p>
      <p class="d tool" :style="at(4)"><b>{{ t("onboarding.demo_edit") }}</b><code>scripts/export.py</code><small class="add">+12 −3</small></p>
      <p class="d tool" :style="at(5)"><b>{{ t("onboarding.demo_run") }}</b><code>pytest</code><small class="ok">12 <StudioIcon name="check" /></small></p>
      <p class="d done" :style="at(6)">{{ t("onboarding.demo_done") }}</p>
    </div>
  </div>
</template>
