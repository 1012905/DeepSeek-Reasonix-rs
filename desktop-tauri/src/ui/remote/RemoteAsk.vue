<script lang="ts">
import type { RemoteAsk as RemoteAskQuestion } from "../../port/remote";

// A question the link layer stopped for. Declared in a plain block so the type
// can be imported without the component.
export interface RemoteAskProps {
  ask: RemoteAskQuestion;
  /** The decision. `remember` is only ever true for a fingerprint whose reader
   *  asked to keep it; a secret is never written anywhere, so it carries false.
   *
   *  `text` is the third argument rather than a second callback: a secret is the
   *  form an acceptance takes, not a second decision, and a caller that only
   *  cares about the two booleans (a fingerprint) can ignore it. */
  onDecide: (accept: boolean, remember: boolean, text?: string) => void;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import { listenAction } from "../listen";
import { useModalFocus } from "../useModalFocus";

const props = defineProps<RemoteAskProps>();

const sheet = ref<HTMLElement | null>(null);
useModalFocus(sheet);

const text = ref("");
const box = ref<HTMLInputElement | null>(null);
const decline = ref<HTMLButtonElement | null>(null);

// Off unless the reader turns it on: a fingerprint nobody compared is the one
// thing this dialog exists to prevent, and a remembered one is never asked again.
const remember = ref(false);

const secret = computed(() => props.ask.kind !== "hostkey");
// A question carrying no fingerprint gives nothing to compare, so there is
// nothing an acceptance could have been based on.
const checkable = computed(() => secret.value || !!props.ask.fingerprint);

const title = computed(() =>
  secret.value
    ? props.ask.kind === "password"
      ? t("remote.host_wants_a_password", { host: props.ask.host })
      : t("remote.the_private_key_is_passphrase")
    : t("remote.first_connection_to_host", { host: props.ask.host }),
);

const hint = computed(() =>
  props.ask.identityFile
    ? t("remote.to_unlock_file_it_lives", { file: props.ask.identityFile })
    : t("remote.it_lives_in_memory_for"),
);

const acceptLabel = computed(() =>
  secret.value ? t("common.continue") : remember.value ? t("remote.they_match_remember_it") : t("common.confirm"),
);

/** decide is the only way an answer leaves this component. Both buttons and
 *  Escape reach it; nothing else does, so a question left alone stays
 *  unanswered rather than being accepted by omission. */
function decide(accept: boolean): void {
  if (accept && !checkable.value) return;
  props.onDecide(accept, accept && !secret.value && remember.value, secret.value ? text.value : "");
}

function onInput(event: Event): void {
  text.value = (event.target as HTMLInputElement).value;
}

// A new question is a new dialog: the previous secret and the previous consent
// must not carry into it, and the Escape listener belongs to one question at a
// time so an answer typed for one can never settle the next.
watch(
  () => props.ask.askId,
  (_id, _was, onCleanup) => {
    text.value = "";
    remember.value = false;
    void nextTick(() => {
      (secret.value ? box.value : decline.value)?.focus();
    });
    const off = listenAction(
      window,
      "keydown",
      {
        action: "remote-ask.answer",
        value: "decline",
        listener: ((event: KeyboardEvent) => {
          if (event.key !== "Escape") return;
          event.stopPropagation();
          decide(false);
        }) as EventListener,
      },
      true,
    );
    onCleanup(off);
  },
  { immediate: true, flush: "post" },
);
</script>

<template>
  <div ref="sheet" class="askveil" role="dialog" aria-modal="true" aria-labelledby="remote-ask-t" tabindex="-1">
    <div class="askcard">
      <h2 id="remote-ask-t">{{ title }}</h2>

      <template v-if="secret">
        <p class="h">{{ hint }}</p>
        <input
          ref="box"
          data-action-keydown="remote-ask.answer"
          type="password"
          :value="text"
          :aria-label="title"
          autocomplete="off"
          spellcheck="false"
          @input="onInput"
          @keydown.enter.prevent="decide(true)"
        />
      </template>

      <template v-else>
        <p class="h">{{ t("remote.this_machine_has_not_been") }}</p>
        <dl class="askfacts">
          <dt>{{ t("remote.address") }}</dt>
          <dd>{{ ask.address || ask.host }}</dd>
          <dt>{{ t("remote.algorithm") }}</dt>
          <dd>{{ ask.keyType }}</dd>
          <dt>{{ t("common.fingerprint") }}</dt>
          <!-- Wrapped, never truncated: a fingerprint with its middle cut out is
               one nobody can compare. -->
          <dd class="fp">{{ ask.fingerprint }}</dd>
        </dl>
        <label class="askck">
          <input v-model="remember" data-action="remote-ask.remember" type="checkbox" />
          <!-- The same word the approval cards use for the same choice: this is
               the one place the answer is kept rather than asked again. -->
          <span>{{ t("chat.suggest_remember") }}</span>
        </label>
      </template>

      <div class="askact">
        <!-- The way out takes the focus on a fingerprint: Enter is a reflex, and
             accepting a key nobody read is the one thing this prevents. -->
        <button ref="decline" data-action="remote-ask.answer" data-value="decline" @click="decide(false)">
          {{ t("common.cancel") }}
        </button>
        <button data-action="remote-ask.answer" data-value="accept" data-go :disabled="!checkable" @click="decide(true)">
          {{ acceptLabel }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* The one line .askcard does not already style: its input rule is a text field,
   and a checkbox stretched to its width reads as something to type in. */
.askck { display: flex; align-items: center; gap: 8px; margin-top: 14px; font: var(--w-reg) var(--fs-ui) var(--ui); color: var(--muted); cursor: pointer; }
.askck input { flex: none; width: auto; margin: 0; }
</style>
