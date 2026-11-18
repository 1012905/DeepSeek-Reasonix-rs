<script lang="ts">
export interface OnboardingProjectProps {
  /** The folder the work happens in, once the person has chosen one. */
  onPick: (root: string) => void;
  onSkip: () => void;
  /** Failures the picker hands back. Shown here too: the box is what the
   *  person is looking at, and an error bar behind the guide explains nothing. */
  onError: (e: unknown) => void;
  /** Projects this machine already remembers. Choosing one needs no dialog,
   *  and a folder already listed is the answer to this step for most people. */
  roots?: string[];
}
</script>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { host } from "../port/host";
import { install, useFileDrop } from "./filedrop";
import StudioIcon from "./StudioIcon.vue";

const props = defineProps<OnboardingProjectProps>();

const nameOf = (path: string) => path.replace(/[/\\]+$/, "").split(/[/\\]/).pop() || path;

const dir = ref("");
const typed = ref("");
const manual = ref(false);
const hover = ref(false);
const busy = ref(false);
const err = ref("");
const zone = ref<HTMLElement | null>(null);

// install wires the window's drop listener once, at boot. Wiring it here as
// well is what lets this step work in a window that reached onboarding before
// anything else asked for it; install is idempotent.
onMounted(install);

const register = useFileDrop(
  (d: { paths: string[] }) => {
    const first = d.paths[0];
    if (!first || busy.value) return;
    dir.value = first;
    err.value = "";
  },
  (over: boolean) => { hover.value = over; },
);
watch(zone, (node, _was, onCleanup) => { onCleanup(register(node)); }, { flush: "post" });

const remembered = computed(() => (props.roots ?? []).filter((root) => root && root !== dir.value));

function fail(e: unknown) {
  err.value = reason(e);
  props.onError(e);
}

// null is "this shell has no picker at all" and "" is the dialog dismissed. Only
// the first has something to do about it: a path typed by hand is the fallback
// the note under the field explains.
const pick = async () => {
  if (busy.value) return;
  busy.value = true;
  try {
    const got = await host().pickFolder(dir.value);
    if (got === null) manual.value = true;
    else if (got) {
      dir.value = got;
      err.value = "";
    }
  } catch (e) {
    fail(e);
  } finally {
    busy.value = false;
  }
};

const start = () => {
  if (busy.value) return;
  const root = (dir.value || typed.value).trim();
  if (!root) {
    // Unreachable through the button, which is disabled while empty; kept so a
    // path that was only whitespace can never open a workspace at "".
    err.value = t("kernel.a_folder_path_is_required");
    return;
  }
  busy.value = true;
  err.value = "";
  props.onPick(root);
};
</script>

<template>
  <h1 class="onb-t">{{ t("onboarding.which_project") }}</h1>
  <p class="onb-s">{{ t("onboarding.drop_to_choose") }}</p>

  <div ref="zone" class="onb-drop" :data-over="hover ? '' : undefined" :data-chosen="dir ? '' : undefined">
    <div v-if="dir" class="onb-chosen">
      <StudioIcon name="folder" />
      <b>{{ nameOf(dir) }}</b>
      <small :title="dir">{{ dir }}</small>
    </div>
    <template v-else>
      <StudioIcon name="folder" />
      <span>{{ t("onboarding.drop_a_folder_here") }}</span>
    </template>
    <button type="button" class="onb-skip" data-action="onboarding.pick-folder" :disabled="busy" @click="void pick()">
      {{ t(dir ? "onboarding.choose_another" : "onboarding.choose_a_folder") }}
    </button>
  </div>

  <div v-if="manual" class="onb-field">
    <label for="onb-path">{{ t("session.folder_path_to_add") }}</label>
    <input
      id="onb-path"
      :value="typed"
      spellcheck="false"
      autocomplete="off"
      placeholder="/path/to/project"
      :disabled="busy"
      @input="typed = ($event.target as HTMLInputElement).value"
    />
    <p class="onb-note">{{ t("session.the_folder_picker_cannot_open") }}</p>
  </div>

  <div v-if="remembered.length" class="onb-services" role="listbox" :aria-label="t('onboarding.which_project')">
    <button
      v-for="root in remembered"
      :key="root"
      type="button"
      role="option"
      class="onb-service"
      data-action="onboarding.pick-folder"
      :aria-selected="dir === root"
      :disabled="busy"
      @click="dir = root; typed = ''; err = ''"
    >
      <b>{{ nameOf(root) }}</b>
      <small :title="root">{{ root }}</small>
    </button>
  </div>

  <div v-if="err" class="onb-err" role="alert">{{ err }}</div>

  <div class="onb-acts">
    <button
      class="btn onb-go"
      data-primary
      data-action="onboarding.start"
      :disabled="busy || !(dir || typed).trim()"
      @click="start"
    >{{ t("common.start") }}</button>
    <button type="button" class="onb-skip" data-action="onboarding.skip" :disabled="busy" @click="onSkip">{{ t("onboarding.not_now") }}</button>
  </div>
</template>
