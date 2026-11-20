<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { t } from "../i18n";
import { reason } from "../i18n/kernel";
import type { ProviderPreset } from "../port/port";
import { KIND_LABEL, hostOf } from "./vendors";
import { VendorMark, hasVendorMark, vendorName } from "./vendormarks.vue";

const COMMON_VENDORS = 8;

const props = defineProps<{
  load: () => Promise<ProviderPreset[]>;
  onPick: (preset: ProviderPreset) => void;
  disabled: boolean;
}>();

const presets = ref<ProviderPreset[] | null>(null);
const failed = ref("");
const open = ref(false);
const query = ref("");
const vendor = ref("");

const read = () => {
  failed.value = "";
  Promise.resolve().then(() => props.load()).then((items) => {
    presets.value = items;
  }).catch((e) => {
    presets.value = null;
    failed.value = reason(e);
  });
};
onMounted(read);
watch(() => props.load, read);

const vendors = computed(() => {
  const seen = [...new Set((presets.value ?? []).map((p) => p.vendor).filter((v): v is string => !!v))];
  return [...seen.slice(0, COMMON_VENDORS), ...seen.slice(COMMON_VENDORS).sort((a, b) => vendorName(a).localeCompare(vendorName(b)))];
});

const groups = computed(() => {
  const needle = query.value.trim().toLowerCase();
  const shown = (presets.value ?? []).filter((p) => (!vendor.value || p.vendor === vendor.value) && (!needle || `${p.label} ${p.id} ${hostOf(p.baseUrl)}`.toLowerCase().includes(needle)));
  const byHost = new Map<string, ProviderPreset[]>();
  for (const p of shown) byHost.set(hostOf(p.baseUrl), [...(byHost.get(hostOf(p.baseUrl)) ?? []), p]);
  return [...byHost];
});
</script>

<template>
  <p v-if="failed" class="addp-presets-note" role="alert">
    {{ t("persona.the_list_of_common_services", { detail: failed }) }}
    <button type="button" class="act" data-action="provider.preset-retry" @click="read">{{ t("common.retry") }}</button>
  </p>
  <template v-else-if="presets !== null">
    <p v-if="presets.length === 0" class="addp-presets-note">{{ t("persona.the_kernel_offers_no_service") }}</p>
    <div v-else class="addp-presets">
      <button type="button" class="addp-presets-toggle" data-action="provider.preset-toggle" :aria-expanded="open" @click="open = !open">
        <strong>{{ t("persona.start_from_a_common_service") }}</strong>
        <span>{{ presets.length }}</span>
      </button>
      <template v-if="open">
        <div class="fields">
          <label class="grow full">
            <input type="search" data-action="provider.draft" data-value="preset-query" :aria-label="t('persona.search_services')" :placeholder="t('persona.search_services')" :value="query" spellcheck="false" @input="query = ($event.target as HTMLInputElement).value" />
          </label>
        </div>
        <div v-if="vendors.length > 0" class="addp-vendors" role="group" :aria-label="t('persona.filter_by_vendor')">
          <button type="button" data-action="provider.preset-vendor" data-target="" :aria-pressed="vendor === ''" @click="vendor = ''">{{ t("common.all") }}</button>
          <button v-for="v in vendors" :key="v" type="button" data-action="provider.preset-vendor" :data-target="v" :aria-pressed="vendor === v" @click="vendor = v">
            <VendorMark v-if="hasVendorMark(v)" :slug="v" :size="14" />
            {{ vendorName(v) }}
          </button>
        </div>
        <p v-if="groups.length === 0" class="addp-presets-note">{{ t("persona.no_matching_service") }}</p>
        <section v-for="[host, items] in groups" :key="host" class="addp-presets-group" :aria-label="host">
          <h4>{{ host }}</h4>
          <div class="addp-presets-grid">
            <button v-for="preset in items" :key="preset.id" type="button" class="addp-preset" data-action="provider.preset" :data-target="preset.id" :disabled="disabled" @click="onPick(preset)">
              <span><VendorMark v-if="hasVendorMark(preset.vendor)" :slug="preset.vendor" />{{ preset.label }}</span>
              <small>{{ t(KIND_LABEL[preset.kind] ?? preset.kind) }}</small>
            </button>
          </div>
        </section>
      </template>
    </div>
  </template>
</template>
