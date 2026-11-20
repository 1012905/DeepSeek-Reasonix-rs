<script setup lang="ts">
import { computed, ref } from "vue";
import { t } from "../../../i18n";
import type { AgentPort } from "../../../port/port";
import { COMMUNITY, profileUrl } from "../../communityLinks";
import CopyButton from "../../CopyButton.vue";
import qrSrc from "../../../assets/qq-group-qr.svg";
import douyinSrc from "../../../assets/douyin-qr.jpg";
import contributors from "../../../data/contributors.json";

// Who to ask, and who built it. Everything here is a link or a code to copy; no
// entry changes anything on this machine.
const COLLAPSED = 24;

const props = defineProps<{ port: Pick<AgentPort, "openExternal"> }>();

const all = ref(false);
const failed = ref("");
const douyinQr = ref(false);

const open = (url: string) => {
  void props.port.openExternal(url).then(() => { failed.value = ""; }, () => { failed.value = t("common.could_not_open_a_browser", { at: url }); });
};

const people = computed(() => (contributors as string[]).flatMap((login) => {
  const url = profileUrl(login);
  return url ? [{ login, url }] : [];
}));
const shown = computed(() => (all.value ? people.value : people.value.slice(0, COLLAPSED)));
</script>

<template>
  <div class="comm">
    <div class="comm-join">
      <img class="comm-qr" :src="qrSrc" :alt="t('feedback.qq_group_qr_code')" width="185" height="185" />
      <div class="comm-qq">
        <span class="comm-name">{{ COMMUNITY.qqName }}</span>
        <span class="comm-num">
          <span class="comm-k">{{ t("feedback.group_number") }}</span>
          <code>{{ COMMUNITY.qqNumber }}</code>
          <CopyButton :text="COMMUNITY.qqNumber" icon-only show-feedback :label="t('feedback.copy_group_number')" />
        </span>
        <span class="comm-how">{{ t("feedback.scan_the_qr_code_with") }}</span>
        <span class="comm-acts">
          <button type="button" class="btn" data-primary data-action="community.join" @click="open(COMMUNITY.qqJoin)">{{ t("feedback.join_the_qq_group") }}</button>
          <button type="button" class="btn" data-action="community.discord" @click="open(COMMUNITY.discord)">Discord</button>
          <button type="button" class="btn" data-action="community.issues" @click="open(COMMUNITY.issues)">GitHub issues</button>
        </span>
      </div>
    </div>

    <div class="comm-dy">
      <span class="comm-k">{{ t("feedback.douyin") }}</span>
      <span>{{ COMMUNITY.douyinName }}</span>
      <span class="comm-k">{{ t("feedback.douyin_id") }}</span>
      <code>{{ COMMUNITY.douyinId }}</code>
      <CopyButton :text="COMMUNITY.douyinId" icon-only show-feedback :label="t('feedback.copy_douyin_id')" />
      <button type="button" class="btn sm" data-action="community.douyin-qr" :aria-expanded="douyinQr" @click="douyinQr = !douyinQr">
        {{ t(douyinQr ? "feedback.hide_qr_code" : "feedback.show_qr_code") }}
      </button>
    </div>
    <img v-if="douyinQr" class="comm-dy-qr" :src="douyinSrc" :alt="t('feedback.douyin_qr_code')" width="180" height="269" />

    <p v-if="failed" class="comm-fail" role="alert">{{ failed }}</p>

    <div class="comm-who">
      <div class="comm-who-hd">
        <h4>{{ t("feedback.contributors") }}</h4>
        <span class="comm-k">{{ t("feedback.sorted_by_commits_the_list") }}</span>
        <button type="button" class="btn sm" data-action="community.contributors" @click="open(COMMUNITY.contributors)">{{ t("feedback.view_all_contributors") }}</button>
      </div>
      <ul class="comm-list" :aria-label="t('feedback.contributors')">
        <li v-for="person in shown" :key="person.login">
          <a :href="person.url" data-action="community.profile" :data-target="person.login" @click.prevent="open(person.url)">{{ person.login }}</a>
        </li>
      </ul>
      <button v-if="people.length > COLLAPSED" type="button" class="btn sm comm-more" data-action="community.more" :aria-expanded="all" @click="all = !all">
        {{ t(all ? "common.collapse" : "feedback.show_all_n", { n: people.length }) }}
      </button>
    </div>
  </div>
</template>
