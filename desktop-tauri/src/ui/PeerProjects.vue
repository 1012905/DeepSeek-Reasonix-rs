<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { HubPort, TreeWorkspace } from "../port/hub";
import type { AgentPort, PeersSettings } from "../port/port";
import type { PeersSettingsStore } from "./usePeersSettings";

// The projects whose sessions take a message from the user's other sessions
// without a card. Adding one is choosing from the projects the person has already
// added to this window; taking one back is always one press. The list is written
// whole, so it is read again just before each change: another window may have
// changed it since this one looked.
const props = defineProps<{ port: AgentPort; hub: Pick<HubPort, "tree">; settings: PeersSettingsStore }>();

const state = computed(() => props.settings.state.value);
const loadError = computed(() => props.settings.loadError.value);
const publish = (next: PeersSettings) => props.settings.publish(next);

const known = ref<TreeWorkspace[]>([]);
const pick = ref("");
const busy = ref(false);
const error = ref("");

watch(() => props.hub, (hub, _was, onCleanup) => {
  let alive = true;
  hub.tree({ limit: 1 }).then(
    (tree) => { if (alive) known.value = tree.filter((w) => w.remembered && w.root.length > 0); },
    () => { if (alive) known.value = []; },
  );
  onCleanup(() => { alive = false; });
}, { immediate: true });

const write = async (change: (current: string[]) => string[]) => {
  busy.value = true;
  error.value = "";
  try {
    const fresh = await props.port.peersSettings();
    publish(await props.port.savePeersSettings({ release_projects: change(fresh.release_projects.map((p) => p.path)) }));
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const revoke = (path: string) => void write((now) => now.filter((p) => p !== path));
const allow = () => void write((now) => [...now, target.value]);
const onPick = (e: Event) => { pick.value = (e.target as HTMLSelectElement).value; };

const shownError = computed(() => error.value || loadError.value);
const released = computed(() => new Set((state.value?.release_projects ?? []).map((p) => p.path)));
const choices = computed(() => known.value.filter((w) => !released.value.has(w.root)));
const target = computed(() => (choices.value.some((w) => w.root === pick.value) ? pick.value : choices.value[0]?.root ?? ""));
const off = computed(() => busy.value || !state.value?.available);
</script>

<template>
  <div v-if="!state && shownError" class="why" role="alert">{{ shownError }}</div>
  <div v-else-if="state" class="box">
    <ul v-if="state.release_projects.length > 0" class="peerreleased" :aria-label="t('remote.projects_released_for_good')">
      <li v-for="p in state.release_projects" :key="p.path">
        <span class="tx"><span class="lb">{{ p.name }}</span><span class="ds">{{ p.path }}</span></span>
        <button class="act" data-action="peers.release-revoke" :data-target="p.path" :disabled="off"
          :aria-label="t('remote.revoke_release_name', { name: p.name })"
          @click="revoke(p.path)">
          {{ t("remote.revoke") }}
        </button>
      </li>
    </ul>
    <div v-else class="peernote">{{ t("remote.no_project_is_allowed_through") }}</div>
    <div class="peerform">
      <div v-if="!state.enabled" class="peernote">{{ t("remote.turn_on_session_messages_first") }}</div>
      <div v-else-if="choices.length === 0" class="peernote">{{ t("remote.there_is_no_project_to") }}</div>
      <template v-else>
        <label class="peerhead" for="peer-release-pick">{{ t("remote.allow_this_project_through") }}</label>
        <select id="peer-release-pick" class="peerfield" data-action-change="peers.release-pick" :aria-label="t('remote.allow_this_project_through')" :value="target" :disabled="off" @change="onPick">
          <option v-for="w in choices" :key="w.root" :value="w.root">{{ `${w.name} — ${w.root}` }}</option>
        </select>
        <div class="acts">
          <button class="act" data-action="peers.release-add" data-primary="true" :disabled="off || !target" @click="allow">{{ t("common.allow") }}</button>
        </div>
      </template>
    </div>
    <div v-if="!state.available" class="peernote" role="status">{{ t("remote.not_supported_in_this_session") }}</div>
    <div v-if="state.pending" class="peernote" role="status">{{ t("remote.saved_takes_effect_from_the") }}</div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
