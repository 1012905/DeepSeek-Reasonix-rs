<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from "vue";
import { bytes } from "../../../i18n/format";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type { UpdateProgress, VersionEntry, VersionHub, VersionNotes } from "../../../port/version";
import { refreshUpdateWatch } from "../../../state/updatewatch";
import { deltaSkippedCopy, failureCopy, releasePage } from "../../versionFailure";
import { NOTES_LANGUAGE_NAMES, splitNotes } from "../../notesLanguages";
import LazyMarkdown from "../../LazyMarkdown.vue";

// A shell that declared no install: a build run from source, not a failure.
const NO_INSTALL = "studio.no_install";
// Work is running and would end with the restart; the person decides.
const RESTART_BUSY = "update.restart_busy";
// The release has no notes published: nothing to retry, only the page to read.
const NOTES_ABSENT = "studio.notes_absent";

// Phases during which a move owns the install and nothing else may start.
const MOVING = new Set<UpdateProgress["phase"]>(["downloading", "verifying", "downloaded", "applying", "authorizing", "relaunching"]);

// The newest few older releases stay on the page; the rest are history one click
// away.
const PAST_SHOWN = 3;

type NotesState = { phase: "loading" } | { phase: "ok"; markdown: string } | { phase: "err"; why: string; absent: boolean };

// The panel takes only what it calls, so the hub's full surface is not implied
// by what is drawn here.
const props = defineProps<{
  versions: (refresh?: boolean) => Promise<VersionHub>;
  pinVersion: (v: string) => Promise<void>;
  goToVersion: (v: string) => Promise<void>;
  cancelVersion: (v: string) => Promise<void>;
  restartToVersion: (v: string, force: boolean) => Promise<void>;
  onUpdateProgress: (cb: (p: UpdateProgress) => void) => () => void;
  versionNotes?: (version: string, retry?: boolean) => Promise<VersionNotes>;
  openExternal?: (url: string) => Promise<void>;
}>();

const hub = ref<VersionHub | null>(null);
const busy = ref(false);
const starting = ref("");
const cancelling = ref("");
const failed = ref("");
const unread = ref("");
const uninstalled = ref(false);
const progress = ref<UpdateProgress | null>(null);
const later = ref("");
const running = ref(0);
const allPast = ref(false);
const openNotes = ref("");
const notes = ref<Record<string, NotesState>>({});
const noteLang = ref<Record<string, "first" | "english">>({});

function when(iso: string): string {
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return "";
  const days = Math.floor((Date.now() - at) / 86400000);
  if (days <= 0) return t("common.today");
  if (days === 1) return t("common.yesterday");
  if (days < 30) return t("common.n_days_ago", { n: days });
  return new Date(at).toLocaleDateString();
}

// The phase is the sentence. Verifying gets its own because it is the pause after
// the bar fills, which otherwise reads as a hang on a large artifact.
function sayPhase(p: UpdateProgress): string {
  switch (p.phase) {
    case "downloading":
      if (p.stalls) return t("about.download_paused_the_connection_delivered", { n: p.stalls });
      if (p.delta_skipped) {
        return p.total > 0
          ? t("about.downloading_the_full_package_got_2", { got: bytes(p.received), all: bytes(p.total) })
          : t("about.downloading_the_full_package_got", { got: bytes(p.received) });
      }
      return p.total > 0 ? t("about.downloading_got_all", { got: bytes(p.received), all: bytes(p.total) }) : t("about.downloading_got", { got: bytes(p.received) });
    case "verifying": return t("about.verifying_the_signature");
    case "downloaded": return t("about.preparing_to_install");
    case "applying": return t("about.installing");
    case "authorizing": return t("about.waiting_for_permission");
    case "relaunching": return t("about.restarting_into_the_new_version");
    case "ready": return t("about.downloaded_waiting_for_a_restart");
    case "error": return t("about.install_failed");
    case "cancelled": return t("about.download_cancelled");
    case "idle": return t("about.starting");
  }
}

// The kernel says why it cannot answer — a shell that never declared an install,
// a server that does not carry this at all. Folding that back into null spent the
// one sentence it gave us and left the panel reading as a request still in flight.
async function load(refresh: boolean): Promise<void> {
  try {
    hub.value = await props.versions(refresh);
    unread.value = "";
    uninstalled.value = false;
  } catch (e) {
    hub.value = null;
    const none = e instanceof HttpError && e.reason?.code === NO_INSTALL;
    uninstalled.value = none;
    unread.value = none ? "" : reason(e);
  }
}

let stop: (() => void) | undefined;
watch(() => props.versions, () => {
  stop?.();
  void load(false);
  // Opening the panel is a look at the releases, so the background watch reads
  // now instead of at its next daily turn: with automatic download chosen, a
  // release published since the last read starts without a click.
  void refreshUpdateWatch();
  stop = props.onUpdateProgress((p) => { progress.value = p; });
}, { immediate: true });
onScopeDispose(() => stop?.());

// The kernel's next frame ends the local "cancelling…" state, whatever the
// cancellation turned out to be.
watch(progress, (p) => {
  if (!p) return;
  if (cancelling.value && p.version === cancelling.value && p.phase !== "downloading" && p.phase !== "verifying") cancelling.value = "";
  // Starting ends once the kernel's own answer names the move; from then on the
  // row follows that answer rather than a local guess.
  if (starting.value && p.version === starting.value && p.phase !== "idle") starting.value = "";
});

async function pin(version: string): Promise<void> {
  busy.value = true;
  failed.value = "";
  try {
    await props.pinVersion(version);
    await load(true);
  } catch (e) {
    failed.value = reason(e);
  } finally {
    busy.value = false;
  }
}

// Answered once the download is under way. It stops at a verified release waiting
// for the restart, which is the person's to allow.
async function goTo(version: string): Promise<void> {
  starting.value = version;
  later.value = "";
  running.value = 0;
  progress.value = null;
  try {
    await props.goToVersion(version);
  } catch (e) {
    progress.value = { version, phase: "error", received: 0, total: 0, err: reason(e) };
    starting.value = "";
  }
}

async function cancel(version: string): Promise<void> {
  if (cancelling.value) return;
  cancelling.value = version;
  failed.value = "";
  try {
    await props.cancelVersion(version);
  } catch (e) {
    cancelling.value = "";
    failed.value = reason(e);
  }
}

async function restart(version: string, force: boolean): Promise<void> {
  failed.value = "";
  try {
    await props.restartToVersion(version, force);
    running.value = 0;
  } catch (e) {
    if (e instanceof HttpError && e.reason?.code === RESTART_BUSY) {
      running.value = Number(e.reason.params?.n) || 1;
      return;
    }
    failed.value = reason(e);
  }
}

function loadNotes(version: string, retry: boolean): void {
  if (!props.versionNotes) return;
  notes.value = { ...notes.value, [version]: { phase: "loading" } };
  props.versionNotes(version, retry)
    .then((r) => { notes.value = { ...notes.value, [version]: { phase: "ok", markdown: r.markdown } }; })
    .catch((e) => {
      notes.value = { ...notes.value, [version]: { phase: "err", why: reason(e), absent: e instanceof HttpError && e.reason?.code === NOTES_ABSENT } };
    });
}

function toggleNotes(version: string): void {
  if (openNotes.value === version) { openNotes.value = ""; return; }
  openNotes.value = version;
  const have = notes.value[version];
  if (!have || have.phase === "err") loadNotes(version, false);
}

const list = computed(() => hub.value?.versions ?? []);
const dev = computed(() => !hub.value?.current || hub.value?.current === "dev");
const moving = computed(() => (progress.value && MOVING.has(progress.value.phase) ? progress.value.version : ""));
const ready = computed(() => (progress.value?.phase === "ready" ? progress.value.version : ""));
const locked = computed(() => busy.value || starting.value !== "" || moving.value !== "");
const readyOlder = computed(() => {
  const at = ready.value;
  return at !== "" && list.value.some((v: VersionEntry) => v.version === at && v.older);
});
const shown = computed(() => {
  if (allPast.value) return list.value;
  const past = list.value.filter((v) => v.older && !v.current);
  const held = new Set([moving.value, ready.value, hub.value?.pinned, openNotes.value].filter(Boolean) as string[]);
  return list.value.filter((v) => !v.older || v.current || past.indexOf(v) < PAST_SHOWN || held.has(v.version));
});
const hidden = computed(() => list.value.length - shown.value.length);
const failure = computed(() => (progress.value?.phase === "error" ? failureCopy(progress.value, hub.value?.current ?? "") : null));

// The notes in the language the window draws in, with a switch to the other one
// when the document carries both. The accessors below keep every branch of the
// template reading one field rather than re-narrowing the same union.
const notesBody = (markdown: string) => splitNotes(markdown);
// A row the catalog does not carry has no date, and saying so beats an empty
// column: it is why this version has no download page.
const whenSaid = (entry: VersionEntry): string => (entry.publishedAt ? when(entry.publishedAt) : entry.current ? t("about.not_published") : "");
const noteLoading = (version: string) => !notes.value[version] || notes.value[version].phase === "loading";
const noteText = (version: string) => (notes.value[version]?.phase === "ok" ? (notes.value[version] as { markdown: string }).markdown : "");
const noteWhy = (version: string) => (notes.value[version]?.phase === "err" ? (notes.value[version] as { why: string }).why : "");
const noteAbsent = (version: string) => notes.value[version]?.phase === "err" && (notes.value[version] as { absent: boolean }).absent;
const langOf = (version: string) => noteLang.value[version] ?? "first";
const setLang = (version: string, lang: "first" | "english") => { noteLang.value = { ...noteLang.value, [version]: lang }; };

const openInstaller = (version: string) => {
  if (!props.openExternal) return;
  void props.openExternal(releasePage(version)).catch(() => {});
};
</script>

<template>
  <p v-if="uninstalled" class="acct-note">{{ t("about.this_is_a_development_build") }}</p>
  <div v-else-if="unread" class="find" data-lvl="warn" role="alert">
    <span class="t">{{ t("about.version_information_could_not_be") }}</span>
    <span class="why">{{ unread }}<button type="button" class="lnk" data-action="versions.reload" @click="load(true)">{{ t("common.retry") }}</button></span>
  </div>
  <p v-else-if="hub === null" class="acct-note">{{ t("about.loading_versions") }}</p>

  <div v-else class="vers">
    <div class="vnow">
      <span class="cur">{{ hub.current || "dev" }}</span>
      <span class="lb">{{ t(dev ? "about.local_build" : "about.current_version") }}</span>
      <span v-if="hub.pinned && !hub.stalePin" class="pin">{{ t("common.pinned") }}</span>
    </div>

    <!-- Severity language is the transcript's: a coloured left rule, no icons.
         Pinned is not a problem, so it is ok-coloured; a stale pin is. -->
    <div v-if="hub.withdrawn" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("about.v_was_withdrawn", { v: hub.withdrawn.version }) }}</span>
      <span class="why">
        <template v-if="hub.withdrawn.reason">{{ t("about.reason_reason", { reason: hub.withdrawn.reason }) }}</template>
        {{ t(hub.newer ? "about.update_to_v_from_its" : "about.pick_a_version_below", { v: hub.latest }) }}
      </span>
    </div>
    <div v-if="hub.err" class="find" data-lvl="warn">
      <span class="t">{{ t("about.cannot_reach_the_version_index") }}</span>
      <span class="why">{{ t("about.err_nothing_local_is_affected", { err: hub.err }) }}</span>
    </div>
    <div v-if="failed" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("common.that_did_not_go_through") }}</span>
      <span class="why">{{ failed }}</span>
    </div>
    <div v-if="hub.pinned && !hub.stalePin" class="find" data-lvl="ok">
      <span class="t">{{ t("about.pinned_to_v_newer_versions", { v: hub.pinned }) }}</span>
      <span class="why">
        {{ t("about.a_rollback_pins_the_version") }}
        <button type="button" class="lnk" data-action="versions.pin" :disabled="locked" @click="pin('')">{{ t("about.unpin") }}</button>
      </span>
    </div>
    <div v-if="hub.stalePin" class="find" data-lvl="warn">
      <span class="t">{{ t("about.pinned_to_pinned_what_is", { pinned: hub.pinned, current: hub.current }) }}</span>
      <span class="why">
        {{ t("about.this_pin_holds_nothing_and") }}
        <button type="button" class="lnk" data-action="versions.pin" :disabled="busy" @click="pin('')">{{ t("about.clear_it_now") }}</button>
      </span>
    </div>
    <div v-if="!hub.err && hub.newer && !hub.pinned && !hub.withdrawn && !ready && !moving && !failure" class="find" data-lvl="ok">
      <span class="t">{{ t("about.v_is_out", { v: hub.latest }) }}</span>
      <span class="why">{{ t("about.install_it_from_the_row") }}</span>
    </div>
    <div v-if="ready && later !== ready" class="find" :data-lvl="running > 0 ? 'warn' : 'ok'" role="status">
      <span class="t">{{ running > 0 ? t("about.n_tasks_are_running", { n: running }) : t("about.v_is_downloaded_and_its", { v: ready }) }}</span>
      <span class="why">
        {{ t(running > 0 ? "about.restarting_now_would_stop_them" : "about.it_takes_effect_when_studio") }}
        <template v-if="readyOlder"> {{ t("about.a_session_written_by_a") }}</template>
      </span>
      <span class="acts">
        <button type="button" class="btn sm" data-primary data-action="versions.restart" @click="restart(ready, running > 0)">
          {{ t(running > 0 ? "about.restart_anyway" : "about.restart_now") }}
        </button>
        <button type="button" class="btn sm" data-action="versions.later" @click="later = ready; running = 0">{{ t("about.later") }}</button>
      </span>
    </div>
    <div v-if="failure && progress" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ failure.title }}</span>
      <span class="why">
        {{ failure.why }}
        <a v-if="failure.manual" class="lnk" :href="releasePage(progress.version)" target="_blank" rel="noreferrer noopener" data-action="versions.full-installer" @click.prevent="openInstaller(progress.version)">
          {{ t("about.download_the_full_installer") }}
        </a>
      </span>
      <span v-if="progress.err" class="gapd">{{ progress.err }}</span>
    </div>

    <!-- Newest first: the list reads as history, and where you are in it is
         marked the way every other "this one" in the app is. -->
    <div class="vlist">
      <template v-for="v in shown" :key="v.version">
        <div class="vrow" :data-open="openNotes === v.version ? '' : undefined" :data-on="v.current ? '' : undefined" :data-side="v.current ? 'now' : v.older ? 'past' : 'ahead'">
          <span class="nm">{{ v.version }}</span>
          <span class="ds">{{ t(v.current ? "about.running_now" : v.older ? "about.an_earlier_version" : "about.a_newer_version") }}</span>
          <span class="sc">{{ whenSaid(v) }}</span>
          <span v-if="moving === v.version && progress" class="sa" data-keep="">
            <span class="sa-txt" :title="progress.delta_skipped ? deltaSkippedCopy(progress.delta_skipped) : undefined">{{ sayPhase(progress) }}</span>
            <button
              v-if="progress.phase === 'downloading' || progress.phase === 'verifying'"
              type="button"
              class="lnk"
              data-action="versions.cancel"
              :disabled="cancelling !== ''"
              @click="cancel(v.version)"
            >{{ t(cancelling === v.version ? "common.stopping" : "about.cancel_download") }}</button>
          </span>
          <span v-else-if="starting === v.version" class="sa" data-keep="">{{ t("about.starting") }}</span>
          <button v-else-if="ready === v.version" type="button" class="sa lnk" data-keep="" data-action="versions.restart" @click="later = ''; restart(v.version, false)">
            {{ t("about.restart_to_finish") }}
          </button>
          <button v-else-if="!v.current" type="button" class="sa lnk" data-action="versions.activate" :disabled="locked" @click="goTo(v.version)">
            {{ t(v.older ? "about.roll_back_to_this_version" : "about.install_this_version") }}
          </button>
          <button v-if="v.current && !hub.pinned" type="button" class="sa lnk" data-action="versions.pin" :disabled="locked" @click="pin(v.version)">
            {{ t("about.pin_here") }}
          </button>
          <button v-if="v.hasNotes && versionNotes" type="button" class="vn lnk" data-action="versions.notes" :aria-expanded="openNotes === v.version" @click="toggleNotes(v.version)">
            {{ t(openNotes === v.version ? "about.hide_whats_new" : "about.whats_new") }}
          </button>
        </div>
        <section v-if="v.hasNotes && versionNotes && openNotes === v.version" :id="`vnotes-${v.version}`" class="vnotes" :aria-label="t('about.whats_new_in_v', { v: v.version })">
          <p v-if="noteLoading(v.version)" class="acct-note">{{ t("common.loading_whats_new") }}</p>
          <template v-else-if="notes[v.version].phase === 'ok'">
            <div v-if="notesBody(noteText(v.version)).english" class="vnotes-lang seg" role="group" :aria-label="t('about.language_of_the_notes')">
              <button type="button" data-action="versions.notes-lang" data-value="first" :aria-pressed="langOf(v.version) === 'first'" @click="setLang(v.version, 'first')">{{ t(NOTES_LANGUAGE_NAMES.first) }}</button>
              <button type="button" data-action="versions.notes-lang" data-value="english" :aria-pressed="langOf(v.version) === 'english'" @click="setLang(v.version, 'english')">{{ NOTES_LANGUAGE_NAMES.english }}</button>
            </div>
            <LazyMarkdown :text="langOf(v.version) === 'english' ? notesBody(noteText(v.version)).english ?? '' : notesBody(noteText(v.version)).first" :images="false" />
          </template>
          <div v-else class="find" data-lvl="warn" role="alert">
            <span class="t">{{ t("about.whats_new_could_not_be") }}</span>
            <span class="why">
              {{ noteWhy(v.version) }}
              <button v-if="!noteAbsent(v.version)" type="button" class="lnk" data-action="versions.notes-retry" @click="loadNotes(v.version, true)">{{ t("common.retry") }}</button>
              <a class="lnk" :href="releasePage(v.version)" target="_blank" rel="noreferrer noopener">{{ t("about.view_on_github") }}</a>
            </span>
          </div>
        </section>
      </template>
      <button v-if="hidden > 0" type="button" class="lnk vmore" data-action="versions.more" @click="allPast = true">
        {{ t("about.show_n_earlier_versions", { n: hidden }) }}
      </button>
    </div>
  </div>
</template>
