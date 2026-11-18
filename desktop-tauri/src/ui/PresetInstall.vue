<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import { HttpError } from "../port/http_error";
import type { ProviderPreset } from "../port/port";
import type { Port } from "./Providers.vue";
import { KIND_LABEL, hostOf, sourceNameUsable } from "./vendors";
import StudioIcon from "./StudioIcon.vue";

const NAME_CODES = new Set(["provider.name_required", "provider.name_invalid", "provider.name_taken"]);

const props = defineProps<{
  port: Port;
  preset: ProviderPreset;
  onDone: () => void;
  onCancel: () => void;
  onBack: () => void;
}>();

const name = ref("");
const apiKey = ref("");
const busy = ref(false);
const err = ref("");
const nameErr = ref("");
const keyBox = ref<HTMLInputElement | null>(null);
const nameBad = computed(() => name.value.trim() !== "" && !sourceNameUsable(name.value));

onMounted(() => keyBox.value?.focus());

const install = async () => {
  busy.value = true;
  err.value = "";
  nameErr.value = "";
  try {
    await props.port.installProviderPreset({ presetId: props.preset.id, name: name.value.trim(), apiKey: apiKey.value.trim() });
    props.onDone();
  } catch (e) {
    if (e instanceof HttpError && e.reason?.code && NAME_CODES.has(e.reason.code)) nameErr.value = reason(e);
    else err.value = reason(e);
  } finally {
    busy.value = false;
  }
};
</script>

<template>
  <fieldset class="addp addp-preset-form" :disabled="busy">
    <div class="addp-head">
      <div>
        <span class="step">{{ t(KIND_LABEL[preset.kind] ?? preset.kind) }} · {{ hostOf(preset.baseUrl) }}</span>
        <strong>{{ preset.label }}</strong>
        <p>{{ t("persona.the_kernel_creates_the_model") }}</p>
      </div>
      <button class="addp-close" data-action="provider.preset-cancel" @click="onCancel" :aria-label="t('common.cancel')"><StudioIcon name="close" /></button>
    </div>
    <div class="fields">
      <label class="grow">
        <span>{{ t("common.source_name") }}</span>
        <input :aria-label="t('common.source_name')" data-action="provider.draft" data-value="preset-name" :value="name" :placeholder="t('persona.the_presets_own_name')" spellcheck="false" :aria-invalid="nameBad || nameErr !== '' || undefined" @input="name = ($event.target as HTMLInputElement).value; nameErr = ''" />
        <i class="tip" :data-bad="nameBad || nameErr !== '' || undefined">{{ nameErr !== "" ? nameErr : t("persona.leave_empty_to_use_the") }}</i>
      </label>
      <label class="grow full">
        <span>API Key</span>
        <input ref="keyBox" type="password" data-action="provider.draft" data-value="preset-credential" aria-label="API Key" :value="apiKey" spellcheck="false" @input="apiKey = ($event.target as HTMLInputElement).value" />
      </label>
      <p class="addp-privacy">{{ t("common.the_api_key_is_stored") }}</p>
    </div>
    <div v-if="err !== ''" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("common.could_not_save") }}</span>
      <span class="why">{{ err }}</span>
    </div>
    <div class="acts addp-footer">
      <button class="act" data-action="provider.add" data-primary @click="install" :disabled="busy || nameBad">{{ t(busy ? "common.saving" : "common.add_source") }}</button>
      <button class="act" data-action="provider.preset-back" @click="onBack">{{ t("common.new_code") }}</button>
      <button class="act" data-action="provider.preset-cancel" @click="onCancel">{{ t("common.cancel") }}</button>
    </div>
  </fieldset>
</template>
