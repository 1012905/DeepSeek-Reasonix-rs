<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import type { AgentPort, ModelEntry, SessionStatus } from "../../../port/port";
import { t } from "../../../i18n";
import Group from "../../Group.vue";
import Providers, { type Account } from "../../Providers.vue";
import CoreProviderHealth from "./core-providerhealth.vue";
import { protocols, rememberProtocol } from "./core-protocol";
import { activeKind, groupVendors } from "../../Models";
import { KIND_LABEL } from "../../vendors";
import { reason } from "../../../i18n/kernel";

// Every model source this machine holds, grouped by account rather than by
// config row: one endpoint answering two protocols is one service to the person
// paying for it, so the protocol is a switch on that account.
const props = defineProps<{
  port: AgentPort;
  status: SessionStatus | null;
  at?: string;
  onChanged: () => void;
}>();
const p = computed(() => props);

// A refused switch is not a failed save, and it belongs beside the list it
// concerns rather than on the page's one note.
const failed = ref("");

// The accounts as the services list groups them. Read from the model catalogue
// the model page reads too, so "which door is this account showing" answers
// about the account the list drew and not about a second account of its own.
const models = ref<ModelEntry[]>([]);

const reloadModels = () => {
  p.value.port.models("all").then((list) => { models.value = list; }).catch(() => { models.value = []; });
};

const accounts = computed(() => new Map(groupVendors(models.value).map((v) => [v.key, v])));

const kindFor = (account: Account): string => {
  const vendor = accounts.value.get(account.key);
  return vendor ? protocols.value[account.key] ?? activeKind(vendor, p.value.status?.modelRef) : account.kinds[0] ?? "";
};

// `declare` opens one source's reasoning fields, which is what the composer's
// effort menu asked for with its "providers:effort-declare" route. A model ref's
// first segment is the source's name.
const declare = computed(() => (p.value.at ?? "").split(":")[1] === "effort-declare"
  ? p.value.status?.modelRef?.split("/")[0]
  : undefined);

// One act with two meanings: on an account nothing runs on it only decides
// whose model list is shown; on the running one it is a real switch through
// setModel, and a door without the same model stays put rather than showing a
// door the session is not on until the next read derives it back.
const onProtocol = (account: Account, kind: string) => {
  failed.value = "";
  const vendor = accounts.value.get(account.key);
  const current = p.value.status?.modelRef;
  const running = vendor && current ? Object.values(vendor.byKind).flat().some((m) => m.ref === current) : false;
  if (!vendor || !running) {
    rememberProtocol(account.key, kind);
    return;
  }
  const here = models.value.find((m) => m.ref === current);
  const same = (vendor.byKind[kind] ?? []).find((m) => m.model === here?.model);
  if (!same) {
    failed.value = t("settings.door_does_not_offer_model", {
      door: t(KIND_LABEL[kind] ?? kind),
      model: here?.model ?? current ?? "",
    });
    return;
  }
  rememberProtocol(account.key, kind);
  if (same.ref === current) return;
  p.value.port
    .setModel(same.ref)
    .then(() => p.value.onChanged())
    .catch((e) => { failed.value = reason(e); });
};

const onChanged = () => {
  reloadModels();
  p.value.onChanged();
};

const onFailed = (why: string) => { failed.value = why; };

onMounted(reloadModels);
</script>

<template>
  <Group id="providers" :title="t('common.model_services')">
    <p v-if="failed" class="find" data-lvl="warn" role="alert">
      <span class="t">{{ t("common.that_did_not_go_through") }}</span>
      <span class="why">{{ failed }}</span>
    </p>
    <Providers
      :port="p.port"
      :on-changed="onChanged"
      :on-failed="onFailed"
      :protocol="protocols"
      :on-protocol="onProtocol"
      :active-kind-for="kindFor"
      :declare="declare"
    />
  </Group>

  <!-- Opening the model list can also ask each endpoint what it is; that is a
       switch beside the list it concerns, not a page of its own. -->
  <CoreProviderHealth :port="p.port" :on-changed="onChanged" :on-failed="onFailed" />
</template>
