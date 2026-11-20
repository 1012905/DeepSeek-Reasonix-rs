<script lang="ts">
// Folders on another machine, read over the link alone: no kernel has to be
// installed over there to answer, which is why browsing works on a machine
// nothing is open on. Declared without the component so the type is importable.
//
// This is the modal. settings/parts/data-remote-dirs.vue draws a folder list on
// the same endpoint as a row group inside the settings form, and is not this
// component inlined: a Teleport with aria-modal and a focus trap cannot be used
// from inside a sheet that already is one. The two spell their actions apart
// (`remote.dirs-*` here, `remotes.dirs-*` there) on purpose.
export interface RemoteDirsProps {
  hub: import("../../port/hub").HubPort;
  host: string;
  /** Where to open on, verbatim: the machine's own default beats its login
   *  home as a first screen, and a path cut on this side is what "up" is for. */
  start?: string;
  onPick: (path: string) => void;
  onClose: () => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import { reason } from "../../i18n/kernel";
import type { RemoteListing } from "../../port/remote";
import StudioIcon from "../StudioIcon.vue";
import { useModalFocus } from "../useModalFocus";

const props = defineProps<RemoteDirsProps>();

const sheet = ref<HTMLElement | null>(null);
const box = ref<HTMLInputElement | null>(null);

useModalFocus(sheet);

const listing = ref<RemoteListing | null>(null);
// What the field holds, kept apart from where we are: typing is not navigating,
// or every keystroke would be a round trip to another machine.
const draft = ref(props.start ?? "");
const busy = ref(true);
const error = ref("");
// Which listing answers on screen. A slower earlier answer arriving late must
// not replace the folder the reader has since moved to.
let generation = 0;

async function go(path: string): Promise<void> {
  const mine = ++generation;
  busy.value = true;
  error.value = "";
  try {
    const answer = await props.hub.remoteDirs(props.host, path);
    if (mine !== generation) return;
    listing.value = answer;
    // The far machine resolved it — ~ is expanded over there and a symlink
    // answers with what it points at. Showing what was typed instead would put
    // a path in the field that stepping up from leads somewhere else.
    draft.value = answer.path;
  } catch (e) {
    // The rows on screen stay: a mistyped path is fixed by editing the field,
    // and clearing them takes away the only account of where the reader is.
    if (mine === generation) error.value = reason(e);
  } finally {
    if (mine === generation) busy.value = false;
  }
}

const here = computed(() => listing.value?.path ?? "");

watch(
  () => [props.hub, props.host, props.start] as const,
  (_v, _was, onCleanup) => {
    void go(props.start ?? "");
    void nextTick(() => box.value?.focus());
    const off = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      props.onClose();
    };
    addEventListener("keydown", off, true);
    onCleanup(() => removeEventListener("keydown", off, true));
  },
  { immediate: true, flush: "post" },
);

function submit(): void {
  if (busy.value) return;
  void go(draft.value);
}

function stepUp(): void {
  if (busy.value || !listing.value?.parent) return;
  void go(listing.value.parent);
}
</script>

<template>
  <Teleport to="body">
    <div
      ref="sheet"
      class="pickveil"
      role="dialog"
      aria-modal="true"
      aria-labelledby="remote-dirs-t"
      tabindex="-1"
    >
      <div class="pickcard">
        <h2 id="remote-dirs-t">{{ t("remote.pick_a_folder_on_host", { host }) }}</h2>

        <form class="pickpath" @submit.prevent="submit">
          <button

            type="button"
            class="pickup"
            data-action="remote.dirs-up"
            :disabled="busy || !listing?.parent"
            :title="t('remote.up_one_level')"
            :aria-label="t('remote.up_one_level')"
            @click="stepUp"
          >
            <StudioIcon name="arrow" />
          </button>
          <input
            ref="box"
            v-model="draft"
            spellcheck="false"
            dir="ltr"
            :placeholder="t('remote.a_path_on_that_machine')"
            :aria-label="t('remote.a_path_on_that_machine')"
          />
          <button type="submit" class="pickgo" data-action="remote.dirs-go" :disabled="busy || !draft.trim()">
            {{ t("remote.go") }}
          </button>
        </form>

        <p v-if="error" class="pickerr" role="alert">{{ error }}</p>

        <div class="picklist" :aria-label="t('remote.subfolders')" :data-busy="busy ? '' : undefined">
          <button
            v-for="folder in listing?.folders ?? []"
            :key="folder.path"
            type="button"
            class="pickrow"
            data-action="remote.dirs-open"
            :data-target="folder.path"
            :disabled="busy"
            @click="go(folder.path)"
          >
            <StudioIcon name="folder" />
            <span dir="ltr">{{ folder.name }}</span>
          </button>
          <!-- The first answer may be a cold dial, and it may stop to ask for a
               host key on the way. A dimmed empty box says none of that. -->
          <p v-if="!listing && busy" class="pickempty">{{ t("remote.connecting") }}</p>
          <p v-else-if="listing && !listing.folders.length && !listing.unlisted && !busy" class="pickempty">
            {{ t("remote.nothing_below_this_folder_it") }}
          </p>
        </div>

        <!-- Empty rows under a folder that has contents would read as "nothing
             here", and a cap that says nothing reads as "not there". -->
        <p v-if="listing?.unlisted" class="picknote">{{ t("remote.this_machine_will_not_list") }}</p>
        <p v-else-if="listing?.truncated" class="picknote">{{ t("remote.this_folder_is_too_big") }}</p>

        <div class="pickact">
          <button type="button" data-action="remote.dirs-close" @click="onClose">{{ t("common.cancel") }}</button>
          <button type="button" data-go data-action="remote.dirs-pick" :disabled="!here || busy" @click="onPick(here)">
            {{ t("remote.use_this_folder") }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
