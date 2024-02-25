<script setup lang="ts">
// The tab reporting overall progress from the active build.
import { ref } from "vue";

import type {
  BuildCompleteMessage,
  BuildStartedMessage,
  CommandLaunchedMessage,
  PhaseStartedMessage,
  ServerQuittingMessage
} from "./messages.js";

const log = ref("")

function onBuildStarted(_msg: BuildStartedMessage) {
  log.value = `Starting new build at ${new Date().toISOString()}\n`;
}

function onPhaseStarted(msg: PhaseStartedMessage) {
  log.value += `\n→ phase ${msg.phase_started}\n`;
}

function onCommandLaunched(msg: CommandLaunchedMessage) {
  log.value += `\n→ launching \`${msg.command_launched}\` ...\n`;
}

function onBuildComplete(msg: BuildCompleteMessage) {
  const e = msg.build_complete.elapsed.toFixed(1);

  if (msg.build_complete.success) {
    log.value += `\n<span class="success">Build successful in ${e} seconds</span>\n`;
  } else {
    log.value += `\n<span class="error">Build failed after ${e} seconds</span>\n`;
  }
}

function onServerQuitting(_msg: ServerQuittingMessage) {
  log.value += "\n(server quitting)";
}

defineExpose({ onBuildStarted, onPhaseStarted, onCommandLaunched, onBuildComplete, onServerQuitting });
</script>

<template>
  <p>Progress of the current (or most recent) build operation.</p>

  <pre class="log"><code ref="code">{{ log }}</code></pre>
</template>

<style scoped>
.log {
  width: 100%;
  min-height: 10rem;
  overflow: scroll;
  padding: 5px;
  color: #FFF;
  background-color: #000;
}

.success {
  font-weight: bold;
  color: rgb(138, 228, 138);
}

.error {
  font-weight: bold;
  color: rgb(228, 92, 92);
}
</style>
