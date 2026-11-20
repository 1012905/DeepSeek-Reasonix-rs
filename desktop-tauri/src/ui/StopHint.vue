<script setup lang="ts">
import { t } from "../i18n";

defineProps<{ armed: boolean }>();
</script>

<template>
  <div class="stophint" role="status" aria-live="polite" :hidden="!armed">
    {{ armed ? t("composer.press_esc_again_to_stop") : "" }}
  </div>
</template>