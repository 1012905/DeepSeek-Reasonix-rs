<script lang="ts">
export interface WelcomeProps {
  /** Into the guide. */
  onStart: () => void;
  /** Past the welcome for this launch. */
  onSkip: () => void;
  /** `welcomeSeen()`: whether this machine has met the application before. A
   *  machine that has is shown the introduction without waiting out the whole
   *  sequence — it is a reminder, not a first meeting. */
  seen: boolean;
}
</script>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { t } from "../i18n";
import { host } from "../port/host";
import logoWordmark from "../assets/logo-wordmark.svg";

const props = defineProps<WelcomeProps>();

// The two beats the sequence is built from, matching the stylesheet's own
// timings (oobesay ends at 9.6s, the collapse follows it). Only the ending is
// decided here: the beats themselves are the sheet's.
const FULL_MS = 9600;
const SHORT_MS = 6200;

const calm = () => typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

// Nothing draws these: `done` is the latch that lets the timer and a key press
// race without both answering, and the other two are only what the sheet's
// selectors animate.
let done = false;
let timer: number | undefined;
const playing = ref(false);
const leaving = ref(false);

// The sheet's oobe-* rows are set in the monospaced face, which says "a machine
// produced this". A product name and the words under it are prose, so the text
// inside them takes the interface face and keeps only their geometry and
// entrance.
const UI_FACE = { fontFamily: "var(--ui)" };
const ACTS = { display: "flex", alignItems: "center", gap: "10px", marginTop: "26px" };

function stop() {
  if (timer !== undefined) window.clearTimeout(timer);
  timer = undefined;
  removeEventListener("keydown", onPress);
  removeEventListener("pointerdown", onPress);
}

function finish() {
  if (done) return;
  done = true;
  stop();
  leaving.value = true;
  window.setTimeout(() => props.onSkip(), 380);
}

// A press on one of the two buttons is theirs: it must neither end the sequence
// under their finger nor race the click they are about to make.
function onPress(e: KeyboardEvent | PointerEvent) {
  if ((e.target as Element | null)?.closest?.("button")) return;
  finish();
}

const start = () => {
  done = true;
  stop();
  props.onStart();
};

const skip = () => {
  done = true;
  stop();
  props.onSkip();
};

onMounted(() => {
  // Reduced motion gets the still scene and its two buttons instead of a
  // sequence to sit through: there is nothing to watch, and a zero-length timer
  // would only make the screen flash past.
  if (calm()) return;
  playing.value = true;
  timer = window.setTimeout(finish, props.seen ? SHORT_MS : FULL_MS);
  addEventListener("keydown", onPress);
  addEventListener("pointerdown", onPress);
});
onBeforeUnmount(stop);

// Double-clicking the brand row means the same thing it means everywhere else
// in this shell: maximise or restore.
function zoomOnTitleBar(e: MouseEvent) {
  const d = document.documentElement.dataset;
  if (!(d.titlebar === "app" && (d.platform === "darwin" || d.platform === "windows"))) return;
  if ((e.target as HTMLElement | null)?.closest("button, input, textarea")) return;
  host().toggleMaximiseWindow();
}
</script>

<template>
  <div
    class="oobe"
    :data-play="playing && !leaving ? '' : undefined"
    :data-leaving="leaving ? '' : undefined"
    @dblclick="zoomOnTitleBar"
  >
    <div class="oobe-glow g1" />
    <div class="oobe-glow g2" />
    <div class="oobe-glow g3" />
    <div class="oobe-vig" />

    <div class="oobe-scene">
      <div class="oobe-lockup">
        <img class="oobe-mark" :src="logoWordmark" alt="Reasonix" draggable="false" />
        <h1 class="oobe-brand" aria-label="Reasonix Studio"><span :style="UI_FACE">studio</span></h1>
      </div>
      <p class="oobe-sub"><span :style="UI_FACE">coding agent</span></p>

      <div class="oobe-lines" aria-live="polite">
        <div class="oobe-line l1">
          <b>{{ t("onboarding.welcome_to_reasonix") }}</b>
        </div>
        <div class="oobe-line l2">
          <b>{{ t("onboarding.you_hand_it_a_task") }}</b>
          <span>{{ t("onboarding.reading_code_checking_facts_doing") }}</span>
        </div>
        <div class="oobe-line l3">
          <b>{{ t("onboarding.every_step_stays_on_the") }}</b>
          <span>{{ t("onboarding.which_file_changed_which_command") }}</span>
        </div>
      </div>

      <!-- The two answers sit under the sequence rather than replacing it: the
           introduction has to still be on screen when the first question is
           asked, or the two read as unrelated. They are also what a reader who
           does not want to wait uses on the first frame. -->
      <div :style="ACTS">
        <button type="button" class="btn onb-go" data-primary data-action="onboarding.begin" @click="start">
          {{ t("onboarding.set_up_reasonix") }}
        </button>
        <button type="button" class="onb-skip" data-action="onboarding.skip" @click="skip">
          {{ t("onboarding.not_now") }}
        </button>
      </div>
    </div>

    <div class="oobe-prog"><i /></div>
    <span class="oobe-skip"><span :style="UI_FACE">{{ t("onboarding.press_any_key_to_skip") }}</span></span>
  </div>
</template>
