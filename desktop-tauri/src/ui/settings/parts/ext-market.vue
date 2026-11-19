<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { HttpError } from "../../../port/http_error";
import type {
  AgentPort, MarketDetail, MarketKind, MarketPackage, MarketPlan, MarketQuery,
} from "../../../port/port";
import MarketCacheNotice from "../../MarketCache.vue";
import ModPreviewBlock from "../../ModPreviewBlock.vue";
import { arrowRadios } from "../../tablist";
import StudioIcon from "../../StudioIcon.vue";

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

const props = defineProps<{ port: AgentPort; onInstalled: () => void }>();

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
  busy.value = "detail";
  try {
    detail.value = await props.port.marketDetail(slug);
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

const approvalSaid = (p: Pick<MarketPackage, "approvalRate" | "upCount" | "downCount">) => p.approvalRate == null
  ? t("ext.no_votes_yet")
  : t("ext.pct_positive_n_votes", { pct: Math.round(p.approvalRate * 100), n: p.upCount + p.downCount });
</script>

<template>
  <!-- The entry: one package, the version a reviewer let through, and the plan
       that decides whether anything lands on this machine. -->
  <div v-if="open" class="mkt mkt-entry">
    <div class="mkt-hd">
      <span class="nm">{{ detail?.package.name ?? open }}</span>
      <span v-if="detail" class="mkt-kind">{{ t(KIND_NAME[detail.package.kind] ?? detail.package.kind) }}</span>
      <span v-if="detail?.package.verified" class="mkt-badge" data-tone="ok">{{ t("ext.admin_marked_trusted") }}</span>
      <span class="mkt-fill" />
      <button class="act" type="button" data-action="market.back" @click="back">{{ t("ext.back_to_list") }}</button>
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
          <span class="t">{{ t("ext.you_chose_to_trust_this") }}</span>
          <span class="why">{{ t("ext.this_is_what_the_source") }}</span>
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
          <button class="act" type="button" data-action="market.cancel" :disabled="busy === 'install'" @click="plan = null">{{ t("common.back") }}</button>
          <button class="act" type="button" data-action="market.install" data-primary :disabled="busy === 'install'" @click="void install()">
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

  <div v-else class="mkt" role="region" :aria-label="t('ext.community_market')">
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
</template>
