<script setup lang="ts">
import { computed, ref } from "vue";
import StudioIcon from "./StudioIcon.vue";
import { t } from "../i18n";
import type { ExtensionSurface } from "../port/wire";
import ExtensionView from "./cards/ExtensionView.vue";
import { SLOTS, placement } from "./slots";

// A view plus the one control the host adds to it: where it goes. The handle is
// the host's, not the extension's — an extension asks for a place, and this is
// where the user overrules it. It stays hidden until the surface is hovered,
// because the common case is reading the thing, not moving it.
const props = withDefaults(defineProps<{
  ext: ExtensionSurface;
  assigned?: Record<string, string>;
  onAction: (actionId: string) => void;
  onMove: (slot: string) => void;
}>(), { assigned: () => ({}) });

const open = ref(false);
const grip = ref<HTMLButtonElement | null>(null);
const here = computed(() => placement(props.ext, props.assigned));
// Hiding the menu takes the row that had focus with it, and the caret drops
// to the document — so closing hands it back the way Menu does.
const close = (then: () => void) => { open.value = false; grip.value?.focus(); then(); };
</script>

<template>
  <div class="slotted">
    <ExtensionView :body="ext.view?.body ?? []" :on-action="onAction" />
    <button
      ref="grip"
      class="movegrip"
      :aria-label="t('app.move_id_s_view', { id: ext.pluginId })"
      :aria-expanded="open"
      @click="open = !open"
    >
      <StudioIcon name="more" />
    </button>
    <!-- hidden rather than conditional: the menu has to stay in the DOM to have
         anything to animate on the way out. -->
    <div class="movemenu" role="menu" :hidden="!open">
      <span class="who">{{ ext.pluginId }}</span>
      <button
        v-for="slot in SLOTS"
        :key="slot.id"
        role="menuitemradio"
        data-action="panel.move"
        :aria-checked="here === slot.id"
        @click="close(() => onMove(slot.id))"
      >
        {{ slot.label }}<StudioIcon v-if="here === slot.id" name="check" />
      </button>
      <!-- Clearing hands the choice back to the extension rather than
           hiding the surface: it is a different act from moving it. -->
      <button role="menuitem" data-action="panel.delegate-placement" @click="close(() => onMove(''))">
        {{ t("app.hand_back_to_the_extension") }}
      </button>
    </div>
  </div>
</template>