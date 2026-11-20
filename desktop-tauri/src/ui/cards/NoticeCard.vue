<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { Item } from "../../state/session";
import { leftoverDetail, noticeWording } from "../../i18n/notice_wording";
import { workspaceLeaseDetail } from "../../i18n/workspace_lease";
import StudioIcon from "../StudioIcon.vue";
import LazyMarkdown from "../LazyMarkdown.vue";
import { settingScreen } from "../settingScreens";

// A host notice is a bar, not a card: one sentence with a glyph, and under it
// the diagnostic the kernel wrote beside the headline it wrote for a person.
const PERMISSION = new Set(["permission_saved", "permission_covered", "setting_changed", "setting_open"]);
// These carry what the model wrote as their detail, so it reads the way the
// model's own replies do.
const AUTHORED = new Set(["await_user"]);

const props = defineProps<{
  item: Extract<Item, { t: "notice" }>;
  onSettings?: (at?: string) => void;
}>();

const lvl = computed(() => (props.item.level === "error" ? "err" : props.item.level === "warn" ? "warn" : undefined));
// The kernel writes in English for its own logs; where this build has the same
// notice in the reader's language, that is the one to show.
const wording = computed(() => noticeWording(props.item.code, props.item.detail));
const detail = computed(() => {
  const claim = props.item.workspaceLease;
  return claim
    ? workspaceLeaseDetail(claim, props.item.code !== "workspace_lease_resumed") || props.item.detail
    : leftoverDetail(props.item.code, props.item.detail, wording.value?.vars);
});
</script>

<template>
  <div class="notice-bar" :data-lvl="lvl ?? 'info'" :role="lvl === 'err' ? 'alert' : 'status'">
    <span class="sr-only">{{ t(lvl === "err" ? "chat.error" : lvl === "warn" ? "chat.warning" : "chat.notice") }}：</span>
    <StudioIcon :name="lvl === 'err' ? 'fail' : lvl === 'warn' ? 'warning' : 'info'" class="nb-ic" aria-hidden="true" />
    <div class="nb-body">
      <span class="t">
        {{ wording ? t(wording.text, wording.vars) : item.text }}
        <!-- Repeats are folded rather than stacked: three identical lines say the
             same thing as one and a count. -->
        <b v-if="item.count && item.count > 1" class="ntimes">×{{ item.count }}</b>
      </span>
      <button v-if="item.code === 'setting_open' && onSettings" class="nbtn" data-action="settings.section" :data-value="item.detail ?? ''" @click="onSettings(settingScreen(item.detail ?? ''))">{{ t("chat.open_settings") }}</button>
      <template v-if="detail">
        <code v-if="PERMISSION.has(item.code ?? '')" class="nrule" :title="item.text">{{ item.detail }}</code>
        <div v-else-if="AUTHORED.has(item.code ?? '')" class="nmd"><LazyMarkdown :text="detail" /></div>
        <span v-else class="nwhy">{{ detail }}</span>
      </template>
    </div>
  </div>
</template>
