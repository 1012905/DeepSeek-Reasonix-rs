<script lang="ts">
import type { BrowserLoadFailure } from "../../port/host";

export interface BrowserFailureProps {
  failure: BrowserLoadFailure;
  retry: () => void;
  onExternal: () => void;
  trust: () => void;
}
</script>

<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import StudioIcon from "../StudioIcon.vue";

const props = defineProps<BrowserFailureProps>();

// Chromium's net error names are machine facts. What a person needs is what to
// do next, and only the family of the name says that. Electron draws no error
// page, so without this a failed load is a blank view with no reason at all.
function explain(name: string): string {
  if (name.startsWith("ERR_CERT_") || name === "ERR_SSL_PINNED_KEY_NOT_IN_CERT_CHAIN") {
    return t("workbench.the_sites_certificate_is_not");
  }
  if (name.startsWith("ERR_SSL_") || name === "ERR_BAD_SSL_CLIENT_AUTH_CERT") {
    return t("workbench.a_secure_connection_could_not");
  }
  if (name.startsWith("ERR_PROXY_") || name.startsWith("ERR_TUNNEL_")) {
    return t("workbench.the_system_proxy_could_not");
  }
  switch (name) {
    case "ERR_NAME_NOT_RESOLVED":
    case "ERR_NAME_RESOLUTION_FAILED":
      return t("workbench.this_address_could_not_be");
    case "ERR_FILE_NOT_FOUND":
      return t("workbench.this_file_could_not_be");
    case "ERR_ACCESS_DENIED":
      return t("workbench.you_do_not_have_permission");
    case "ERR_CONNECTION_REFUSED":
      return t("workbench.the_connection_was_refused_check");
    case "ERR_CONNECTION_TIMED_OUT":
    case "ERR_TIMED_OUT":
    case "ERR_ADDRESS_UNREACHABLE":
      return t("workbench.the_connection_timed_out_or");
    case "ERR_INVALID_AUTH_CREDENTIALS":
      return t("workbench.sign_in_failed_check_the");
    default:
      return t("workbench.the_page_did_not_load");
  }
}

// The system opener is never handed a file: link — the way out it offers would
// do nothing for one.
function isFile(raw: string): boolean {
  try {
    return new URL(raw).protocol === "file:";
  } catch {
    return false;
  }
}

const cert = computed(() => props.failure.certificate);
const external = computed(() => !isFile(props.failure.url));
const detail = computed(() => `${props.failure.reason || props.failure.code} · ${props.failure.url}`);
const expiry = computed(() => (cert.value?.expiry ? new Date(cert.value.expiry * 1000).toLocaleDateString() : "—"));
</script>

<template>
  <div class="bfail" role="alert">
    <StudioIcon name="globe" />
    <b>{{ t("workbench.this_page_could_not_be") }}</b>
    <p>{{ explain(failure.reason) }}</p>
    <code>{{ detail }}</code>
    <!-- What the person is about to trust, shown before they trust it: the
         issuer is how a company certificate is told from an impostor's. -->
    <dl v-if="cert" class="bfail-cert">
      <dt>{{ t("workbench.issued_to") }}</dt>
      <dd>{{ cert.subject }}</dd>
      <dt>{{ t("workbench.issued_by") }}</dt>
      <dd>{{ cert.issuer }}</dd>
      <dt>{{ t("workbench.valid_until") }}</dt>
      <dd>{{ expiry }}</dd>
      <dt>{{ t("common.fingerprint") }}</dt>
      <dd class="fp">{{ cert.fingerprint }}</dd>
    </dl>
    <div class="bfail-acts">
      <button type="button" class="btn sm" data-action="browser.control" data-value="retry" @click="props.retry()">
        {{ t("common.retry") }}
      </button>
      <button v-if="external" type="button" class="btn sm" data-action="browser.external" @click="props.onExternal()">
        {{ t("workbench.open_in_external_browser") }}
      </button>
      <button v-if="cert" type="button" class="btn sm" data-action="browser.trust-certificate" @click="props.trust()">
        {{ t("workbench.proceed_anyway_this_run_only") }}
      </button>
    </div>
    <p v-if="cert" class="bfail-note">{{ t("workbench.continue_only_if_you_know") }}</p>
  </div>
</template>
