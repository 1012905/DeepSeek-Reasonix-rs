<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../../../i18n";
import type { Appearance as Look } from "../../../port/port";
import { installed, MONO_FAMILIES, UI_FAMILIES } from "../../look";
import Group from "../../Group.vue";

const props = defineProps<{ look: Look; onLook: (next: Look) => void }>();

// Only families this machine can really draw with, asked of the font system
// rather than assumed per platform. Measured after mount, and skipped where the
// page has no canvas to measure with — an empty list still leaves the field
// free to take any name.
const uiFonts = ref<string[]>([]);
const monoFonts = ref<string[]>([]);
onMounted(() => {
  try {
    uiFonts.value = installed(UI_FAMILIES);
    monoFonts.value = installed(MONO_FAMILIES);
  } catch {
    uiFonts.value = [];
    monoFonts.value = [];
  }
});

const set = (patch: Partial<Look>) => props.onLook({ ...props.look, ...patch });

const placeholder = (options: string[]) =>
  options.length ? t("look.default_n_available_on_this", { n: options.length }) : t("common.default");

// The samples are the value being previewed, not interface prose: they are set
// in the chosen family on purpose, which is the only way to tell whether a name
// actually resolved to something on this machine.
const uiSample = computed(() => t("look.typeface_preview_aa_bb_0123"));
const monoSample = "func main() { fmt.Println(0O1lI) }";

const sample = (value: string | undefined, mono: boolean) =>
  value ? { fontFamily: `"${value}", ${mono ? "monospace" : "sans-serif"}` } : undefined;

const typed = (e: Event, patch: "fontUi" | "fontMono") => {
  set({ [patch]: (e.target as HTMLInputElement).value } as Partial<Look>);
};
</script>

<template>
  <Group id="font" :title="t('look.typeface')" :hint="t('look.enter_a_typeface_name_or')">
    <div class="fontrow">
      <div class="prow">
        <span class="tx">{{ t("look.interface") }}</span>
        <input
          class="fontown"
          :aria-label="t('look.interface')"
          list="fonts-ui"
          :value="look.fontUi ?? ''"
          :placeholder="placeholder(uiFonts)"
          spellcheck="false"
          autocomplete="off"
          data-action="appearance.font"
          data-target="ui"
          @input="typed($event, 'fontUi')"
        />
        <datalist id="fonts-ui">
          <option v-for="f in uiFonts" :key="f" :value="f" />
        </datalist>
        <button
          v-if="look.fontUi"
          class="nowbtn"
          data-action="appearance.font"
          data-target="ui"
          data-value="default"
          :title="t('look.back_to_the_default_typeface')"
          @click="set({ fontUi: '' })"
        >
          {{ t("common.clear_2") }}
        </button>
      </div>
      <div class="fontsample" data-slot="ui" :style="sample(look.fontUi, false)">
        {{ uiSample }}
      </div>
    </div>

    <div class="fontrow">
      <div class="prow">
        <span class="tx">{{ t("look.monospace_font") }}</span>
        <input
          class="fontown"
          :aria-label="t('look.monospace_font')"
          list="fonts-mono"
          :value="look.fontMono ?? ''"
          :placeholder="placeholder(monoFonts)"
          spellcheck="false"
          autocomplete="off"
          data-action="appearance.font"
          data-target="mono"
          @input="typed($event, 'fontMono')"
        />
        <datalist id="fonts-mono">
          <option v-for="f in monoFonts" :key="f" :value="f" />
        </datalist>
        <button
          v-if="look.fontMono"
          class="nowbtn"
          data-action="appearance.font"
          data-target="mono"
          data-value="default"
          :title="t('look.back_to_the_default_typeface')"
          @click="set({ fontMono: '' })"
        >
          {{ t("common.clear_2") }}
        </button>
      </div>
      <div class="fontsample" data-slot="mono" :style="sample(look.fontMono, true)">
        {{ monoSample }}
      </div>
    </div>
  </Group>
</template>
