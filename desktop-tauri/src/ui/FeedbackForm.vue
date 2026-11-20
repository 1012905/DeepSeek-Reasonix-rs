<script lang="ts">
import { defineComponent, type VNodeChild } from "vue";

export const SECURITY_URL = "https://github.com/esengine/DeepSeek-Reasonix/security/policy";

const Node = defineComponent({ props: ["value"], setup(props) { return () => props.value as VNodeChild; } });
</script>

<script setup lang="ts">
import { computed, h, nextTick, onMounted, onUnmounted, ref, watch, type Ref } from "vue";
import { t } from "../i18n";
import { systemLabel } from "./osname";
import { memoryLines } from "./memoryrows";
import { tx } from "../i18n/rich";
import { FEEDBACK_CATEGORIES, FEEDBACK_CODE, FEEDBACK_REPO_ISSUES, type FeedbackCategory, type FeedbackEnv, type FeedbackProcessMemory, type FeedbackReceipt } from "../port/feedback";
import { host } from "../port/host";
import type { AgentPort } from "../port/port";
import CopyButton from "./CopyButton.vue";
import StudioIcon from "./StudioIcon.vue";
import { dropDraft, heldDraft, holdDraft, rememberContact } from "./feedbackdraft";
import { feedbackFailure, type FeedbackFailure } from "./feedbackfailure";
import FeedbackShot from "./FeedbackShot.vue";
import LazyMarkdown from "./LazyMarkdown.vue";
import { admit, megabytes, payload, readShot, REFUSED_LABEL, type Refused, type Shot } from "./feedbackshots";
import { useFileDrop } from "./filedrop";
import { listenAction } from "./listen";

const CATEGORY_LABEL: Record<FeedbackCategory, string> = {
  bug: "feedback.bug",
  idea: "feedback.idea",
  question: "feedback.question",
  other: "ext.other",
};

const CATEGORY_HINT: Record<FeedbackCategory, string> = {
  bug: "feedback.bug_hint",
  idea: "feedback.idea_hint",
  question: "feedback.question_hint",
  other: "feedback.other_hint",
};

const encoder = new TextEncoder();

function uuid(): string {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return "fb-" + Date.now().toString(36) + Math.random().toString(36).slice(2);
}

const props = defineProps<{
  port: AgentPort;
  // A ref to the box the paste listener belongs on. The parent passes the ref
  // itself, which the template type checker reads as already unwrapped.
  pasteRoot: Ref<HTMLElement | null> | HTMLElement | null;
  active: boolean;
  onMine: () => void;
  onClose: () => void;
  onFile: (url: string) => void;
  onConceal: (hidden: boolean) => void;
}>();

const pasteNode = (): HTMLElement | null => {
  const root = props.pasteRoot;
  return root !== null && typeof root === "object" && "value" in root ? (root as Ref<HTMLElement | null>).value : root;
};

const held = heldDraft();
const env = ref<FeedbackEnv | null>(null);
const shellMemory = ref<FeedbackProcessMemory[]>([]);
const envFailure = ref<FeedbackFailure | null>(null);
const category = ref<FeedbackCategory>(held.category);
const body = ref(held.body);
const contact = ref(held.contact);
const preview = ref(false);
const name = ref("");
const shots = ref<Shot[]>(held.shots);
const refused = ref<Refused[]>([]);
const over = ref(false);
const phase = ref<"editing" | "sending" | "sent">("editing");
const failure = ref<FeedbackFailure | null>(null);
const logsFailed = ref(false);
const sent = ref<FeedbackReceipt | null>(null);
let key = uuid();
let attempt = "";
let shotEpoch = 0;
let mounted = true;
let focusFrame = 0;

const picker = ref<HTMLInputElement | null>(null);
const field = ref<HTMLTextAreaElement | null>(null);
const submitBtn = ref<HTMLButtonElement | null>(null);
const done = ref<HTMLElement | null>(null);
const captureButton = ref<HTMLButtonElement | null>(null);
const capture = ref<"idle" | "taking" | "failed">("idle");

const shell = host();
const canCapture = shell.capturesWindow();
const revealsLogs = shell.revealsLogs();

const load = () => {
  envFailure.value = null;
  props.port
    .feedbackEnv(document.documentElement.lang)
    .then((e) => {
      env.value = e;
      if (!name.value) name.value = e.displayName;
    })
    .catch((e) => { envFailure.value = feedbackFailure(e); });
};
watch(() => props.port, load, { immediate: true });

let aliveMemory = true;
onMounted(() => {
  void shell.processMemory().then((rows) => { if (aliveMemory) shellMemory.value = rows; });
});
onUnmounted(() => {
  aliveMemory = false;
  mounted = false;
  cancelAnimationFrame(focusFrame);
});

watch(env, (e) => {
  if (e && document.activeElement?.closest('[role="tablist"]')) field.value?.focus();
});

watch([category, body, contact, shots, phase], () => {
  if (phase.value !== "sent") holdDraft({ category: category.value, body: body.value, contact: contact.value, shots: shots.value });
});

watch(phase, async (now) => {
  if (now !== "sent") return;
  await nextTick();
  done.value?.focus();
});

watch(failure, (f) => {
  if (f) submitBtn.value?.focus();
});

watch(() => env.value?.limits, (limits) => {
  if (!limits) return;
  const fit = shots.value.filter((s) => s.bytes <= limits.uploadBytes).slice(0, limits.images);
  if (fit.length !== shots.value.length) shots.value = fit;
});

const add = async (files: File[]) => {
  const limits = env.value?.limits;
  if (!limits || files.length === 0) return;
  const { taken, refused: no } = admit(shots.value.length, files, limits);
  refused.value = no;
  if (taken.length === 0) return;
  const epoch = shotEpoch;
  const read = await Promise.allSettled(taken.map(readShot));
  if (epoch !== shotEpoch) return;
  const ok = read.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
  shots.value = [...shots.value, ...ok].slice(0, limits.images);
};

const takeWindow = async () => {
  if (capture.value === "taking") return;
  capture.value = "taking";
  props.onConceal(true);
  await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(r, 60))));
  const bytes = await shell.captureWindow();
  // The send panel is mounted for as long as the dialog is, so a form that is gone
  // has nothing left to unveil, and setting state then is a write into a torn-down page.
  if (!mounted) return;
  props.onConceal(false);
  capture.value = bytes ? "idle" : "failed";
  if (bytes) await add([new File([bytes as BlobPart], "window.png", { type: "image/png" })]);
  if (mounted) focusFrame = requestAnimationFrame(() => captureButton.value?.focus());
};

const zoneEl = ref<HTMLElement | null>(null);
const registerZone = useFileDrop((d) => void add(d.files), (o) => { over.value = o; });
watch(zoneEl, (node, _was, onCleanup) => {
  onCleanup(registerZone(node));
}, { flush: "post" });

// Registered in the layout phase, so the listener exists in the same commit that
// draws the form: a paste in the frame after the form appears is taken, not dropped.
watch(
  [() => pasteNode(), () => props.active, phase, () => env.value?.limits],
  ([root, active, now, limits], _was, onCleanup) => {
    if (!root || !active || now !== "editing" || !limits) return;
    onCleanup(listenAction(root, "paste", {
      action: "feedback.shot.add",
      listener: (event) => {
        const data = (event as ClipboardEvent).clipboardData;
        if (!data) return;
        const offered = [...data.files, ...[...(data.items ?? [])].flatMap((i) => (i.kind === "file" ? [i.getAsFile()] : []))];
        const images = offered.filter((f): f is File => f !== null && f.type.startsWith("image/"));
        const files = images.filter((f, i) => images.findIndex((g) => g.name === f.name && g.size === f.size && g.type === f.type) === i);
        if (files.length === 0) return;
        event.preventDefault();
        void add(files);
      },
    }));
  },
  { flush: "post", immediate: true },
);

const limits = computed(() => env.value?.limits);
const bytes = computed(() => encoder.encode(body.value).length);
const nameChars = computed(() => [...name.value.trim()].length);
const contactChars = computed(() => [...contact.value.trim()].length);
const sending = computed(() => phase.value === "sending");
const room = computed(() => (limits.value ? limits.value.images - shots.value.length : 0));

const problem = computed(() => {
  const fit = limits.value;
  if (!fit) return "env";
  if (body.value.trim() === "") return "body";
  if (bytes.value > fit.bodyBytes) return "body-long";
  if (nameChars.value === 0) return "name";
  if (nameChars.value > fit.nameChars) return "name-long";
  if (contactChars.value > fit.contactChars) return "contact-long";
  return "";
});

const send = async () => {
  if (problem.value || sending.value) return;
  const sig = JSON.stringify([category.value, body.value, name.value, contact.value, shots.value.map((s) => s.id)]);
  if (sig !== attempt) key = uuid();
  attempt = sig;
  phase.value = "sending";
  failure.value = null;
  try {
    const receipt = await props.port.sendFeedback({
      idempotencyKey: key,
      category: category.value,
      body: body.value.trim(),
      displayName: name.value.trim(),
      contact: contact.value.trim() || undefined,
      locale: document.documentElement.lang,
      images: payload(shots.value),
      ...(shellMemory.value.length > 0 ? { memory: shellMemory.value } : {}),
    });
    shotEpoch++;
    rememberContact(contact.value.trim());
    dropDraft();
    sent.value = receipt;
    phase.value = "sent";
  } catch (e) {
    failure.value = feedbackFailure(e);
    phase.value = "editing";
  }
};

const again = () => {
  key = uuid();
  attempt = "";
  body.value = "";
  shots.value = [];
  refused.value = [];
  sent.value = null;
  failure.value = null;
  phase.value = "editing";
};

const onCategory = (c: FeedbackCategory) => { category.value = c; };
const onBody = (e: Event) => { body.value = (e.target as HTMLTextAreaElement).value; };
const onName = (e: Event) => { name.value = (e.target as HTMLInputElement).value; };
const onContact = (e: Event) => { contact.value = (e.target as HTMLInputElement).value; };
const onShots = (e: Event) => {
  const input = e.target as HTMLInputElement;
  void add([...(input.files ?? [])]);
  input.value = "";
};
const removeShot = (id: string) => { shots.value = shots.value.filter((x) => x.id !== id); };
const openLogs = () => {
  logsFailed.value = false;
  void shell.revealLogs().then((why) => { logsFailed.value = why !== null; });
};
const refusedText = (r: Refused) =>
  t(REFUSED_LABEL[r.why], { name: r.name, size: megabytes(limits.value?.uploadBytes ?? 0), n: limits.value?.images ?? 0 });

const envSummary = computed(() => (env.value
  ? t("feedback.sent_along_version_and_system", { version: `Studio ${env.value.env.version}`, system: systemLabel(env.value.env.os, env.value.env.osVersion, env.value.env.arch) })
  : ""));
const envSystem = computed(() => (env.value ? systemLabel(env.value.env.os, env.value.env.osVersion, env.value.env.arch) : ""));
const memoryAll = computed(() => (env.value ? memoryLines([...shellMemory.value, ...(env.value.env.memory ?? [])]) : []));
const securityLine = computed(() => tx("feedback.security_issues_go", {
  link: h("a", { href: SECURITY_URL, "data-action": "feedback.link", onClick: (e: MouseEvent) => { e.preventDefault(); props.onFile(SECURITY_URL); } },
    t("feedback.report_it_privately_under_the")),
}));
</script>

<template>
  <div v-if="phase === 'sent' && sent" class="fbs-sent" data-stage="sent">
    <span class="fbs-sent-mark"><StudioIcon name="check" /></span>
    <h3 ref="done" tabindex="-1">{{ t("feedback.your_feedback_was_received") }}</h3>
    <div class="fbs-receipt">
      <code :aria-label="t('feedback.receipt_number')">{{ sent.receipt }}</code>
      <CopyButton :text="sent.receipt" icon-only :label="t('feedback.copy_receipt_number')" />
    </div>
    <p class="fbs-sent-note">{{ t("feedback.keep_the_receipt_number_it") }}</p>
    <p v-if="sent.underReview" class="fbs-sent-note" data-review="">{{ t("feedback.this_report_is_under_review") }}</p>
    <p v-if="sent.redacted" class="fbk-note" data-tone="info">
      <StudioIcon name="shield" />
      {{ t("feedback.text_in_your_report_that") }}
    </p>
    <div class="fbs-sent-acts">
      <button type="button" class="fbs-btn" data-action="feedback.mine" @click="onMine">{{ t("feedback.view_my_feedback") }}</button>
      <button type="button" class="fbs-btn" data-ghost="" data-action="feedback.another" @click="again">{{ t("feedback.write_another") }}</button>
    </div>
  </div>

  <form v-else ref="zoneEl" class="fbs-form" :data-over="over ? '' : undefined" :aria-busy="sending" novalidate data-action-submit="feedback.send"
    tabindex="-1" @submit.prevent="send">
    <div v-if="envFailure" class="fbk-note" role="alert" data-tone="error">
      <StudioIcon name="warning" />
      <span>{{ envFailure.message }}</span>
      <button type="button" class="btn sm" data-action="feedback.retry-env" @click="load">{{ t("common.retry") }}</button>
    </div>

    <fieldset class="fbs-fields" :disabled="sending || !env">
      <legend class="sr-only">{{ t("feedback.feedback_details") }}</legend>

      <div class="fbs-cats" role="radiogroup" :aria-label="t('feedback.feedback_category')">
        <label v-for="c in FEEDBACK_CATEGORIES" :key="c" class="fbs-cat" :data-on="category === c ? '' : undefined">
          <input class="sr-only" type="radio" name="fbk-category" :aria-label="t(CATEGORY_LABEL[c])" :checked="category === c" @change="onCategory(c)" data-action="feedback.category" :data-value="c" />
          <b>{{ t(CATEGORY_LABEL[c]) }}</b>
          <small>{{ t(CATEGORY_HINT[c]) }}</small>
        </label>
      </div>

      <div class="fbs-field">
        <div class="fbs-labelrow">
          <label class="fbs-label" for="fbk-body">{{ t("feedback.what_happened") }}</label>
          <button type="button" class="fbs-link" :aria-pressed="preview" :disabled="!body.trim()" @click="preview = !preview" data-action="feedback.preview">
            {{ t(preview ? "common.edit" : "common.preview") }}
          </button>
        </div>
        <template v-if="preview">
          <div class="fbs-preview" :aria-label="t('feedback.body_preview')" role="region"><LazyMarkdown :text="body" :images="false" /></div>
          <span class="fbs-hint">{{ t("feedback.a_public_issue_shows_your") }}</span>
        </template>
        <template v-else>
          <textarea
            id="fbk-body"
            ref="field"
            class="fbs-body"
            data-action="feedback.body"
            :value="body"
            :rows="5"
            :placeholder="t('feedback.describe_what_you_ran_into')"
            :aria-invalid="problem === 'body-long' ? true : undefined"
            aria-describedby="fbk-count"
            @input="onBody"
          />
          <div class="fbs-under">
            <span class="fbs-public"><b>{{ t("feedback.it_becomes_public_only_if") }}</b>{{ t("feedback.do_not_put_secrets_passwords") }}</span>
            <span id="fbk-count" class="fbs-count" :data-over="problem === 'body-long' ? '' : undefined">
              {{ t("feedback.n_max_bytes", { n: bytes.toLocaleString(), max: (limits?.bodyBytes ?? 0).toLocaleString() }) }}
            </span>
          </div>
        </template>
      </div>

      <div class="fbs-field">
        <span class="sr-only" id="fbk-shots-t">{{ t("feedback.screenshots_optional_public_if_filed") }}</span>
        <div class="fbs-shots" role="group" aria-labelledby="fbk-shots-t">
          <ul v-if="shots.length > 0" class="fbk-shots">
            <FeedbackShot v-for="s in shots" :key="s.id" :shot="s" :on-remove="() => removeShot(s.id)" />
          </ul>
          <button v-if="canCapture" ref="captureButton" type="button" class="fbs-btn" data-action="feedback.shot.capture" :disabled="room <= 0 || capture === 'taking'" @click="takeWindow">
            <StudioIcon name="image" />{{ t("feedback.capture_this_window") }}
          </button>
          <button type="button" class="fbs-btn" data-ghost="" data-action="feedback.shot.pick" :disabled="room <= 0" @click="picker?.click()">
            <StudioIcon name="plus" />{{ t("feedback.add_screenshots") }}
          </button>
          <input
            ref="picker"
            class="sr-only"
            type="file"
            accept="image/png,image/jpeg"
            multiple
            tabindex="-1"
            :aria-label="t('feedback.choose_screenshot_files')"
            data-action="feedback.shot.pick"
            @change="onShots"
          />
        </div>
        <span class="fbs-hint">{{ limits ? t("feedback.you_can_also_paste_or", { n: limits.images, size: megabytes(limits.uploadBytes) }) : "" }}</span>
        <span v-if="capture === 'failed'" class="fbs-hint" role="alert" data-tone="error">{{ t("feedback.could_not_capture_the_window") }}</span>
        <p class="fbs-warn">{{ t("feedback.screenshots_show_what_the_window_shows") }}</p>
        <ul v-if="refused.length > 0" class="fbk-refused" role="status">
          <li v-for="(r, i) in refused" :key="i">{{ refusedText(r) }}</li>
        </ul>
      </div>

      <div class="fbs-row">
        <label class="fbs-field">
          <span class="fbs-label">{{ t("feedback.nickname_public_if_filed_as") }}</span>
          <input
            class="fbs-input"
            data-action="feedback.name"
            :value="name"
            autocomplete="nickname"
            :aria-invalid="problem === 'name-long' ? true : undefined"
            @input="onName"
          />
          <span class="fbs-hint">{{ t("feedback.if_it_is_filed_as", { name: name.trim() || "…" }) }}</span>
        </label>
        <label class="fbs-field">
          <span class="fbs-label">{{ t("feedback.contact_optional_private") }}</span>
          <input
            class="fbs-input"
            data-action="feedback.contact"
            :value="contact"
            autocomplete="off"
            :aria-invalid="problem === 'contact-long' ? true : undefined"
            @input="onContact"
          />
          <span class="fbs-hint">{{ t("feedback.email_or_qq_only_the") }}</span>
        </label>
      </div>
    </fieldset>

    <section v-if="env" class="fbs-env" aria-labelledby="fbk-env-t">
      <details>
        <summary>{{ envSummary }}</summary>
        <h3 id="fbk-env-t" class="sr-only">{{ t("feedback.environment_sent_with_your_report") }}</h3>
        <dl>
          <dt>{{ t("common.version") }}</dt>
          <dd>{{ env.env.version }}{{ env.env.commit ? ` (${env.env.commit})` : "" }}</dd>
          <dt>{{ t("common.system") }}</dt>
          <dd>{{ envSystem }}</dd>
          <dt>{{ t("feedback.surface") }}</dt>
          <dd>{{ env.env.surface }}</dd>
          <dt>{{ t("common.language") }}</dt>
          <dd>{{ env.env.locale }}</dd>
          <dt>{{ t("feedback.channel") }}</dt>
          <dd>{{ env.env.channel }}</dd>
          <dt>{{ t("common.model_services") }}</dt>
          <dd>{{ env.env.providerKind }}</dd>
          <dt>{{ t("feedback.memory") }}</dt>
          <dd>
            <div v-for="line in memoryAll" :key="line">{{ line }}</div>
          </dd>
        </dl>
        <p class="fbs-hint">{{ t("feedback.read_only_it_holds_no") }}</p>
        <div v-if="revealsLogs" class="fbs-logs">
          <button type="button" class="fbs-btn" data-ghost="" data-action="feedback.logs" @click="openLogs">
            {{ t("feedback.open_logs_folder") }}
          </button>
          <p class="fbs-hint">{{ t("feedback.when_a_maintainer_asks_for") }}</p>
          <p v-if="logsFailed" class="fbs-hint" role="alert" data-tone="error">{{ t("feedback.the_logs_folder_could_not") }}</p>
        </div>
      </details>
    </section>

    <div v-if="failure" class="fbk-note" role="alert" data-tone="error" :data-code="failure.code">
      <StudioIcon name="warning" />
      <span>{{ failure.message }}</span>
      <button v-if="failure.code === FEEDBACK_CODE.duplicate" type="button" class="btn sm" data-action="feedback.mine" @click="onMine">{{ t("feedback.view_my_feedback") }}</button>
      <button v-if="failure.code === FEEDBACK_CODE.disabled || failure.code === FEEDBACK_CODE.unavailable" type="button" class="btn sm" data-action="feedback.link" @click="onFile(FEEDBACK_REPO_ISSUES + 'new/choose')">
        {{ t("feedback.go_to_github") }}
      </button>
    </div>

    <div class="fbs-foot">
      <span class="fbs-security"><Node v-for="(piece, i) in securityLine" :key="i" :value="piece" /></span>
      <span class="fbs-fill" />
      <button class="fbs-btn" type="button" data-ghost="" data-action="feedback.close" @click="onClose">{{ t("common.cancel") }}</button>
      <button ref="submitBtn" class="fbs-btn" type="submit" data-primary="" :disabled="problem !== '' || sending">
        <StudioIcon name="send" />
        {{ t(sending ? "common.sending" : failure?.retry === "same" ? "feedback.retry_sending" : "common.send") }}
      </button>
    </div>
  </form>
</template>
