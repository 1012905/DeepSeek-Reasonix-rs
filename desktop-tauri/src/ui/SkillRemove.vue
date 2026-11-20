<script setup lang="ts">
import { ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort, SkillEntry } from "../port/port";
import { HttpError } from "../port/http_error";

const CHANGED = "skill.changed";

// What removing a skill would delete, shown before the person agrees. The
// request carries the scope and path drawn here, so a skill that changed under
// the same name since the list was read is refused, not removed.
const props = defineProps<{ port: AgentPort; sk: SkillEntry; onClose: () => void; onDone: () => void }>();
const busy = ref(false);
const error = ref("");
const stale = ref(false);

const remove = async () => {
  busy.value = true;
  error.value = "";
  try {
    await props.port.removeSkill(props.sk.name, props.sk.scope ?? "", props.sk.path ?? "", undefined);
    props.onDone();
  } catch (e) {
    stale.value = e instanceof HttpError && e.reason?.code === CHANGED;
    error.value = reason(e);
    busy.value = false;
  }
};
</script>

<template>
  <div class="skupd mkt addpkg" data-stage="confirm" :aria-busy="busy">
    <div class="find" data-lvl="warn">
      <span class="t">{{ t("ext.remove_the_skill_name", { name: sk.name }) }}</span>
      <span class="why">{{ t(sk.removesFolder ? "ext.this_deletes_the_folder_below" : "ext.this_deletes_the_file_below") }}</span>
      <code>{{ sk.removes }}</code>
    </div>
    <div class="acts">
      <button v-if="stale" class="act" data-action="skills.remove-reload" @click="onDone">{{ t("common.read_again") }}</button>
      <button class="act" data-action="skills.remove-cancel" :disabled="busy" @click="onClose">{{ t("common.cancel") }}</button>
      <button class="act danger" data-action="skills.remove-confirm" :disabled="busy" @click="remove()">{{ t(busy ? "ext.removing" : "ext.remove") }}</button>
    </div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>