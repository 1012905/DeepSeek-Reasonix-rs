<script setup lang="ts">
import { computed, h, provide, ref, watch, watchEffect, type Component } from "vue";
import { t } from "../i18n";
import type { SettingsProps } from "./SettingsProps";
import {
  NAV, SECTION_DESC, SECTION_NAME, SETTING_AT, SETTINGS, pageHome,
  type ApplySemantics, type Section, type SettingEntry,
} from "./prefsnav";
import { searchSettings } from "./settingSearch";
import { pageIndexOf } from "./settingPageIndex";
import { settingScreen } from "./settingScreens";
import { SCOPE_SAID } from "./Group.vue";
import { SettingsLeaveContext, useSettingsLeave } from "./SettingsLeave";
import { listenAction } from "./listen";
import { useModalFocus } from "./useModalFocus";
import { useSettingLanding } from "./useSettingLanding";
import { sectionView } from "./settings/registry";
import Boundary from "./Boundary.vue";
import SettingsNav from "./SettingsNav.vue";
import SettingsPageStyle from "./SettingsPageStyle.vue";
import PageIndex from "./PageIndex.vue";
import StudioIcon from "./StudioIcon.vue";

const props = defineProps<SettingsProps>();

const navigation = useSettingsLeave();
provide(SettingsLeaveContext, navigation.register);

// Two destinations named before the catalogue had sections of its own: the
// rail and the wallet still open them by these words.
const SOLO: Record<string, Section> = { extensions: "ext", spend: "usage" };

/** The section an anchor is drawn on. A caller may name a page and a block that
 *  belongs to another one (display:sandbox), and the catalogue — not the
 *  caller's word — says where the block lives. With no anchor the section
 *  stands, so each section's own view stays reachable. */
const pageFor = (section: Section, anchor: string): Section => SETTING_AT(anchor)?.section ?? section;

/** An `at` is "section:anchor", a bare section, a setting id the model named
 *  (which settingScreen translates), or nothing at all. */
function parseAt(at: string | undefined): { section: Section; anchor: string } {
  const raw = (at ?? "").trim();
  const screen = raw.includes(":") ? raw : settingScreen(raw) ?? raw;
  const [name = "", named = ""] = screen.split(":", 2);
  const anchor = named || (name === "memory" ? "memory" : "");
  const section = (SOLO[name] ?? (name || "session")) as Section;
  return { section: pageFor(section, anchor), anchor };
}

const opened = parseAt(props.at);
const at = ref<Section>(opened.section);
const landed = ref(opened.anchor);
const query = ref("");
const root = ref<HTMLDivElement | null>(null);
const results = ref<HTMLElement | null>(null);
useModalFocus(root);

// The search corpus is this table, not the mounted DOM: sections mount one at
// a time, so reading the screen would only ever find what is already open, and
// mounting every page to make search whole would start every page's reads.
const visible = computed(() => SETTINGS.filter((entry) => !entry.hidden));
const found = computed(() => {
  const q = query.value.trim();
  return q ? searchSettings(visible.value, q) : null;
});
const pageItems = computed(() => pageIndexOf(visible.value, at.value));
const view = computed<Component | undefined>(() => sectionView(at.value));

// A section that is drawn inside another page (providers on model) has no row
// of its own; the row it belongs under is the one to mark while it is shown.
const navRows = computed(() => new Set(NAV.flatMap(([, items]) => items.map(([id]) => id))));
const active = computed<Section>(() => (navRows.value.has(at.value) ? at.value : pageHome(at.value)));
const group = computed(() => NAV.find(([, items]) => items.some(([id]) => id === active.value)));
const name = computed(() => SECTION_NAME[at.value] ?? at.value);
const desc = computed(() => SECTION_DESC[active.value]);

/** Where a search result sits, folded pages included: "代理" under 网络 says
 *  nothing about whether it is this project's proxy or this machine's, which
 *  is what the scope half of the row is for. */
const pathOf = (section: Section): string => {
  const home = pageHome(section);
  const page = t(SECTION_NAME[home] ?? home);
  return home === section ? page : `${page} › ${t(SECTION_NAME[section] ?? section)}`;
};

const APPLY_SAID: Partial<Record<ApplySemantics, string>> = {
  "runtime-rebuild": "settings.rebuilds_the_runtime",
  restart: "settings.takes_effect_after_a_restart",
  "on-open": "settings.applies_to_new_sessions",
  "next-turn": "settings.applies_from_the_next_turn",
  "next-rebuild": "settings.applies_at_the_next_session",
};
const costOf = (entry: SettingEntry): string => {
  const apply = APPLY_SAID[entry.apply];
  return [t(SCOPE_SAID[entry.scope]), apply ? t(apply) : ""].filter(Boolean).join(" · ");
};

const close = () => navigation.leave(() => props.onClose());

const jump = (entry: SettingEntry) => go(entry.section, entry.anchor);

function go(requested: Section, anchor: string) {
  const section = pageFor(requested, anchor);
  const land = () => {
    at.value = section;
    query.value = "";
    landed.value = anchor;
  };
  // The persona editor holds an unsaved draft, so arriving there is a leave
  // like any other; a jump inside the page already shown changes nothing it
  // guards, and asking would only make the jump wait on that draft.
  if (section === at.value && anchor !== "persona") land();
  else navigation.leave(land);
}

function pick(section: Section) {
  if (section === at.value) return;
  navigation.leave(() => {
    landed.value = "";
    at.value = section;
  });
}

function walk(step: number) {
  const options = [...(results.value?.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? [])];
  if (options.length === 0) return;
  const from = options.indexOf(document.activeElement as HTMLButtonElement);
  const to = from < 0
    ? (step > 0 ? 0 : options.length - 1)
    : Math.min(options.length - 1, Math.max(0, from + step));
  options[to].focus();
}

function searchKey(event: KeyboardEvent) {
  // Backing out one layer at a time: the query first, the whole sheet second.
  if (event.key === "Escape" && query.value) {
    event.stopPropagation();
    query.value = "";
    return;
  }
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    walk(event.key === "ArrowDown" ? 1 : -1);
  }
}

// The list is a listbox, so its rows carry the arrows too; Escape gives the
// filter back rather than closing the sheet from underneath the reader.
function listKey(event: KeyboardEvent) {
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Escape") return;
  event.stopPropagation();
  if (event.key === "Escape") {
    query.value = "";
    return;
  }
  event.preventDefault();
  walk(event.key === "ArrowDown" ? 1 : -1);
}

watchEffect((onCleanup) => onCleanup(listenAction(window, "keydown", {
  action: "settings.close",
  listener: ((event: KeyboardEvent) => { if (event.key === "Escape") close(); }) as EventListener,
})));

// Opened again at another setting while already on screen: that is a request
// for the new destination, not for the page that happens to be showing.
watch(() => props.at, (next) => {
  const parsed = parseAt(next);
  at.value = parsed.section;
  landed.value = parsed.anchor;
});

useSettingLanding(root, landed, at, (next) => { landed.value = next; });

/** A section that throws while rendering falls back to this instead of taking
 *  the sheet, and the page's own top, down with it. */
const failed = () => h("div", { class: "find", "data-lvl": "err", role: "alert" }, [
  h("span", { class: "t" }, t("settings.this_settings_section_failed")),
  h("span", { class: "why" }, t("settings.other_sections_and_your_session")),
]);
</script>

<template>
  <div
    ref="root"
    class="prefs"
    role="dialog"
    aria-modal="true"
    aria-labelledby="prefs-title"
    tabindex="-1"
  >
    <div class="prefs-sheet">
      <div class="prefs-hd">
        <h1 id="prefs-title">{{ t("common.settings") }}</h1>
        <input
          v-model="query"
          class="prefs-find"
          type="search"
          data-action-change="settings.search"
          data-action-keydown="settings.search"
          :placeholder="t('settings.search_settings')"
          :aria-label="t('settings.search_settings')"
          spellcheck="false"
          @keydown="searchKey"
        >
        <div v-if="found" ref="results" class="prefs-found" role="listbox" :aria-label="t('settings.search_settings')">
          <p v-if="found.length === 0" class="none">{{ t("settings.no_matching_setting") }}</p>
          <button
            v-for="entry in found"
            :key="entry.anchor"
            type="button"
            role="option"
            :aria-selected="false"
            data-action="settings.section"
            :data-value="entry.section"
            :data-target="entry.anchor"
            @click="jump(entry)"
            @keydown="listKey"
          >
            <span class="where">{{ pathOf(entry.section) }}</span>
            <span class="what">{{ t(entry.title) }}</span>
            <span class="cost">{{ costOf(entry) }}</span>
          </button>
        </div>
        <button
          class="btn prefs-close"
          type="button"
          data-action="settings.close"
          :aria-label="t('common.close')"
          :title="`${t('common.close')} Esc`"
          @click="close"
        >
          <StudioIcon name="close" aria-hidden="true" />
        </button>
      </div>

      <div class="prefs-body">
        <SettingsNav :at="active" :on-select="pick" />
        <div class="prefs-main" role="tabpanel" :aria-labelledby="`prefs-${active}`" :data-sec="at">
          <div class="prefs-pagetitle">
            <span v-if="group?.[0]" class="group">{{ t(group[0]) }}<StudioIcon name="chevron" /></span>
            <h2>{{ t(name) }}</h2>
            <span v-if="desc || pageItems.length" class="desc">
              <span v-if="desc" class="what">{{ t(desc) }}</span>
              <span v-if="pageItems.length" class="count">{{ t("settings.n_items", { n: pageItems.length }) }}</span>
            </span>
            <p class="usual">{{ t("settings.page_usual") }}</p>
          </div>

          <PageIndex :items="pageItems" :on-go="jump" />

          <!-- Keyed on the section: replacing this column is the navigation,
               and it is the one thing here whose mount really is the cause. -->
          <div :key="at" class="prefs-col">
            <SettingsPageStyle :section="at" />
            <Boundary :retry-key="at" :fallback="failed()">
              <component :is="view" v-if="view" :settings-props="props" />
              <div v-else class="find" data-lvl="warn" :data-section-empty="at" role="status">
                <span class="t">{{ t("settings.not_in_this_version_yet") }}</span>
                <span class="why">{{ t(name) }}</span>
              </div>
            </Boundary>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* The sheet is one grid and .prefs-hd is display: contents, so this button is
   placed in the title's cell at its trailing edge rather than taking a row of
   its own; the page title and the nav keep the other cells. */
.prefs-close {
  grid-column: 1;
  grid-row: 1;
  z-index: 6;
  justify-self: end;
  align-self: center;
  width: 28px;
  height: 28px;
  padding: 0;
  margin-inline-end: 10px;
  justify-content: center;
}
</style>
