<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";
import type { BrowserTab } from "../port/port";

const props = defineProps<{ tab: BrowserTab }>();

const LONGEST = 500;
const UNSEEN = /[\u0000-\u0008\u000E-\u001F\u007F-\u009F\u061C\u200B-\u200F\u202A-\u202E\u2060-\u2064\u2066-\u2069\uFEFF]/g;

function plain(text: string): string {
  const clean = text.replace(UNSEEN, "").replace(/\s+/g, " ").trim();
  const marks = Array.from(clean);
  return marks.length > LONGEST ? `${marks.slice(0, LONGEST).join("")}…` : clean;
}

const url = computed(() => plain(props.tab.url));
</script>

<template>
  <div class="bpage" role="region" :aria-label="t('persona.page_the_agent_is_viewing')">
    <b data-page-title="true" dir="auto">{{ plain(tab.title) || t("common.blank_page") }}</b>
    <code v-if="url" data-page-url dir="ltr">{{ url }}</code>
    <p>{{ t("persona.read_only_the_page_itself") }}</p>
  </div>
</template>