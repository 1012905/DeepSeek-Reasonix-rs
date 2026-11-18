<script setup lang="ts">
import { computed } from "vue";
import type { McpEntry } from "../../port/port";
import { t } from "../../i18n";

// Only the servers that need a decision get a row. A healthy MCP fleet is the
// least interesting thing on this rail: it is all green and never changes, and
// where its tools came from is already stamped on the tool card that used them.
// The endpoint's own error text stays out: it is written for a terminal, and
// nothing here can act on it. Settings prints it beside the switch and the
// retry, so the row carries the way there instead — named, not hover-revealed,
// because a row that can be pressed and does not say so reads as a dead status.

// The rail reports, it does not enumerate — the same cap the delegate and file
// panels take, at the count two-line rows stop fitting beside everything else.
const SHOWN = 3;

const props = defineProps<{ servers: McpEntry[]; onOpen: () => void }>();

// A server the repository declared and nobody has answered for needs a
// decision as much as a broken one does — and unlike a broken one, it has
// never run, so nothing else on screen would show it is missing.
const waiting = computed(() => props.servers.filter((s) => s.state === "failed" || s.state === "pending"));
const pending = computed(() => waiting.value.filter((s) => s.state === "pending").length);
const shown = computed(() => waiting.value.slice(0, SHOWN));
const count = computed(() => pending.value === waiting.value.length
  ? t("ext.n_awaiting_approval", { n: pending.value })
  : pending.value === 0
    ? t("ext.n_unreachable", { n: waiting.value.length })
    : t("ext.n_unreachable_p_awaiting_approval", { n: waiting.value.length - pending.value, p: pending.value }));
</script>

<template>
  <div v-if="waiting.length > 0" class="block" data-b="mcp">
    <div class="lbl">
      {{ t("ext.external_services") }}<span class="c">{{ count }}</span>
    </div>
    <div class="srvs">
      <button v-for="s in shown" :key="s.name" class="srvrow" data-action="chrome.settings" @click="onOpen"
        :title="s.state === 'pending' ? t('ext.this_server_was_declared_by') : t('ext.fix_it_in_the_mcp')">
        <span class="hd">
          <i class="pip" :data-s="s.state" />
          <span class="nm">{{ s.name }}</span>
        </span>
        <span class="fix">{{ s.state === "pending" ? t("ext.approve") : t("ext.fix") }}</span>
      </button>
      <span v-if="waiting.length > shown.length" class="more">{{ t("ext.n_more_all_in_settings", { n: waiting.length - shown.length }) }}</span>
    </div>
  </div>
</template>