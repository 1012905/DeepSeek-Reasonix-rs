<script lang="ts">
import type { HubPort } from "../port/hub";
import type { AgentPort } from "../port/port";

export interface OnboardingProps {
  /** The window's own surface: the folder step is remembered by it, and it is
   *  what actually opens the project. */
  hub: HubPort;
  /** This pane's surface: the model service, and the record that the welcome
   *  has been shown. */
  port: AgentPort;
  /** The guide is over. `projectRoot` is set only when this run opened one. */
  onDone: (result: { projectRoot?: string }) => void;
  onError: (e: unknown) => void;
  /** Leave for the model-service settings, at the anchor named when there is
   *  one. The guide is the wrong place to edit an advanced endpoint. */
  onOpenSettings?: (at?: string) => void;
}

export type OnboardingStep = "welcome" | "connect" | "model" | "project";

/** The three things being set up, in order. The welcome is not one of them: it
 *  is the introduction, not a step, and a progress bar over it would claim the
 *  sequence had started before the person had agreed to it. */
export const STEPS: ReadonlyArray<readonly [OnboardingStep, string]> = [
  ["connect", "onboarding.connect_a_model_service"],
  ["model", "onboarding.choose_the_default_model"],
  ["project", "onboarding.open_a_workspace"],
];
</script>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { ProviderProbe, ProviderSetup, Protocol } from "../port/port";
import { KIND_LABEL, nameFrom } from "./vendors";
import { hasVendorMark, VendorMark } from "./vendormarks.vue";
import ConnectNote, { type ConnectOffer } from "./ConnectNote.vue";
import ModelChoice from "./ModelChoice.vue";
import StudioIcon from "./StudioIcon.vue";
import RMark from "./RMark.vue";
import WindowControls from "./chrome/WindowControls.vue";
import OnboardingProject from "./OnboardingProject.vue";
import { startRise } from "./firstRise";
import { useFirstRunRead, useLaunchHealth } from "./launchhealth";
import Demo from "./onboarding/Demo.vue";
import { KIND_DETAIL, MODEL_LIST_MAX, SHORTCUTS, FIX_AT_VENDOR, failed, type Failed, type Shortcut } from "./onboarding/shortcuts";
import { acknowledgeLaunch, readCrashes, type LaunchCrashes } from "./onboarding/launch";

const props = defineProps<OnboardingProps>();

const step = ref<OnboardingStep>("welcome");
const back = ref(false);
// Whether this window can open a workspace at all. Asked before the folder step
// is offered: a kernel without the route leaves a question that can only be
// answered by a refusal, so the guide ends at the model instead.
const canProject = ref(true);

const setup = ref<ProviderSetup | null | undefined>(undefined);
const welcomed = ref<boolean | undefined>(undefined);

const pick = ref(SHORTCUTS[0].label);
const url = ref(SHORTCUTS[0].url);
const apiKey = ref("");
const completed = ref("");
const busy = ref(false);
const connected = ref(false);
const leaving = ref(false);
const err = ref<Failed | null>(null);
const found = ref<ProviderProbe | null>(null);
const picked = ref<string[]>([]);
const catalog = ref<Protocol[]>([]);
const kind = ref("");
const model = ref("");
const projectRoot = ref("");
const crashes = ref<LaunchCrashes | null>(null);

const steps = computed(() => (canProject.value ? STEPS : STEPS.filter(([id]) => id !== "project")));
const at = computed(() => steps.value.findIndex(([id]) => id === step.value));

const vendor = computed<Shortcut | undefined>(() => SHORTCUTS.find((s) => s.label === pick.value));
// "" is the relay entry. A listed service's address is filled in for it; only a
// relay is ever asked where it is.
const custom = computed(() => vendor.value?.url === "");
const mark = (s: Shortcut) => (hasVendorMark(s.mark) ? s.mark! : "");

// The refusals settled on the vendor's site rather than here carry a way there.
const fixAt = computed(() => (err.value?.code && FIX_AT_VENDOR.has(err.value.code) ? vendor.value?.console : undefined));

const kinds = computed(() => (found.value ? (found.value.kinds?.length ? found.value.kinds : [found.value.kind]) : []));
// Something is only asked how it is spoken to when nothing else can tell: a
// relay answered the model list on more than one wire, so the choice is the
// reader's rather than a finding.
const asks = computed(() => custom.value && !!found.value && (found.value.ambiguous || kinds.value.length > 1));
const protocols = computed(() => [...new Set([...catalog.value.map((entry) => entry.kind), ...kinds.value])]);

const keyEnv = computed(() => setup.value?.keyEnv ?? "");
// The way out for someone who would rather configure the service themselves.
// Not on the welcome — nothing is wrong yet — and not on the step that already
// asks for a key, where a second "connect" would be two answers to one
// question. Gone as soon as an endpoint has answered.
const offer = computed<ConnectOffer | null>(() => (!found.value && (step.value === "model" || step.value === "project") ? {
  none: !setup.value?.provider,
  open: () => { if (props.onOpenSettings) props.onOpenSettings("providers"); else go("connect"); },
} : null));

function go(next: OnboardingStep, backwards = false) {
  back.value = backwards;
  step.value = next;
}

// The card shrinks and fades before the conversation takes over, and the
// composer rises into the place it left.
function leave(then: () => void) {
  leaving.value = true;
  startRise();
  window.setTimeout(then, typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 240);
}

function open(target: string) {
  void props.port.openExternal(target).catch(() => {
    err.value = { text: t("common.could_not_open_a_browser", { at: target }) };
  });
}

function choose(entry: Shortcut) {
  pick.value = entry.label;
  url.value = entry.url;
  found.value = null;
  kind.value = "";
  picked.value = [];
  err.value = null;
}

// The endpoint's own words are the answer: a 401, a wrong path and "no chat
// models" send the reader to three different fixes, so none of them collapses
// into "connection failed". Nothing is written here — the answer is shown for
// confirmation, because only the person holding the key knows what they bought.
async function connect() {
  const base = url.value.trim();
  const credential = apiKey.value.trim();
  if (!base || !credential || busy.value) return;
  busy.value = true;
  err.value = null;
  try {
    const probe = await props.port.probeProvider(base, credential);
    found.value = probe;
    const changed = !!probe.baseUrl && probe.baseUrl !== base;
    if (changed) url.value = probe.baseUrl!;
    completed.value = changed ? probe.baseUrl! : "";
    kind.value = probe.kind;
    picked.value = probe.models.slice();
    model.value = probe.default || probe.models[0] || "";
    // The button says it worked, and the banner with it, before the page moves.
    connected.value = true;
    await new Promise((done) => setTimeout(done, 750));
    go("model");
  } catch (e) {
    err.value = failed(e);
  } finally {
    busy.value = false;
    connected.value = false;
  }
}

// Saves the service and makes it the default. The folder step is only reached
// when the window can open one; otherwise the guide ends here.
async function start() {
  if (!found.value || !kind.value || !model.value || busy.value) return;
  busy.value = true;
  err.value = null;
  const name = nameFrom(url.value.trim());
  try {
    await props.port.saveProvider({
      name,
      kind: kind.value,
      baseUrl: url.value.trim(),
      apiKey: apiKey.value.trim(),
      models: picked.value,
      default: model.value,
      authHeader: found.value.authHeader,
      noProxy: found.value.noProxy,
      effort: "",
      vision: found.value.vision.filter((m) => picked.value.includes(m)),
      replace: true,
    });
    await props.port.setModel(`${name}/${model.value}`);
    await props.port.setDefaultModel(`${name}/${model.value}`);
    busy.value = false;
    if (canProject.value) go("project");
    else void finish();
  } catch (e) {
    err.value = failed(e);
    busy.value = false;
  }
}

// Every route out of the guide ends here. The welcome is marked seen before the
// launch is acknowledged: a window that has not got past its own introduction
// has not come up, and retiring the way back from inside it would discard the
// rollback on a launch the person cannot use yet.
async function finish() {
  await props.port.markWelcomed().catch(() => {});
  if (crashes.value) await acknowledgeLaunch(props.port);
  const root = projectRoot.value;
  leave(() => props.onDone(root ? { projectRoot: root } : {}));
}

function skip() {
  void finish();
}

function onProjectPicked(root: string) {
  if (busy.value) return;
  busy.value = true;
  err.value = null;
  props.hub
    .addWorkspace(root)
    .then((workspace) => {
      projectRoot.value = workspace.root || root;
      busy.value = false;
      void finish();
    })
    .catch((e: unknown) => {
      busy.value = false;
      err.value = { text: reason(e) };
      props.onError(e);
    });
}

function setPicked(next: string[]) {
  picked.value = next;
  // The default has to be one of the enabled models, or saving would set a
  // model the endpoint no longer offers.
  if (!next.includes(model.value)) model.value = next[0] ?? "";
}

function toggleModel(name: string) {
  setPicked(picked.value.includes(name) ? picked.value.filter((m) => m !== name) : [...picked.value, name]);
}

// A name the endpoint did not report is still a model: gateways hide ones they
// serve, and refusing a typed id here would make the guide the authority on
// what an endpoint offers.
function addModel(name: string) {
  const id = name.trim();
  if (!id) return;
  setPicked(picked.value.includes(id) ? picked.value : [...picked.value, id]);
  if (!model.value) model.value = id;
}

function setDefault(name: string) {
  model.value = name;
}

function selectProtocol(value: string) {
  kind.value = value;
  const mapped = vendor.value?.protocolUrls?.[value];
  const known = Object.values(vendor.value?.protocolUrls ?? {});
  if (mapped && known.includes(url.value.trim().replace(/\/$/, ""))) url.value = mapped;
}

const detail = (name: string) => [
  name === found.value?.default ? t("common.default") : "",
  found.value?.vision.includes(name) ? t("common.reads_images") : "",
].filter(Boolean).join(" · ");

// Both of these are watchers, so they are registered here rather than in
// onMounted: their cleanup is what stops the launch probe from firing after the
// guide has gone.
useFirstRunRead(() => props.port, (v) => { setup.value = v; }, (v) => { welcomed.value = v; });
useLaunchHealth(() => props.port, setup, welcomed);

onMounted(() => {
  void props.port.protocols().then((items) => { catalog.value = items; }).catch(() => { catalog.value = []; });
  void props.hub.hostCapabilities()
    .then((caps) => { canProject.value = caps.addWorkspace; })
    .catch(() => { canProject.value = true; });
  void readCrashes(props.port).then((got) => { crashes.value = got; });
});

// The key field is where the first thing worth typing goes, so it takes focus
// when its step arrives.
watch(step, (now) => {
  if (now !== "connect") return;
  requestAnimationFrame(() => document.getElementById("onb-key")?.focus());
});
</script>

<template>
  <div class="onb-stage" :data-leaving="leaving ? '' : undefined">
    <header class="onb-brandbar">
      <span class="onb-lockup">
        <span class="onb-brandsub" aria-hidden="true">studio</span>
      </span>
      <WindowControls />
    </header>

    <div class="onb-shell">
      <main class="onb onb-flow">
        <div v-if="step !== 'welcome'" class="onb-steps">
          <ol :aria-label="t('onboarding.set_up_reasonix')">
            <li
              v-for="([id, text], i) in steps"
              :key="id"
              :data-state="i < at ? 'done' : i === at ? 'current' : 'todo'"
              :aria-current="i === at ? 'step' : undefined"
            >
              <i><StudioIcon v-if="i < at" name="check" /><template v-else>{{ i + 1 }}</template></i>
              <span>{{ t(text) }}</span>
            </li>
          </ol>
          <button type="button" class="onb-skip" data-action="onboarding.skip" :disabled="busy" @click="skip">
            {{ t("onboarding.not_now") }}
          </button>
        </div>

        <div :key="step" class="onb-card" :data-step="step" :data-back="back ? '' : undefined">

          <template v-if="step === 'welcome'">
            <div class="onb-welcome">
              <img class="onb-welcome-mark" src="../assets/logo-wordmark.svg" alt="" aria-hidden="true" draggable="false" />
              <h1 class="onb-t">{{ t("onboarding.hand_it_a_task") }}</h1>
              <p class="onb-s">{{ t("onboarding.what_it_does_and_shows") }}</p>
              <div class="onb-acts">
                <button class="btn onb-go" data-primary data-action="onboarding.begin" @click="go('connect')">
                  {{ t("onboarding.set_up_reasonix") }}
                </button>
                <button type="button" class="onb-skip" data-action="onboarding.skip" @click="skip">
                  {{ t("onboarding.not_now") }}
                </button>
              </div>
              <p class="onb-note">{{ t("onboarding.about_a_minute") }}</p>
            </div>
            <Demo />
          </template>

          <template v-else-if="step === 'connect'">
            <h1 class="onb-t">{{ t("onboarding.connect_one_model_service") }}</h1>
            <p class="onb-s">{{ t("onboarding.pick_one_and_fill_the_key") }}</p>

            <div class="onb-services" role="radiogroup" :aria-label="t('onboarding.common_services')">
              <button
                v-for="entry in SHORTCUTS"
                :key="entry.label"
                type="button"
                role="radio"
                class="onb-service"
                data-action="onboarding.service"
                :data-value="entry.label"
                :aria-checked="pick === entry.label"
                :disabled="busy"
                @click="choose(entry)"
              >
                <b><VendorMark v-if="mark(entry)" :slug="mark(entry)" :size="16" />{{ t(entry.label) }}</b>
                <small>{{ t(entry.hint) }}</small>
              </button>
            </div>

            <div v-if="custom" class="onb-field">
              <label for="onb-url">{{ t("onboarding.service_address") }}</label>
              <input
                id="onb-url"
                v-model="url"
                spellcheck="false"
                autocomplete="off"
                :placeholder="t('onboarding.https_your_host_v1')"
                :disabled="busy"
                @input="found = null; kind = ''; picked = []"
              />
            </div>

            <div class="onb-field">
              <label for="onb-key">
                API key
                <code v-if="keyEnv">{{ keyEnv }}</code>
                <button v-if="vendor?.console" type="button" class="onb-getkey" data-action="external.open" @click="open(vendor.console)">
                  {{ t("onboarding.no_key_yet_get_it_from", { name: t(pick) }) }}
                </button>
              </label>
              <input
                id="onb-key"
                v-model="apiKey"
                data-action-keydown="onboarding.connect"
                type="password"
                spellcheck="false"
                autocomplete="off"
                placeholder="sk-…"
                :disabled="busy"
                :aria-invalid="err ? true : undefined"
                @input="found = null; kind = ''; picked = []"
                @keydown.enter="void connect()"
              />
              <div v-if="err" class="onb-err" role="alert">
                {{ err.text }}
                <button v-if="fixAt" type="button" class="onb-fix" data-action="external.open" @click="open(fixAt!)">
                  {{ t("onboarding.open_the_name_console", { name: t(pick) }) }}
                </button>
              </div>
            </div>

            <div v-if="connected && found" class="onb-found" role="status">
              <span class="tick"><StudioIcon name="check" /></span>
              <span>{{ t("onboarding.connected_to_name_n_models", { name: t(pick), n: found.models.length }) }}</span>
            </div>

            <div class="onb-acts">
              <button
                class="btn onb-go"
                data-primary
                data-action="onboarding.connect"
                :data-state="connected ? 'ok' : busy ? 'busy' : undefined"
                :disabled="busy || !url.trim() || !apiKey.trim()"
                :aria-label="busy ? t('onboarding.checking_the_key') : undefined"
                @click="void connect()"
              >
                <StudioIcon v-if="connected" name="check" class="onb-check" />
                <RMark v-else-if="busy" class="rmark onb-spin" />
                <template v-else>{{ t("onboarding.connect") }}</template>
              </button>
              <button type="button" class="onb-skip" data-action="onboarding.back" :disabled="busy" @click="go('welcome', true)">
                {{ t("common.back") }}
              </button>
            </div>
            <p v-if="!custom" class="onb-note">
              {{ t("onboarding.address_filled_for_name", { name: t(pick), host: url.replace(/^https?:\/\//, "").replace(/\/.*$/, "") }) }}
            </p>
          </template>

          <template v-else-if="step === 'model' && found">
            <div class="onb-found" role="status">
              <span class="tick"><StudioIcon name="check" /></span>
              <span>{{ t("onboarding.connected_to_name_n_models", { name: t(pick), n: found.models.length }) }}</span>
            </div>
            <p v-if="completed !== '' && completed === url.trim()" class="probe-ok" role="status">
              {{ t("common.address_completed_to_url", { url: completed }) }}
            </p>

            <div v-if="asks" class="onb-field">
              <span class="lb">{{ t("onboarding.how_it_is_spoken_to") }}</span>
              <p class="onb-why">{{ t("common.both_access_methods_answer_with") }}</p>
              <div class="onb-protocols" role="radiogroup" :aria-label="t('common.protocol')">
                <button
                  v-for="value in protocols"
                  :key="value"
                  type="button"
                  role="radio"
                  data-action="onboarding.protocol"
                  :data-value="value"
                  :aria-checked="kind === value"
                  :disabled="busy"
                  @click="selectProtocol(value)"
                >
                  <i aria-hidden="true" />
                  <span><b>{{ t(KIND_LABEL[value] || value) }}</b><small>{{ KIND_DETAIL[value] || value }}</small></span>
                </button>
              </div>
            </div>

            <div class="onb-field">
              <span class="lb">{{ t("onboarding.pick_a_default_model") }}</span>
              <!-- The list the settings page uses once the catalogue is long: a
                   gateway publishes hundreds of names that differ by a date
                   suffix, and eight radio rows would be a sample, not the list. -->
              <ModelChoice
                v-if="found.models.length > MODEL_LIST_MAX"
                :models="found.models"
                :picked="picked"
                :vision="found.vision"
                :def="model"
                :on-toggle="toggleModel"
                :on-add="addModel"
                :on-default="setDefault"
              />
              <div v-else class="onb-models" role="radiogroup" :aria-label="t('onboarding.pick_a_default_model')">
                <button
                  v-for="name in found.models"
                  :key="name"
                  type="button"
                  role="radio"
                  class="onb-model"
                  data-action="onboarding.model"
                  :data-value="name"
                  :aria-checked="model === name"
                  :disabled="busy"
                  @click="model = name"
                >
                  <i aria-hidden="true" /><b>{{ name }}</b><small v-if="detail(name)">{{ detail(name) }}</small>
                </button>
              </div>
              <p class="onb-note">{{ t("onboarding.each_conversation_can_switch") }}</p>
            </div>

            <div v-if="err" class="onb-err" role="alert">{{ err.text }}</div>

            <div class="onb-acts">
              <button
                class="btn onb-go"
                data-primary
                data-action="onboarding.start"
                :disabled="busy || !kind || !model"
                @click="void start()"
              >{{ t(busy ? "common.saving_2" : "onboarding.continue") }}</button>
              <button type="button" class="onb-skip" data-action="onboarding.back" :disabled="busy" @click="go('connect', true)">
                {{ t("onboarding.previous_step") }}
              </button>
            </div>
          </template>

          <template v-else-if="step === 'project'">
            <OnboardingProject :on-pick="onProjectPicked" :on-skip="skip" :on-error="props.onError" />
          </template>
        </div>

        <!-- Only while the guide has no service of its own: the card above is
             then the shorter path, and this is what a person who put it off
             once and came back finds. -->
        <ConnectNote v-if="offer" :offer="offer" />

        <!-- What a previous launch left behind. Shown beside the folder so it
             can actually be opened, and acknowledged by leaving. -->
        <div v-if="crashes" class="find" data-lvl="warn" role="status">
          <span class="t">{{ t("about.logs_and_abnormal_exit_records") }}</span>
          <span class="why">{{ t("data.records_left_when_the_kernel") }}</span>
          <span class="why">{{ crashes.dir }}</span>
          <span v-for="file in crashes.names" :key="file" class="why">{{ file }}</span>
          <span v-if="crashes.more" class="why">{{ t("data.older_records_are_not_listed") }}</span>
        </div>
      </main>
    </div>

    <footer class="onb-foot">{{ t("onboarding.local_first_credentials_stay_in") }}</footer>
  </div>
</template>
