<script setup lang="ts">
import { computed, defineComponent, type VNodeChild } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import { t } from "../../../i18n";
import { MODS_SHOWN } from "../../modsgate";
import { useRuntimeReload } from "../../RuntimeReload";
import SectionCard from "../../SectionCard.vue";
import Group from "../../Group.vue";
import StudioIcon from "../../StudioIcon.vue";
import ExtMarket from "../parts/ext-market.vue";
import ExtPlugins from "../parts/ext-plugins.vue";
import ExtMcp from "../parts/ext-mcp.vue";
import ExtSkills from "../parts/ext-skills.vue";
import ExtMods from "../parts/ext-mods.vue";

// Every installed list here answers for one project, and the shell holds several
// at once. The market and the packages are machine-wide, while skills and MCP
// servers are switched per project — so the scope control rides the page rather
// than being re-chosen inside each block.
const props = defineProps<{ settingsProps: SettingsProps }>();

const home = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
const canProject = computed(() => !!props.settingsProps.status?.workspaceRoot);
const { action: reloadAction, note: reloadNote } = useRuntimeReload(() => props.settingsProps.port, props.settingsProps.onChanged);

const Content = defineComponent({
  props: ["value"],
  setup(p) { return () => p.value as VNodeChild; },
});
</script>

<template>
  <SectionCard :title="t('ext.community_market')">
    <Group id="market" :title="t('ext.community_market')" :hint="t('ext.skills_plugins_mcp_servers_and')">
      <ExtMarket
        :port="settingsProps.port"
        :account="settingsProps.account"
        :on-installed="settingsProps.onChanged"
        :on-sign-in="settingsProps.onChanged"
      />
    </Group>
  </SectionCard>

  <SectionCard :title="t('settings.plugin_packages')">
    <Group id="plugins" :title="t('settings.plugin_packages')" :hint="t('settings.one_package_can_bring_skills')">
      <ExtPlugins :port="settingsProps.port" :on-changed="settingsProps.onChanged" />
    </Group>
  </SectionCard>

  <SectionCard :title="t('ext.external_services')">
    <!-- Below the packages: what was added by hand. A server the person typed in
         themselves is not part of anyone's package, and filing it under one would
         misname where it came from. -->
    <Group id="mcp" :title="t('ext.external_services')" :hint="t('settings.mcp_services_you_connected_yourself')">
      <ExtMcp :port="settingsProps.port" :can-project="canProject" />
    </Group>
    <Group id="skills" :title="t('settings.skills')" :hint="t('settings.skills_from_the_working_directory')">
      <ExtSkills :port="settingsProps.port" root="" :on-changed="settingsProps.onChanged" />
    </Group>
    <div class="ext-foot" role="note">
      <StudioIcon name="refresh" />
      <span>{{ t("settings.changes_apply_from_the_next_turn") }}</span>
      <Content :value="reloadAction" />
    </div>
    <Content :value="reloadNote" />
  </SectionCard>

  <!-- The mod host stays out of sight, and out of settings search, until the
       receipt marker and the protocol ruling land; the kernel refuses to turn it
       on until then. One line in modsgate.ts flips both. -->
  <SectionCard v-if="MODS_SHOWN" :title="t('ext.run_claude_code_mods')">
    <Group id="mods" :title="t('ext.run_claude_code_mods')" :hint="t('ext.a_mod_in_a_package')">
      <ExtMods :port="home" />
    </Group>
  </SectionCard>
</template>
