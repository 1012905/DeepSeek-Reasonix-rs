<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, McpDraft, McpEntry, McpInstallResult, McpInstallScope, McpLoad, McpCatalog } from "../../../port/port";
import Switch from "../../Switch.vue";
import McpRail from "../../panels/Mcp.vue";

// standby is the ordinary state of a working server, not a lesser kind of
// connected: its tools are in the catalog and the process starts on the first
// call. Calling that "not connected" beside a Retry button reads as broken.
const STATE: Record<string, string> = {
  ready: "remote.connected",
  connecting: "remote.connecting_2",
  failed: "models.unreachable",
  disabled: "ext.disabled",
  standby: "ext.standby_starts_on_first_call",
  idle: "common.not_connected",
  pending: "ext.awaiting_approval_declared_by_the",
};

// How a withheld tool differs from what was approved, keyed by the kernel's own
// class. A class this build has no line for is shown as the kernel wrote it.
const HELD: Record<string, string> = {
  added: "ext.added",
  changed: "ext.definition_changed",
  unverifiable: "ext.cannot_be_checked",
};

const SCOPES: readonly (readonly [McpInstallScope, string, string])[] = [
  ["user", "ext.mine", "ext.available_in_every_project"],
  ["local", "ext.this_project_only", "ext.stays_out_of_the_repository"],
  ["project", "ext.commit_to_the_repo", "ext.whoever_clones_it_gets_it"],
];

const props = defineProps<{ port: AgentPort; canProject: boolean }>();

const cat = ref<McpCatalog | null>(null);
const busy = ref("");
const failedNote = ref("");
const confirming = ref("");
const adding = ref(false);
const draftText = ref("");
const draft = ref<McpDraft | null>(null);
const scope = ref<McpInstallScope>("user");
const results = ref<McpInstallResult[]>([]);
const parseError = ref("");

const load = async (port: AgentPort) => {
  cat.value = null;
  try {
    cat.value = await port.mcp();
  } catch (e) {
    failedNote.value = reason(e);
    cat.value = null;
  }
};

watch(() => props.port, (port) => {
  failedNote.value = "";
  void load(port);
}, { immediate: true });

const servers = computed(() => cat.value?.servers ?? []);
const live = computed(() => cat.value?.live !== false);
const broken = computed(() => servers.value.filter((s) => s.state === "failed" || s.state === "pending"));

const run = async (what: string, fn: () => Promise<unknown>) => {
  busy.value = what;
  failedNote.value = "";
  try {
    const out = (await fn()) as { error?: string } | void;
    if (out && typeof out === "object" && out.error) failedNote.value = out.error;
  } catch (e) {
    failedNote.value = reason(e);
  } finally {
    busy.value = "";
    await load(props.port);
  }
};

const toggle = (m: McpEntry) => void run(`toggle:${m.name}`, () => props.port.setMcpEnabled(m.name, !m.enabled, "project"));

const remove = (m: McpEntry) => {
  confirming.value = "";
  void run(`remove:${m.name}`, async () => {
    const out = await props.port.removeMcp(m.name);
    // A lower-precedence declaration with the same name may take over; saying so
    // beats a list that looks like the delete failed.
    if (out.stillConfigured) failedNote.value = t("ext.another_declaration_of_the_same");
  });
};

const loadMode = (m: McpEntry, mode: McpLoad) => {
  if ((mode === "always") === !!m.alwaysLoad) return;
  void run(`load:${m.name}`, () => props.port.setMcpLoad(m.name, mode));
};

const approve = (m: McpEntry, follow: boolean) => void run(`${follow ? "follow" : "trust"}:${m.name}`, async () => {
  const approved = await props.port.trustMcp(m.name, m.heldDigest ?? "");
  // Following is asked for after an approval, never instead of one: a refused
  // approval stops here.
  if (follow && !approved.error) await props.port.followMcp(m.name, true);
});

const parse = async () => {
  if (!draftText.value.trim() || busy.value) return;
  busy.value = "parse";
  parseError.value = "";
  try {
    draft.value = await props.port.parseMcp(draftText.value);
    results.value = [];
  } catch (e) {
    draft.value = null;
    parseError.value = reason(e);
  } finally {
    busy.value = "";
  }
};

const remaining = computed(() => draft.value?.servers.filter((s) => !results.value.some((r) => r.name === s.name && r.state !== "issue")) ?? []);

const install = async () => {
  if (!draft.value) return;
  busy.value = "install";
  parseError.value = "";
  const out: McpInstallResult[] = [];
  try {
    for (const server of remaining.value) out.push(await props.port.installMcp(server, scope.value));
  } catch (e) {
    parseError.value = reason(e);
  } finally {
    results.value = [...new Map([...results.value, ...out].map((r) => [r.name, r])).values()];
    busy.value = "";
    if (out.some((r) => r.state !== "issue")) await load(props.port);
  }
};

const closeAdd = () => {
  adding.value = false;
  draft.value = null;
  draftText.value = "";
  results.value = [];
  parseError.value = "";
};

// The rail's own block only lists the servers that need a decision, and hands
// the reader back to this group rather than printing an endpoint's error text.
const reveal = () => {
  document.getElementById("set-mcp")?.scrollIntoView?.({ block: "start" });
};
</script>

<template>
  <McpRail v-if="broken.length > 0" :servers="broken" :on-open="reveal" />

  <div v-if="!cat" class="empty">{{ failedNote || t("ext.could_not_read_name", { name: "MCP" }) }}</div>
  <div v-else class="box">
    <div v-if="!live" class="peernote" role="status">{{ t("common.not_connected") }}</div>

    <div v-for="m in servers" :key="m.name" class="srv" :data-st="m.state">
      <details :open="confirming === m.name || !!m.held?.length || !!m.error || undefined">
        <summary>
          <i class="pip" />
          <span class="srv-main">
            <span class="nm">{{ m.name }}</span>
            <span class="srv-sub">
              <span class="ds">{{ t(STATE[m.state] ?? m.state) }}{{ m.transport ? ` · ${m.transport}` : "" }}{{ m.tools ? ` · ${t("ext.n_tools", { n: m.tools })}` : "" }}</span>
            </span>
          </span>
          <Switch
            data-action="mcp.enabled"
            :data-target="m.name"
            :on="m.enabled"
            :busy="busy === `toggle:${m.name}`"
            :label="t(m.enabled ? 'common.disable_name' : 'common.enable_name', { name: m.name })"
            :on-click="() => toggle(m)"
          />
        </summary>

        <p class="peernote">{{ m.description || t("ext.this_service_does_not_describe") }}</p>

        <!-- A repository's server is approved by what it launches, so the line
             it starts sits beside the switch that approves it. -->
        <p v-if="m.state === 'pending' && m.launch" class="peernote">{{ t("ext.will_start") }} · <code>{{ m.launch }}</code></p>

        <section v-if="m.held?.length && m.heldDigest" class="srv-held" :aria-label="t('ext.tools_waiting_for_your_approval')">
          <span class="ds">{{ t("ext.n_of_this_services_tools", { n: m.held.length + (m.heldMore ?? 0) }) }}</span>
          <ul>
            <li v-for="h in m.held" :key="h.name">
              <b>{{ h.name }}</b>
              <i class="ds">{{ t(HELD[h.class] ?? h.class) }}</i>
            </li>
          </ul>
          <span v-if="m.heldMore" class="ds">{{ t("ext.n_more_are_not_listed", { n: m.heldMore }) }}</span>
          <span v-if="live" class="acts">
            <button class="act" type="button" data-action="mcp.trust" :data-target="m.name" :disabled="!!busy" @click="approve(m, false)">
              {{ t(busy === `trust:${m.name}` ? "ext.approving" : "ext.approve_and_reconnect") }}
            </button>
            <button class="act" type="button" data-action="mcp.follow" :data-target="m.name" :disabled="!!busy" @click="approve(m, true)">
              {{ t(busy === `follow:${m.name}` ? "ext.approving" : "ext.accept_and_accept_this_servers") }}
            </button>
          </span>
        </section>

        <!-- The scope names are the server's words: shown as written, never read
             for meaning — the kernel decides what a grant means. -->
        <section v-if="m.scopeUpgrade?.length" class="srv-held" :aria-label="t('ext.this_server_asks_for_more')">
          <span class="ds">{{ t("ext.this_server_refused_a_call") }}</span>
          <code>{{ m.scopeUpgrade.join(" ") }}</code>
          <button v-if="live" class="act" type="button" data-action="mcp.upgrade" :data-target="m.name" :disabled="!!busy" @click="void run(`upgrade:${m.name}`, () => props.port.upgradeMcp(m.name))">
            {{ t(busy === `upgrade:${m.name}` ? "ext.waiting_for_the_browser_authorization" : "ext.grant_more_access") }}
          </button>
        </section>

        <section v-if="m.followsChanges" class="srv-held" :aria-label="t('ext.changes_from_this_server_are')">
          <span class="ds">{{ t("ext.this_servers_changes_are_accepted") }}</span>
          <button v-if="live" class="act" type="button" data-action="mcp.unfollow" :data-target="m.name" :disabled="!!busy" @click="void run(`unfollow:${m.name}`, () => props.port.followMcp(m.name, false))">
            {{ t(busy === `unfollow:${m.name}` ? "common.stopping" : "ext.stop_following") }}
          </button>
        </section>

        <!-- The tool list is long and the least urgent, so it folds behind its own
             count. A schema this host rejected stays listed with why it cannot run. -->
        <details v-if="m.toolList?.length" class="peek">
          <summary class="ds">{{ t("ext.n_tools", { n: m.tools }) }}</summary>
          <div v-for="tool in m.toolList" :key="tool.name" class="trow" :data-bad="tool.error ? '' : undefined">
            <span class="nm">{{ tool.name }}</span>
            <span class="ds">{{ tool.error || tool.description || t("ext.no_description_written") }}</span>
            <span class="face">
              <i v-if="tool.destructive" class="dg">{{ t("ext.modifies_data") }}</i>
              <i v-else-if="tool.readOnly" class="ro">{{ t("common.read_only") }}</i>
            </span>
          </div>
        </details>

        <div v-if="live && m.enabled && (m.toolList?.length ?? 0) > 0" class="load">
          <span class="lb">{{ t("ext.loading") }}</span>
          <div class="seg" data-text role="group" :aria-label="t('ext.loading')">
            <button
              v-for="mode in (['deferred', 'always'] as McpLoad[])"
              :key="mode"
              type="button"
              data-action="mcp.load"
              :data-target="m.name"
              :data-value="mode"
              :aria-pressed="(mode === 'always') === !!m.alwaysLoad"
              :disabled="!!busy"
              @click="loadMode(m, mode)"
            >{{ t(mode === "always" ? "ext.always_loaded" : "ext.on_demand") }}</button>
          </div>
          <span class="hn">{{ t(m.alwaysLoad ? "ext.connected_at_startup_the_tools" : "ext.searched_for_and_loaded_when") }}</span>
        </div>

        <p v-if="m.error" class="why" role="alert">{{ m.error }}</p>

        <span class="acts">
          <button
            v-if="live && m.enabled && m.state !== 'ready' && m.state !== 'pending'"
            class="act"
            type="button"
            data-action="mcp.retry"
            :data-target="m.name"
            :disabled="!!busy"
            @click="void run(`retry:${m.name}`, () => props.port.reconnectMcp(m.name))"
          >{{ t(busy === `retry:${m.name}` ? "ext.connecting" : m.state === "standby" ? "ext.connect_now" : "ext.reconnect") }}</button>
          <!-- The server answered 401: signing in is the one thing that answers
               it, so it sits beside the retry rather than anywhere else. -->
          <button
            v-if="live && m.enabled && m.state === 'failed' && m.httpStatus === 401"
            class="act"
            type="button"
            data-action="mcp.signin"
            :data-target="m.name"
            :disabled="!!busy"
            @click="void run(`signin:${m.name}`, () => props.port.signinMcp(m.name))"
          >{{ t(busy === `signin:${m.name}` ? "ext.waiting_for_the_browser_sign" : "common.sign_in") }}</button>
          <button
            v-if="live"
            class="act ghost"
            type="button"
            data-action="mcp.remove"
            :data-target="m.name"
            :aria-label="t('common.remove_name', { name: m.name })"
            :disabled="!!busy"
            @click="confirming = m.name"
          >{{ t("common.remove") }}</button>
        </span>

        <!-- Removal is the one act here that cannot be undone by clicking again,
             so it asks, and the question names where the declaration lives. -->
        <div v-if="confirming === m.name" class="confirm">
          <span class="q">{{ t("ext.delete_name_from_where_if", { name: m.name, where: m.source || t("ext.the_config") }) }}</span>
          <button class="act" type="button" data-action="mcp.remove" :data-target="m.name" data-value="cancel" :disabled="!!busy" @click="confirming = ''">
            {{ t("common.cancel") }}
          </button>
          <button class="act danger" type="button" data-action="mcp.remove" :data-target="m.name" data-value="confirm" :disabled="!!busy" @click="remove(m)">
            {{ t(busy === `remove:${m.name}` ? "ext.removing_2" : "common.remove") }}
          </button>
        </div>
      </details>
    </div>

    <div v-if="servers.length === 0 && !adding" class="empty">{{ t("settings.no_external_services_connected") }}</div>

    <button v-if="!adding" class="act" type="button" data-action="mcp.add" @click="adding = true">{{ t("ext.add_service") }}</button>
    <p v-if="!adding" class="peernote">{{ t("ext.add_service_hint") }}</p>

    <div v-if="adding" class="addsrv" :data-stage="draft ? 'confirm' : 'paste'" :aria-busy="busy === 'parse' || busy === 'install'">
      <template v-if="!draft">
        <textarea v-model="draftText" class="paste" rows="4" :aria-label="t('ext.add_service')" :placeholder="'{&quot;mcpServers&quot;: {&quot;github&quot;: {&quot;command&quot;: &quot;npx&quot;, &quot;args&quot;: [&quot;-y&quot;, &quot;@modelcontextprotocol/server-github&quot;]}}}'" :disabled="busy === 'parse'" @keydown.ctrl.enter.prevent="void parse()" @keydown.meta.enter.prevent="void parse()" />
        <div class="acts">
          <span class="note">{{ t("ext.a_block_of_json_a") }}</span>
          <button class="act" type="button" :disabled="busy === 'parse'" @click="closeAdd">{{ t("common.cancel") }}</button>
          <button class="act" type="button" data-action="mcp.inspect" data-primary :disabled="!draftText.trim() || busy === 'parse'" @click="void parse()">
            {{ t(busy === "parse" ? "ext.reading" : "ext.see_what_it_is") }}
          </button>
        </div>
      </template>

      <template v-else>
        <div v-for="r in results" :key="r.name" class="outcome" :data-state="r.state">
          <i class="pip" />
          <span class="nm">{{ r.name }}</span>
          <span class="ds">
            {{ r.state === "ready" ? t("ext.in_place_n_tools_ready", { n: r.toolCount }) : r.state === "action_required" ? t("ext.the_configuration_was_kept_authorise") : (r.message || t("ext.not_added_nothing_was_left")) }}
          </span>
        </div>
        <div v-for="server in remaining" :key="server.name" class="cand">
          <div class="cand-hd">
            <span class="nm">{{ server.name }}</span>
            <span class="ds">{{ server.transport }}</span>
          </div>
          <!-- What the person is agreeing to: a command this machine will run, an
               endpoint it will talk to, or a credential written out in full. -->
          <div v-for="k in draft.risks.filter((r) => r.server === server.name)" :key="k.field" class="risk" :data-kind="k.kind">
            <span class="lb">{{ k.kind === "shell" ? t("ext.will_start") : k.kind === "unknown-host" ? t("ext.will_connect_to") : t("ext.needs_filling_in") }}</span>
            <span class="dt">{{ k.kind === "secret" ? k.field.split(".").pop() : k.kind === "unknown-host" ? (server.displayUrl ?? "<redacted>") : [server.displayCommand ?? "<redacted>", ...(server.displayArgs ?? [])].join(" ") }}</span>
            <span v-if="k.kind === 'secret'" class="why">{{ t("ext.really_runs_this_command") }}</span>
          </div>
        </div>
        <!-- Installing is a global act; how far it reaches is a separate
             question. Only the third option edits a tracked file, so it says so
             out loud and is never the default. -->
        <div class="scope" role="radiogroup" :aria-label="t('ext.where_to_install_it')">
          <button
            v-for="[id, label, why] in SCOPES"
            :key="id"
            type="button"
            role="radio"
            data-action="mcp.scope"
            :data-value="id"
            :aria-checked="scope === id"
            :disabled="busy === 'install' || (id !== 'user' && !canProject)"
            @click="scope = id"
          >
            {{ t(label) }}<i class="ds">{{ t(why) }}</i>
          </button>
        </div>
        <p v-if="scope === 'project'" class="note">{{ t("ext.this_edits_a_config_file") }}</p>
        <div class="acts">
          <button class="act" type="button" :disabled="busy === 'install'" @click="draft = null; results = []; parseError = ''">{{ t("common.back") }}</button>
          <button class="act" type="button" data-action="mcp.add" data-primary :disabled="busy === 'install' || remaining.length === 0" @click="void install()">
            {{ t(busy === "install" ? "ext.connecting" : "ext.connect") }}
          </button>
          <button class="act" type="button" @click="closeAdd">{{ t("common.done") }}</button>
        </div>
      </template>

      <div v-if="parseError" class="why" role="alert">{{ parseError }}</div>
      <div v-if="failedNote" class="why" role="alert">{{ failedNote }}</div>
    </div>
  </div>
</template>
