<script lang="ts">
export const UNCONFIGURED_REF = "unconfigured/none";

export interface ConnectOffer {
  none: boolean;
  open: () => void;
}
</script>

<script setup lang="ts">
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";

defineProps<{ offer: ConnectOffer }>();
</script>

<template>
  <div class="rtbar posturenote" data-kind="connect" data-lvl="warn" role="status">
    <StudioIcon name="warning" />
    <span class="t">{{ offer.none ? t("composer.no_model_service_is_connected") : t("composer.the_selected_model_has_no") }}</span>
    <button data-action="onboarding.reopen" @click="offer.open">{{ t("composer.connect") }}</button>
  </div>
</template>