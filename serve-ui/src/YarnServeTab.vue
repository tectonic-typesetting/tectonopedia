<script setup lang="ts">
// The tab with output from the `yarn serve` process.
import { ref } from "vue";
import { NLog } from "naive-ui";

import { type YarnOutputMessage } from "./messages.js";

const log = ref("");

function onYarnOutput(msg: YarnOutputMessage) {
  for (const line of msg.yarn_output.lines) {
    log.value += line + "\n";
  }
}

defineExpose({ onYarnOutput });
</script>

<template>
  <p>The output from <code>yarn serve</code>.</p>

  <n-log class="log" :log="log" />
</template>

<style scoped>
.log {
  width: 100%;
  padding: 5px;
  color: #FFF;
  background-color: #000;
}
</style>
