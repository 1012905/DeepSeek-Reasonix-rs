<script lang="ts">
import type { AgentPort } from "../../port/port";

/** What the pane hands its browser column. Wire it from Pane.vue: `manual` and
 *  `onManual` are the pane's own takeover flag, `full`/`onFull` the workbench
 *  size, and `onError` whatever the pane already reports refusals to. */
export interface BrowserPanelProps {
  port: AgentPort;
  visible: boolean;
  /** A person is driving the browser themselves. */
  manual: boolean;
  onManual: (on: boolean) => void;
  onError: (e: unknown) => void;
  full: boolean;
  onFull: (on: boolean) => void;
}

/** The page a manual browser starts on. Reasonix owns this scheme, so it never
 *  reaches the network as something to load. */
export const START = "reasonix://start";

/** normalise turns what someone typed into an address, or null when it cannot
 *  be one. A path on disk is refused rather than read as a host named by its
 *  drive letter: this column draws a web page, and a file is not one. */
export function normalise(value: string): string | null {
  const raw = value.trim();
  if (!raw) return START;
  if (/^reasonix:\/\//i.test(raw)) return START;
  if (/^(?:[a-z]:[\\/]|\\\\|\/)/i.test(raw)) return null;
  const local = /^(?:localhost|127(?:\.\d+){3}|10(?:\.\d+){3}|192\.168(?:\.\d+){2}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d+){2}|\[::1\]|[^./:]+)(?::\d+)?(?:\/|$)/i.test(raw);
  const candidate = local ? `http://${raw}` : /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`;
  try {
    const url = new URL(candidate);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}

/** The disk path a file: page was opened from, or "" for every other page. Only
 *  a local page can have its source edited and saved. */
export function localFilePath(url: string): string {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return "";
  }
  if (parsed.protocol !== "file:" || parsed.host) return "";
  try {
    const path = decodeURIComponent(parsed.pathname);
    return /^\/[A-Za-z]:\//.test(path) ? path.slice(1) : path;
  } catch {
    return "";
  }
}

/** The host a manual page is called by. Several browsers all named 浏览器 name
 *  none of them, and only this panel knows where each of them has got to. */
export function hostOf(url: string): string {
  if (!url || url === START) return "";
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
}
</script>

<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, shallowRef, watch } from "vue";
import { ArrowLeft, ArrowRight } from "lucide-vue-next";
import { t } from "../../i18n";
import { host, type BrowserControl, type BrowserLoadFailure, type BrowserPickEnd, type PickedElement } from "../../port/host";
import type { BrowserTab } from "../../port/port";
import AgentPageNote from "../AgentPageNote.vue";
import BrowserElementEdit from "../BrowserElementEdit.vue";
import StudioIcon from "../StudioIcon.vue";
import TooltipButton from "../TooltipButton.vue";
import { observePageTabs, openPage, readPageTabs } from "../browser_open";
import { occluded, visibleBox } from "../occlusion";
import { pageEditor } from "../pageedit";
import { SWAP_MARK } from "../swap";
import { useBrowserSelection } from "../useBrowserSelection";
import BrowserFailure from "./BrowserFailure.vue";
import BrowserFullToggle from "./BrowserFullToggle.vue";
import BrowserLogin from "./BrowserLogin.vue";
import BrowserTabMenu, { type BrowserCloseMode, type BrowserTabMenuState } from "./BrowserTabMenu.vue";
import BrowserTools from "./BrowserTools.vue";
import ManualBrowsers, { type ManualPageView } from "./ManualBrowsers.vue";

const props = defineProps<BrowserPanelProps>();

// The template reads bindings from <script setup> only, and START lives in the
// plain block beside it.
const startPage = START;

const shell = host();
// What this shell can do cannot change under a mounted column, so every branch
// below is decided once rather than re-read on each render.
const native = shell.drawsBrowserViews();
const picks = shell.picksBrowserElements();
const editor = pageEditor(props.port);

// ---- the agent's tabs ---------------------------------------------------
const tabs = shallowRef<BrowserTab[]>([]);
const chosen = ref("");       // "" follows whichever tab the agent has in front
const blank = ref(false);     // a new tab was asked for and nothing is in it yet
const failures = ref<Record<string, BrowserLoadFailure>>({});
const slot = ref<HTMLDivElement | null>(null);
const frame = ref("");
const still = ref(false);     // the shell's own freeze is standing in for the page

const agent = computed<BrowserTab | null>(() => {
  if (blank.value) return null;
  return tabs.value.find((tab) => tab.target === chosen.value) ?? tabs.value.find((tab) => tab.active) ?? tabs.value[0] ?? null;
});
const target = computed(() => agent.value?.target ?? "");
const failure = computed(() => (target.value ? failures.value[target.value] : undefined));
// Which pages' login prompts this column owns. Takeover draws its own frames,
// and a frame never raises one, so nothing filters them there.
const keep = computed(() => (props.manual ? undefined : tabs.value.map((tab) => tab.target)));

let reading = 0;
const readTabs = async () => {
  const turn = ++reading;
  try {
    const next = await readPageTabs(props.port);
    if (turn !== reading) return;
    tabs.value = next;
    observePageTabs(props.port, next, true);
  } catch (e) {
    props.onError(e);
  }
};

const selection = useBrowserSelection(() => props.port, (key) => {
  chosen.value = key.replace(/^browser:/, "");
  blank.value = false;
}, tabs);

// The column keeps its own subscription: an agent that opens a page while this
// is open is exactly what has to arrive here. It runs only while the column is
// showing, so a hidden pane costs no connection.
watch(() => props.visible, (shown, _was, onCleanup) => {
  if (!shown) return;
  void readTabs();
  onCleanup(props.port.subscribe((ev) => { if (ev.kind === "browser_tabs_changed") void readTabs(); }));
}, { immediate: true });

const offLoad = shell.onBrowserLoadState(({ targetId, failure: next }) => {
  const prev = failures.value;
  if (!next && !prev[targetId]) return;
  const out = { ...prev };
  if (next) out[targetId] = next;
  else delete out[targetId];
  failures.value = out;
});
onScopeDispose(offLoad);

// ---- the person's own pages --------------------------------------------
// Kept apart from the agent's tabs on purpose: a page someone opened is not a
// page the agent can drive, and merging the two would hand it over silently.
interface Book { id: string; history: string[]; at: number }

const books = ref<Book[]>([]);
const bookID = ref("");
const reload = ref(0);        // re-mounts a frame: refresh for a page this window draws

const book = computed<Book | null>(() => books.value.find((one) => one.id === bookID.value) ?? null);
const address = computed(() => {
  if (props.manual) return book.value?.history[book.value.at] ?? START;
  return agent.value?.url ?? "";
});
// Only a page opened from a file on this machine has a source to edit, and the
// address bar is what says which file that is.
const documentPath = computed(() => localFilePath(address.value));
const pages = computed<ManualPageView[]>(() => books.value.map((one) => ({ id: one.id, url: one.history[one.at] ?? START })));
const canBack = computed(() => (props.manual ? (book.value?.at ?? 0) > 0 : false));
const canForward = computed(() => (props.manual ? !!book.value && book.value.at < book.value.history.length - 1 : false));

let minted = 0;
const addBook = () => {
  const id = `manual-${++minted}`;
  books.value = [...books.value, { id, history: [START], at: 0 }];
  bookID.value = id;
  reload.value += 1;
};

const closeBook = (id: string) => {
  const at = books.value.findIndex((one) => one.id === id);
  if (at < 0) return;
  books.value = books.value.filter((one) => one.id !== id);
  if (bookID.value === id) bookID.value = books.value[Math.min(at, books.value.length - 1)]?.id ?? "";
};

// Takeover always has at least one page: a manual browser with no tab is a
// blank column with nothing to type into.
watch(() => props.manual, (on) => { if (on && !books.value.length) addBook(); }, { immediate: true });

// ---- the address bar ----------------------------------------------------
const typed = ref("");
const invalid = ref(false);
const field = ref<HTMLInputElement | null>(null);

// What is typed starts as whatever the page in front shows, and follows it when
// the page moves on its own.
watch(address, (next) => { typed.value = next; invalid.value = false; }, { immediate: true });

const visit = (to: string) => {
  const open = book.value;
  if (!open) return;
  const history = open.history.slice(0, open.at + 1).concat(to);
  books.value = books.value.map((one) => (one.id === open.id ? { ...one, history, at: history.length - 1 } : one));
  reload.value += 1;
};

const submit = async () => {
  const next = normalise(typed.value);
  if (!next) { invalid.value = true; return; }
  invalid.value = false;
  if (props.manual) {
    if (!book.value) addBook();
    if (next === address.value) reload.value += 1;
    else visit(next);
    return;
  }
  if (next === startPage) { newTab(); return; }
  if (native && target.value && !blank.value) {
    const refusal = await shell.navigateBrowserView(target.value, next);
    if (refusal) invalid.value = true;
    return;
  }
  // Nothing to drive here: the kernel's browser is where a page opens, and it
  // opens one rather than repointing the tab it already has.
  try {
    const opened = await openPage(props.port, next);
    blank.value = false;
    chosen.value = opened.target;
    selection.follow(`browser:${opened.target}`);
    await readTabs();
  } catch (e) {
    props.onError(e);
  }
};

const move = (step: -1 | 1) => {
  const open = book.value;
  if (!open) return;
  const at = open.at + step;
  if (at < 0 || at >= open.history.length) return;
  books.value = books.value.map((one) => (one.id === open.id ? { ...one, at } : one));
  reload.value += 1;
};

const control = (action: BrowserControl) => {
  if (props.manual) {
    if (action === "back") move(-1);
    else if (action === "forward") move(1);
    else if (action === "reload") reload.value += 1;
    return;
  }
  if (!native) { if (action === "reload") reload.value += 1; return; }
  if (target.value) shell.controlBrowserView(target.value, action);
};

const external = () => {
  const to = props.manual ? address.value : agent.value?.url ?? "";
  if (to && to !== START) void shell.openExternal(to);
};

const retry = () => {
  const failed = failure.value;
  if (!failed) return;
  if (native && target.value) void shell.navigateBrowserView(target.value, failed.url);
  else void readTabs();
};

const trust = () => { if (target.value) void shell.trustBrowserCertificate(target.value); };

// ---- opening and closing ------------------------------------------------
const newTab = () => {
  if (props.manual) { addBook(); void nextTick(() => field.value?.focus()); return; }
  blank.value = true;
  chosen.value = "";
  typed.value = "";
  invalid.value = false;
  void nextTick(() => field.value?.focus());
};

const closeTab = async () => {
  if (props.manual) { if (book.value) closeBook(book.value.id); return; }
  const gone = agent.value;
  if (!gone) return;
  try {
    await props.port.browserClose(gone.id);
  } catch (e) {
    props.onError(e);
    return;
  }
  if (chosen.value === gone.target) chosen.value = "";
  await readTabs();
  if (!tabs.value.length) blank.value = true;
};

// ---- the tab menu -------------------------------------------------------
const menu = ref<BrowserTabMenuState | null>(null);

const openMenu = (event: MouseEvent, key: string, name: string) => {
  event.preventDefault();
  menu.value = { target: key, label: name, x: event.clientX, y: event.clientY, anchor: event.currentTarget as HTMLElement };
};

const pageName = (one: ManualPageView) => hostOf(one.url) || t("common.blank_page");
const order = computed(() => (props.manual ? books.value.map((one) => `manual:${one.id}`) : tabs.value.map((tab) => `browser:${tab.target}`)));
const index = computed(() => (menu.value ? order.value.indexOf(menu.value.target) : -1));
const others = computed(() => (index.value < 0 ? 0 : order.value.length - 1));
const right = computed(() => (index.value < 0 ? 0 : order.value.length - index.value - 1));
const all = computed(() => order.value.length);

const closeMode = async (mode: BrowserCloseMode) => {
  const from = index.value;
  menu.value = null;
  if (from < 0) return;
  const doomed = mode === "one"
    ? order.value.slice(from, from + 1)
    : mode === "others"
      ? order.value.filter((_key, i) => i !== from)
      : mode === "right"
        ? order.value.slice(from + 1)
        : [...order.value];
  if (props.manual) {
    for (const key of doomed) closeBook(key.replace(/^manual:/, ""));
    if (!books.value.length) addBook();
    return;
  }
  for (const key of doomed) {
    const tab = tabs.value.find((one) => `browser:${one.target}` === key);
    if (!tab) continue;
    try {
      await props.port.browserClose(tab.id);
    } catch (e) {
      props.onError(e);
    }
  }
  chosen.value = "";
  await readTabs();
  if (!tabs.value.length) blank.value = true;
};

// ---- element picking ----------------------------------------------------
const picking = ref(false);
const pickEnd = ref("");
const element = ref<PickedElement | null>(null);

// Each ending is a different thing to say; an empty one is someone who meant
// to cancel.
const PICK_ENDED: Partial<Record<BrowserPickEnd, () => string>> = {
  timeout: () => t("workbench.pick_ended_timeout"),
  hidden: () => t("workbench.pick_ended_hidden"),
  navigation: () => t("workbench.pick_ended_navigation"),
  gone: () => t("workbench.pick_ended_gone"),
};

const pick = async () => {
  const page = target.value;
  if (!page || props.manual) return;
  if (picking.value) {
    shell.cancelBrowserPick(page);
    picking.value = false;
    return;
  }
  picking.value = true;
  pickEnd.value = "";
  element.value = null;
  const answer = await shell.pickBrowserElement(page, {
    color: t("workbench.edit_color"),
    background: t("workbench.edit_background"),
    font: t("workbench.edit_font_size"),
    frame: t("workbench.pick_ended_gone"),
    shadow: t("workbench.pick_ended_gone"),
  });
  picking.value = false;
  if (answer.cancelled) {
    pickEnd.value = PICK_ENDED[answer.reason]?.() ?? "";
    return;
  }
  element.value = answer.element;
};

// A pick belongs to the page it was armed on: another page, or another address
// on the same one, leaves it nothing to point at.
watch([address, () => props.manual], () => {
  element.value = null;
  if (!picking.value) return;
  picking.value = false;
  if (target.value && !props.manual) shell.cancelBrowserPick(target.value);
});

// ---- the picture --------------------------------------------------------
const shot = () => {
  if (!native || props.manual) { reload.value += 1; return; }
  if (still.value) { still.value = false; frame.value = ""; return; }
  void shell.freezeBrowserView().then((picture) => {
    still.value = picture !== "";
    frame.value = picture;
  });
};

// A page the shell draws is a native view laid over everything, so the column
// has to say where it may go and when something drawn over it must be seen
// instead. Everything else — no page, a hidden column, a page that did not
// load — is answered by putting the view away.
watch([() => props.visible, () => props.manual, target, () => !!failure.value, still], ([shown, manual, page, broken, frozen], _was, onCleanup) => {
  // Only a picture the shell is no longer standing in for goes away; the one
  // the shot button asked for has to stay where it is.
  if (!frozen) frame.value = "";
  if (!native) return;
  const el = slot.value;
  if (!el || !shown || manual || !page || broken || frozen) {
    shell.hideBrowserView();
    return;
  }
  let timer = 0;
  let live = true;
  let freezing = 0;   // which freeze is current: an older picture is stale
  let freezes = 0;
  const place = () => {
    timer = 0;
    const box = visibleBox(el);
    if (!box) { freezing = 0; shell.hideBrowserView(); return; }
    if (document.documentElement.hasAttribute(SWAP_MARK) || occluded(el, box)) {
      if (freezing) return;
      const turn = (freezing = ++freezes);
      void shell.freezeBrowserView().then((picture) => { if (live && freezing === turn) frame.value = picture; });
      return;
    }
    if (freezing) { freezing = 0; frame.value = ""; }
    shell.showBrowserView(page, box);
  };
  const schedule = () => { if (!timer) timer = window.setTimeout(place, 48); };
  place();
  const resize = new ResizeObserver(schedule);
  resize.observe(el);
  // A layer can leave without touching the column's own box — a transition
  // ending, a hover, a scroll. The events catch most of it; the poll is what
  // makes a page that is no longer covered come back whatever moved.
  const overlays = new MutationObserver(schedule);
  overlays.observe(document.body, { childList: true, subtree: true, attributes: true });
  overlays.observe(document.documentElement, { attributes: true, attributeFilter: [SWAP_MARK, "style", "class"] });
  document.addEventListener("transitionend", schedule, true);
  document.addEventListener("animationend", schedule, true);
  addEventListener("resize", schedule);
  const poll = window.setInterval(schedule, 400);
  onCleanup(() => {
    live = false;
    if (timer) clearTimeout(timer);
    clearInterval(poll);
    resize.disconnect();
    overlays.disconnect();
    document.removeEventListener("transitionend", schedule, true);
    document.removeEventListener("animationend", schedule, true);
    removeEventListener("resize", schedule);
    shell.hideBrowserView();
  });
}, { immediate: true, flush: "post" });

// A native view outlives the component that asked for it, so leaving the column
// has to take it down explicitly rather than wait for a render that never comes.
onScopeDispose(() => shell.hideBrowserView());
</script>

<template>
  <div class="bpanel">
    <div class="bbar">
      <div class="vtabs" role="tablist">
        <template v-if="manual">
          <button
            v-for="one in pages"
            :key="one.id"
            type="button"
            class="tab btab"
            role="tab"
            data-action="browser.tab"
            :data-target="one.id"
            :aria-selected="one.id === bookID"
            :title="one.url"
            @click="bookID = one.id"
            @contextmenu="openMenu($event, 'manual:' + one.id, pageName(one))"
          >{{ pageName(one) }}</button>
        </template>
        <template v-else>
          <button
            v-for="tab in tabs"
            :key="tab.target"
            type="button"
            class="tab btab"
            role="tab"
            data-action="browser.tab"
            :data-target="tab.id"
            :aria-selected="tab.target === target"
            :title="tab.url"
            @click="chosen = tab.target; blank = false"
            @contextmenu="openMenu($event, 'browser:' + tab.target, tab.title || tab.url || t('common.blank_page'))"
          >{{ tab.title || tab.url || t("common.blank_page") }}</button>
        </template>
        <TooltipButton
          class="bbtn"
          data-action="browser.new"
          :aria-label="t('workbench.new_tab')"
          :title="t('workbench.new_tab')"
          @click="newTab"
        ><StudioIcon name="plus" /></TooltipButton>
      </div>
      <div class="bnav">
        <button type="button" class="bbtn" data-action="browser.control" data-value="back"
          :aria-label="t('workbench.back')" :title="t('workbench.back')" :disabled="!canBack" @click="control('back')">
          <ArrowLeft :size="16" aria-hidden="true" />
        </button>
        <button type="button" class="bbtn" data-action="browser.control" data-value="forward"
          :aria-label="t('workbench.forward')" :title="t('workbench.forward')" :disabled="!canForward" @click="control('forward')">
          <ArrowRight :size="16" aria-hidden="true" />
        </button>
        <button type="button" class="bbtn" data-action="browser.control" data-value="reload"
          :aria-label="t('common.reload')" :title="t('common.reload')" :disabled="!manual && !target" @click="control('reload')">
          <StudioIcon name="refresh" />
        </button>
        <button type="button" class="bbtn" data-action="browser.control" data-value="stop"
          :aria-label="t('common.stop')" :title="t('common.stop')" :disabled="manual || !native || !target" @click="control('stop')">
          <StudioIcon name="stop" />
        </button>
        <BrowserTools
          :page="manual ? !!book : !!target"
          :can-pick="picks && !manual"
          :picking="picking"
          :still="still"
          :manual="manual"
          :on-pick="pick"
          :on-shot="shot"
          :on-exit="() => props.onManual(false)"
        />
        <button type="button" class="bbtn" data-action="browser.external"
          :aria-label="manual ? t('workbench.open_in_external_browser') : t('workbench.open_this_page_in_the')"
          :title="manual ? t('workbench.open_in_external_browser') : t('workbench.open_this_page_in_the')"
          :disabled="!address || address === startPage" @click="external">
          <StudioIcon name="external" />
        </button>
        <button type="button" class="bbtn" data-action="browser.tab-close"
          :aria-label="t('workbench.close_tab')" :title="t('workbench.close_tab')"
          :disabled="manual ? !book : !target" @click="closeTab">
          <StudioIcon name="close" />
        </button>
        <form class="baddr" data-action="browser.navigate" @submit.prevent="submit">
          <input
            ref="field"
            v-model="typed"
            data-action="browser.navigate"
            :aria-label="t('workbench.web_address')"
            :aria-invalid="invalid ? 'true' : undefined"
            :placeholder="t('workbench.web_address')"
            spellcheck="false"
            autocapitalize="off"
            autocomplete="off"
          />
        </form>
        <BrowserFullToggle :full="full" :on-toggle="() => props.onFull(!props.full)" />
      </div>
    </div>
    <p v-if="invalid" class="studio-browser-error" role="alert">{{ t("workbench.enter_a_valid_http_or") }}</p>
    <p v-if="picking" class="studio-browser-note" role="status">{{ t("workbench.picking_element") }}</p>
    <p v-else-if="pickEnd" class="studio-browser-note" role="status">{{ pickEnd }}</p>
    <BrowserElementEdit
      v-if="element"
      :key="element.selector"
      :element="element"
      :file-path="documentPath"
      :editor="editor"
      :on-saved="() => control('reload')"
      :on-close="() => (element = null)"
    />
    <ManualBrowsers
      v-if="manual"
      :pages="pages"
      :active="bookID"
      :reload="reload"
      :start="startPage"
      :on-external="(to) => void shell.openExternal(to)"
    />
    <div v-else ref="slot" class="bview">
      <BrowserFailure
        v-if="failure"
        :failure="failure"
        :retry="retry"
        :on-external="external"
        :trust="trust"
      />
      <img v-else-if="frame" class="bframe" :src="frame" alt="" draggable="false" />
      <!-- Without a shell view there is nothing on this machine to place over
           the slot: the page is drawn on the agent's, and what this window can
           honestly say is which page that is. -->
      <AgentPageNote v-else-if="agent" :tab="agent" />
      <div v-else class="bpage">
        <b>{{ t("common.built_in_browser") }}</b>
        <p>{{ t("workbench.enter_an_address_above_to") }}</p>
        <p>{{ t("workbench.some_sites_do_not_allow") }}</p>
      </div>
    </div>
    <BrowserLogin :targets="keep" />
    <BrowserTabMenu
      v-if="menu"
      :menu="menu"
      :others="others"
      :right="right"
      :all="all"
      :on-dismiss="() => (menu = null)"
      :on-close="closeMode"
    />
  </div>
</template>
