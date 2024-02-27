<script setup lang="ts">
// The tab reporting overall progress from the active build.
import { ref, type Ref } from "vue";

import type {
  AlertMessage,
  BuildCompleteMessage,
  BuildStartedMessage,
  CommandLaunchedMessage,
  ErrorMessage,
  NoteMessage,
  PhaseStartedMessage,
  ServerQuittingMessage,
  ToolOutputMessage,
  WarningMessage,
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

function onToolOutput(msg: ToolOutputMessage) {
  const text = msg.tool_output.lines.join("\n") + "\n";

  if (msg.tool_output.stream == "stderr") {
    appendSpan("error", text);
  } else {
    appendSpan("default", text);
  }
}

function onAlert(cls: string, prefix: string, msg: AlertMessage) {
  let text = `${prefix}: ${msg.message}`;

  if (msg.context.length > 0) {
    text += "\n  " + msg.context.join("\n  ");
  }

  text += "\n";
  appendSpan(cls, text);
}

function onNote(msg: NoteMessage) {
  onAlert("default", "note", msg.note);
}

function onWarning(msg: WarningMessage) {
  onAlert("warning", "warning", msg.warning);
}

function onError(msg: ErrorMessage) {
  onAlert("error", "error", msg.error);
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

defineExpose({
  onBuildComplete,
  onBuildStarted,
  onCommandLaunched,
  onError,
  onNote,
  onPhaseStarted,
  onServerQuitting,
  onToolOutput,
  onWarning,
});
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

.warning {
  font-weight: bold;
  color: rgb(222, 210, 46);
}

.error {
  font-weight: bold;
  color: rgb(228, 92, 92);
}
</style>
