<script setup lang="ts">
import { computed } from "vue";
import type { TreeWorkspace } from "../port/hub";
import { t } from "../i18n";

const props = defineProps<{ workspace: TreeWorkspace; searching: boolean; onRetry?: () => void }>();

const failed = computed(() => props.workspace.listingState === "degraded");
</script>

<template>
  <small v-if="workspace.countsComplete === false" class="sessnote" role="status" :aria-busy="!failed">{{ failed ? t("workbench.some_history_summaries_are_unavailable") : searching ? t("workbench.still_searching_some_historical_conversations") : t("workbench.reading_history_summaries") }}<template v-if="failed && onRetry"> <button type="button" class="ws-reading-retry" data-action="tree.retry" @click="onRetry">{{ t("common.retry") }}</button></template></small>
</template>