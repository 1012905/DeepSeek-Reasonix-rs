<script lang="ts">
import type { AgentPort } from "../../port/port";

/** What the Extensions panel needs. `surfaces` is the count the pane already
 *  reported, so the header can speak before the replay lands; the rows
 *  themselves come from the kernel's replay, which is the only place a surface's
 *  body lives. */
export interface ExtensionsPanelProps {
  port: AgentPort;
  surfaces?: number;
  onError: (e: unknown) => void;
  onOpenSettings?: (at?: string) => void;
}

/** One surface is addressed by the pair, because an extension may publish
 *  several and they are placed independently. */
export const surfaceKey = (s: { pluginId: string; surfaceId: string }) => `${s.pluginId}:${s.surfaceId}`;
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import type { ExtensionSurface } from "../../port/wire";
import SlottedView from "../SlottedView.vue";

// The rail reports; it does not enumerate. A plugin publishing a hundred rows
// gets three here, and the count in the header stays honest about the total.
const SHOWN = 3;

const props = defineProps<ExtensionsPanelProps>();

const all = ref<ExtensionSurface[]>([]);
const incomplete = ref(false);
const note = ref("");

// GET /extensions/surfaces replays what the session had, which is what a rail
// opened mid-session needs: the frames that would have announced them are gone.
const read = () => {
  props.port
    .extensionSurfaces()
    .then((replay) => {
      all.value = replay.surfaces;
      // A log that reached its retention cap says nothing; only a surface frame
      // having been refused means this list is short.
      incomplete.value = replay.incomplete === "dropped";
      note.value = "";
    })
    .catch((e: unknown) => {
      note.value = e instanceof Error ? e.message : String(e);
      props.onError(e);
    });
};

watch(() => props.port, read, { immediate: true });

// A view carrying an anchor has a home elsewhere in the window (the composer,
// the transcript); only the ones with none resolve to a rail slot.
const panels = computed(() => all.value.filter((s) => s.kind === "panel"));
const views = computed(() => all.value.filter((s) => s.kind === "view" && !s.view?.anchor));
const shown = computed(() => panels.value.slice(0, SHOWN));
const more = computed(() => Math.max(0, panels.value.length - shown.value.length));
// The pane's reported count and this list can disagree while a read is in
// flight. The larger is the honest header: a surface that has been announced but
// not yet replayed still exists.
const total = computed(() => Math.max(props.surfaces ?? 0, panels.value.length + views.value.length));

const invoke = (name: string) => {
  props.port
    .invokeExtensionAction(name)
    // The kernel's answer is what the extension said back; an empty one is an
    // action that reports nothing, which is not an error.
    .then((message) => { if (message.trim()) note.value = message; })
    .catch((e: unknown) => props.onError(e));
};
</script>

<template>
  <div v-if="total > 0 || note" class="block" data-b="extpanels">
    <div class="lbl">
      <span>{{ t("ext.extension") }}</span>
      <span class="c">{{ total }}</span>
    </div>
    <p v-if="note" class="ctxnote" role="alert">{{ note }}</p>
    <p v-if="incomplete" class="ctxnote">{{ t("ext.n_more_are_not_listed", { n: "" }) }}</p>

    <div v-for="v in views" :key="surfaceKey(v)" class="extpanel">
      <SlottedView :ext="v" :on-action="invoke" :on-move="() => onOpenSettings?.('extensions')" />
    </div>

    <div v-for="p in shown" :key="surfaceKey(p)" class="extpanel">
      <template v-if="p.panel">
        <div class="extpanel-hd">
          <span class="nm">{{ p.panel.title || p.pluginId }}</span>
          <span v-if="p.panel.title" class="src">{{ p.pluginId }}</span>
        </div>
        <div v-if="p.panel.text" class="extpanel-tx">{{ p.panel.text }}</div>
        <span v-if="p.panel.progress !== undefined" class="extbar" role="progressbar"
          :aria-label="p.panel.title || p.pluginId" :aria-valuenow="Math.round(p.panel.progress * 100)">
          <i :style="{ width: `${Math.max(0, Math.min(1, p.panel.progress)) * 100}%` }" />
        </span>
        <dl v-if="p.panel.fields?.length" class="extkv">
          <div v-for="f in p.panel.fields" :key="f.key">
            <dt>{{ f.key }}</dt>
            <dd>{{ f.value }}</dd>
          </div>
        </dl>
        <div v-if="p.panel.actions?.length" class="extacts">
          <button v-for="a in p.panel.actions" :key="a.actionId" type="button" class="btn"
            @click="invoke(`/${p.pluginId}:${a.actionId}`)">
            {{ a.label || a.actionId }}
          </button>
        </div>
      </template>
    </div>

    <!-- The rest live in Settings rather than behind a scroll: a rail listing
         every surface is a second settings page nobody asked for. -->
    <button v-if="more > 0 && onOpenSettings" type="button" class="extmore" @click="onOpenSettings('extensions')">
      {{ t("ext.n_more_all_in_settings", { n: more }) }}
    </button>
  </div>
</template>
