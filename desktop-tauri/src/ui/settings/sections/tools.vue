<script setup lang="ts">
import type { SettingsProps } from "../../SettingsProps";
import { t } from "../../../i18n";
import SectionCard from "../../SectionCard.vue";
import Group from "../../Group.vue";
import ToolsApproval from "../parts/tools-approval.vue";
import ToolsExternalOrigin from "../parts/tools-external-origin.vue";
import ToolsToolList from "../parts/tools-tool-list.vue";
import ToolsRules from "../parts/tools-rules.vue";
import ToolsSandbox from "../parts/tools-sandbox.vue";
import ToolsShell from "../parts/tools-shell.vue";
import ToolsWriteLease from "../parts/tools-write-lease.vue";
import ToolsWebAccess from "../parts/tools-web-access.vue";
import ToolsBrowser from "../parts/tools-browser.vue";

// Approval decides when to ask; the boundary decides what may happen once it is
// answered. They share one page because a reader changing one is usually about
// to revisit the other.
const props = defineProps<{ settingsProps: SettingsProps }>();

// These are machine-wide limits kept in the window owner's file. A pane reached
// over the network has its own port, and asking through it would write that
// machine's config instead of this one's.
const home = () => props.settingsProps.networkPort ?? props.settingsProps.port;
</script>

<template>
  <SectionCard :title="t('settings.permissions_and_safety')">
    <Group id="approval" :title="t('settings.tool_approvals')" :hint="t('safety.approvals_decide_when_to_ask')">
      <ToolsApproval :port="home()" :status="settingsProps.status" :on-changed="settingsProps.onChanged" />
    </Group>
    <Group id="external-origin" :title="t('settings.outside_text')" :hint="t('safety.ask_before_copied_text')">
      <ToolsExternalOrigin :port="home()" :on-changed="settingsProps.onChanged" />
    </Group>
    <Group id="tool-list" :title="t('ext.tools_the_model_can_use')" :hint="t('ext.a_tool_you_turn_off')">
      <ToolsToolList :port="home()" />
    </Group>
    <Group id="rules" :title="t('settings.rules_you_spell_out')" :hint="t('safety.allow_and_deny_by_command')">
      <ToolsRules :port="home()" :on-changed="settingsProps.onChanged" />
    </Group>
  </SectionCard>

  <SectionCard :title="t('settings.sandbox_and_execution')">
    <Group id="sandbox" :title="t('settings.sandbox')" :hint="t('safety.commands_write_only_here')">
      <ToolsSandbox :port="home()" :on-changed="settingsProps.onChanged" />
    </Group>
    <Group id="shell" :title="t('common.which_program_runs_commands')" :hint="t('safety.which_program_runs_a_command')">
      <ToolsShell :port="home()" :on-changed="settingsProps.onChanged" />
    </Group>
    <Group id="write-lease" :title="t('session.several_sessions_writing_one_project')" :hint="t('safety.when_two_sessions_edit_one')">
      <ToolsWriteLease :port="home()" />
    </Group>
  </SectionCard>

  <SectionCard :title="t('safety.web')">
    <Group id="web-access" :title="t('models.web_search_and_fetch')" :hint="t('models.which_backends_the_agent_uses')">
      <ToolsWebAccess :port="home()" />
    </Group>
    <Group id="browser-tools" :title="t('common.built_in_browser')" :hint="t('workbench.let_it_open_pages_in_the_browser')">
      <ToolsBrowser :port="home()" :on-changed="settingsProps.onChanged" />
    </Group>
  </SectionCard>
</template>
