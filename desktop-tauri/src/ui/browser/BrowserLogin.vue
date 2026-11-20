<script lang="ts">
import { shallowRef } from "vue";
import { host, type BrowserLogin as Ask } from "../../port/host";

// The shell raises each login request once for the whole window, so the queue
// is shared rather than per instance: a panel in every pane would otherwise
// open one card per copy for the same prompt.
const waiting = shallowRef<Ask[]>([]);
let attached = false;

function attach() {
  if (attached) return;
  attached = true;
  host().onBrowserLogin((next) => { waiting.value = [...waiting.value, next]; });
}

/** Drops one ask. An empty username is how the shell is told to cancel. */
function settle(id: string, username: string, password: string) {
  host().answerBrowserLogin(id, username, password);
  waiting.value = waiting.value.filter((ask) => ask.id !== id);
}

/** For tests: what is queued, and the way back to an empty queue. */
export function pendingLogins(): Ask[] {
  return waiting.value;
}

export function resetLogins(next: Ask[] = []) {
  waiting.value = next;
  attached = true;
}
</script>

<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, useId, watch } from "vue";
import { t } from "../../i18n";
import { listenAction } from "../listen";
import { useModalFocus } from "../useModalFocus";
import StudioIcon from "../StudioIcon.vue";

// targets names the pages this panel draws. A page belongs to exactly one tab,
// so this keeps a prompt from being drawn once per open pane.
const props = defineProps<{ targets?: string[] }>();

// A page or a proxy in the built-in browser asking for a login. Without an
// answer the shell cancels it, which is how an intranet site behind Basic or
// NTLM authentication reads as "cannot be opened". What is typed goes to the
// window's main process only; the kernel and the agent never see it.
attach();

const ask = computed(() => waiting.value.find((one) => !props.targets || props.targets.includes(one.targetId)) ?? null);
const sheet = ref<HTMLDivElement | null>(null);
const first = ref<HTMLInputElement | null>(null);
const username = ref("");
const password = ref("");
const uid = useId();
const pid = useId();
const titleId = useId();

useModalFocus(sheet, () => !!ask.value);

const answer = (name: string, secret: string) => {
  const current = ask.value;
  if (!current) return;
  settle(current.id, name, secret);
  username.value = "";
  password.value = "";
};

// A card that leaves the document while its page still waits is the one state
// that must not exist: the shell would hold the page open until it timed out.
onScopeDispose(() => {
  const current = ask.value;
  if (current) settle(current.id, "", "");
});

watch(ask, (next, _was, onCleanup) => {
  if (!next) return;
  username.value = "";
  password.value = "";
  void nextTick(() => first.value?.focus());
  const off = listenAction(window, "keydown", {
    action: "browser.login",
    value: "cancel",
    listener: ((event: KeyboardEvent) => {
      if (event.key === "Escape") answer("", "");
    }) as EventListener,
  });
  onCleanup(off);
}, { immediate: true, flush: "post" });

const where = computed(() => (ask.value?.port ? `${ask.value.host}:${ask.value.port}` : ask.value?.host ?? ""));
const heading = computed(() => (ask.value?.proxy
  ? t("workbench.the_proxy_host_asks_you", { host: where.value })
  : t("workbench.host_asks_you_to_sign", { host: where.value })));
const canSubmit = computed(() => username.value !== "");
const submit = () => { if (canSubmit.value) answer(username.value, password.value); };

const enter = (event: KeyboardEvent) => {
  // IME confirmation is not a submit: the person is still composing the name.
  if (event.key === "Enter" && !event.isComposing) submit();
};
</script>

<template>
  <div v-if="ask" ref="sheet" class="askveil" role="dialog" aria-modal="true" :aria-labelledby="titleId">
    <div class="blogin">
      <div class="blogin-head">
        <b :id="titleId">{{ heading }}</b>
        <button type="button" class="blogin-x" data-action="browser.login" data-value="cancel"
          :aria-label="t('common.close')" :title="t('common.close')" @click="answer('', '')">
          <StudioIcon name="close" />
        </button>
      </div>
      <div class="blogin-body">
        <p v-if="ask.realm" class="blogin-realm">{{ t("workbench.the_site_says_realm", { realm: ask.realm }) }}</p>
        <label :for="uid">{{ t("workbench.username") }}</label>
        <input :id="uid" ref="first" v-model="username" data-action-keydown="browser.login" autocomplete="username" @keydown="enter" />
        <label :for="pid">{{ t("workbench.password") }}</label>
        <input :id="pid" v-model="password" data-action-keydown="browser.login" type="password" autocomplete="current-password" @keydown="enter" />
        <p class="blogin-note"><StudioIcon name="lock" />{{ t("workbench.the_username_and_password_go") }}</p>
      </div>
      <div class="blogin-foot">
        <button type="button" class="blogin-cancel" data-action="browser.login" data-value="cancel" @click="answer('', '')">
          {{ t("common.cancel") }}
        </button>
        <button type="button" class="blogin-go" data-action="browser.login" data-value="submit" :disabled="!canSubmit" @click="submit">
          {{ t("common.sign_in") }}
        </button>
      </div>
    </div>
  </div>
</template>
