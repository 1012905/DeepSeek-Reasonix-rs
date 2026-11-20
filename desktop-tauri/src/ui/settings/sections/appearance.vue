<script setup lang="ts">
import type { SettingsProps } from "../../SettingsProps";
import WindowSection from "../../WindowSection.vue";
import SectionCard from "../../SectionCard.vue";
import SettingRow from "../../SettingRow.vue";
import Switch from "../../Switch.vue";
import { t } from "../../../i18n";
import { useExternal } from "../../../state/external";
import { effectsMode, onEffectsChange, setEffectsMode } from "../../../state/prefs";
import LookGraphics from "../parts/look-graphics.vue";
import LookScheme from "../parts/look-scheme.vue";
import LookWallpaper from "../parts/look-wallpaper.vue";
import LookFont from "../parts/look-font.vue";
import LookSize from "../parts/look-size.vue";
import LookSeg from "../parts/look-seg.vue";

// The look family's own contract: every value here is one of the window's
// port's, held by the shell that paints it, not a second copy kept here.
defineProps<{ settingsProps: SettingsProps }>();

// "" follows the system's own accessibility setting; the rest are explicit.
// Weight is a second axis: coloured brighter, a 400-weight Han glyph at 11px is
// still thin. Default follows the language — Han strokes are denser and blur at
// the same size sooner than Latin ones.
const WEIGHTS: [string, string, string][] = [
  ["", "look.match_the_language", "look.medium_for_chinese_regular_for"],
  ["light", "look.regular", "look.thinner_strokes_looser_texture"],
  ["heavy", "look.bolder", "look.firmer_at_small_sizes_and"],
];

const CONTRASTS: [string, string, string][] = [
  ["", "look.follow_the_system", "look.uses_the_strongest_step_when"],
  ["soft", "look.soft", "look.softer_body_text_easier_over"],
  ["normal", "look.standard", "look.between_the_two"],
  ["strong", "look.stronger", "look.bright_surroundings_or_when_edges"],
];

// Decoration is the one appearance setting that costs frames rather than
// clarity, so it is a switch rather than a third step.
const effects = useExternal(onEffectsChange, effectsMode);
</script>

<template>
  <SectionCard :title="t('settings.theme_card')">
    <LookScheme :port="settingsProps.port" :reload-themes="settingsProps.reloadThemes" />
    <LookWallpaper :port="settingsProps.port" :look="settingsProps.look" :on-look="settingsProps.onLook" />
    <SettingRow id="effects" :title="t('look.visual_effects')" :desc="t('settings.frosted_glass_glow_effects')">
      <Switch
        data-action="appearance.effects"
        :data-value="effects"
        :on="effects === 'full'"
        :label="t('look.visual_effects')"
        :on-click="() => setEffectsMode(effects === 'full' ? 'reduced' : 'full')"
      />
    </SettingRow>
  </SectionCard>

  <SectionCard :title="t('settings.text_card')">
    <LookFont :look="settingsProps.look" :on-look="settingsProps.onLook" />
    <LookSize :look="settingsProps.look" :on-look="settingsProps.onLook" />
    <SettingRow id="weight" :title="t('look.text_weight')" :desc="t('settings.weight_of_text')">
      <LookSeg
        :label="t('look.text_weight')"
        :value="settingsProps.weight"
        action="appearance.weight"
        :options="WEIGHTS.map(([id, name]) => [id, t(name)] as const)"
        :pick="settingsProps.onWeight"
      />
    </SettingRow>
    <SettingRow id="contrast" :title="t('look.text_contrast')" :desc="t('settings.contrast_between_text_and_background')">
      <LookSeg
        :label="t('look.text_contrast')"
        :value="settingsProps.contrast"
        action="appearance.contrast"
        :options="CONTRASTS.map(([id, name]) => [id, t(name)] as const)"
        :pick="settingsProps.onContrast"
      />
    </SettingRow>
  </SectionCard>

  <SectionCard :title="t('common.window')">
    <WindowSection :port="settingsProps.port" />
    <LookGraphics />
  </SectionCard>
</template>
