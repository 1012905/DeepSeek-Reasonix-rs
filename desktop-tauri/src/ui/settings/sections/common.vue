<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import type { SettingsProps } from "../../SettingsProps";
import type { DefaultModeSettings, NotifyPrefs } from "../../../port/port";
import { STORAGE, t } from "../../../i18n";
import { reason } from "../../../i18n/kernel";
import { setSendShortcut, useSendShortcut, type SendShortcut as SendMode } from "../../../state/prefs";
import { acceleratorLabel } from "../../keys";
import SectionCard from "../../SectionCard.vue";
import SettingRow from "../../SettingRow.vue";
import Switch from "../../Switch.vue";
import ThemeScheduleFields from "../../ThemeScheduleFields.vue";
import UpdateMode from "../parts/data-update-mode.vue";

// The six settings changed most often, all of them the machine's: nothing here
// belongs to a session, so every write goes through the window's own port and,
// for the two the window shell keeps, the home machine's (`networkPort`).
const props = defineProps<{ settingsProps: SettingsProps }>();
const p = computed(() => props.settingsProps);
const home = computed(() => p.value.networkPort ?? p.value.port);

// The catalogue is read once at boot, so a language change is a reload. The
// save gets a beat to leave before the page goes, or a fast disk shows the old
// language once more.
const RELOAD_AFTER_SAVE_MS = 140;

const LANGS: readonly (readonly [string, string])[] = [
  ["", "look.follow_the_system"],
  ["zh", "settings.text"],
  ["en", "settings.english"],
];

const SCHEMES: readonly (readonly [string, string])[] = [
  ["auto", "look.follow_the_system"],
  ["light", "look.light"],
  ["dark", "look.dark"],
  ["scheduled", "look.scheduled"],
];

// The update-mode row lives in ../parts/data-update-mode.vue, which draws the
// `update-mode` anchor the catalogue files under this section. It is not
// inlined here: a second copy of that block is a second `set-update-mode` for a
// search hit to land on.

// The boot language is the local copy; the kernel's is what a machine that
// never opened this window adopts, so both are read here.
const readStoredLang = (): string => {
  try {
    return localStorage.getItem(STORAGE) ?? p.value.look.language ?? "";
  } catch {
    return p.value.look.language ?? "";
  }
};

const langNow = ref(readStoredLang());

const setLang = (next: string) => {
  if (next === langNow.value) return;
  langNow.value = next;
  try {
    localStorage.setItem(STORAGE, next);
  } catch {
    /* the kernel's copy is what the next launch adopts */
  }
  p.value.onLook({ ...p.value.look, language: next });
  window.setTimeout(() => location.reload(), RELOAD_AFTER_SAVE_MS);
};

// --- which mode a new conversation starts in --------------------------------

const start = ref<DefaultModeSettings | null>(null);
const startBusy = ref(false);
const startError = ref("");

const loadStart = () => {
  p.value.port
    .defaultMode()
    .then((s) => { start.value = s; })
    .catch(() => { start.value = null; });
};

const pickStart = async (next: "agent" | "plan") => {
  const now = start.value;
  if (!now || (next === "plan") === now.plan || startBusy.value) return;
  startBusy.value = true;
  startError.value = "";
  try {
    start.value = await p.value.port.saveDefaultMode(next === "plan");
  } catch (e) {
    startError.value = reason(e);
  } finally {
    startBusy.value = false;
  }
};

// --- the send key -----------------------------------------------------------

const sendShortcut = useSendShortcut();
// The label of the modifier belongs to the platform this window runs on: a Mac
// that showed "Ctrl+Enter" would be naming a key it does not have.
const sendOptions: readonly (readonly [SendMode, string])[] = [
  ["enter", "Enter"],
  ["modifier_enter", acceleratorLabel("CommandOrControl+Enter")],
];

// --- what this machine may interrupt for ------------------------------------

const notify = ref<NotifyPrefs | null>(null);
const notifyError = ref("");

const loadNotify = () => {
  p.value.port
    .notifyPrefs()
    .then((n) => { notify.value = n; })
    .catch(() => { notify.value = null; });
};

// One writer for every switch: the window answers with what is true afterwards,
// not with what was asked, so its answer is what is kept.
type NotifyFlag = "enabled" | "sound" | "turnDone" | "approval" | "ask" | "feedbackReply" | "diskLow";

const toggle = async (key: NotifyFlag) => {
  const now = notify.value;
  if (!now) return;
  notifyError.value = "";
  const patch: Partial<NotifyPrefs> = { [key]: !now[key] };
  try {
    notify.value = await p.value.port.setNotifyPrefs({ ...now, ...patch });
  } catch (e) {
    notifyError.value = reason(e);
  }
};

// Either switch wakes the occasions: a person may want the chime and no toast,
// and greying the occasions out under an off toast would say the opposite.
const notifyActive = computed(() => !!notify.value && (notify.value.enabled || notify.value.sound));

// --- what this machine changed from the defaults ----------------------------

const changed = ref<string[] | null>(null);
const changedOpen = ref(false);

const loadChanged = () => {
  p.value.port
    .changedSettings()
    .then((read) => { changed.value = read.keys; })
    .catch(() => { changed.value = null; });
};

const load = () => {
  loadStart();
  loadNotify();
  loadChanged();
};

onMounted(load);
// Another home machine is another set of stored answers.
watch(home, load);
// The shell adopts the kernel's language on boot, which is how a machine that
// never opened this window shows the choice already on file; the row follows it
// rather than a local copy that predates the adoption.
watch(() => p.value.look.language, (language) => {
  if (language !== undefined) langNow.value = language;
});
</script>

<template>
  <SectionCard :title="t('common.start')">
    <SettingRow
      id="default-mode"
      :title="t('safety.how_new_conversations_start')"
      :desc="t('safety.which_mode_a_new_chat')"
    >
      <div v-if="!start" class="srow-desc">{{ t("safety.could_not_read_the_setting") }}</div>
      <div v-else class="seg" data-text role="radiogroup" :aria-label="t('safety.how_new_conversations_start')">
        <button
          role="radio" data-action="default-mode.plan" data-value="agent"
          :aria-checked="!start.plan" :disabled="startBusy"
          @click="pickStart('agent')"
        >{{ t("Agent") }}</button>
        <button
          role="radio" data-action="default-mode.plan" data-value="plan"
          :aria-checked="start.plan" :disabled="startBusy"
          @click="pickStart('plan')"
        >{{ t("common.plan") }}</button>
      </div>
      <div v-if="startError" class="why" role="alert">{{ startError }}</div>
    </SettingRow>

    <!-- A kernel that cannot say how it treats new releases has no row to offer:
         the mode lives in the user's file and no project can change it. -->
    <UpdateMode :port="home" />
  </SectionCard>

  <SectionCard :title="t('settings.input_and_reminders')">
    <SettingRow id="language" :title="t('common.language')" :desc="t('settings.interface_language')">
      <div class="seg" data-text role="radiogroup" :aria-label="t('common.language')">
        <button
          v-for="[id, name] in LANGS" :key="id || 'system'"
          role="radio" data-action="appearance.language" :data-value="id"
          :aria-checked="langNow === id"
          @click="setLang(id)"
        >{{ t(name) }}</button>
      </div>
    </SettingRow>

    <SettingRow
      id="send-shortcut"
      :title="t('composer.send_shortcut')"
      :desc="t('composer.send_with_enter_or_with')"
    >
      <div class="seg" data-text role="radiogroup" :aria-label="t('composer.send_shortcut')">
        <button
          v-for="[id, name] in sendOptions" :key="id"
          role="radio" data-action="composer.send-shortcut" :data-value="id"
          :aria-checked="sendShortcut === id"
          @click="setSendShortcut(id)"
        >{{ name }}</button>
      </div>
    </SettingRow>

    <!-- A kernel with no window answers nothing, and this draws nothing: a
         notification fired there would land on the server's desktop. -->
    <template v-if="notify">
      <SettingRow id="notify" :title="t('common.notifications')" :desc="t('settings.a_system_notification_when_a')">
        <Switch
          data-action="notify.enabled"
          :on="notify.enabled"
          :label="t('settings.send_system_notifications')"
          :on-click="() => toggle('enabled')"
        />
      </SettingRow>
      <div class="srow-more">
        <div class="lrow">
          <span class="tx">
            <span class="lb">{{ t("settings.play_a_sound") }}</span>
            <span class="ds">{{ t("settings.off_by_default_plays_a") }}</span>
          </span>
          <Switch
            data-action="notify.sound"
            :on="notify.sound"
            :label="t('settings.play_a_sound')"
            :on-click="() => toggle('sound')"
          />
        </div>
        <!-- Branches of the first switch rather than rows that grey out three
             at a time when it is off and leave the reader guessing why. -->
        <div class="lrow subrow" :data-off="notifyActive ? undefined : ''">
          <span class="tx">
            <span class="lb">{{ t("settings.when_the_turn_ends") }}</span>
            <span class="ds">{{ t("settings.the_answer_is_written_or") }}</span>
          </span>
          <Switch
            data-action="notify.turn-done"
            :on="notify.turnDone"
            :busy="!notifyActive"
            :label="t('settings.when_the_turn_ends')"
            :on-click="() => toggle('turnDone')"
          />
        </div>
        <div class="lrow subrow" :data-off="notifyActive ? undefined : ''">
          <span class="tx">
            <span class="lb">{{ t("common.when_something_waits_for_approval") }}</span>
            <span class="ds">{{ t("common.the_model_wants_to_touch") }}</span>
          </span>
          <Switch
            data-action="notify.approval"
            :on="notify.approval"
            :busy="!notifyActive"
            :label="t('common.when_something_waits_for_approval')"
            :on-click="() => toggle('approval')"
          />
        </div>
        <div class="lrow subrow" :data-off="notifyActive ? undefined : ''">
          <span class="tx">
            <span class="lb">{{ t("common.when_the_model_asks_you") }}</span>
            <span class="ds">{{ t("common.the_model_stopped_to_ask") }}</span>
          </span>
          <Switch
            data-action="notify.ask"
            :on="notify.ask"
            :busy="!notifyActive"
            :label="t('common.when_the_model_asks_you')"
            :on-click="() => toggle('ask')"
          />
        </div>
        <div class="lrow subrow" :data-off="notifyActive ? undefined : ''">
          <span class="tx">
            <span class="lb">{{ t("settings.when_feedback_gets_a_reply") }}</span>
            <span class="ds">{{ t("settings.one_alert_when_a_report") }}</span>
          </span>
          <Switch
            data-action="notify.feedback-reply"
            :on="notify.feedbackReply"
            :busy="!notifyActive"
            :label="t('settings.when_feedback_gets_a_reply')"
            :on-click="() => toggle('feedbackReply')"
          />
        </div>
        <div class="lrow subrow" :data-off="notifyActive ? undefined : ''">
          <span class="tx">
            <span class="lb">{{ t("settings.when_disk_space_is_low") }}</span>
            <span class="ds">{{ t("settings.warns_when_the_drive_holding") }}</span>
          </span>
          <Switch
            data-action="notify.disk-low"
            :on="notify.diskLow"
            :busy="!notifyActive"
            :label="t('settings.when_disk_space_is_low')"
            :on-click="() => toggle('diskLow')"
          />
        </div>
      </div>
      <p v-if="notifyError" class="note" role="alert">{{ notifyError }}</p>
    </template>

    <SettingRow id="mode" :title="t('settings.light_and_dark')" :desc="t('settings.follow_the_system_light_or')">
      <div class="seg" data-text role="radiogroup" :aria-label="t('settings.light_and_dark')">
        <button
          v-for="[id, name] in SCHEMES" :key="id"
          role="radio" data-action="appearance.scheme" :data-value="id"
          :aria-checked="p.theme === id"
          @click="p.onTheme(id)"
        >{{ t(name) }}</button>
      </div>
    </SettingRow>
    <ThemeScheduleFields v-if="p.theme === 'scheduled'" />
  </SectionCard>

  <!-- Not a setting of its own: the count of what this machine differs on, and
       the list when the reader asks for it. -->
  <SectionCard v-if="changed" :title="t('settings.changed_settings')">
    <div class="srow">
      <div class="srow-meta">
        <span class="srow-name">
          {{ changed.length === 0 ? t("settings.every_setting_is_the_default") : t("settings.n_differ_from_the_defaults", { n: changed.length }) }}
        </span>
      </div>
      <div v-if="changed.length > 0" class="srow-ctl">
        <button type="button" class="spill" data-action="settings.changed-toggle" @click="changedOpen = !changedOpen">
          {{ t(changedOpen ? "common.collapse" : "settings.look_at") }}
        </button>
      </div>
    </div>
    <ul v-if="changedOpen" class="changed-keys">
      <li v-for="key in changed" :key="key">{{ key }}</li>
    </ul>
  </SectionCard>
</template>
