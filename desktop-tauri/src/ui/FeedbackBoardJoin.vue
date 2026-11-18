<script setup lang="ts">
import { computed, useId } from "vue";
import { t } from "../i18n";
import { boardNameProblem } from "./boardname";

const props = defineProps<{
  checked: boolean;
  name: string;
  disabled?: boolean;
  onChecked: (on: boolean) => void;
  onName: (name: string) => void;
}>();

const WORDS = {
  short: "feedback.the_name_needs_at_least",
  long: "feedback.the_name_can_have_at",
  chars: "feedback.the_name_may_use_letters",
} as const;

const id = useId();
const typed = computed(() => props.name.trim());
const problem = computed(() => (props.checked && typed.value ? boardNameProblem(typed.value) : null));

const onCheck = (e: Event) => props.onChecked((e.target as HTMLInputElement).checked);
const onInput = (e: Event) => props.onName((e.target as HTMLInputElement).value);
</script>

<template>
  <div class="fbk-board-join">
    <label class="fbk-board-join-pick">
      <input type="checkbox" data-action-change="feedback.board-join-choice" :checked="checked" :disabled="disabled" @change="onCheck" />
      <span>{{ t("feedback.join_the_board") }}</span>
    </label>
    <p class="fbk-hint">{{ t("feedback.the_board_is_public_anyone") }}</p>
    <template v-if="checked">
      <input
        :id="id"
        class="fbk-board-name-field"
        data-action-change="feedback.board-name"
        :aria-label="t('feedback.name_shown_on_the_board')"
        :aria-invalid="problem ? true : undefined"
        :value="name"
        :maxlength="40"
        :disabled="disabled"
        :placeholder="t('feedback.name_shown_on_the_board')"
        @input="onInput"
      />
      <p v-if="problem" class="fbk-hint" role="alert" data-tone="error">{{ t(WORDS[problem]) }}</p>
      <p v-else-if="typed" class="fbk-hint" aria-live="polite">{{ t("feedback.it_will_be_shown_on", { name: typed }) }}</p>
      <p v-else class="fbk-hint">{{ t("feedback.enter_a_name_or_untick") }}</p>
    </template>
  </div>
</template>