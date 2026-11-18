<script setup lang="ts">
import { ref, watch } from "vue";
import type { AgentPort, TrayPrefs } from "../port/port";
import { t } from "../i18n";
import { keepsAwake, setKeepsAwake } from "../state/prefs";
import { ApplyNote } from "./Group.vue";
import Switch from "./Switch.vue";

const props = defineProps<{ port: AgentPort }>();

// null in a browser tab, where there is no window to keep running and no
// icon to bring one back. The whole section goes with it.
const tray = ref<TrayPrefs | null>(null);
const awake = ref(keepsAwake());

watch(() => props.port, (port) => {
  let live = true;
  port.trayPrefs().then((p) => { if (live) tray.value = p; }).catch(() => { if (live) tray.value = null; });
  return () => { live = false; };
}, { immediate: true });

// The window answers with what is true afterwards rather than what was
// asked: turning the icon off turns backgrounding off with it, and the
// switch has to show that rather than the request.
const flipTray = (patch: Partial<TrayPrefs>) => {
  if (!tray.value) return;
  const next = { ...tray.value, ...patch };
  props.port
    .setTrayPrefs(next.icon, next.closeToTray)
    .then((got) => { if (got) tray.value = got; })
    .catch(() => {});
};
</script>

<template>
  <section v-if="tray" class="grp" id="set-window" data-setting="window">
    <div class="grp-hd">
      <h3>{{ t("common.window") }}</h3>
    </div>
    <p class="hint">
      {{ t("settings.after_the_window_closes_the") }}
    </p>
    <div class="grp-items">
      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("settings.keep_the_computer_awake_while") }}</span>
          <span class="ds">{{ t("settings.while_a_session_is_running") }}</span>
        </span>
        <Switch
          data-action="chrome.keep-awake"
          :on="awake"
          :label="t('settings.keep_the_computer_awake_while')"
          :on-click="() => { setKeepsAwake(!awake); awake = !awake; }"
        />
      </div>
      <div class="lrow">
        <span class="tx">
          <span class="lb">{{ t("settings.show_a_status_icon_in") }}</span>
          <span class="ds">
            {{ tray.icon === tray.live
              ? t("settings.running_or_waiting_on_you")
              : tray.icon
                ? t("settings.appears_on_the_next_launch")
                : t("settings.gone_from_the_next_launch") }}
          </span>
        </span>
        <Switch data-action="tray.icon" :on="tray.icon" :label="t('settings.show_a_status_icon_in')" :on-click="() => flipTray({ icon: !tray!.icon })" />
      </div>
      <!-- The second switch only means anything while there is an icon,
           so it is drawn as a branch of the first rather than as a rule
           you have to discover by watching it grey out. -->
      <div class="lrow subrow" :data-off="tray.icon && tray.live ? undefined : ''">
        <span class="tx">
          <span class="lb">{{ t("settings.keep_running_in_the_tray") }}</span>
          <span class="ds">{{ t("settings.closing_the_window_does_not") }}</span>
        </span>
        <Switch
          data-action="tray.close-to-tray"
          :on="tray.closeToTray"
          :busy="!tray.icon || !tray.live"
          :label="t('settings.keep_running_in_the_tray')"
          :on-click="() => flipTray({ closeToTray: !tray!.closeToTray })"
        />
      </div>
    </div>
    <ApplyNote id="window" />
  </section>
</template>