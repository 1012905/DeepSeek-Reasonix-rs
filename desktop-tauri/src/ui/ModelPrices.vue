<script lang="ts">
import { t } from "../i18n";
import type { ModelRate } from "../port/port";
import { shortId } from "./ModelLimits.vue";

export interface PriceText {
  inp: string;
  out: string;
  hit: string;
  cur: string;
  src: string;
  chk: string;
}

const CURRENCIES = ["CNY", "USD"] as const;
const blank = (cur = "CNY"): PriceText => ({ inp: "", out: "", hit: "", cur, src: "", chk: "" });
const typed = (n: number) => String(n);

export function priceTextOf(own: ModelRate | undefined): PriceText {
  return own ? { inp: typed(own.input), out: typed(own.output), hit: own.cacheHit === own.input ? "" : typed(own.cacheHit), cur: own.currency || "CNY", src: own.sourceUrl ?? "", chk: own.checked ?? "" } : blank();
}

// Digits and at most one point: a price is not an integer, and a sign or an
// exponent is how a typo becomes a rate nobody meant.
export const decimal = (s: string) => {
  const cleaned = s.replace(/[^\d.]/g, "");
  const at = cleaned.indexOf(".");
  return at < 0 ? cleaned : cleaned.slice(0, at + 1) + cleaned.slice(at + 1).replace(/\./g, "");
};

const parsed = (s: string) => (s === "" || s === "." ? null : Number(s));

// A row is a price once input and output are both typed; one of them alone is a
// half-finished row, which is neither a price nor an empty one.
export const incomplete = (row: PriceText | undefined) => !!row && (row.inp === "") !== (row.out === "");

const MAX_SOURCE = 500;
const utcToday = () => new Date().toISOString().slice(0, 10);

// A source is a link to read: http or https, a host, no credentials, no blanks,
// control or format characters. Nothing here opens or fetches it.
export function sourceProblem(src: string): boolean {
  if (src === "") return false;
  if (src.length > MAX_SOURCE || /[\s\p{Cc}\p{Cf}]/u.test(src)) return true;
  try {
    const u = new URL(src);
    return !((u.protocol === "http:" || u.protocol === "https:") && u.hostname !== "" && u.username === "" && u.password === "");
  } catch {
    return true;
  }
}

// A check date is a calendar date, no later than today in UTC.
export function checkedProblem(chk: string): boolean {
  if (chk === "") return false;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(chk)) return true;
  const d = new Date(`${chk}T00:00:00Z`);
  return Number.isNaN(d.getTime()) || d.toISOString().slice(0, 10) !== chk || chk > utcToday();
}

export const sourceBad = (row: PriceText | undefined) => !!row && (sourceProblem(row.src) || checkedProblem(row.chk));

// What a row sends: the source and date ride with the price they describe.
const provenance = (row: PriceText): Pick<ModelRate, "sourceUrl" | "checked"> => ({
  ...(row.src !== "" ? { sourceUrl: row.src } : {}),
  ...(row.chk !== "" ? { checked: row.chk } : {}),
});

const hostOf = (src: string) => { try { return new URL(src).host; } catch { return ""; } };

// An empty row clears the model's own price; an incomplete one changes nothing.
// A blank cached-input price is the input price: no discount is assumed.
export function pricesToSend(picked: string[], text: Record<string, PriceText>, stored: Record<string, ModelRate>): Record<string, ModelRate | null> {
  const out: Record<string, ModelRate | null> = {};
  for (const model of picked) {
    const row = text[model] ?? blank();
    if (incomplete(row)) {
      if (stored[model]) out[model] = stored[model];
      continue;
    }
    const input = parsed(row.inp);
    const output = parsed(row.out);
    if (input === null || output === null) {
      if (stored[model]) out[model] = null;
      continue;
    }
    out[model] = { input, output, cacheHit: parsed(row.hit) ?? input, currency: row.cur, ...provenance(row) };
  }
  return out;
}
</script>

<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(defineProps<{
  models: string[];
  labels?: Record<string, string>;
  value: Record<string, PriceText>;
  onChange: (next: Record<string, PriceText>) => void;
  official?: boolean;
  stored?: Record<string, ModelRate>;
  onOpenSource?: (url: string) => void;
}>(), { official: false, stored: () => ({}) });

const set = (model: string, patch: Partial<PriceText>) =>
  props.onChange({ ...props.value, [model]: { ...(props.value[model] ?? blank()), ...patch } });

const rows = computed(() => props.models.map((model) => {
  const cur = props.value[model] ?? blank();
  const own = cur.inp !== "" || cur.out !== "";
  const foreign = props.official && own && props.stored[model] !== undefined && props.stored[model]!.official === false;
  const freeCached = foreign && cur.hit !== "" && Number(cur.hit) === 0 && Number(cur.inp) > 0;
  return { model, cur, own, foreign, freeCached };
}));
</script>

<template>
  <div class="ml-group mp-group" role="group" :aria-label="t('usage.price_by_model')">
    <div class="ml-head">
      <strong>{{ t("usage.price_by_model") }}</strong>
      <span>{{ t("usage.price_per_million_tokens_left") }}</span>
    </div>
    <ul class="me-list ml-list">
      <li v-for="r in rows" :key="r.model" class="ml-row mp-row" :data-model="r.model" :data-own="r.own || undefined">
        <span class="me-name" :title="r.model">{{ labels?.[r.model] ?? shortId(r.model) }}</span>
        <span class="unit-field">
          <input data-action="provider.draft" data-value="model-price-input" :aria-label="t('usage.model_input_price', { model: r.model })"
            :aria-invalid="incomplete(r.cur) && r.cur.inp === '' ? true : undefined" inputmode="decimal" :value="r.cur.inp" :placeholder="t('usage.unknown')"
            @input="set(r.model, { inp: decimal(($event.target as HTMLInputElement).value) })" /><i>{{ t("common.input") }}</i>
        </span>
        <span class="unit-field">
          <input data-action="provider.draft" data-value="model-price-output" :aria-label="t('usage.model_output_price', { model: r.model })"
            :aria-invalid="incomplete(r.cur) && r.cur.out === '' ? true : undefined" inputmode="decimal" :value="r.cur.out" :placeholder="t('usage.unknown')"
            @input="set(r.model, { out: decimal(($event.target as HTMLInputElement).value) })" /><i>{{ t("common.output") }}</i>
        </span>
        <span class="unit-field">
          <input data-action="provider.draft" data-value="model-price-cache" :aria-label="t('usage.model_cache_hit_price', { model: r.model })"
            inputmode="decimal" :value="r.cur.hit" :placeholder="t('usage.same_as_input')"
            @input="set(r.model, { hit: decimal(($event.target as HTMLInputElement).value) })" /><i>{{ t("common.cache_hit") }}</i>
        </span>
        <select data-action="provider.draft" data-value="model-price-currency" :aria-label="t('usage.model_currency', { model: r.model })" :value="r.cur.cur"
          @change="set(r.model, { cur: ($event.target as HTMLSelectElement).value })">
          <option v-for="c in CURRENCIES" :key="c" :value="c">{{ c }}</option>
        </select>
        <details v-if="r.own" class="mp-source" :data-model="r.model">
          <summary>{{ t("usage.source_and_check_date") }}</summary>
          <span class="mp-source-fields">
            <span class="unit-field mp-src">
              <input type="text" data-action="provider.draft" data-value="model-price-source" :aria-label="t('usage.source_of_the_price_for', { model: r.model })"
                :aria-invalid="sourceProblem(r.cur.src) ? true : undefined" inputmode="url" autocomplete="off" :spellcheck="false" :value="r.cur.src"
                placeholder="https://" @input="set(r.model, { src: ($event.target as HTMLInputElement).value.trim() })" /><i>{{ t("common.source") }}</i>
            </span>
            <span class="unit-field mp-chk">
              <input type="text" data-action="provider.draft" data-value="model-price-checked" :aria-label="t('usage.check_date_of_the_price', { model: r.model })"
                :aria-invalid="checkedProblem(r.cur.chk) ? true : undefined" inputmode="numeric" autocomplete="off" :value="r.cur.chk"
                placeholder="YYYY-MM-DD" @input="set(r.model, { chk: ($event.target as HTMLInputElement).value.trim() })" /><i>{{ t("usage.checked_on") }}</i>
            </span>
          </span>
          <span v-if="sourceProblem(r.cur.src)" class="why">{{ t("usage.the_source_must_be_an") }}</span>
          <span v-if="checkedProblem(r.cur.chk)" class="why">{{ t("usage.the_check_date_cannot_be") }}</span>
        </details>
        <span v-if="r.own && r.cur.src !== '' && !sourceProblem(r.cur.src)" class="mp-source-line">
          <span class="mp-host" :title="r.cur.src">{{ hostOf(r.cur.src) }}</span>
          <span v-if="r.cur.chk !== '' && !checkedProblem(r.cur.chk)" class="mp-checked">{{ t("usage.checked_on_date_utc", { date: r.cur.chk }) }}</span>
          <button v-if="onOpenSource" type="button" class="act" data-action="provider.price-source" :data-target="r.model" :aria-label="t('usage.open_the_price_source_of', { model: r.model })"
            @click="onOpenSource(r.cur.src)">{{ t("common.open") }}</button>
        </span>
        <span v-if="incomplete(r.cur)" class="why">{{ t("usage.fill_in_both_input_and") }}</span>
        <span v-if="r.foreign" class="why mp-foreign">
          <span>{{ t("usage.this_price_is_not_in") }}</span>
          <span v-if="r.freeCached">{{ t("usage.the_cache_hit_price_is") }}</span>
          <button type="button" class="act" data-action="provider.draft" data-value="model-price-restore" :aria-label="t('usage.model_restore_official_price', { model: r.model })"
            @click="set(r.model, { inp: '', out: '', hit: '' })">{{ t("usage.restore_official_price") }}</button>
        </span>
      </li>
    </ul>
  </div>
</template>