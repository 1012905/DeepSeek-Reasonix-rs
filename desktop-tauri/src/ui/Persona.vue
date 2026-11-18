<script setup lang="ts">
import { computed, ref, useId, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { AgentPort } from "../port/port";
import type { PersonaDraft, PersonaPreset, PersonaSettings } from "../port/boundary";
import Group from "./Group.vue";
import Switch from "./Switch.vue";

// The kernel refuses a prefix over 32 KiB and a total over 64 KiB; the box
// bounds the field it can see instead of letting a huge paste grow silently.
const LIMIT = 32768;

const BLANK: PersonaDraft = { prefix: "", suffix: "", include_runtime_context: true };

const draftOf = (s: PersonaSettings): PersonaDraft => ({
  prefix: s.prefix,
  suffix: s.suffix,
  include_runtime_context: s.include_runtime_context,
});

const sameDraft = (a: PersonaDraft, b: PersonaDraft) =>
  a.prefix === b.prefix && a.suffix === b.suffix &&
  a.include_runtime_context === b.include_runtime_context;

const isPreset = (p: PersonaPreset, d: PersonaDraft) =>
  sameDraft({ prefix: p.prefix, suffix: p.suffix, include_runtime_context: p.include_runtime_context }, d);

// Presets are served with their prompt text and a stable id; the words the
// page shows about them are this interface's, so they go through t() here.
function presetName(id: string): string {
  switch (id) {
    case "engineer": return t("persona.rigorous_engineer");
    case "concise": return t("persona.concise");
    case "mentor": return t("persona.mentor");
    case "critic": return t("persona.critic");
    default: return id;
  }
}

function presetAbout(id: string): string {
  switch (id) {
    case "engineer": return t("persona.verify_before_concluding_make_the");
    case "concise": return t("persona.answer_briefly_and_directly_skip");
    case "mentor": return t("persona.explain_trade_offs_and_reasoning");
    case "critic": return t("persona.review_adversarially_name_the_failure");
    default: return "";
  }
}

const props = defineProps<{ port: AgentPort; onDirtyChange?: (dirty: boolean) => void; onBusyChange?: (busy: boolean) => void; onSaved?: () => void }>();

const state = ref<PersonaSettings | null>(null);
const draft = ref<PersonaDraft>({ ...BLANK });
const busy = ref(false);
const error = ref("");
const saved = ref(false);
const prefixId = useId();
const suffixId = useId();

function adopt(s: PersonaSettings) {
  state.value = s;
  draft.value = draftOf(s);
  saved.value = false;
}

watch(() => props.port, (port) => {
  port.persona().then(adopt).catch(() => { state.value = null; });
}, { immediate: true });

function change(patch: Partial<PersonaDraft>) {
  draft.value = { ...draft.value, ...patch };
  saved.value = false;
  error.value = "";
}

async function save() {
  if (!state.value) return;
  busy.value = true;
  error.value = "";
  try {
    adopt(await props.port.savePersonaSettings(draft.value));
    saved.value = true;
    props.onSaved?.();
  } catch (e) {
    error.value = reason(e);
  } finally {
    busy.value = false;
  }
}

const preset = computed(() => state.value?.presets.find((p) => isPreset(p, draft.value)) ?? null);
const blank = computed(() => !draft.value.prefix && !draft.value.suffix);
// A draft the presets do not describe is the user's own; saying so is the
// answer to "which of these am I looking at".
const now = computed(() => !state.value ? undefined : preset.value ? presetName(preset.value.id) : blank.value ? undefined : t("common.custom"));
const changed = computed(() => !!state.value && !sameDraft(draft.value, draftOf(state.value)));
watch([() => props.onDirtyChange, changed], ([cb, c]) => { cb?.(c); }, { immediate: true });
watch([() => props.onBusyChange, busy], ([cb, b]) => { cb?.(b); }, { immediate: true });
</script>

<template>
  <Group
    id="persona"
    :title="t('persona.system_persona')"
    :now="now"
    :hint="t('persona.the_system_persona_shapes_the')"
  >
    <div v-if="!state" class="empty">{{ t("persona.couldnt_read_the_persona_settings") }}</div>
    <div v-else class="box">
      <template v-if="state.presets.length > 0">
        <div class="seg" data-text="true" role="group" :aria-label="t('persona.presets')">
          <button
            v-for="p in state.presets"
            :key="p.id"
            type="button"
            data-action="persona.preset"
            :data-value="p.id"
            :aria-pressed="preset?.id === p.id"
            :disabled="busy"
            @click="change({
              prefix: p.prefix,
              suffix: p.suffix,
              include_runtime_context: p.include_runtime_context,
            })"
          >
            {{ presetName(p.id) }}
          </button>
        </div>
        <p class="persona-name">
          {{ preset
            ? presetAbout(preset.id)
            : blank
              ? t("persona.no_preset_applied_fill_in")
              : t("persona.the_preset_text_has_been") }}
        </p>
      </template>

      <div class="pers-field">
        <label class="lb" :for="prefixId">{{ t("persona.start_of_the_system_prompt") }}</label>
        <span class="ds">{{ t("persona.leads_the_system_prompt_ahead") }}</span>
        <textarea
          :id="prefixId"
          dir="auto"
          :rows="5"
          :maxlength="LIMIT"
          :spellcheck="false"
          :aria-label="t('persona.start_of_the_system_prompt')"
          data-action-change="persona.prefix"
          :value="draft.prefix"
          :disabled="busy"
          @input="change({ prefix: ($event.target as HTMLTextAreaElement).value })"
        />
      </div>

      <div class="pers-field">
        <label class="lb" :for="suffixId">{{ t("persona.additional_instructions") }}</label>
        <span class="ds" :id="`${suffixId}-help`">{{ t("persona.placed_after_the_built_in") }}</span>
        <textarea
          :id="suffixId"
          dir="auto"
          :rows="5"
          :maxlength="LIMIT"
          :spellcheck="false"
          :aria-label="t('persona.additional_instructions')"
          :aria-describedby="`${suffixId}-help`"
          data-action-change="persona.suffix"
          :value="draft.suffix"
          :disabled="busy"
          @input="change({ suffix: ($event.target as HTMLTextAreaElement).value })"
        />
      </div>

      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("persona.do_not_inject_optional_runtime") }}</span>
          <span class="ds">{{ t("persona.when_on_the_probed_environment") }}</span>
        </span>
        <Switch
          data-action="persona.runtime-context"
          :on="!draft.include_runtime_context"
          :busy="busy"
          :label="t('persona.do_not_inject_optional_runtime')"
          :on-click="() => change({ include_runtime_context: !draft.include_runtime_context })"
        />
      </div>

      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("common.save") }}</span>
          <span class="ds">{{ t("persona.writes_the_user_config_applies") }}</span>
        </span>
        <button class="act" data-action="persona.save" :disabled="busy || !changed" @click="save()">
          {{ t(busy ? "common.saving" : "common.save") }}
        </button>
      </div>

      <div v-if="saved" class="find" data-lvl="ok" role="status">
        <span class="t">{{ t("persona.saved_applies_at_the_next") }}</span>
        <span class="why"><code>{{ state.path }}</code></span>
      </div>
      <div v-if="error" class="find" data-lvl="err" role="alert">
        <span class="t">{{ t("common.that_did_not_go_through") }}</span>
        <span class="why">{{ error }}</span>
      </div>
    </div>
  </Group>
</template>