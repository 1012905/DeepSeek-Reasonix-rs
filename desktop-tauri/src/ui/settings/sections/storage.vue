<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import { useExternal } from "../../../state/external";
import { hidesLegacySessions, onHidesLegacySessionsChange, setHidesLegacySessions } from "../../../state/prefs";
import type { SettingsProps } from "../../SettingsProps";
import SettingRow from "../../SettingRow.vue";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import Path from "../../Path.vue";
import MemorySection from "./memory.vue";
import UsageSection from "./usage.vue";
import { declarePage } from "./nesting";
import StorageUsage from "../parts/data-storage-usage.vue";
import AutoArchive from "../parts/data-auto-archive.vue";
import BackupFile from "../parts/data-backup-file.vue";
import LegacySessions from "../parts/data-legacy-sessions.vue";
import TraceExport from "../parts/data-trace-export.vue";

// What this machine keeps: what the agent remembers, what it has spent, how
// much is on disk and where, how long a conversation survives without being
// archived, and the file the person carries off it.
//
// Memory and usage are pages of their own as well as blocks on this one
// (FOLDS_INTO), so they are drawn from their own files instead of a second
// copy of the same blocks kept here: one implementation per block, and the
// index under this page's title lands on a row that really exists. The column
// is only ever one page, so `at` being "memory" mounts memory.vue and never
// this file — see ./nesting for the guard that keeps it that way if a page
// ever does nest a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
declarePage("storage");

// Machine-scoped, so the window's own port answers even while a pane is on
// another machine.
const home = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);
const recorded = computed(() => props.settingsProps.status?.cwd || props.settingsProps.workspaceRoot || "—");
const hidden = useExternal(onHidesLegacySessionsChange, hidesLegacySessions);
</script>

<template>
  <!-- The folded pair first, in the order this page's own description names
       them: what it remembers, what it has spent, then the disk. -->
  <MemorySection :settings-props="settingsProps" />
  <UsageSection :settings-props="settingsProps" />

  <SectionCard :title="t('data.session_storage')" :hint="t('data.where_sessions_and_indexes_live')">
    <Group id="storage" :title="t('common.storage')" :hint="t('settings.storage_hint')">
      <StorageUsage :port="home" />
    </Group>

    <AutoArchive :port="home" />

    <SettingRow
      id="session-dir"
      :title="t('session.where_this_session_writes')"
      :desc="t('settings.where_session_records_go')"
      :more="t('session.folders_are_managed_in_the')"
    >
      <span class="srow-val"><Path :of="recorded" /></span>
    </SettingRow>

    <SettingRow
      id="legacy-sessions"
      :title="t('data.sessions_from_1_x')"
      :desc="t('settings.migrated_sessions_in_the_sidebar')"
      :more="t('data.whether_sessions_migrated_from_reasonix')"
    >
      <div class="seg" data-text role="radiogroup" :aria-label="t('data.sessions_from_1_x')">
        <button role="radio" data-action="appearance.legacy-sessions" data-value="show" :aria-checked="!hidden" @click="setHidesLegacySessions(false)">{{ t("common.show") }}</button>
        <button role="radio" data-action="appearance.legacy-sessions" data-value="hide" :aria-checked="hidden" @click="setHidesLegacySessions(true)">{{ t("data.hide") }}</button>
      </div>
    </SettingRow>

    <LegacySessions
      :hub="settingsProps.hub"
      :workspace="settingsProps.workspaceRoot ?? ''"
      :on-recovered="() => settingsProps.onSessionsRecovered?.()"
    />
  </SectionCard>

  <SectionCard :title="t('data.backup')" :hint="t('data.take_your_configuration_to_another')">
    <Group id="backup-file" :title="t('data.export_and_import_config')" :hint="t('settings.config_in_one_file')">
      <BackupFile :port="home" />
    </Group>
  </SectionCard>

  <SectionCard :title="t('data.trace_export')">
    <TraceExport :port="home" />
  </SectionCard>
</template>
