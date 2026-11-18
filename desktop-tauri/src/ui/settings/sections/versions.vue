<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../../i18n";
import type { SettingsProps } from "../../SettingsProps";
import Group from "../../Group.vue";
import SectionCard from "../../SectionCard.vue";
import { drawsHere } from "./nesting";
import Versions from "../parts/data-versions.vue";
import Logs from "../parts/data-logs.vue";
import Community from "../parts/data-community.vue";
import Sponsor from "../../Sponsor.vue";
import { COMMUNITY } from "../../communityLinks";

// What is running, whether something is wrong with it, and who to thank. Every
// control on this page is one that actually works — a button that cannot do what
// it says is worse than no button.
//
// Two pages draw these blocks: this one when `at` names "versions", and account,
// which versions folds into. The column holds one of them at a time; ./nesting
// is what keeps it one if a page ever nests a section it already is.
const props = defineProps<{ settingsProps: SettingsProps }>();
const drawn = drawsHere("versions");

const home = computed(() => props.settingsProps.networkPort ?? props.settingsProps.port);

const open = (url: string) => { void home.value.openExternal(url).catch(() => {}); };
</script>

<template>
  <template v-if="drawn">
    <SectionCard :title="t('settings.version_and_diagnostics')">
      <Group id="versions" :title="t('common.version')" :hint="t('settings.versions_hint')">
        <Versions
          :versions="(refresh) => home.versions(refresh)"
          :pin-version="(v) => home.pinVersion(v)"
          :go-to-version="(v) => home.goToVersion(v)"
          :cancel-version="(v) => home.cancelVersion(v)"
          :restart-to-version="(v, force) => home.restartToVersion(v, force)"
          :on-update-progress="(cb) => home.onUpdateProgress(cb)"
          :version-notes="(v, retry) => home.versionNotes(v, retry)"
          :open-external="(url) => home.openExternal(url)"
        />
        <!-- The update-mode row is not drawn here. prefsnav files that anchor
             under the common section and ../parts/data-update-mode.vue draws it
             there, once; a second copy on this page would be a second
             `set-update-mode` for a landing or a search hit to choose between.
             What this block governs is the reads and downloads above. -->
      </Group>

      <Group id="logs" :title="t('about.logs_and_abnormal_exit_records')" :hint="t('settings.logs_hint')">
        <Logs :port="home" />
      </Group>
    </SectionCard>

    <SectionCard :title="t('settings.about')">
      <p class="srow-desc">{{ t("settings.about_note") }}</p>

      <Group id="community" :title="t('common.community_and_contributors')" :hint="t('about.join_the_community_for_help')">
        <div class="comm-author">
          <span class="comm-k">{{ t("app.author_and_maintainer") }}</span>
          <a :href="COMMUNITY.authorUrl" data-action="community.author" @click.prevent="open(COMMUNITY.authorUrl)">{{ COMMUNITY.author }}</a>
        </div>
        <Community :port="home" />
      </Group>

      <Group id="sponsor" :title="t('about.support_the_project')">
        <Sponsor :port="home" />
      </Group>
    </SectionCard>
  </template>
</template>
