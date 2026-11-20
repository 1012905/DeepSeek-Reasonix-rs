<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import { staticDocument } from "./staticDocument";

const props = defineProps<{ text: string }>();
const document = computed(() => staticDocument(props.text));
</script>

<template>
  <p class="document-notice">{{ t("chat.static_preview_scripts_and_external") }}</p>
  <iframe class="document-html" :title="t('chat.static_html_preview')" sandbox="" :srcdoc="document" />
</template>