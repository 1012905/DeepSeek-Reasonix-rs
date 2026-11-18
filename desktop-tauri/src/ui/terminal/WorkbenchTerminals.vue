<script lang="ts">
import type { AgentPort } from "../../port/port";
import type { ShellChoice, TerminalInfo } from "../../port/terminal";

export interface WorkbenchTerminalsProps {
  port: AgentPort;
  /** The terminal tabs as the panel holds them, from useTerminalTabs. */
  terminals: TerminalInfo[];
  /** The tab in front, "" when another surface is. */
  active: string;
  visible: boolean;
  scheme: "light" | "dark";
  shells: ShellChoice[];
  onLoadShells: () => void;
  /** Opens another terminal in that shell, beside this one. */
  onShell: (kind: string) => void;
  onQuote?: (text: string, source: string) => void;
  onOpenPage: (url: string) => void;
  onCommand: (id: string, line: string) => void;
  onRestart: (id: string) => Promise<void>;
  onClose: (id: string) => void;
  onError: (e: unknown) => void;
  /** Asks before a terminal with a command running is closed; the panel supplies it. */
  asking?: { anchor: HTMLElement; run: () => void } | null;
  onCancelAsk: () => void;
  onConfirmAsk: () => void;
}
</script>

<script setup lang="ts">
import { defineAsyncComponent, ref, watch } from "vue";
import { t } from "../../i18n";
import TerminalCloseAsk from "./TerminalCloseAsk.vue";

const props = defineProps<WorkbenchTerminalsProps>();

// The screens load with the first terminal, not with the workbench: the
// emulator is most of what a terminal costs.
const TerminalLayer = defineAsyncComponent(() => import("./TerminalLayer.vue"));

// A layer that has been asked for stays mounted: disposing the emulator on a
// switch back to another tab would throw away the screen the reader left.
const wanted = ref(props.terminals.length > 0);
watch(() => props.terminals.length, (n) => { if (n > 0) wanted.value = true; }, { immediate: true });

// A quoted selection arrives as text; the tab it came from is named beside it,
// which is the only source this view has.
const quote = props.onQuote ? (text: string) => props.onQuote?.(text, t("terminal.name")) : undefined;
</script>

<template>
  <TerminalLayer
    v-if="wanted"
    :port="port"
    :terminals="terminals"
    :active="active"
    :visible="visible"
    :scheme="scheme"
    :shells="shells"
    :on-load-shells="onLoadShells"
    :on-shell="onShell"
    :on-quote="quote"
    :on-open-page="onOpenPage"
    :on-command="onCommand"
    :on-restart="onRestart"
    :on-close="onClose"
    :on-error="onError"
  />
  <TerminalCloseAsk
    v-if="asking"
    :anchor="asking.anchor"
    :on-confirm="onConfirmAsk"
    :on-cancel="onCancelAsk"
  />
</template>
