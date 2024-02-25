<script setup lang="ts">
// The tab with output from the `yarn serve` process.
import { ref } from "vue";
import { NButton, NFlex } from "naive-ui";

import type { ServerQuittingMessage, YarnOutputMessage } from "./messages.js";

const log = ref("")

function onYarnOutput(msg: YarnOutputMessage) {
  const s = msg.yarn_output.lines.join("\n") + "\n";
  log.value += s;
}

function onServerQuitting(_msg: ServerQuittingMessage) {
  log.value += "\n(server quitting)";
}

function onClear() {
  log.value = "";
}

defineExpose({ onServerQuitting, onYarnOutput });
</script>

<template>
  <n-flex align="center" justify="end">
    <p class="desc">The output from <code>yarn serve</code>.</p>

    <n-button @click="onClear" strong secondary type="error">Clear Output</n-button>
  </n-flex>

  <pre class="log"><code ref="code">{{ log }}</code></pre>
</template>

<style scoped>
.desc {
  flex: 1;
}

.log {
  width: 100%;
  min-height: 10rem;
  overflow: scroll;
  padding: 5px;
  color: #FFF;
  background-color: #000;
}
</style>
