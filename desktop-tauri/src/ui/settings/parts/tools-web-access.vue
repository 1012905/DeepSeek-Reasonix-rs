<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type { AgentPort, SearxngProbe, WebAccessPatch, WebAccessSettings } from "../../../port/port";
import Switch from "../../Switch.vue";
import StudioIcon from "../../StudioIcon.vue";

type Kind = "search" | "fetch";

const NAME: Record<Kind, Record<string, string>> = {
  search: { searxng: "SearXNG", deepseek_api: "models.deepseek_search" },
  fetch: { local: "models.local_fetch", deepseek_api: "models.deepseek_fetch" },
};
const NOTE: Record<Kind, Record<string, string>> = {
  search: { searxng: "models.a_search_service_you_run", deepseek_api: "models.deepseeks_own_search_api_billed" },
  fetch: { local: "models.this_machine_downloads_the_page", deepseek_api: "models.deepseeks_own_page_fetch_api" },
};
// Keyed by the kernel's own code; a code with no entry shows the status instead.
const PROBE: Record<string, string> = {
  "search.searxng_json_disabled": "models.this_searxng_does_not_allow",
  "search.searxng_unreachable": "models.cannot_reach_this_address_check",
  "search.searxng_not_searxng": "models.this_address_answers_but_not",
  "search.searxng_redirected": "models.this_address_redirects_elsewhere_enter",
  "search.searxng_address_refused": "models.the_test_will_not_connect",
};

const props = defineProps<{ port: AgentPort }>();

const state = ref<WebAccessSettings | null>(null);
const url = ref("");
const busy = ref(false);
const probe = ref<SearxngProbe | null>(null);
const probing = ref(false);
const error = ref("");
// A window the server has not granted editing to is read-only; the first
// refused save is how that is learned, and nothing here asks again.
const readOnly = ref(false);

watch(() => props.port, (port) => {
  let live = true;
  state.value = null;
  error.value = "";
  readOnly.value = false;
  port.webAccessSettings()
    .then((s) => {
      if (!live) return;
      state.value = s;
      url.value = s.searxng_url;
    })
    .catch(() => { if (live) state.value = null; });
  return () => { live = false; };
}, { immediate: true });

const reading = (e: unknown) => {
  if (e instanceof HttpError && e.reason?.code === "web_access.editing_disabled") readOnly.value = true;
  error.value = reason(e);
};

// Consent to be billed is its own switch, written only by the person's click and
// withdrawn the same way; nothing here ever ticks it.
const apply = async (patch: WebAccessPatch) => {
  busy.value = true;
  error.value = "";
  try {
    const next = await props.port.saveWebAccessSettings(patch);
    state.value = next;
    url.value = next.searxng_url;
  } catch (e) {
    reading(e);
  } finally {
    busy.value = false;
  }
};

const test = async () => {
  probing.value = true;
  probe.value = null;
  error.value = "";
  try {
    const typed = url.value.trim();
    if (typed !== state.value?.searxng_url) {
      const next = await props.port.saveWebAccessSettings({ searxng_url: typed });
      state.value = next;
      url.value = next.searxng_url;
    }
    probe.value = await props.port.probeSearxng();
  } catch (e) {
    reading(e);
  } finally {
    probing.value = false;
  }
};

const probeSaid = (r: SearxngProbe) => r.ok
  ? t("models.connected_this_searxng_returns_json")
  : PROBE[r.code ?? ""]
    ? t(PROBE[r.code ?? ""])
    : t("models.this_searxng_refused_the_test", { status: String(r.status ?? "") });

const said = (kind: Kind, id: string) => (NAME[kind][id] ? t(NAME[kind][id]) : id);
const locked = computed(() => busy.value || readOnly.value);

// Each row is a backend's place in the order, so an enabled one carries the
// arrows that move it and the switch stays the on/off it reads as.
const rows = (on: string[], known: string[]) => [
  ...on.filter((id) => known.includes(id)),
  ...known.filter((id) => !on.includes(id)),
];
const knownOf = (kind: Kind) => (kind === "search" ? state.value?.known_search_backends ?? [] : state.value?.known_fetch_backends ?? []);
const onOf = (kind: Kind) => (kind === "search" ? state.value?.search_backends ?? [] : state.value?.fetch_backends ?? []);
const allowedOf = (kind: Kind) => (kind === "search" ? state.value?.search_paid_allowed ?? [] : state.value?.fetch_paid_allowed ?? []);
const paidOf = (id: string) => state.value?.paid_backends.includes(id) ?? false;

// An ordered list is the whole value, so a move is a save of the order, not of
// a single row: swapping is what the kernel reads back.
const order = (kind: Kind, next: string[]) => void apply(kind === "search" ? { search_backends: next } : { fetch_backends: next });
const consent = (kind: Kind, next: string[]) => void apply(kind === "search" ? { search_paid_allowed: next } : { fetch_paid_allowed: next });

const swap = (on: string[], at: number, withAt: number) => on.map((x, i) => (i === at ? on[withAt] : i === withAt ? on[at] : x));

const toggle = (kind: Kind, id: string, enabled: boolean) => {
  const on = onOf(kind);
  order(kind, enabled ? on.filter((x) => x !== id) : [...on, id]);
};
</script>

<template>
  <div v-if="!state" class="empty">{{ t("models.web_search_and_fetch") }}</div>
  <div v-else class="box">
    <template v-for="kind in (['search', 'fetch'] as Kind[])" :key="kind">
      <div class="wa-sec" role="group" :aria-label="kind === 'search' ? t('common.web_search') : t('models.web_fetch')">
        <div class="peernote">
          <b>{{ kind === "search" ? t("common.web_search") : t("models.web_fetch") }}</b>
          {{ " " }}{{ t("models.tried_in_order_the_free") }}
        </div>
        <div v-if="onOf(kind).length === 0" class="peernote">
          {{ kind === "search" ? t("models.no_search_backend_is_on") : t("models.no_fetch_backend_is_on") }}
        </div>
        <ul class="wa-list">
          <li v-for="id in rows(onOf(kind), knownOf(kind))" :key="id">
            <span class="tx">
              <span class="lb">{{ said(kind, id) }}</span>
              <span class="ds">{{ NOTE[kind][id] ? t(NOTE[kind][id]) : "" }}</span>
            </span>
            <span v-if="onOf(kind).includes(id)" class="acts">
              <button
                class="act"
                type="button"
                data-action="web-access.move"
                data-value="up"
                :disabled="locked || onOf(kind).indexOf(id) === 0"
                :aria-label="t('common.move_name_up', { name: said(kind, id) })"
                @click="order(kind, swap(onOf(kind), onOf(kind).indexOf(id), onOf(kind).indexOf(id) - 1))"
              ><StudioIcon name="arrow" /></button>
              <button
                class="act"
                type="button"
                data-action="web-access.move"
                data-value="down"
                :disabled="locked || onOf(kind).indexOf(id) === onOf(kind).length - 1"
                :aria-label="t('common.move_name_down', { name: said(kind, id) })"
                @click="order(kind, swap(onOf(kind), onOf(kind).indexOf(id), onOf(kind).indexOf(id) + 1))"
              ><StudioIcon name="arrowDown" /></button>
            </span>
            <Switch
              data-action="web-access.backend"
              :data-value="id"
              :on="onOf(kind).includes(id)"
              :busy="locked"
              :label="t('common.enable_name', { name: said(kind, id) })"
              :on-click="() => toggle(kind, id, onOf(kind).includes(id))"
            />
            <div v-if="paidOf(id) && (onOf(kind).includes(id) || allowedOf(kind).includes(id))" class="wa-consent">
              <span class="tx">
                <span class="lb">{{ t(kind === "search" ? "models.agree_to_be_billed_per_2" : "models.agree_to_be_billed_per") }}</span>
                <span class="ds">
                  {{ state.deepseek_api_ready || allowedOf(kind).includes(id)
                    ? t("models.once_you_agree_it_is")
                    : t("models.this_needs_a_deepseek_provider") }}
                </span>
              </span>
              <Switch
                data-action="web-access.consent"
                :data-value="`${kind}:${id}`"
                :on="allowedOf(kind).includes(id)"
                :busy="locked || (!state.deepseek_api_ready && !allowedOf(kind).includes(id))"
                :label="t(kind === 'search' ? 'models.agree_to_be_billed_per_2' : 'models.agree_to_be_billed_per')"
                :on-click="() => consent(kind, allowedOf(kind).includes(id) ? allowedOf(kind).filter((x) => x !== id) : [...allowedOf(kind), id])"
              />
            </div>
          </li>
        </ul>
        <div v-if="kind === 'search' && onOf(kind).includes('searxng')" class="wa-url">
          <label class="peernote" for="searxng-url">{{ t("models.searxng_address") }}</label>
          <input
            id="searxng-url"
            v-model="url"
            class="peerfield"
            data-action-change="web-access.searxng-url"
            :aria-label="t('models.searxng_address')"
            placeholder="http://127.0.0.1:8080"
            :disabled="locked"
            @blur="url.trim() !== state.searxng_url && void apply({ searxng_url: url.trim() })"
          />
          <div class="wa-test">
            <button class="act" type="button" data-action="web-access.probe" :disabled="readOnly || probing || !url.trim()" @click="void test()">
              {{ t(probing ? "models.testing" : "common.test_it") }}
            </button>
            <div v-if="probe" :class="probe.ok ? 'peernote' : 'peernote bad'" role="status">{{ probeSaid(probe) }}</div>
          </div>
        </div>
      </div>
    </template>

    <div class="wa-sec" role="group" :aria-label="t('models.video_addresses')">
      <div class="peernote"><b>{{ t("models.video_addresses") }}</b> {{ " " }}{{ t("models.the_model_can_hand_a") }}</div>
      <ul class="wa-list">
        <li>
          <span class="tx">
            <span class="lb">{{ t("models.hand_video_addresses_to_the") }}</span>
            <span class="ds">{{ t("models.when_on_the_address_is") }}</span>
          </span>
          <Switch
            data-action="web-access.video-url"
            :on="state.video_url_passthrough"
            :busy="locked"
            :label="t('models.hand_video_addresses_to_the')"
            :on-click="() => void apply({ video_url_passthrough: !state!.video_url_passthrough })"
          />
        </li>
      </ul>
    </div>

    <div class="peernote">{{ t("models.a_saved_change_reaches_only") }}<code>{{ state.path }}</code></div>
    <div v-if="readOnly" class="peernote" role="status">{{ t("common.these_settings_are_read_only") }}</div>
    <div v-if="error" class="why" role="alert">{{ error }}</div>
  </div>
</template>
