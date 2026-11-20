<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { HubPort } from "../../../port/hub";
import type { RemoteListing } from "../../../port/remote";
import StudioIcon from "../../StudioIcon.vue";

// Folders on the far machine, read over the link alone: no kernel has to be
// installed over there to answer it, which is why picking one works on a machine
// nothing is open on. An empty path is that machine's login home.
//
// Not the same component as ui/remote/RemoteDirs.vue, and not a copy of it
// that drifted: that one is a full-bleed modal the machines panel opens, with a
// typed path field, Escape, and a focus trap. This one is drawn inline in the
// settings form, where those would fight the sheet already around it. The
// actions are spelled `remotes.dirs-*` against its `remote.dirs-*` so a search
// for either lands on the surface it names.
const props = defineProps<{
  hub: HubPort;
  host: string;
  start: string;
  onClose: () => void;
  onPick: (path: string) => void;
}>();

const listing = ref<RemoteListing | null>(null);
const busy = ref(false);
const error = ref("");

function read(path: string): void {
  busy.value = true;
  error.value = "";
  props.hub.remoteDirs(props.host, path)
    .then((answer) => { listing.value = answer; })
    .catch((e) => { error.value = reason(e); })
    .finally(() => { busy.value = false; });
}

watch(() => [props.hub, props.host, props.start], () => read(props.start), { immediate: true });

const here = () => listing.value?.path ?? "";
</script>

<template>
  <div class="rmtdirs">
    <div class="sublb">{{ t("remote.pick_a_folder_on_host", { host }) }}</div>
    <div class="prule">
      <code dir="ltr">{{ here() || "…" }}</code>
      <button type="button" class="act ghost" data-action="remotes.dirs-up" :disabled="busy || !listing?.parent" @click="read(listing?.parent ?? '')">
        {{ t("remote.up_one_level") }}
      </button>
      <button type="button" class="act" data-action="remotes.dirs-pick" :disabled="busy || !here()" @click="onPick(here())">
        {{ t("remote.use_this_folder") }}
      </button>
      <button type="button" class="act ghost" data-action="remotes.dirs-close" @click="onClose">{{ t("common.cancel") }}</button>
    </div>
    <p v-if="busy" class="rmthint">{{ t("common.loading") }}</p>
    <p v-else-if="error" class="rmthint" role="alert">{{ error }}</p>
    <template v-else-if="listing">
      <p v-if="listing.unlisted" class="rmthint">{{ t("remote.this_machine_will_not_list") }}</p>
      <p v-else-if="listing.folders.length === 0" class="rmthint">{{ t("remote.nothing_below_this_folder_it") }}</p>
      <div v-for="folder in listing.folders" :key="folder.path" class="prule">
        <code dir="ltr">{{ folder.name }}</code>
        <button type="button" class="act ghost" data-action="remotes.dirs-open" :data-target="folder.path" :disabled="busy" @click="read(folder.path)">
          <StudioIcon name="folder" />
        </button>
      </div>
      <p v-if="listing.truncated" class="rmthint">{{ t("remote.this_folder_is_too_big") }}</p>
    </template>
  </div>
</template>
