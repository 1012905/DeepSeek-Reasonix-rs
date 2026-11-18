<script setup lang="ts">
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";
import type { SkillPin } from "./skillpin";

/** A skill invocation as a message: the skill's name, what the person typed
 *  after it, and the skill's own text folded away until asked for. */
defineProps<{ pin: SkillPin }>();
</script>

<template>
  <details class="skillpin">
    <summary>
      <StudioIcon name="spark" />
      <b>{{ pin.name }}</b>
      <span v-if="pin.args" class="skillpin-args">{{ pin.args }}</span>
      <span class="skillpin-more">{{ t("chat.skill_text") }}</span>
    </summary>
    <p v-if="pin.description" class="skillpin-desc">{{ pin.description }}</p>
    <pre class="skillpin-text">{{ pin.text }}</pre>
  </details>
</template>
