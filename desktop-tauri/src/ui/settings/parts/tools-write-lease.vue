<script setup lang="ts">
import { ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, WriteLeaseSettings } from "../../../port/port";
import Switch from "../../Switch.vue";

// One switch over how sessions in one project wait on each other. On trades the
// whole-workspace hold for concurrency; off is strict. Neither reaches a session
// already open: the lease is built with the session.
const props = defineProps<{ port: AgentPort }>();

const state = ref<WriteLeaseSettings | null>(null);
const busy = ref(false);
const error = ref("");
const wait = ref("");

watch(() => props.port, (port) => {
  let live = true;
  state.value = null;
  error.value = "";
  port.writeLease()
    .then((s) => {
      if (!live) return;
      state.value = s;
      wait.value = String(s.maxWaitMinutes);
    })
    .catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

const flip = async () => {
  const now = state.value;
  if (!now) return;
  busy.value = true;
  error.value = "";
  try {
    state.value = await props.port.saveWriteLease({ optimistic: !now.optimistic });
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const commitWait = async () => {
  const now = state.value;
  if (!now) return;
  const minutes = Number(wait.value);
  if (!Number.isFinite(minutes) || minutes === now.maxWaitMinutes) return;
  busy.value = true;
  error.value = "";
  try {
    const next = await props.port.saveWriteLease({ maxWaitMinutes: minutes });
    state.value = next;
    wait.value = String(next.maxWaitMinutes);
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <div v-if="!state" class="empty">{{ t("safety.could_not_read_the_multi") }}</div>
  <div v-else class="box">
    <div class="lrow">
      <span class="tx">
        <span class="lb">{{ t("safety.optimistic_mode_sessions_in_parallel") }}</span>
        <span class="ds">{{ t("safety.an_operation_that_declares_no") }}</span>
      </span>
      <Switch
        data-action="write-lease.optimistic"
        :on="state.optimistic"
        :busy="busy"
        :label="t('safety.optimistic_mode_sessions_in_parallel')"
        :on-click="() => void flip()"
      />
    </div>
    <div class="lrow threshold-row">
      <span class="tx">
        <span class="lb">{{ t("safety.longest_wait_for_another_session") }}</span>
        <span class="ds">{{ t("safety.while_another_session_holds_a") }}</span>
      </span>
      <div class="threshold-control">
        <span class="unit-field">
          <input
            v-model="wait"
            type="text"
            inputmode="numeric"
            data-action-keydown="write-lease.max-wait"
            data-action-change="write-lease.max-wait"
            :aria-label="t('safety.longest_wait_for_another_session')"
            :disabled="busy"
            @input="wait = wait.replace(/\D/g, '')"
            @blur="void commitWait()"
            @keydown.enter="($event.target as HTMLInputElement).blur()"
          />
          <i>{{ t("safety.minutes") }}</i>
        </span>
      </div>
    </div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
