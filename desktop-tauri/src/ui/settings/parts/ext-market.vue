<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type {
  AccountState, AgentPort, MarketDetail, MarketKind, MarketPackage, MarketPlan, MarketPublished, MarketQuery, MarketSubmission, MarketVote,
} from "../../../port/port";
import MarketCacheNotice from "../../MarketCache.vue";
import ModPreviewBlock from "../../ModPreviewBlock.vue";
import { arrowRadios } from "../../tablist";
import StudioIcon from "../../StudioIcon.vue";

type View = "browse" | "mine" | "publish";
type Sort = "recommended" | "trending" | "installs" | "new";

const KINDS: readonly (readonly [MarketKind | "", string])[] = [
  ["", "common.all"],
  ["skill", "settings.skills"],
  ["plugin", "ext.plugins"],
  ["mcp", "persona.mcp_servers"],
  ["theme", "ext.theme"],
];
const SORTS: readonly (readonly [Sort, string])[] = [
  ["recommended", "composer.recommended"],
  ["trending", "ext.trending"],
  ["installs", "ext.most_installed"],
  ["new", "ext.newest"],
];
const KIND_NAME: Record<string, string> = { skill: "settings.skills", plugin: "ext.plugins", mcp: "persona.mcp_servers", theme: "ext.theme" };
// The registry's review state, as the publisher reads it. A state this build has
// no line for is shown as the registry wrote it.
const STATUS: Record<string, readonly [string, string | undefined]> = {
  pending: ["feedback.in_review", undefined],
  active: ["ext.public", "ok"],
  rejected: ["ext.rejected", "err"],
  hidden: ["ext.hidden", undefined],
  private: ["ext.private", undefined],
};
// What a source has to look like before the registry will take it. The kernel
// refuses anything else before the request leaves the machine.
const SOURCE_TIP: Record<MarketKind, string> = {
  skill: "ext.an_https_address_of_a_2",
  plugin: "ext.a_github_directory_pinned_to",
  mcp: "ext.an_https_address_of_a",
  theme: "ext.a_github_directory_pinned_to",
};

const props = defineProps<{ port: AgentPort; account: AccountState | null; onInstalled: () => void; onSignIn: () => void }>();

const view = ref<View>("browse");
const kind = ref<MarketKind | "">("");
const sort = ref<Sort>("recommended");
const query = ref("");
const pinned = ref(true);
const packages = ref<MarketPackage[] | null>(null);
const more = ref(false);
const offset = ref(0);
const loadingMore = ref(false);
const failedNote = ref("");
const cache = ref<MarketDetail["cache"]>(undefined);
const filterUnsupported = ref(false);

// The entry is a view of the same page rather than a second route: a card opens
// it in place, and Back returns the list exactly as it was left.
const open = ref("");
const inspecting = ref(false);
const detail = ref<MarketDetail | null>(null);
const detailError = ref("");
const plan = ref<MarketPlan | null>(null);
const done = ref<MarketPlan | null>(null);
const busy = ref("");
const vote = ref<MarketVote | null>(null);

// Publishing spends the account session; the form only collects, and the
// registry decides what the fields may say.
const draft = ref<MarketSubmission & { tagInput: string; private: boolean }>({
  kind: "skill", name: "", source: "", summary: "", description: "", repoUrl: "", version: "", tags: [],
  visibility: "public", tagInput: "", private: false,
});
const published = ref<MarketPublished | null>(null);
const mine = ref<MarketPackage[] | null>(null);

// A read that started before the list changed must not overwrite the newer one.
let asked = 0;

const load = async (from: number, refresh = false) => {
  const n = ++asked;
  failedNote.value = "";
  filterUnsupported.value = false;
  if (from > 0) loadingMore.value = true;
  else more.value = false;
  const q: MarketQuery = { kind: kind.value, q: query.value.trim(), sort: sort.value, offset: from, pinned: pinned.value, ...(refresh ? { refresh } : {}) };
  try {
    const page = await props.port.marketList(q);
    if (n !== asked) return;
    offset.value = from + page.packages.length;
    cache.value = page.cache ?? (from === 0 ? undefined : cache.value);
    packages.value = from === 0 || !packages.value
      ? page.packages
      : [...packages.value, ...page.packages.filter((p) => !packages.value!.some((seen) => seen.slug === p.slug))];
    more.value = page.packages.length >= page.limit;
  } catch (e) {
    if (n !== asked) return;
    failedNote.value = reason(e);
    filterUnsupported.value = pinned.value && e instanceof HttpError && e.reason?.code === "market.filter_unsupported";
    if (from === 0) {
      packages.value = [];
      cache.value = undefined;
    }
  } finally {
    if (n === asked) loadingMore.value = false;
  }
};

// The query is debounced, because every keystroke is a registry read and the
// list is shown before the answer arrives rather than after.
watch([() => props.port, kind, sort, pinned], () => { void load(0); }, { immediate: true });
let timer: ReturnType<typeof setTimeout> | undefined;
watch(query, () => {
  clearTimeout(timer);
  timer = setTimeout(() => void load(0), 250);
});

const openEntry = async (slug: string, inspect = false) => {
  open.value = slug;
  inspecting.value = inspect;
  detail.value = null;
  detailError.value = "";
  plan.value = null;
  done.value = null;
  vote.value = null;
  busy.value = "detail";
  try {
    detail.value = await props.port.marketDetail(slug);
    vote.value = await props.port.marketMyVote(slug).catch(() => null);
    if (inspect && detail.value.pinned && !detail.value.cache && !(detail.value.installed)) await preview();
  } catch (e) {
    detailError.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const back = () => {
  open.value = "";
  inspecting.value = false;
  detail.value = null;
  plan.value = null;
  done.value = null;
  detailError.value = "";
};

const update = computed(() => !!detail.value?.installed && detail.value.installed.version !== detail.value.package.latestVersion);

// trust accepts a version no reviewer pinned; the kernel then pins the install
// to this preview's digest instead of the reviewer's.
const preview = async (trust = false) => {
  if (!detail.value) return;
  busy.value = "plan";
  detailError.value = "";
  try {
    const next = await props.port.planMarket({ slug: detail.value.package.slug, replace: update.value, ...(trust ? { trust } : {}) });
    if (!next.ok) detailError.value = next.error || next.next || t("ext.there_is_nothing_installable_at");
    else plan.value = next;
  } catch (e) {
    detailError.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const install = async () => {
  if (!plan.value || !detail.value) return;
  busy.value = "install";
  detailError.value = "";
  try {
    const pin = plan.value.unreviewed ? { trust: true, digest: plan.value.contentDigest } : {};
    const out = await props.port.installMarket({ slug: detail.value.package.slug, version: plan.value.version, planId: plan.value.planId, replace: update.value, ...pin });
    done.value = out;
    if (out.applied) props.onInstalled();
  } catch (e) {
    detailError.value = reason(e);
  } finally {
    busy.value = "";
  }
};

// One vote per account: pressing the lit button again withdraws it. Whether this
// account may vote is the registry's answer, never guessed here.
const cast = async (value: -1 | 1) => {
  if (!detail.value) return;
  busy.value = "vote";
  detailError.value = "";
  try {
    vote.value = await props.port.voteMarket(detail.value.package.slug, vote.value?.value === value ? 0 : value);
  } catch (e) {
    detailError.value = reason(e);
    if (e instanceof HttpError && e.reason?.code === "market.signed_out") vote.value = { signedIn: false, value: 0 };
  } finally {
    busy.value = "";
  }
};

const approvalSaid = (p: Pick<MarketPackage, "approvalRate" | "upCount" | "downCount">) => p.approvalRate == null
  ? t("ext.no_votes_yet")
  : t("ext.pct_positive_n_votes", { pct: Math.round(p.approvalRate * 100), n: p.upCount + p.downCount });

const loadMine = async () => {
  mine.value = null;
  failedNote.value = "";
  try {
    mine.value = await props.port.myMarket();
  } catch (e) {
    mine.value = [];
    failedNote.value = reason(e);
  }
};

watch(view, (next) => {
  if (next === "mine") void loadMine();
}, { immediate: true });

// A private package can also be sent to review, which is the one act that moves
// it out of the publisher's own hands.
const submit = async (slug: string) => {
  busy.value = `submit:${slug}`;
  failedNote.value = "";
  try {
    const next = await props.port.submitMarket(slug);
    mine.value = mine.value?.map((p) => (p.slug === slug ? { ...p, ...next, installed: p.installed } : p)) ?? null;
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

// A publisher installing their own package: the kernel fetches it from the
// registry as the account and installs only what matches the preview's digest.
const ownInstall = async (pkg: MarketPackage) => {
  open.value = pkg.slug;
  inspecting.value = true;
  detail.value = null;
  detailError.value = "";
  plan.value = null;
  done.value = null;
  vote.value = null;
  busy.value = "detail";
  try {
    const next = await props.port.planOwnMarket({ slug: pkg.slug, replace: !!pkg.installed && pkg.installed.version !== pkg.latestVersion });
    if (!next.ok) detailError.value = next.error || next.next || t("ext.there_is_nothing_installable_at");
    else plan.value = next;
  } catch (e) {
    detailError.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const ownApply = async (slug: string) => {
  if (!plan.value) return;
  busy.value = "install";
  detailError.value = "";
  try {
    const out = await props.port.installOwnMarket({ slug, version: plan.value.version, planId: plan.value.planId, digest: plan.value.contentDigest });
    done.value = out;
    if (out.applied) props.onInstalled();
  } catch (e) {
    detailError.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const publish = async () => {
  busy.value = "publish";
  failedNote.value = "";
  const tags = [...(draft.value.tags ?? []).filter(Boolean), ...draft.value.tagInput.split(/[,，]/).map((x) => x.trim()).filter(Boolean)];
  try {
    published.value = await props.port.publishMarket({ ...draft.value, tags, visibility: draft.value.private ? "private" : "public" });
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const prepareVersion = (pkg: MarketPackage) => {
  draft.value = {
    kind: pkg.kind, name: pkg.name, source: "", summary: pkg.summary, description: pkg.description,
    repoUrl: pkg.repoUrl, version: "", tags: [...pkg.tags], tagInput: "", private: pkg.status === "private",
  };
  published.value = null;
  view.value = "publish";
};

const handle = computed(() => (props.account?.signedIn ? props.account.user?.handle : undefined));
const canPublish = computed(() => draft.value.name.trim() !== "" && draft.value.source.trim() !== "" && busy.value === "");
</script>

<template>
  <!-- Signed out, publishing is not offered at all: it would refuse on click. -->
  <div v-if="!handle" class="peernote">
    {{ t("ext.sign_in_to_publish_skills") }}
    <button class="act" type="button" data-action="market.signin" @click="onSignIn">{{ t("ext.sign_in") }}</button>
  </div>
  <div v-else class="seg mkt-views" data-text role="radiogroup" :aria-label="t('ext.community_market')" @keydown="arrowRadios">
    <button
      v-for="[id, name] in ([['browse', 'ext.browse'], ['mine', 'ext.my_packages'], ['publish', 'ext.publish']] as [View, string][])"
      :key="id"
      type="button"
      role="radio"
      data-action="market.view"
      :data-value="id"
      :aria-checked="view === id"
      @click="view = id"
    >{{ t(name) }}</button>
  </div>

  <!-- The entry: one package, the version a reviewer let through, and the plan
       that decides whether anything lands on this machine. -->
  <div v-if="open" class="mkt mkt-entry">
    <div class="mkt-hd">
      <span class="nm">{{ detail?.package.name ?? open }}</span>
      <span v-if="detail" class="mkt-kind">{{ t(KIND_NAME[detail.package.kind] ?? detail.package.kind) }}</span>
      <span v-if="detail?.package.verified" class="mkt-badge" data-tone="ok">{{ t("ext.admin_marked_trusted") }}</span>
      <span class="mkt-fill" />
      <button class="act" type="button" data-action="market.back" @click="back">{{ t(view === "mine" ? "ext.back_to_my_packages" : "ext.back_to_list") }}</button>
    </div>

    <MarketCacheNotice v-if="detail?.cache" :cache="detail.cache" detail :on-retry="() => void openEntry(open, true)" />
    <div v-if="!detail && !detailError" class="empty" role="status">{{ t("common.loading") }}</div>
    <div v-if="detailError" class="find" data-lvl="err" role="alert">
      <span class="t">{{ t("ext.could_not_read_name", { name: open }) }}</span>
      <span class="why">{{ detailError }}</span>
    </div>

    <template v-if="detail">
      <p v-if="detail.package.summary" class="mkt-sum">{{ detail.package.summary }}</p>
      <p v-if="detail.package.description" class="ds">{{ detail.package.description }}</p>
      <div v-if="!detail.pinned" class="find" data-lvl="warn" data-unpinned="">
        <span class="t">{{ t("ext.content_not_pinned_by_review") }}</span>
        <span class="why">{{ t("ext.no_content_digest_was_recorded", { handle: detail.package.handle }) }}</span>
      </div>
      <div class="mkt-meta">
        <span>{{ `@${detail.package.handle}` }}</span>
        <span>{{ `v${detail.approved?.version || detail.package.latestVersion}` }}</span>
        <span>{{ t("ext.n_installs", { n: detail.package.installCount }) }}</span>
        <span>{{ approvalSaid(detail.package) }}</span>
      </div>

      <div class="mkt-vote" :data-mine="vote?.value ?? 0" :aria-busy="busy === 'vote'">
        <span class="mkt-vote-rate">{{ approvalSaid({ approvalRate: vote?.approvalRate ?? detail.package.approvalRate, upCount: vote?.upCount ?? detail.package.upCount, downCount: vote?.downCount ?? detail.package.downCount }) }}</span>
        <div class="mkt-vote-btns" role="group" :aria-label="t('ext.vote')">
          <button class="act" type="button" data-action="market.vote" data-value="up" :aria-pressed="vote?.value === 1" :disabled="!vote?.signedIn || vote?.canVote === false || busy === 'vote'" @click="void cast(1)">
            <StudioIcon name="thumbsUp" /> {{ t("ext.upvote") }}
          </button>
          <button class="act" type="button" data-action="market.vote" data-value="down" :aria-pressed="vote?.value === -1" :disabled="!vote?.signedIn || vote?.canVote === false || busy === 'vote'" @click="void cast(-1)">
            <StudioIcon name="thumbsDown" /> {{ t("ext.downvote") }}
          </button>
        </div>
        <button v-if="vote && !vote.signedIn" class="act" type="button" data-action="market.signin" @click="onSignIn">{{ t("ext.sign_in_to_vote") }}</button>
        <span v-if="vote?.own" class="note">{{ t("ext.you_cannot_vote_on_a") }}</span>
        <span v-else-if="vote && vote.emailVerified === false" class="note">{{ t("ext.verify_your_account_email_to") }}</span>
      </div>

      <div class="acts">
        <span class="note">
          {{ detail.cache ? t("ext.showing_cached_data_installing_needs")
            : detail.installed && !update ? t("ext.installed_version", { version: detail.installed.version })
              : update && detail.package.kind === "skill" ? t("ext.a_skill_is_not_overwritten")
                : t("ext.everything_it_would_install_is") }}
        </span>
        <button
          v-if="!plan && detail.pinned && !detail.cache && (!detail.installed || update)"
          class="act"
          type="button"
          data-action="market.inspect"
          data-primary
          :disabled="!!busy"
          @click="void preview()"
        >{{ t(busy === "plan" ? "ext.reading" : update ? "ext.see_what_the_update_installs" : "ext.see_what_it_installs") }}</button>
        <button
          v-if="!plan && !detail.pinned && !detail.cache && !detail.installed"
          class="act"
          type="button"
          data-action="market.trust"
          data-primary
          :disabled="!!busy"
          @click="void preview(true)"
        >{{ t(busy === "plan" ? "ext.reading" : "ext.trust_and_install") }}</button>
      </div>

      <!-- The confirmation every market install passes: the plan as the kernel
           graded it. What the pin is — a reviewer's digest or this preview's own
           — is the plan's to say, so an unreviewed install cannot read as one. -->
      <div v-if="plan" class="mkt addpkg mkc" data-stage="confirm" :aria-busy="busy === 'install'">
        <div class="mkc-head">
          <b>{{ t("ext.name_version_will_install_the", { name: plan.slug, version: plan.version }) }}</b>
          <button class="act" type="button" data-action="market.cancel" :aria-label="t('ext.close_install_confirmation')" :disabled="busy === 'install'" @click="plan = null">
            {{ t("common.cancel") }}
          </button>
        </div>
        <div v-if="plan.unreviewed" class="find" data-lvl="warn" data-unreviewed="">
          <span class="t">{{ t(view === "mine" ? "ext.unreviewed_visible_only_to_you" : "ext.you_chose_to_trust_this") }}</span>
          <span class="why">{{ t(view === "mine" ? "ext.this_is_your_own_version" : "ext.this_is_what_the_source") }}</span>
        </div>
        <div v-else class="find" data-pinned="">
          <span class="t">{{ t("ext.checked_by_content_digest_identical") }}</span>
          <span v-if="plan.contentDigest" class="why"><code>{{ plan.contentDigest }}</code></span>
        </div>
        <div v-if="plan.actions?.some((a) => a.kind === 'mcp')" class="why">{{ t("ext.for_an_mcp_server_only") }}</div>
        <div v-for="(a, i) in plan.actions ?? []" :key="`${a.kind}:${a.name}:${i}`" class="cand">
          <div class="cand-hd">
            <span class="nm">{{ a.name || a.kind }}</span>
            <span class="ds">{{ [a.version, a.manifestKind, a.transport].filter(Boolean).join(" · ") }}</span>
          </div>
          <div v-if="a.runtime" class="risk" data-kind="shell">
            <span class="lb">{{ t("ext.long_running_process") }}</span>
            <span class="dt">{{ [a.runtime.command, ...(a.runtime.args ?? [])].join(" ") }}</span>
          </div>
          <div v-if="a.command" class="risk" data-kind="shell">
            <span class="lb">{{ t("ext.will_start") }}</span>
            <span class="dt">{{ [a.command, ...(a.args ?? [])].join(" ") }}</span>
          </div>
          <div v-if="a.url" class="risk" data-kind="unknown-host">
            <span class="lb">{{ t("ext.will_connect_to") }}</span>
            <span class="dt">{{ a.url }}</span>
          </div>
          <ModPreviewBlock v-if="a.mod" :mod="a.mod" />
        </div>
        <div v-if="plan.hiddenActions || plan.previewTruncated" class="why">
          {{ plan.hiddenActions ? t("ext.n_more_steps_are_not", { n: plan.hiddenActions }) : "" }}{{ plan.hiddenActions && plan.previewTruncated ? " " : "" }}{{ plan.previewTruncated ? t("ext.some_text_is_too_long") : "" }}
        </div>
        <div v-for="w in plan.warnings ?? []" :key="w" class="why">{{ w }}</div>
        <div class="acts">
          <span class="note">
            {{ t(view === "mine" || view === "publish" ? "ext.install_to_mine_available_in" : "ext.install_to_mine_available_in") }}
          </span>
          <button class="act" type="button" data-action="market.cancel" :disabled="busy === 'install'" @click="plan = null">{{ t("common.back") }}</button>
          <button class="act" type="button" data-action="market.install" data-primary :disabled="busy === 'install'" @click="view === 'mine' ? void ownApply(open) : void install()">
            {{ t(busy === "install" ? "ext.installing" : "ext.install") }}
          </button>
        </div>
      </div>

      <div v-if="done" class="find" :data-lvl="done.ok ? 'ok' : 'warn'" role="status">
        <span class="t">{{ done.ok ? t("ext.installed_version", { version: done.version }) : t("ext.not_all_items_were_installed") }}</span>
        <span v-if="done.reloadError" class="why">{{ done.reloadError }}</span>
        <span v-else-if="done.error" class="why">{{ done.error }}</span>
      </div>
    </template>
  </div>

  <div v-else-if="view === 'browse'" class="mkt" role="region" :aria-label="t('ext.community_market')">
    <div class="mkt-bar">
      <input v-model="query" class="mkt-q" type="search" data-action="market.search" :placeholder="t('ext.search_skills_plugins_mcp_servers')" :aria-label="t('ext.search_the_community_market')" />
      <div class="seg" data-text role="radiogroup" :aria-label="t('common.type')" @keydown="arrowRadios">
        <button v-for="[id, name] in KINDS" :key="id || 'all'" type="button" role="radio" data-action="market.kind" :data-value="id || 'all'" :aria-checked="kind === id" @click="kind = id">
          {{ t(name) }}
        </button>
      </div>
      <div class="seg" data-text role="radiogroup" :aria-label="t('ext.sort')" @keydown="arrowRadios">
        <button v-for="[id, name] in SORTS" :key="id" type="button" role="radio" data-action="market.sort" :data-value="id" :aria-checked="sort === id" @click="sort = id">
          {{ t(name) }}
        </button>
      </div>
      <label class="mkt-pin">
        <input v-model="pinned" type="checkbox" data-action="market.pinned" />
        {{ t("ext.pinned_only") }}
      </label>
    </div>

    <div v-if="failedNote" class="find" data-lvl="err" role="alert">
      <span class="t">{{ t("ext.could_not_read_the_community") }}</span>
      <span class="why">{{ failedNote }}</span>
      <div class="acts">
        <button v-if="filterUnsupported" class="act" type="button" data-action="market.show-all" @click="pinned = false">{{ t("ext.see_all_packages") }}</button>
        <button v-else class="act" type="button" data-action="market.retry" @click="void load(0, true)">{{ t("common.retry") }}</button>
      </div>
    </div>
    <MarketCacheNotice v-if="cache && !failedNote" :cache="cache" :on-retry="() => void load(0, true)" />
    <div v-if="packages === null" class="empty" role="status">{{ t("common.loading") }}</div>
    <div v-if="packages?.length === 0 && !failedNote" class="empty">
      {{ t(pinned ? "ext.no_pinned_packages_found_turn" : "ext.no_matching_packages") }}
    </div>

    <ul class="mkt-list">
      <li v-for="p in packages ?? []" :key="p.slug" class="mkt-card">
        <button class="mkt-row" type="button" data-action="market.open" :data-value="p.slug" @click="void openEntry(p.slug)">
          <span class="mkt-card-hd">
            <span class="mkt-ico" aria-hidden="true"><StudioIcon :name="p.kind === 'skill' ? 'spark' : p.kind === 'plugin' ? 'layers' : p.kind === 'mcp' ? 'plug' : 'eye'" /></span>
            <span class="mkt-who">
              <span class="nm">{{ p.name }}<StudioIcon v-if="p.verified" name="check" class="mkt-verified" :aria-label="t('ext.admin_marked_trusted')" /></span>
              <span class="mkt-id">{{ `@${p.handle} · v${p.latestVersion}` }}</span>
            </span>
            <span v-if="p.installed" class="mkt-state">
              {{ t(p.installed.version === p.latestVersion ? "ext.installed" : "ext.update_available") }}
            </span>
          </span>
          <span class="mkt-sum">{{ p.summary }}</span>
          <span class="mkt-card-ft">
            <span class="mkt-kind">{{ t(KIND_NAME[p.kind] ?? p.kind) }}</span>
            <span class="mkt-pinned" :data-on="p.pinned ? '' : undefined" :title="p.pinned === false ? t('ext.no_content_digest_was_recorded_2') : undefined">
              {{ t(p.pinned === undefined ? "ext.pin_status_unknown" : p.pinned ? "common.pinned" : "ext.not_pinned") }}
            </span>
            <span class="mkt-fill" />
            <span class="mkt-n">{{ t("ext.n_installs", { n: p.installCount }) }}</span>
            <span class="mkt-n">{{ approvalSaid(p) }}</span>
          </span>
        </button>
        <button v-if="!p.installed && p.pinned && !cache" class="mkt-get" type="button" data-action="market.inspect" :data-value="p.slug" :title="t('ext.install')" @click="void openEntry(p.slug, true)">
          {{ t("ext.install") }}
        </button>
      </li>
    </ul>

    <button v-if="more" class="act" type="button" data-action="market.more" aria-live="polite" :disabled="loadingMore" @click="void load(offset)">
      {{ t(loadingMore ? "common.loading" : failedNote ? "common.retry" : "ext.load_more") }}
    </button>
  </div>

  <!-- The account's own packages, in whatever state review has them: installing
       and submitting for review are both offered here, because the publisher is
       the only one who can act on either. -->
  <div v-else-if="view === 'mine'" class="mkt" :aria-busy="mine === null">
    <div v-if="failedNote" class="find" data-lvl="err" role="alert">
      <span class="t">{{ t("ext.could_not_read_your_packages") }}</span>
      <span class="why">{{ failedNote }}</span>
      <div class="acts"><button class="act" type="button" data-action="market.mine-retry" @click="void loadMine()">{{ t("common.retry") }}</button></div>
    </div>
    <div v-if="mine === null && !failedNote" class="empty" role="status">{{ t("common.loading") }}</div>
    <div v-if="mine?.length === 0 && !failedNote" class="empty">{{ t("ext.nothing_published_yet") }}</div>
    <ul class="mkt-list" data-rows="">
      <li v-for="p in mine ?? []" :key="p.slug" class="mkt-row" data-static="">
        <span class="mkt-hd">
          <span class="nm">{{ p.name }}</span>
          <span class="mkt-kind">{{ t(KIND_NAME[p.kind] ?? p.kind) }}</span>
          <span class="mkt-state" :data-status="p.status">{{ t(STATUS[p.status]?.[0] ?? p.status) }}</span>
          <span v-if="p.installed" class="mkt-badge">{{ t(p.installed.version === p.latestVersion ? "ext.installed" : "ext.update_available") }}</span>
        </span>
        <span v-if="p.summary" class="mkt-sum">{{ p.summary }}</span>
        <span class="mkt-meta">
          <span class="mkt-id">{{ `@${p.handle} · v${p.latestVersion}` }}</span>
          <span v-if="p.status === 'active'">{{ t("ext.n_installs", { n: p.installCount }) }}</span>
          <span v-if="p.status === 'private'">{{ t("ext.visible_only_to_you_not") }}</span>
        </span>
        <span class="acts">
          <button class="act" type="button" data-action="market.prepare-version" :data-value="p.slug" :disabled="!!busy" @click="prepareVersion(p)">{{ t("ext.publish_new_version") }}</button>
          <button v-if="p.status === 'private'" class="act" type="button" data-action="market.submit" :data-value="p.slug" :disabled="!!busy" @click="void submit(p.slug)">
            {{ t(busy === `submit:${p.slug}` ? "ext.submitting" : "ext.submit_for_review") }}
          </button>
          <button v-if="!p.installed || p.installed.version !== p.latestVersion" class="act" type="button" data-action="market.own-inspect" :data-value="p.slug" @click="void ownInstall(p)">
            {{ t(p.installed ? "ext.see_what_the_update_installs" : "ext.install") }}
          </button>
        </span>
      </li>
    </ul>
  </div>

  <div v-else class="mkt mkt-pub" :aria-busy="busy === 'publish'">
    <div v-if="published" class="find" data-lvl="ok" role="status">
      <span class="t">
        {{ published.package.status === "private"
          ? t("ext.saved_slug_version_visible_only", { slug: published.package.slug, version: published.version })
          : t("ext.submitted_slug_version", { slug: published.package.slug, version: published.version }) }}
      </span>
      <span class="why">{{ t(published.package.status === "private" ? "ext.it_skips_review_and_never" : "ext.it_goes_public_in_the") }}</span>
    </div>
    <template v-else>
      <h3 class="mkt-pub-title">{{ t("ext.publish_to_the_market") }}</h3>
      <p class="mkt-sum">{{ t(draft.private ? "ext.saved_as_handle_visible_only" : "ext.submitted_as_handle_and_made", { handle: handle ?? "" }) }}</p>
      <div class="seg" data-text role="radiogroup" :aria-label="t('common.type')" @keydown="arrowRadios">
        <button
          v-for="[id, name] in KINDS.filter(([id]) => id !== '')"
          :key="id"
          type="button"
          role="radio"
          data-action="market.draft"
          data-value="kind"
          :aria-checked="draft.kind === id"
          :disabled="busy === 'publish'"
          @click="draft.kind = id as MarketKind"
        >{{ t(name) }}</button>
      </div>
      <p v-if="draft.kind === 'theme'" class="note">{{ t("ext.a_theme_is_published_as") }}</p>
      <div class="mkt-fields">
        <label class="full">
          <span>{{ t("ext.source_address") }}</span>
          <input v-model="draft.source" class="mono" spellcheck="false" data-action="market.draft" data-value="source" :disabled="busy === 'publish'" />
          <em class="mkt-tip">{{ t(SOURCE_TIP[draft.kind]) }}</em>
        </label>
        <label>
          <span>{{ t("common.name") }}</span>
          <input v-model="draft.name" placeholder="my-package" spellcheck="false" data-action="market.draft" data-value="name" :disabled="busy === 'publish'" />
          <em class="mkt-tip">{{ t("ext.lowercase_letters_digits_dots_underscores") }}</em>
        </label>
        <label>
          <span>{{ t("common.version") }}</span>
          <input v-model="draft.version" placeholder="0.1.0" spellcheck="false" data-action="market.draft" data-value="version" :disabled="busy === 'publish'" />
          <em class="mkt-tip">{{ t("ext.blank_starts_a_new_package") }}</em>
        </label>
        <label class="full">
          <span>{{ t("ext.summary") }}</span>
          <input v-model="draft.summary" maxlength="200" data-action="market.draft" data-value="summary" :disabled="busy === 'publish'" />
        </label>
        <label class="full">
          <span>{{ t("ext.description") }}</span>
          <textarea v-model="draft.description" rows="4" maxlength="8000" data-action="market.draft" data-value="description" :disabled="busy === 'publish'" />
        </label>
        <label>
          <span>{{ t("ext.repository") }}</span>
          <input v-model="draft.repoUrl" class="mono" placeholder="https://github.com/…" spellcheck="false" data-action="market.draft" data-value="repoUrl" :disabled="busy === 'publish'" />
        </label>
        <label>
          <span>{{ t("ext.tags") }}</span>
          <input v-model="draft.tagInput" :placeholder="t('ext.comma_separated_up_to_8')" data-action="market.draft" data-value="tags" :disabled="busy === 'publish'" />
        </label>
      </div>
      <label class="mkt-seen mkt-private">
        <input v-model="draft.private" type="checkbox" data-action="market.draft" data-value="visibility" :disabled="busy === 'publish'" />
        {{ t("ext.only_me_skip_review_in") }}
      </label>
      <div v-if="failedNote" class="find" data-lvl="err" role="alert">
        <span class="t">{{ t("ext.the_submission_did_not_go") }}</span>
        <span class="why">{{ failedNote }}</span>
      </div>
      <div class="acts">
        <span class="note">{{ t(draft.private ? "ext.only_you_will_see_it" : "ext.it_joins_the_review_queue") }}</span>
        <button class="act" type="button" data-action="market.publish" data-primary :disabled="!canPublish" @click="void publish()">
          {{ t(busy === "publish" ? "ext.submitting" : draft.private ? "ext.save_as_private" : "ext.submit_for_review") }}
        </button>
      </div>
    </template>
    <div class="acts">
      <button class="act" type="button" data-action="market.publish-again" @click="published = null">{{ t("ext.publish_another") }}</button>
      <button class="act" type="button" data-action="market.view" data-value="mine" @click="view = 'mine'">{{ t("ext.see_my_packages") }}</button>
    </div>
  </div>
</template>
