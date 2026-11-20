<script setup lang="ts">
import { computed, ref, type CSSProperties } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, ThemePack } from "../../../port/port";
import { useThemeInventory } from "../../useThemeInventory";
import StudioIcon from "../../StudioIcon.vue";
import ThemeRemove from "../../ThemeRemove.vue";
import Group from "../../Group.vue";

const props = defineProps<{ port: AgentPort; reloadThemes: () => void }>();

const { packs, unread, load } = useThemeInventory(() => props.port);
const failed = ref("");
const importing = ref(false);
const note = ref("");
const landed = ref("");
const file = ref<HTMLInputElement | null>(null);
const broken = ref<Record<string, boolean>>({});

// A zip carries a whole pack; loose files are a pack's folder opened and
// selected, which is what a page can read without learning a path.
const ACCEPT = ".zip,.json,.webp,.png,.jpg,.jpeg";

/** The card paints itself from the pack's own tokens, so a palette is picked by
 *  looking at it rather than by reading its name — and a pack with no picture
 *  or whose picture fails to load still previews correctly. The resolved scheme
 *  is read off the document, which is the only place "auto" has an answer. */
const paletteStyle = (pack?: ThemePack): CSSProperties | undefined => {
  const scheme = document.documentElement.dataset.theme === "light" ? "light" : "dark";
  const tokens = pack && (pack.tokens[scheme] ?? pack.tokens.light ?? pack.tokens.dark);
  if (!tokens) return undefined;
  return {
    "--pal-page": tokens.bg,
    "--pal-surface": tokens.bgSoft ?? tokens.bg,
    "--pal-panel": tokens.panel ?? tokens.bgSoft ?? tokens.bg,
    "--pal-border": tokens.border,
    "--pal-text": tokens.fg,
    "--pal-dim": tokens.fgDim ?? tokens.fg,
    "--pal-accent": tokens.accent,
  } as CSSProperties;
};

const preview = (p: ThemePack) => { broken.value = { ...broken.value, [p.id]: true }; };

// Activating repaints through the window's own theme effect, so this only
// refreshes the list and asks for the pack to be re-read. A refused activation
// says why here, beside the thing that did not move.
const pick = (id: string) => {
  failed.value = "";
  props.port
    .activateTheme(id)
    .then(() => { load(); props.reloadThemes(); })
    .catch((e) => { failed.value = reason(e); });
};

const remove = (id: string) =>
  props.port
    .removeTheme(id)
    .then(() => { load(); props.reloadThemes(); })
    .catch((e) => { failed.value = reason(e); });

const custom = computed(() => packs.value.some((p) => p.active));
// Calm, workbench-like packs stay on the first shelf. Illustrated packs are
// still first-class, but they no longer dominate a page opened merely to switch
// between light and dark. An active illustrated pack stays beside them so the
// current state never disappears into a closed disclosure.
const foundations = computed(() => packs.value.filter((p) => !p.background?.image && !p.sky));
const activeIllustrated = computed(() => packs.value.find((p) => p.active && (p.background?.image || p.sky)));
const featured = computed(() => (activeIllustrated.value ? [...foundations.value, activeIllustrated.value] : foundations.value));
const illustrated = computed(() =>
  packs.value.filter((p) => (p.background?.image || p.sky) && p.id !== activeIllustrated.value?.id));
const warned = computed(() => packs.value.filter((p) => p.warnings?.length));

// Importing installs and offers the new pack rather than activating it:
// installing a theme is not the same decision as using it.
const take = (files: File[]) => {
  if (!files.length || importing.value) return;
  importing.value = true;
  failed.value = "";
  note.value = "";
  landed.value = "";
  props.port
    .importTheme(files)
    .then((got) => {
      load();
      const skipped = got.ignored?.length ? " " + t("look.not_read_names", { names: got.ignored.join("、") }) : "";
      note.value = t("look.imported_name", { name: got.pack.name }) + skipped;
      landed.value = got.pack.id;
    })
    .catch((e) => { failed.value = reason(e); })
    .finally(() => { importing.value = false; });
};

const reveal = () => {
  failed.value = "";
  landed.value = "";
  props.port
    .openThemeFolder()
    .then((dir) => { note.value = t("look.themes_folder_path", { path: dir }); })
    .catch((e) => { failed.value = reason(e); });
};

const chosen = (e: Event) => {
  const input = e.target as HTMLInputElement;
  take([...(input.files ?? [])]);
  input.value = "";
};
</script>

<template>
  <Group
    id="scheme"
    :title="t('look.interface_theme')"
    :now="t('look.in_effect_now')"
    :hint="t('look.choose_the_workbench_surfaces_and')"
  >
    <div v-if="failed || unread" class="find" data-lvl="err" role="alert">
      <span class="t">{{ failed || unread }}</span>
    </div>

    <div class="palettes" role="group" :aria-label="t('look.interface_theme')">
      <button
        class="pal"
        type="button"
        data-action="theme.activate"
        data-value=""
        :data-on="custom ? undefined : ''"
        :aria-pressed="!custom"
        :title="t('common.default')"
        @click="pick('')"
      >
        <span class="pal-art">
          <span class="pal-rail" />
          <span class="pal-body">
            <span class="pal-line" />
            <span class="pal-line" data-short />
            <span class="pal-mark" />
          </span>
        </span>
        <span class="pal-nm"><b>{{ t("common.default") }}</b><em>{{ t("common.built_in") }}</em></span>
      </button>

      <div v-for="p in featured" :key="p.id" class="pal-cell">
        <button
          class="pal"
          type="button"
          data-action="theme.activate"
          :data-value="p.id"
          :data-on="p.active ? '' : undefined"
          :aria-pressed="!!p.active"
          :title="p.description ? t(p.description) : p.name"
          @click="pick(p.id)"
        >
          <span class="pal-art" :style="paletteStyle(p)">
            <span class="pal-rail" />
            <span class="pal-body">
              <span class="pal-line" />
              <span class="pal-line" data-short />
              <span class="pal-mark" />
            </span>
            <img
              v-if="p.hasPreview && !broken[p.id]"
              class="pal-shot"
              :src="`/themes/${encodeURIComponent(p.id)}/preview`"
              alt=""
              loading="lazy"
              @error="preview(p)"
            />
          </span>
          <span class="pal-nm">
            <b>{{ t(p.name) }}</b>
            <em>{{ p.author || t("look.third_party") }}</em>
          </span>
        </button>
        <ThemeRemove v-if="p.removable" :name="p.name" :on-remove="() => remove(p.id)" />
      </div>
    </div>

    <details v-if="illustrated.length" class="theme-more" open>
      <summary>
        <span>
          <b>{{ t("look.more_installed_themes") }}</b>
          <em>{{ t("look.illustrated_and_background_themes_you") }}</em>
        </span>
        <span class="now">{{ t("common.n", { n: illustrated.length }) }}</span>
      </summary>
      <div class="palettes" role="group" :aria-label="t('look.more_installed_themes')">
        <div v-for="p in illustrated" :key="p.id" class="pal-cell">
          <button
            class="pal"
            type="button"
            data-action="theme.activate"
            :data-value="p.id"
            :data-on="p.active ? '' : undefined"
            :aria-pressed="!!p.active"
            :title="p.description ? t(p.description) : p.name"
            @click="pick(p.id)"
          >
            <span class="pal-art" :style="paletteStyle(p)">
              <span class="pal-rail" />
              <span class="pal-body">
                <span class="pal-line" />
                <span class="pal-line" data-short />
                <span class="pal-mark" />
              </span>
              <img
                v-if="p.hasPreview && !broken[p.id]"
                class="pal-shot"
                :src="`/themes/${encodeURIComponent(p.id)}/preview`"
                alt=""
                loading="lazy"
                @error="preview(p)"
              />
            </span>
            <span class="pal-nm">
              <b>{{ t(p.name) }}</b>
              <em>{{ p.author || t("look.third_party") }}</em>
            </span>
          </button>
          <ThemeRemove v-if="p.removable" :name="p.name" :on-remove="() => remove(p.id)" />
        </div>
      </div>
    </details>

    <!-- A pack loads with its good tokens and says which ones it lost. The
         author is the only one who can fix that, and they will not read a log —
         so it is here, beside the thing that looks wrong. -->
    <div v-for="p in warned" :key="`warn-${p.id}`" class="find" data-lvl="warn">
      <span class="t">{{ t("look.a_few_of_name_s", { name: p.name }) }}</span>
      <span v-for="w in p.warnings" :key="w" class="why">{{ w }}</span>
    </div>

    <input ref="file" type="file" multiple :accept="ACCEPT" :disabled="importing" hidden data-action="theme.import" @change="chosen" />
    <button
      class="paperpick"
      data-action="theme.import"
      :data-busy="importing ? '' : undefined"
      :disabled="importing"
      @click="file?.click()"
    >
      <span class="plus" aria-hidden="true"><StudioIcon name="upload" /></span>
      {{ t(importing ? "common.importing" : "look.import_theme") }}
    </button>
    <div class="themeacts" :data-landed="landed ? '' : undefined">
      <p class="note" role="status" aria-atomic="true">
        {{ note || t(packs.length === 0 ? "look.no_themes_installed_choose_a" : "look.choose_a_zip_or_select") }}
      </p>
      <button v-if="landed" class="btn sm" data-action="theme.activate" @click="pick(landed); landed = ''">
        {{ t("look.use_now") }}
      </button>
      <button class="btn sm" data-action="theme.folder" :disabled="importing" @click="reveal">
        {{ t("look.open_themes_folder") }}
      </button>
    </div>
  </Group>
</template>
