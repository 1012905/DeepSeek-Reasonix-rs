<script setup lang="ts">
import { NConfigProvider, NButton } from "naive-ui";
import { useKernelStore } from "./stores/kernel";

const kernel = useKernelStore();
</script>

<template>
  <n-config-provider>
    <n-button>{{ kernel.statusText }}</n-button>
  </n-config-provider>
</template>
