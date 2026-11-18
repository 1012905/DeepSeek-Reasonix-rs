<script setup lang="ts">
import { onMounted, ref } from "vue";
import { t } from "../../../i18n";
import { host, type GlobalShortcutInfo } from "../../../port/host";
import { acceleratorLabel } from "../../keys";
import Group from "../../Group.vue";

const STATUS = {
  disabled: "settings.global_shortcut_disabled",
  registered: "settings.registered_value",
  invalid: "settings.invalid_accelerator_use_a_modifier",
  unavailable: "settings.the_os_could_not_register",
  unsupported: "settings.global_shortcuts_are_not_supported",
};
const ERRORS = {
  invalid: STATUS.invalid,
  unavailable: STATUS.unavailable,
  save: "settings.could_not_save_the_shortcut",
  quitting: "settings.studio_is_quitting_the_shortcut",
};

const now = ref<GlobalShortcutInfo | null>(null);
const draft = ref("");
const busy = ref(false);
const failed = ref(false);
const recording = ref(false);
const input = ref<HTMLInputElement | null>(null);
// Nothing is asked of a browser tab: it has no window to raise from another
// application, and the shell answers "unsupported" there anyway.
const shell = host().inShell();
// A tab, or a shell that answers "unsupported", has no global shortcut to offer:
// the block still renders, the form inside it does not.
const available = ref(false);

/** The chord a press means, in the accelerator spelling the kernel stores, or
 *  "" while it is not one it can register: a bare key, an AltGr character, a
 *  repeat, or an IME still composing. */
function capturedBinding(event: KeyboardEvent): string {
  if (event.isComposing || event.repeat || event.getModifierState("AltGraph")) return "";
  const modifiers = [
    event.ctrlKey && "Control", event.altKey && "Alt", event.shiftKey && "Shift", event.metaKey && "Super",
  ].filter(Boolean);
  if (!event.ctrlKey && !event.altKey && !event.metaKey) return "";
  const names: Record<string, string> = { " ": "Space", "+": "Plus", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right" };
  const punctuation: Record<string, string> = {
    Comma: ",", Period: ".", Slash: "/", Semicolon: ";", Quote: "'",
    BracketLeft: "[", BracketRight: "]", Backslash: "\\", Backquote: "`", Minus: "-", Equal: "=",
  };
  const position = /^(Key[A-Z]|Digit[0-9])$/.test(event.code) ? event.code.replace(/^(Key|Digit)/, "") : punctuation[event.code];
  const key = names[event.key] ?? (/^[a-z0-9]$/i.test(event.key) ? event.key.toUpperCase() : position ?? event.key);
  if (!/^(Space|Plus|Up|Down|Left|Right|Enter|Escape|Backspace|Delete|Insert|Home|End|PageUp|PageDown|F([1-9]|1[0-9]|2[0-4])|[A-Z0-9,./;'[\]\\`=-])$/.test(key)) return "";
  return [...modifiers, key].join("+");
}

onMounted(() => {
  if (!shell) return;
  host()
    .globalShortcut()
    .then((value) => {
      now.value = value;
      draft.value = value.binding;
      available.value = value.status !== "unsupported";
    })
    .catch(() => { failed.value = true; });
});

async function apply(binding: string) {
  busy.value = true;
  failed.value = false;
  try {
    const value = await host().setGlobalShortcut(binding);
    now.value = value;
    // A binding the shell refused stays in the field: it is what the person
    // asked for, and the status below says why it did not take.
    if (!value.error) draft.value = value.binding;
  } catch {
    failed.value = true;
  } finally {
    busy.value = false;
  }
}

const submit = (e: Event) => {
  e.preventDefault();
  void apply(draft.value);
};

const startRecording = () => {
  recording.value = true;
  input.value?.focus();
};

const keyed = (event: KeyboardEvent) => {
  if (!recording.value || event.key === "Tab") return;
  event.preventDefault();
  event.stopPropagation();
  if (event.key === "Escape" && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) {
    recording.value = false;
    return;
  }
  const binding = capturedBinding(event);
  if (binding) {
    draft.value = binding;
    recording.value = false;
  }
};
</script>

<template>
  <Group id="global-shortcut" :title="t('settings.global_show_hide_shortcut')" :hint="t('settings.off_by_default_while_studio')">
    <!-- The block always exists, so the anchor a search result lands on is never
         missing; the form inside is what a shell without the feature does not get. -->
    <p v-if="!available" class="note" role="status">{{ t("settings.global_shortcuts_are_not_supported") }}</p>

    <form v-else data-action="window.global-shortcut" data-value="apply" @submit="submit">
      <div class="srow">
        <div class="srow-meta">
          <span class="srow-name">{{ t("settings.shortcut_binding") }}</span>
          <span v-if="now" class="srow-desc" role="status">{{ t(STATUS[now.status], { value: acceleratorLabel(now.binding) }) }}</span>
        </div>
        <div class="srow-ctl">
          <input
            ref="input"
            class="keybind"
            :data-recording="recording ? '' : undefined"
            data-action-change="window.global-shortcut"
            data-action-keydown="window.global-shortcut"
            data-value="binding"
            :aria-label="t('settings.shortcut_binding')"
            maxlength="128"
            :placeholder="recording ? t('settings.press_a_modifier_and_key') : acceleratorLabel('CommandOrControl+Shift+Space')"
            :value="draft"
            :readonly="recording"
            :disabled="!now || busy"
            @input="draft = ($event.target as HTMLInputElement).value"
            @blur="recording = false"
            @keydown="keyed"
          />
        </div>
      </div>
      <div class="srow keyacts">
        <div class="srow-meta">
          <kbd v-if="draft" class="keycap">{{ acceleratorLabel(draft) }}</kbd>
          <span class="srow-desc">{{ t("settings.click_record_and_press_a") }}</span>
        </div>
        <div class="srow-ctl">
          <button type="button" class="spill" data-action="window.global-shortcut" data-value="record" :disabled="!now || busy" @click="startRecording">
            {{ t("settings.record_shortcut") }}
          </button>
          <button type="submit" class="spill" data-action="window.global-shortcut" data-value="apply" :disabled="!now || busy">
            {{ t("settings.apply_shortcut") }}
          </button>
          <button
            type="button"
            class="spill"
            data-action="window.global-shortcut"
            data-value="disable"
            :disabled="!now || busy || (!now.binding && !draft)"
            @click="apply('')"
          >
            {{ t("settings.clear_shortcut") }}
          </button>
        </div>
      </div>
    </form>
    <p v-if="now?.error" class="note" role="alert">{{ t(ERRORS[now.error]) }}</p>
    <p v-if="failed" class="note" role="alert">{{ t("settings.could_not_read_or_change") }}</p>
  </Group>
</template>
