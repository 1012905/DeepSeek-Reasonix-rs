<script lang="ts">
import type { ShellChoice } from "../../port/terminal";

export interface TerminalHeadProps {
  /** The shell's short tag (its `kind`), or the terminal's own name when unknown. */
  shell: string;
  dir: string;
  shells: ShellChoice[];
  onLoadShells: () => void;
  /** Opens another terminal in that shell; this one is left running. */
  onShell: (kind: string) => void;
  onClear: () => void;
  onReconnect: () => void;
  onCopyAll: () => void;
  onSize: (change: "larger" | "smaller" | "reset") => void;
}
</script>

<script setup lang="ts">
import { inject, ref } from "vue";
import { t } from "../../i18n";
import { TerminalHostContext } from "../terminal_host";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<TerminalHeadProps>();
const machine = inject(TerminalHostContext, "");

const open = ref<"shell" | "more" | null>(null);

// The shell is named here and picked here; choosing another one opens a
// terminal in it beside this one, so nothing running is cut off.
const toggle = (which: "shell" | "more") => {
  if (open.value !== which) props.onLoadShells();
  open.value = open.value === which ? null : which;
};
const shut = () => { open.value = null; };

// A press outside closes the dropdown; the layer.dismiss vocabulary is what a
// press on the terminal's own screen carries, so both say the same thing.
const blur = (e: FocusEvent) => {
  if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) shut();
};
const key = (e: KeyboardEvent) => {
  if (e.key === "Escape" && open.value) {
    e.stopPropagation();
    shut();
  }
};
const pickShell = (kind: string) => {
  shut();
  if (kind !== props.shell) props.onShell(kind);
};
</script>

<template>
  <div class="term-head" data-action-keydown="layer.dismiss" @blur="blur" @keydown="key">
    <div class="term-drop">
      <button
        type="button"
        class="term-shell"
        data-action="terminal.shell-menu"
        aria-haspopup="menu"
        :aria-expanded="open === 'shell'"
        :aria-label="t('terminal.shell_picker')"
        @click="toggle('shell')"
      >
        {{ shell }}<StudioIcon name="down" />
      </button>
      <div v-if="open === 'shell'" class="term-menu" role="menu" :aria-label="t('terminal.pick_a_shell')">
        <button
          v-for="choice in shells"
          :key="choice.kind"
          type="button"
          role="menuitem"
          data-action="terminal.shell"
          :data-target="choice.kind"
          :data-current="choice.kind === shell ? '' : undefined"
          @click="pickShell(choice.kind)"
        >
          <span>{{ choice.name }}</span><kbd>{{ choice.kind }}</kbd>
          <StudioIcon v-if="choice.kind === shell" name="check" />
        </button>
        <p>{{ t("terminal.default_shell_note") }}</p>
      </div>
    </div>
    <span v-if="machine" class="term-host"><StudioIcon name="server" />{{ t("terminal.on_host", { host: machine }) }}</span>
    <span class="term-dir" :title="dir">{{ dir }}</span>
    <span class="term-grow" />
    <button type="button" class="term-tool" data-action="terminal.clear" :aria-label="t('terminal.clear')" :title="t('terminal.clear')" @click="onClear">
      <StudioIcon name="eraser" />
    </button>
    <button type="button" class="term-tool" data-action="terminal.reconnect" :aria-label="t('terminal.reconnect')" :title="t('terminal.reconnect')" @click="onReconnect">
      <StudioIcon name="refresh" />
    </button>
    <div class="term-drop">
      <button
        type="button"
        class="term-tool"
        data-action="terminal.more"
        aria-haspopup="menu"
        :aria-expanded="open === 'more'"
        :aria-label="t('terminal.more')"
        :title="t('terminal.more')"
        @click="toggle('more')"
      >
        <StudioIcon name="more" />
      </button>
      <div v-if="open === 'more'" class="term-menu" role="menu" :aria-label="t('terminal.more')">
        <button type="button" role="menuitem" data-action="terminal.copy-all" @click="shut(); onCopyAll()">
          <span>{{ t("terminal.copy_all") }}</span>
        </button>
        <button type="button" role="menuitem" data-action="terminal.size" data-value="larger" @click="onSize('larger')">
          <span>{{ t("terminal.font_larger") }}</span>
        </button>
        <button type="button" role="menuitem" data-action="terminal.size" data-value="smaller" @click="onSize('smaller')">
          <span>{{ t("terminal.font_smaller") }}</span>
        </button>
        <button type="button" role="menuitem" data-action="terminal.size" data-value="reset" @click="shut(); onSize('reset')">
          <span>{{ t("terminal.font_reset") }}</span>
        </button>
      </div>
    </div>
  </div>
</template>
