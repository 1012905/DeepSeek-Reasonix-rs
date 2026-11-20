<script setup lang="ts">
import { t } from "../../i18n";
import { clock, elapsed } from "../../i18n/format";
import StudioIcon from "../StudioIcon.vue";

/** The turn's close: a tick and how long it ran. When the turn began is the
 *  hover, since the message above carries it. */
const props = defineProps<{ ms: number; startedAt?: number }>();
</script>

<template>
  <div class="rc" data-worked="">
    <div class="rc-ok" :title="startedAt ? t('chat.started_at_t', { at: clock(startedAt) }) : undefined">
      <StudioIcon name="check" class="rc-tick" />
      <span class="rc-t">{{ t("chat.done_in_t", { t: elapsed(props.ms) }) }}</span>
    </div>
  </div>
</template>
