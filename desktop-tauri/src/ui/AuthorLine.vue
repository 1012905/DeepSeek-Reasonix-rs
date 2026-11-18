<script setup lang="ts">
import { t } from "../i18n";
import type { AgentPort } from "../port/port";
import { COMMUNITY } from "./communityLinks";

const props = defineProps<{ port: Pick<AgentPort, "openExternal"> }>();

const open = (e: MouseEvent) => {
  e.preventDefault();
  void props.port.openExternal(COMMUNITY.authorUrl).catch(() => {});
};
</script>

<template>
  <div class="comm-author">
    <span class="comm-k">{{ t("app.author_and_maintainer") }}</span>
    <a :href="COMMUNITY.authorUrl" data-action="community.author" @click="open">{{ COMMUNITY.author }}</a>
  </div>
</template>