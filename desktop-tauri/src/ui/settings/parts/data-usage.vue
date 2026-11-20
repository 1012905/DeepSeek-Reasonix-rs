<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { count, money, pct, tokens as fmtTokens } from "../../../i18n/format";
import { reason } from "../../../i18n/kernel";
import type { AgentPort } from "../../../port/port";
import type { Money, UsageQuery, UsageReport } from "../../../port/usage";
import { DEFAULT_USAGE_DAYS } from "../../../port/usage";
import UsageStack, { type StackPoint } from "../../UsageStack.vue";
import { usageCsv } from "../../usagecsv";

// Money and token counts go through i18n/format: its whole reason for existing is
// that five files each grew their own rule. Intl also spells CNY as CN¥ in an
// English window, where a bare ¥ reads as yen.
const moneyList = (list?: Money[]): string => (list?.length ? list.map((m) => money(Number(m.amount), m.currency)).join("  ·  ") : "—");

const RANGES: [number, string][] = [[7, "usage.n_7_days"], [30, "usage.n_30_days"], [365, "common.all"]];

type Window = { kind: "today" } | { kind: "days"; days: number } | { kind: "custom"; from: string; to: string };

const localDay = (): string => {
  const now = new Date();
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
};

const props = defineProps<{ port: AgentPort }>();

const range = ref<Window>({ kind: "days", days: DEFAULT_USAGE_DAYS });
const customFrom = ref("");
const customTo = ref("");
const report = ref<UsageReport | null>(null);
const err = ref("");
const labels = ref<Record<string, string>>({});

const query = computed<UsageQuery>(() => {
  if (range.value.kind === "today") { const day = localDay(); return { from: day, to: day }; }
  if (range.value.kind === "days") return { days: range.value.days };
  return { from: range.value.from, to: range.value.to };
});

watch([() => props.port, query], ([port, q]) => {
  let live = true;
  err.value = "";
  (port as AgentPort).usage(q as UsageQuery)
    .then((r) => { if (live) report.value = r; })
    .catch((e) => { if (live) err.value = reason(e); });
  return () => { live = false; };
}, { immediate: true });

// A source can be absent from providers() while still being reachable through the
// model catalog, so the catalog fills first and the canonical metadata overrides.
watch(() => props.port, (port) => {
  let live = true;
  Promise.all([port.providers().catch(() => []), port.models().catch(() => [])]).then(([providers, models]) => {
    if (!live) return;
    const found: Record<string, string> = {};
    for (const m of models) {
      const label = m.displayName?.trim() || [m.kind, m.vendor].filter(Boolean).join(" · ");
      if (label) found[m.provider] = label;
    }
    for (const p of providers) {
      const label = p.displayName?.trim() || [p.kind, (p.baseUrl ?? "").replace(/^https?:\/\//, "").replace(/\/.*$/, "")].filter(Boolean).join(" · ");
      if (label) found[p.name] = label;
    }
    labels.value = found;
  });
  return () => { live = false; };
}, { immediate: true });

const openCustom = () => {
  customFrom.value ||= report.value?.from ?? "";
  customTo.value ||= report.value?.to ?? "";
  if (customFrom.value && customTo.value) range.value = { kind: "custom", from: customFrom.value, to: customTo.value };
};

const customValid = computed(() => customFrom.value !== "" && customTo.value !== "" && customFrom.value <= customTo.value);
const submitCustom = () => { if (customValid.value) range.value = { kind: "custom", from: customFrom.value, to: customTo.value }; };

// The first day that carries a cost. Records older than the cost field have
// tokens and no price, and showing them as free would understate the bill.
const pricedFrom = computed(() => report.value?.daily.find((d) => d.cost?.length)?.day ?? "");
const priced = computed(() => report.value?.daily.filter((d) => d.cost?.length) ?? []);
const unpricedDays = computed(() => (report.value?.daily ?? []).filter((d) => d.total > 0 && !d.cost?.length && !d.estimateCost?.length).length);

// Every currency the range touched, in the order the kernel sorted them.
const currencies = computed(() => {
  const seen: string[] = [];
  for (const day of report.value?.daily ?? []) for (const entry of day.cost ?? []) if (!seen.includes(entry.currency)) seen.push(entry.currency);
  return seen;
});

// The rate partitions the input side only (cache hit + miss), while the report's
// total counts prompt + completion. Stating one against the other would put the
// headline claim on the wrong denominator.
const input = computed(() => (report.value ? report.value.cache_hit + report.value.cache_miss : 0));
const hitRate = computed(() => (input.value > 0 && report.value ? (report.value.cache_hit / input.value) * 100 : 0));

// A one-day window reads in hours; the kernel sends those buckets for exactly
// that case, and "today" stops at the current hour so the empty future does not
// read as a drop to zero.
const singleDay = computed(() => !!report.value && report.value.from === report.value.to);
const hours = computed(() => report.value?.hourly ?? []);
const hourView = computed(() => singleDay.value && hours.value.length > 0);
const visibleHours = computed(() => (hourView.value && report.value?.to === localDay() ? hours.value.slice(0, new Date().getHours() + 1) : hours.value));

const usagePoints = computed<StackPoint[]>(() => (hourView.value
  ? visibleHours.value.map((h) => ({ key: h.hour, axis: h.hour, tip: h.hour, hit: h.cacheHit, miss: h.cacheMiss, total: h.total }))
  : (report.value?.daily ?? []).map((d) => ({ key: d.day, axis: d.day.slice(5), tip: d.day, hit: d.cacheHit, miss: d.cacheMiss, total: d.total }))));

// One series per currency, for the same reason tokens and money get their own
// axis: two currencies do not share one, and keeping them apart everywhere else
// would be undone by a merged plot.
function spendPoints(code: string): StackPoint[] {
  const points = hourView.value
    ? visibleHours.value.map((h) => ({ key: h.hour, axis: h.hour, tip: h.hour, value: Number(h.cost?.find((c) => c.currency === code)?.amount ?? 0) }))
    : priced.value.filter((d) => d.cost?.some((c) => c.currency === code)).map((d) => ({ key: d.day, axis: d.day.slice(5), tip: d.day, value: Number(d.cost?.find((c) => c.currency === code)?.amount ?? 0) }));
  return points.map((p) => ({ key: p.key, axis: p.axis, tip: p.tip, hit: p.value, miss: 0, total: p.value }));
}

// A chart is an overview; a phone cannot hover it precisely. The table is the
// exact record underneath the same data, and it stays available on every input.
const dayRows = computed(() => (report.value?.daily ?? [])
  .slice()
  .sort((a, b) => b.day.localeCompare(a.day))
  .map((d) => ({ key: d.day, label: d.day, total: d.total, requests: d.requests, cost: d.cost, estimateCost: d.estimateCost })));
const hourRows = computed(() => visibleHours.value.map((h) => ({ key: h.hour, label: h.hour, total: h.total, requests: h.requests, cost: h.cost, estimateCost: h.estimateCost })));
const rows = computed(() => (hourView.value ? hourRows.value : dayRows.value));

// A source the catalog never named reads as unknown rather than blank: the token
// count beside it is real, and an empty label beside a number is a bug.
const providerLabel = (provider: string): string => labels.value[provider] ?? t("usage.unknown_source");

function exportCsv(): void {
  const held = report.value;
  if (!held) return;
  const url = URL.createObjectURL(new Blob([usageCsv(held)], { type: "text/csv;charset=utf-8" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `reasonix-usage-${held.from}-${held.to}.csv`;
  link.click();
  URL.revokeObjectURL(url);
}
</script>

<template>
  <div class="usage">
    <div class="uhead">
      <div class="uranges" role="group" :aria-label="t('usage.time_range')">
        <button type="button" data-action="usage.range.today" :aria-pressed="range.kind === 'today'" @click="range = { kind: 'today' }">{{ t("usage.today") }}</button>
        <button v-for="[n, label] in RANGES" :key="n" type="button" data-action="usage.range.days" :data-value="String(n)" :aria-pressed="range.kind === 'days' && range.days === n" @click="range = { kind: 'days', days: n }">
          {{ t(label) }}
        </button>
        <button type="button" data-action="usage.range.custom" :aria-pressed="range.kind === 'custom'" @click="openCustom">{{ t("common.custom") }}</button>
      </div>
      <span v-if="report" class="uspan">{{ `${report.from} → ${report.to}` }}</span>
      <button v-if="report" type="button" class="uexport" data-action="usage.export" @click="exportCsv">{{ t("usage.export_csv") }}</button>
    </div>

    <form v-if="range.kind === 'custom'" class="udates" @submit.prevent="submitCustom">
      <label>{{ t("usage.start_date") }}<input data-action="usage.range.start" type="date" :value="customFrom" :max="customTo || undefined" required @input="customFrom = ($event.target as HTMLInputElement).value" /></label>
      <label>{{ t("usage.end_date") }}<input data-action="usage.range.end" type="date" :value="customTo" :min="customFrom || undefined" required @input="customTo = ($event.target as HTMLInputElement).value" /></label>
      <button type="submit" :disabled="!customValid">{{ t("usage.apply") }}</button>
    </form>

    <div v-if="err" class="uerr" role="alert">{{ err }}</div>
    <div v-else-if="!report" class="uwait">{{ t("usage.reading_the_records") }}</div>

    <template v-if="report">
      <div class="utiles">
        <div class="utile utile-paid">
          <!-- "Paid" is honest only while every folded quote came from the
               provider and priced its call. One fallback-priced or unpriced row
               and the label needs the caveat. -->
          <div class="k">{{ t(report.costEstimated || report.costIncomplete ? "usage.spend_2" : "usage.paid") }}</div>
          <div class="v"><b>{{ moneyList(report.cost) }}</b></div>
          <div v-if="report.estimateCost?.length" class="m">{{ t("usage.api_priced_estimate") }} {{ moneyList(report.estimateCost) }} · {{ t("usage.subscription_use_priced_at_api_2") }}</div>
          <div v-if="pricedFrom" class="m">{{ t("usage.from_day_n_days_recorded", { day: pricedFrom.slice(5), n: priced.length }) }}</div>
          <div v-if="report.costEstimated" class="m">{{ t("usage.includes_estimates") }}</div>
          <div v-if="report.costIncomplete" class="m">
            {{ t("usage.unpriced_calls_not_included") }}
            <template v-if="report.unpricedCalls"> · {{ t("usage.n_calls_unpriced", { n: report.unpricedCalls }) }}</template>
          </div>
        </div>
        <div class="utile">
          <div class="k">{{ t("usage.input") }}</div>
          <div class="v"><b>{{ fmtTokens(input) }}</b></div>
          <div class="m">{{ t("usage.cache_hit_rate_note", { rate: hitRate.toFixed(1) }) }}</div>
        </div>
        <div class="utile">
          <div class="k">{{ t("usage.provider_reported_total") }}</div>
          <div class="v"><b>{{ fmtTokens(report.tokens) }}</b></div>
          <div class="m">{{ t("usage.total_includes_output") }}</div>
        </div>
        <div class="utile">
          <div class="k">{{ t("usage.requests") }}</div>
          <div class="v"><b>{{ fmtTokens(report.requests) }}</b></div>
          <div class="m">{{ t("usage.n_turns", { n: fmtTokens(report.turns) }) }}</div>
          <div class="m">{{ t("usage.of_n_days_avg", { n: report.daily.length, avg: fmtTokens(report.active_days ? Math.round(report.requests / report.active_days) : 0) }) }}</div>
        </div>
      </div>

      <div class="ucharts">
        <section class="ucard">
          <div class="ucard-h">
            <h3>{{ t(hourView ? "usage.hourly_usage" : "usage.daily_usage") }}</h3>
            <span class="hint">{{ t(hourView ? "usage.hover_for_an_hour" : "usage.hover_for_a_day") }}</span>
          </div>
          <UsageStack :points="usagePoints" :label="(v) => fmtTokens(Math.round(v))" :aria="t(hourView ? 'usage.hourly_tokens' : 'usage.daily_tokens')" />
        </section>
        <div class="ucharts-spend">
          <section v-for="(code, at) in currencies" :key="code" class="ucard">
            <div class="ucard-h">
              <h3>{{ t(hourView ? "usage.hourly_spend" : "usage.daily_spend") }}</h3>
              <span class="hint">{{ code }}</span>
            </div>
            <p class="ucard-s">{{ t("usage.drawn_apart_from_usage_different") }}</p>
            <UsageStack single :points="spendPoints(code)" :label="(v) => money(v, code)" :aria="t(hourView ? 'usage.hourly_spend' : 'usage.daily_spend')" />
            <div v-if="at === currencies.length - 1 && unpricedDays > 0" class="unote">{{ t("usage.n_more_days_have_usage", { n: unpricedDays }) }}</div>
          </section>
        </div>
      </div>

      <section class="ucard udaily">
        <div class="ucard-h">
          <h3>{{ t(hourView ? "usage.hourly_details" : "usage.daily_details") }}</h3>
          <span class="hint">{{ t(hourView ? "usage.exact_totals_for_each_hour" : "usage.exact_totals_for_each_day") }}</span>
        </div>
        <div class="utable-wrap" role="region" :aria-label="t(hourView ? 'usage.hourly_details' : 'usage.daily_details')" tabindex="0">
          <table class="utable">
            <thead>
              <tr>
                <th>{{ t(hourView ? "usage.time" : "usage.date") }}</th>
                <th>Tokens</th>
                <th>{{ t("usage.spend") }}</th>
                <th>{{ t("usage.requests") }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in rows" :key="row.key" :data-day="hourView ? undefined : row.key" :data-hour="hourView ? row.key : undefined">
                <td class="ud">{{ row.label }}</td>
                <td>{{ count(row.total) }}</td>
                <td>
                  {{ moneyList(row.cost) }}
                  <span v-if="row.estimateCost?.length" class="udaily-est">{{ t("usage.api_priced_estimate") }} {{ moneyList(row.estimateCost) }}</span>
                </td>
                <td>{{ count(row.requests) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <div class="utwo">
        <section class="ucard">
          <div class="ucard-h"><h3>{{ t("usage.by_model") }}</h3></div>
          <div class="urows">
            <div v-for="m in report.models" :key="m.model" class="urow">
              <div class="urow-t"><span class="n" :title="m.model"><b>{{ m.model }}</b></span><span class="v">{{ fmtTokens(m.tokens) }}  {{ pct(m.percent / 100, 1) }}</span></div>
              <div class="utrack"><i class="ufill" :style="{ width: `${m.percent}%` }" /></div>
            </div>
          </div>
        </section>
        <section class="ucard">
          <div class="ucard-h"><h3>{{ t("common.by_source") }}</h3></div>
          <div class="urows">
            <div v-for="p in report.providers" :key="p.provider" class="urow">
              <div class="urow-t"><span class="n" :title="labels[p.provider] ?? p.provider"><b>{{ providerLabel(p.provider) }}</b></span><span class="v">{{ fmtTokens(p.tokens) }}  {{ pct(p.percent / 100, 1) }}</span></div>
              <div class="utrack"><i class="ufill" :style="{ width: `${p.percent}%` }" /></div>
            </div>
          </div>
        </section>
      </div>
    </template>
  </div>
</template>
