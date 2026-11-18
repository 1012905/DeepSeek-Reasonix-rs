<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onOfficePreviewChange, previewsOffice, setPreviewsOffice } from "../state/prefs";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

const on = useExternal(onOfficePreviewChange, previewsOffice);
</script>

<template>
  <Group id="office-preview" inline :title="t('common.office_document_preview')" :hint="t('settings.preview_office_in_workbench')">
    <Switch data-action="appearance.office-preview" :on="on" :label="t('common.office_document_preview')" :on-click="() => setPreviewsOffice(!on)" />
  </Group>
</template>