<script setup lang="ts">
import { computed, ref, type CSSProperties } from "vue";
import { t } from "../../../i18n";
import { pct } from "../../../i18n/format";
import { reason } from "../../../i18n/kernel";
import type { AgentPort, Appearance as Look, Wallpaper } from "../../../port/port";
import { useCrop } from "../../useCrop";
import StudioIcon from "../../StudioIcon.vue";
import Group from "../../Group.vue";

const props = defineProps<{ port: AgentPort; look: Look; onLook: (next: Look) => void }>();

/** What the picker offers: the kinds of wallpaper the kernel accepts. */
const ACCEPT = "image/png,image/jpeg,image/webp,image/avif,image/gif,video/mp4,video/webm";

const busy = ref(false);
const failed = ref("");
const file = ref<HTMLInputElement | null>(null);
const folder = ref<HTMLInputElement | null>(null);
const view = ref<HTMLElement | null>(null);

const paper = computed(() => props.look.wallpaper);
const crop = useCrop(() => paper.value?.url, () => paper.value?.kind, view);

const set = (patch: Partial<Look>) => props.onLook({ ...props.look, ...patch });
const setPaper = (patch: Partial<Wallpaper>) => {
  if (!paper.value) return;
  set({ wallpaper: { ...paper.value, ...patch } });
};

// WebKit's slider track does not know the thumb's value, so the filled share is
// handed in from here.
const at = (v: number, min: number, max: number) =>
  ({ "--at": `${max > min ? Math.round(((v - min) / (max - min)) * 100) : 0}%` }) as CSSProperties;

const pickPaper = async (chosen: File | undefined) => {
  if (!chosen || busy.value) return;
  busy.value = true;
  failed.value = "";
  try {
    set(await props.port.uploadWallpaper(chosen));
  } catch (e) {
    failed.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const dropPaper = () => {
  failed.value = "";
  props.port
    .clearWallpaper()
    .then(() => set({ wallpaper: undefined }))
    .catch((e) => { failed.value = reason(e); });
};

const chosen = (e: Event) => {
  const input = e.target as HTMLInputElement;
  void pickPaper(input.files?.[0]);
  input.value = "";
};

// A Wallpaper Engine pack is a folder, not a file: the picker is asked for the
// directory and the three entries a scene needs are found inside it.
const pickScene = async (files: FileList | null) => {
  const picked = Array.from(files ?? []);
  const project = picked.find((f) => f.name.toLowerCase() === "project.json");
  const scene = picked.find((f) => f.name.toLowerCase() === "scene.pkg");
  if (!project || !scene) {
    failed.value = t("look.the_folder_needs_project_json");
    return;
  }
  const preview = picked.find((f) => /^preview\.(jpe?g|png|webp|avif)$/i.test(f.name));
  busy.value = true;
  failed.value = "";
  try {
    set(await props.port.uploadScene({ project, scene, preview }));
  } catch (e) {
    failed.value = reason(e);
  } finally {
    busy.value = false;
  }
};

const sceneChosen = (e: Event) => {
  const input = e.target as HTMLInputElement;
  void pickScene(input.files);
  input.value = "";
};

const openFolder = () => {
  const input = folder.value;
  if (!input) return;
  input.setAttribute("webkitdirectory", "");
  input.setAttribute("directory", "");
  input.click();
};

const soundtrack = computed(() =>
  !!(paper.value?.audioTracks?.length || paper.value?.audios?.length || paper.value?.audio));

const volume = computed(() => Math.min(1, Math.max(0, paper.value?.volume ?? 1)));

// A video is drawn by its own element; only a picture is a background image.
const cover = computed(() => (paper.value && paper.value.kind !== "video" ? `url(${JSON.stringify(paper.value.url)})` : undefined));
</script>

<template>
  <Group id="wallpaper" :title="t('look.wallpaper')" :hint="t('look.the_image_fills_only_the')">
    <template v-if="paper" #action>
      <button class="now nowbtn" data-action="wallpaper.remove" @click="dropPaper">{{ t("common.remove") }}</button>
    </template>

    <div v-if="failed" class="find" data-lvl="err" role="alert">
      <span class="t">{{ failed }}</span>
    </div>

    <input ref="file" type="file" :accept="ACCEPT" hidden data-action="wallpaper.change" @change="chosen" />
    <button class="paperpick" data-action="wallpaper.change" :data-busy="busy ? '' : undefined" :disabled="busy" @click="file?.click()">
      <span class="plus" aria-hidden="true"><StudioIcon name="plus" /></span>
      {{ t(paper ? "look.choose_another" : "look.choose_an_image_or_video") }}
    </button>

    <input ref="folder" type="file" multiple hidden data-action="wallpaper.scene" @change="sceneChosen" />
    <button class="paperpick" data-action="wallpaper.scene" :data-busy="busy ? '' : undefined" :disabled="busy" @click="openFolder">
      <span class="plus" aria-hidden="true"><StudioIcon name="upload" /></span>
      {{ t("look.import_wallpaper_engine") }}
    </button>

    <template v-if="paper">
      <!-- Composited the way the window is: the picture, then page colour over
           it. The scrim fades out with the strength, or a picture turned down to
           nothing would still be sitting under a dark wash. -->
      <div
        ref="view"
        class="paperview"
        role="img"
        :aria-label="t('look.wallpaper')"
        :style="{
          backgroundImage: cover,
          backgroundPosition: `${Math.round(paper.focusX * 100)}% ${Math.round(paper.focusY * 100)}%`,
        }"
      >
        <video
          v-if="paper.kind === 'video'"
          :src="paper.url"
          muted
          playsinline
          preload="metadata"
          aria-hidden="true"
          :style="{
            position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover',
            pointerEvents: 'none', objectPosition: `${Math.round(paper.focusX * 100)}% ${Math.round(paper.focusY * 100)}%`,
          }"
        />
        <i :style="{ opacity: 1 - paper.opacity }" />
        <b :style="{ opacity: paper.dim * Math.min(paper.opacity * 4, 1) }" />
      </div>

      <label class="prow">
        <span class="tx">{{ t("look.strength") }}</span>
        <input
          class="slider"
          :style="at(paper.opacity, 0.05, 1)"
          type="range"
          min="0.05"
          max="1"
          step="0.05"
          :value="paper.opacity"
          data-action="appearance.background"
          data-value="opacity"
          @input="setPaper({ opacity: Number(($event.target as HTMLInputElement).value) })"
        />
        <span class="now">{{ pct(paper.opacity) }}</span>
      </label>

      <label class="prow">
        <span class="tx">{{ t("look.dim") }}</span>
        <input
          class="slider"
          :style="at(paper.dim, 0, 1)"
          type="range"
          min="0"
          max="1"
          step="0.05"
          :value="paper.dim"
          data-action="appearance.background"
          data-value="dim"
          @input="setPaper({ dim: Number(($event.target as HTMLInputElement).value) })"
        />
        <span class="now">{{ pct(paper.dim) }}</span>
      </label>

      <!-- A cover picture is cropped on one axis only, so the other axis has no
           room to move whatever the slider says; the row says which is which
           rather than letting a drag do nothing. -->
      <label class="prow" :data-idle="crop.x.value ? undefined : ''">
        <span class="tx">{{ t("look.horizontal_focus") }}</span>
        <input
          class="slider"
          :style="at(paper.focusX, 0, 1)"
          type="range"
          min="0"
          max="1"
          step="0.05"
          :value="paper.focusX"
          :disabled="!crop.x.value"
          data-action="appearance.background"
          data-value="focus-x"
          @input="setPaper({ focusX: Number(($event.target as HTMLInputElement).value) })"
        />
        <span class="now">{{ pct(paper.focusX) }}</span>
      </label>

      <label class="prow" :data-idle="crop.y.value ? undefined : ''">
        <span class="tx">{{ t("look.vertical_focus") }}</span>
        <input
          class="slider"
          :style="at(paper.focusY, 0, 1)"
          type="range"
          min="0"
          max="1"
          step="0.05"
          :value="paper.focusY"
          :disabled="!crop.y.value"
          data-action="appearance.background"
          data-value="focus-y"
          @input="setPaper({ focusY: Number(($event.target as HTMLInputElement).value) })"
        />
        <span class="now">{{ pct(paper.focusY) }}</span>
      </label>

      <label v-if="soundtrack" class="prow">
        <span class="tx">{{ t("look.volume") }}</span>
        <input
          class="slider"
          :style="at(volume, 0, 1)"
          type="range"
          min="0"
          max="1"
          step="0.05"
          :value="volume"
          data-action="wallpaper.volume"
          @input="setPaper({ volume: Number(($event.target as HTMLInputElement).value), muted: Number(($event.target as HTMLInputElement).value) <= 0 })"
        />
        <span class="now">{{ pct(volume) }}</span>
      </label>

      <p v-if="crop.known.value && !(crop.x.value && crop.y.value)" class="note">
        {{ t(crop.x.value
          ? "look.at_the_current_window_size_2"
          : crop.y.value
            ? "look.at_the_current_window_size"
            : "look.this_wallpaper_matches_the_windows") }}
      </p>

      <template v-if="paper.kind === 'scene'">
        <div class="prow">
          <span class="tx">{{ t("look.wallpaper_engine_scene") }}</span>
          <span class="now">{{ paper.title || t("look.untitled_scene") }}</span>
          <button v-if="soundtrack" class="now nowbtn" data-action="wallpaper.sound" @click="setPaper({ muted: !paper.muted })">
            {{ t(paper.muted ? "look.unmute_sound" : "look.mute_sound") }}
          </button>
        </div>
        <p class="note">{{ t("look.the_scene_plays_its_backdrop") }}</p>
      </template>
    </template>
  </Group>
</template>
