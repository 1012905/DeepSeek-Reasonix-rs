<script setup lang="ts">
import { computed, defineComponent, h, onMounted, onUnmounted, ref, Teleport, watch } from "vue";
import { t } from "../i18n";
import { useEscape } from "./dismiss";
import StudioIcon from "./StudioIcon.vue";

const props = defineProps<{
  src?: string;
  name: string;
  className?: string;
}>();

const open = ref(false);
const label = computed(() => t("chat.preview_image_name", { name: props.name }));
watch(() => props.src, () => { open.value = false; });

const arm = (event: MouseEvent) => {
  if (event.detail > 1) event.preventDefault();
};

const ImageViewer = defineComponent({
  name: "ImageViewer",
  props: {
    src: { type: String, required: true },
    name: { type: String, required: true },
  },
  emits: ["close"],
  setup(viewerProps, { emit }) {
    const closeBtn = ref<HTMLButtonElement | null>(null);
    const failed = ref(false);
    const scale = ref(1);
    const pan = ref({ x: 0, y: 0 });
    const size = ref<[number, number] | null>(null);
    let drag: { x: number; y: number; from: { x: number; y: number } } | null = null;
    useEscape(true, () => emit("close"));

    const backdrop = (event: MouseEvent) => {
      if (event.target === event.currentTarget && event.detail < 2) emit("close");
    };
    const download = () => {
      const link = document.createElement("a");
      link.href = viewerProps.src;
      link.download = viewerProps.name;
      link.click();
    };
    const zoom = (factor: number) => {
      scale.value = Math.min(8, Math.max(0.2, scale.value * factor));
    };

    let previous: Element | null = null;
    let overflow = "";
    onMounted(() => {
      previous = document.activeElement;
      overflow = document.body.style.overflow;
      document.body.style.overflow = "hidden";
      closeBtn.value?.focus();
    });
    onUnmounted(() => {
      document.body.style.overflow = overflow;
      if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
    });

    // Transcript and composer ancestors can scroll or carry backdrop filters;
    // the body portal keeps the viewer outside those containing blocks.
    return () => h(Teleport, { to: "body" }, [
      h("div", {
        class: "image-preview",
        role: "dialog",
        "aria-modal": "true",
        "aria-label": viewerProps.name,
        "data-action-click": "image.close",
        "data-action-keydown": "image.focus-trap",
        "data-action-mousedown": "image.focus-trap",
        onClick: backdrop,
        onMousedown: (event: MouseEvent) => {
          if (!(event.target as HTMLElement).closest("button")) event.preventDefault();
        },
        onKeydown: (event: KeyboardEvent) => {
          if (event.key === "Tab") {
            event.preventDefault();
            closeBtn.value?.focus();
          }
        },
      }, [
        h("div", { class: "image-preview-bar" }, [
          h("span", { class: "image-preview-name" }, viewerProps.name),
          size.value && h("span", { class: "image-preview-size" }, `${size.value[0]} × ${size.value[1]}`),
          h("span", { class: "image-preview-gap" }),
          h("button", {
            type: "button",
            "data-action": "image.zoom",
            "data-value": "in",
            "aria-label": t("chat.zoom_in"),
            title: t("chat.zoom_in"),
            onClick: () => zoom(1.25),
          }, [h(StudioIcon, { name: "zoomIn" })]),
          h("button", {
            type: "button",
            "data-action": "image.zoom",
            "data-value": "out",
            "aria-label": t("chat.zoom_out"),
            title: t("chat.zoom_out"),
            onClick: () => zoom(0.8),
          }, [h(StudioIcon, { name: "zoomOut" })]),
          h("button", {
            type: "button",
            "data-action": "image.download",
            "aria-label": t("chat.image_download"),
            title: t("chat.image_download"),
            onClick: download,
          }, [h(StudioIcon, { name: "download" })]),
          h("button", {
            type: "button",
            ref: closeBtn,
            class: "image-preview-close",
            "data-action": "image.close",
            "aria-label": t("chat.close_image_preview"),
            onClick: () => emit("close"),
          }, [h(StudioIcon, { name: "close" })]),
        ]),
        h("div", {
          class: "image-preview-view",
          "data-action-click": "image.close",
          "data-action-wheel": "image.zoom",
          "data-action-pointerdown": "image.pan",
          "data-action-pointermove": "image.pan",
          "data-action-pointerup": "image.pan",
          onClick: backdrop,
          onWheel: (event: WheelEvent) => zoom(event.deltaY < 0 ? 1.1 : 1 / 1.1),
          onPointerdown: (event: PointerEvent) => {
            if (event.target instanceof HTMLImageElement) {
              drag = { x: event.clientX, y: event.clientY, from: pan.value };
              (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
            }
          },
          onPointermove: (event: PointerEvent) => {
            const held = drag;
            if (held) {
              pan.value = {
                x: held.from.x + event.clientX - held.x,
                y: held.from.y + event.clientY - held.y,
              };
            }
          },
          onPointerup: () => {
            drag = null;
          },
        }, [
          failed.value
            ? h("p", { role: "status" }, t("chat.image_unavailable"))
            : h("img", {
              src: viewerProps.src,
              alt: viewerProps.name,
              draggable: "false",
              style: { transform: `translate(${pan.value.x}px, ${pan.value.y}px) scale(${scale.value})` },
              onLoad: (event: Event) => {
                const img = event.currentTarget as HTMLImageElement;
                size.value = [img.naturalWidth, img.naturalHeight];
              },
              onError: () => { failed.value = true; },
            }),
        ]),
        h("div", { class: "image-preview-hint" }, t("chat.image_preview_hint")),
      ]),
    ]);
  },
});
</script>

<template>
  <button
    type="button"
    :class="['image-trigger', className]"
    :disabled="!src"
    data-action-click="image.open"
    data-action-mousedown="image.open"
    :aria-label="label"
    :title="label"
    @click="open = true"
    @mousedown="arm"
  >
    <slot />
  </button>
  <ImageViewer v-if="open && src" :src="src" :name="name" @close="open = false" />
</template>
