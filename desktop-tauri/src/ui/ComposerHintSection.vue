<script setup lang="ts">
import { t } from "../i18n";
import { useExternal } from "../state/external";
import { onComposerHintChange, setShowsComposerHint, showsComposerHint } from "../state/prefs";
import Group from "./Group.vue";

const on = useExternal(onComposerHintChange, showsComposerHint);
</script>

<template>
  <Group id="composer-hint" inline :title="t('composer.composer_hint')" :hint="t('settings.hint_text_in_the_composer')">
    <div class="seg" data-text role="group" :aria-label="t('composer.composer_hint')">
      <button data-action="appearance.composer-hint" data-value="off" :aria-pressed="!on" @click="setShowsComposerHint(false)">{{ t("common.close") }}</button>
      <button data-action="appearance.composer-hint" data-value="on" :aria-pressed="on" @click="setShowsComposerHint(true)">{{ t("common.show") }}</button>
    </div>
  </Group>
</template>