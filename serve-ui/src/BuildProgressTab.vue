<script setup lang="ts">
// The tab reporting overall progress from the active build.
import { ref, watch } from "vue";

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

const spans = ref<Span[]>([]);

function appendSpan(cls: string, content: string) {
  const n = spans.value.length;

  if (n > 0 && spans.value[n - 1].cls == cls) {
    spans.value[n - 1].content += content;
  } else {
    spans.value.push({ cls, content });
  }
}


// Summary stats for the tab-level badge -- we need to propagate this info via
// an event.

const totalWarnings = ref(0);
const totalErrors = ref(0);
const isProcessing = ref(false);

const emit = defineEmits<{
  updateBadge: [kind: "error" | "warning" | "info", value: number, processing: boolean]
}>();

watch([totalWarnings, totalErrors, isProcessing], ([totWarn, totErr, isProc]) => {
  if (totErr > 0) {
    emit("updateBadge", "error", totErr, isProc);
  } else if (totWarn > 0) {
    emit("updateBadge", "warning", totWarn, isProc);
  } else {
    emit("updateBadge", "info", 0, isProc);
  }
}, { immediate: true })


// Events

function onBuildStarted(_msg: BuildStartedMessage) {
  spans.value = [];
  totalWarnings.value = 0;
  totalErrors.value = 0;
  isProcessing.value = true;
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
  if (!!msg.file) {
    return; // Messages associated with specific files go in the output tab
  }

  let text = `${prefix}: ${msg.message}`;

  if (msg.context.length > 0) {
    text += "\n  " + msg.context.join("\n  ");
  }

  text += "\n";
  appendSpan(cls, text);

  // This is a litle hacky ...
  if (cls == "warning") {
    totalWarnings.value++;
  } else if (cls == "error") {
    totalErrors.value++;
  }
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

  isProcessing.value = false;
}

function onServerQuitting(_msg: ServerQuittingMessage) {
  appendSpan("default", "\n(server quitting)");
  isProcessing.value = false;
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
