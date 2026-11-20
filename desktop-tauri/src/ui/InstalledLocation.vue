<script setup lang="ts">
import { toValue, watch, type MaybeRefOrGetter } from "vue";

type Target = { kind: string; name: string } | null;

// The market hands over an applied action. Wait for the installed list to
// refresh before focusing its row; a tab switch can render before that read.
const props = defineProps<{
  root: HTMLDivElement | null;
  target: MaybeRefOrGetter<Target>;
  packages: MaybeRefOrGetter<readonly unknown[]>;
  skills: MaybeRefOrGetter<readonly unknown[]>;
  mcp: MaybeRefOrGetter<readonly unknown[]>;
  refreshing: MaybeRefOrGetter<boolean>;
  onFound: (v: Target | ((cur: Target) => Target)) => void;
}>();

watch([() => toValue(props.target), () => toValue(props.packages), () => toValue(props.skills), () => toValue(props.mcp), () => toValue(props.refreshing), () => props.root], ([target, , , , refreshing]) => {
  if (!target || refreshing) return;
  const group = props.root?.querySelector<HTMLElement>(target.kind === "skill" ? "#set-skills" : target.kind === "mcp" ? "#set-mcp" : "#set-plugins");
  group?.scrollIntoView?.({ block: "start" });
  const action = target.kind === "skill" ? "skill.enabled" : "mcp.enabled";
  const row = target.kind === "plugin" || target.kind === "theme"
    ? [...(group?.querySelectorAll<HTMLDetailsElement>("[data-extension-name]") ?? [])].find((el) => el.dataset.extensionName === target.name)?.querySelector<HTMLElement>("summary")
    : [...(group?.querySelectorAll<HTMLElement>("[data-action]") ?? [])].find((el) => el.dataset.action === action && el.dataset.target === target.name);
  if (row) {
    row.focus();
    props.onFound(null);
  } else {
    props.root?.querySelector<HTMLElement>("[data-action='extensions.tab'][data-value='installed']")?.focus();
    props.onFound(null);
  }
}, { immediate: true, flush: "post" });
</script>

<template></template>