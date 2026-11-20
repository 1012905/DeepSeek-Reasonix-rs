<script lang="ts">
export interface ManualPageView {
  /** The book's own id; the panel selects on this. */
  id: string;
  /** The address this page is showing, or the start page. */
  url: string;
}

export interface ManualBrowsersProps {
  pages: ManualPageView[];
  /** The page in front. Every other one stays mounted but hidden, so switching
   *  back does not reload a site someone was in the middle of. */
  active: string;
  /** Bumped by a refresh or a navigation; re-mounts the frame that it names. */
  reload: number;
  /** The panel's own blank page, drawn in the frame instead of loading it. */
  start: string;
  onExternal: (url: string) => void;
}
</script>

<script setup lang="ts">
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<ManualBrowsersProps>();

const hostName = (url: string) => {
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
};
</script>

<template>
  <div v-if="!pages.length" class="bpage">
    <b>{{ t("common.built_in_browser") }}</b>
    <p>{{ t("workbench.enter_an_address_above_to") }}</p>
  </div>
  <section
    v-for="page in pages"
    v-show="page.id === active"
    :key="page.id"
    class="studio-browser"
    :aria-label="hostName(page.url) || t('workbench.built_in_browser')"
  >
    <div v-if="page.url === start" class="studio-browser-start">
      <div>
        <StudioIcon name="globe" class="browser-start-mark" :size="26" />
        <h1>{{ t("common.built_in_browser") }}</h1>
        <p>{{ t("workbench.enter_an_address_above_to") }}</p>
        <p class="browser-start-hint">{{ t("workbench.some_sites_do_not_allow") }}</p>
      </div>
    </div>
    <template v-else>
      <iframe
        :key="`${page.url}:${reload}`"
        class="studio-browser-frame"
        :title="t('workbench.reasonix_built_in_browser')"
        :src="page.url"
        sandbox="allow-scripts allow-forms allow-popups"
        referrerpolicy="no-referrer"
      ></iframe>
      <!-- A site that refuses to be framed shows the shell's own "refused to
           connect", so the way out sits beside it rather than on an icon
           somewhere else. -->
      <footer class="studio-browser-note">
        <span>{{ t("workbench.some_sites_do_not_allow") }}</span>
        <button type="button" data-action="browser.external" @click="props.onExternal(page.url)">
          {{ t("workbench.open_in_external_browser") }}
        </button>
      </footer>
    </template>
  </section>
</template>
