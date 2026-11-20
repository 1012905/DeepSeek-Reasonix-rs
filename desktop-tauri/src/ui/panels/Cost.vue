<script lang="ts">
import type { AgentPort } from "../../port/port";

/** What the Cost panel needs. The wallet is read through the shared hook so a
 *  refresh here and a refresh in the composer answer about the same connection:
 *  what a run spent is this session's, while what is left is the account's. */
export interface CostPanelProps {
  port: AgentPort;
  onError: (e: unknown) => void;
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { money, tokens } from "../../i18n/format";
import type { CurrencyAmount } from "../../state/session_types";
import { DEFAULT_CURRENCY } from "../../state/usage";
import { warmCacheRate } from "../../state/overview";
import type { SessionUsage } from "../../port/session";
import { MASK, since, useHidesAmounts, useWallet } from "../wallet";
import { Grp, Row } from "./kit";

// The kernel states why a total is not final. One sentence for all four sends a
// reader who has simply not sent anything yet to fix a price table that is fine
// — three of these are not about pricing at all.
const REASON: Record<string, string> = {
  no_usage: "usage.no_usage_yet",
  no_price: "usage.no_pricing_reported",
  mixed_original_currencies: "usage.currencies_not_combined",
  display_unavailable: "usage.display_currency_unavailable",
};

// Where the money went, by the producer the ledger recorded. A name this table
// has no wording for is shown as it came rather than guessed at.
const SRC: Record<string, string> = {
  executor: "usage.main_loop",
  subagent: "common.subagents",
  compaction: "session.compaction",
  planner: "session.planning",
  classifier: "chat.classifying",
  title: "memory.titling",
};

// A session that has not been billed yet still draws its zero, as it always has.
// The default labels that zero and nothing else: a billed round always brings
// the code it was charged in.
const NOTHING: CurrencyAmount[] = [{ currency: DEFAULT_CURRENCY, amount: 0 }];

const props = defineProps<CostPanelProps>();

const usage = ref<SessionUsage | null>(null);
const cost = ref<CurrencyAmount[]>([]);
const coverage = ref("none");
const estimated = ref(false);
const incompleteReason = ref("");
const failed = ref("");

const hidden = useHidesAmounts();
// Read when asked, never on a timer: the balance only moves when a turn spends,
// and the kernel's own wallet endpoint is billed per call on some providers.
const [wallet, refreshWallet] = useWallet(() => props.port);

const read = () => {
  props.port
    .status()
    .then((st) => {
      usage.value = st.sessionUsage ?? null;
      const quote = st.sessionCostQuote;
      // One bucket per currency, joined across sources. Two currencies never
      // meet: they are not addends, and there is no rate here to relate them.
      cost.value = (st.sessionUsage?.sources ?? []).reduce<CurrencyAmount[]>((all, s) => {
        const raw = s.cost?.selected?.amount ?? s.cost?.original.amount ?? "";
        const amount = Number(raw);
        const currency = s.cost?.selected?.currency || s.cost?.original.currency || "";
        if (raw.trim() === "" || !Number.isFinite(amount) || !currency) return all;
        const at = all.findIndex((c) => c.currency === currency);
        return at < 0 ? [...all, { currency, amount }] : all.map((c, i) => (i === at ? { ...c, amount: c.amount + amount } : c));
      }, []);
      estimated.value = !!quote?.estimated;
      coverage.value = quote?.coverage ?? (quote?.costComplete ? "complete" : "none");
      incompleteReason.value = quote?.incompleteReason ?? "";
      failed.value = "";
    })
    .catch((e: unknown) => {
      failed.value = e instanceof Error ? e.message : String(e);
      props.onError(e);
    });
};

watch(() => props.port, read, { immediate: true });

const totals = computed(() => (cost.value.length ? cost.value : NOTHING));
const billed = computed(() => cost.value.some((c) => c.amount > 0));
// A source with no tokens contributed nothing; six rows saying zero is how a
// rail spends its whole width on a session that has only talked.
const sources = computed(() => (usage.value?.sources ?? [])
  .filter((s) => s.promptTokens + s.completionTokens > 0)
  .map((s) => ({
    source: s.source,
    tokens: s.promptTokens + s.completionTokens,
    amount: s.cost?.selected?.amount ?? s.cost?.original.amount ?? "",
    currency: s.cost?.selected?.currency || s.cost?.original.currency || DEFAULT_CURRENCY,
  })));

const warm = computed(() => (usage.value ? warmCacheRate(usage.value) : null));
const walletLines = computed(() => (wallet.value.kind === "read" ? (wallet.value.reading.lines ?? []) : []));
const amountOf = (amount: string, currency: string) => {
  const n = Number(amount);
  return hidden.value ? MASK : money(Number.isFinite(n) ? n : 0, currency);
};
</script>

<template>
  <Grp id="cost" :name="t('usage.cost')" :aside="t('common.this_session_2')">
    <div class="mtop">
      <span v-if="hidden" class="amt">{{ MASK }}</span>
      <template v-else>
        <span v-for="m in totals" :key="m.currency" class="amt">{{ money(m.amount, m.currency) }}</span>
      </template>
      <!-- A number billed at a published price and one estimated from a fallback
           table are different claims; the pill says which this is. -->
      <span v-if="billed" class="pill" :data-tone="estimated ? 'warn' : 'ok'">
        {{ estimated ? t("usage.fallback_price") : t("usage.settled") }}
      </span>
      <span v-else class="msub">{{ t(REASON[incompleteReason] ?? "usage.no_pricing_reported") }}</span>
    </div>
    <p v-if="coverage === 'partial'" class="mnote">{{ t("usage.unpriced_calls_not_included") }}</p>
    <p v-if="failed" class="ctxnote" role="alert">{{ failed }}</p>

    <Row v-for="s in sources" :key="s.source" :k="t(SRC[s.source] ?? s.source)" :v="amountOf(s.amount, s.currency)" />
    <Row v-if="usage && usage.requests > 0" :k="t('panels.tokens')"
      :v="hidden ? MASK : tokens(usage.promptTokens + usage.completionTokens)" />
    <Row v-if="warm !== null" :k="t('usage.input')" :v="`${Math.round(warm * 1000) / 10}% ${t('panels.reused')}`" />

    <!-- The wallet answers the same question about a different thing. Absent
         renders as nothing at all: most providers have no wallet endpoint, and a
         dash there reads as a number that failed to load. -->
    <div v-if="wallet.kind !== 'absent'" class="wallet">
      <div class="drow" :data-tone="wallet.kind === 'unread' ? 'warn' : undefined">
        <span class="k">
          <button type="button" class="walletk" :title="t('usage.read_the_balance_again')" @click="refreshWallet">
            {{ t("usage.wallet") }}
          </button>
          <span v-if="wallet.kind === 'read' && !wallet.reading.available" class="pill" data-tone="warn">{{ t("common.suspended") }}</span>
          <span v-if="wallet.kind === 'read' && wallet.reading.stale" class="msub">{{ since(wallet.reading.fetchedAt, Date.now()) }}</span>
        </span>
        <span class="v">{{ wallet.kind === "unread" ? "—" : hidden ? MASK : (walletLines[0]?.total ?? wallet.reading.display) }}</span>
      </div>
      <!-- Two currencies are two units; stacking them is the only thing done
           here, because adding them would mean inventing an exchange rate. -->
      <Row v-for="l in walletLines.slice(1)" :key="l.currency" :k="l.currency"
        :v="`${l.total}${l.granted ? ` · ${t('usage.incl_amount_credit', { amount: l.granted })}` : ''}`" />
      <p v-if="wallet.kind === 'unread'" class="mnote">{{ wallet.why }}</p>
      <p v-else-if="walletLines.length > 1" class="mnote">{{ t("usage.two_currencies_not_summed") }}</p>
    </div>
  </Grp>
</template>
