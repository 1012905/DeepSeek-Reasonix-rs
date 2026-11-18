<script setup lang="ts">
import { t } from "../i18n";
import StudioIcon from "./StudioIcon.vue";
import { reportProblem } from "./reportproblem";

/** A refused action's reason, kept in the window's corner until it is dismissed.
 *  With onReport it can be sent as a report from where it appeared. */
const props = defineProps<{ text: string; onDismiss: () => void; onReport?: () => void }>();

const report = () => { reportProblem(props.text); props.onReport!(); props.onDismiss(); };
</script>

<template>
  <div class="errbar" role="alert">
    <StudioIcon name="fail" />
    <span>{{ text }}</span>
    <button v-if="onReport" data-action="errbar.report" @click="report">{{ t("app.report_this_problem") }}</button>
    <button @click="onDismiss">{{ t("common.got_it") }}</button>
  </div>
</template>