<script setup lang="ts">
// The tab with output from the `yarn serve` process.
import { ref, type Ref } from "vue";
import { NLog } from "naive-ui";

import type { ServerQuittingMessage, YarnOutputMessage } from "./messages.js";

const lines: Ref<string[]> = ref([]);

function onYarnOutput(msg: YarnOutputMessage) {
  lines.value.push(...msg.yarn_output.lines);
}

function onServerQuitting(_msg: ServerQuittingMessage) {
  // The log component seems to elide empty lines, annoyingly.
  lines.value.push(" ", "(server quitting)");
}

defineExpose({ onServerQuitting, onYarnOutput });
</script>

<template>
  <p>The output from <code>yarn serve</code>.</p>

  <n-log class="log" :lines="lines" />
</template>

<style scoped>
.log {
  width: 100%;
  padding: 5px;
  color: #FFF;
  background-color: #000;
}
</style>
