<script setup lang="ts">
// The tab reporting overall progress from the active build.
import { ref, type Ref } from "vue";

import type {
  BuildCompleteMessage,
  BuildStartedMessage,
  CommandLaunchedMessage,
  PhaseStartedMessage,
  ServerQuittingMessage
} from "./messages.js";

// Styled chunks of log content

interface Span {
  cls: string,
  content: string,
}

const spans: Ref<Span[]> = ref([]);

function appendSpan(cls: string, content: string) {
  const n = spans.value.length;

  if (n > 0 && spans.value[n - 1].cls == cls) {
    spans.value[n - 1].content += content;
  } else {
    spans.value.push({ cls, content });
  }
}

// Events


function onBuildStarted(_msg: BuildStartedMessage) {
  spans.value = [];
  appendSpan("success", `Starting new build at ${new Date().toISOString()}\n`);
}

function onPhaseStarted(msg: PhaseStartedMessage) {
  appendSpan("default", `\n→ phase ${msg.phase_started}\n`);
}

function onCommandLaunched(msg: CommandLaunchedMessage) {
  appendSpan("default", `\n→ launching \`${msg.command_launched}\` ...\n`);
}

function onBuildComplete(msg: BuildCompleteMessage) {
  const e = msg.build_complete.elapsed.toFixed(1);

  if (msg.build_complete.success) {
    appendSpan("success", `\nBuild successful in ${e} seconds\n`);
  } else {
    appendSpan("error", `\nBuild failed after ${e} seconds\n`);
  }
}

function onServerQuitting(_msg: ServerQuittingMessage) {
  appendSpan("default", "\n(server quitting)");
}

defineExpose({ onBuildStarted, onPhaseStarted, onCommandLaunched, onBuildComplete, onServerQuitting });
</script>

<template>
  <p>Progress of the current (or most recent) build operation.</p>

  <pre class="log"><code ref="code" v-for="s in spans"><span :class="s.cls">{{ s.content }}</span></code></pre>
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
