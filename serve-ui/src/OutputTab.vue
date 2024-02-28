<script setup lang="ts">
// The tab reporting build output associated with individual source files.
import { ref, type Ref } from "vue";
import { type MenuOption, NMenu, NSplit } from "naive-ui";

import type {
  AlertMessage,
  BuildStartedMessage,
  ErrorMessage,
  NoteMessage,
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

// Managing file selection

const selected = ref<string | null>(null);

const menuItems: MenuOption[] = [
  {
    label: "txt/foo.tex",
    key: "txt/foo.tex",
  }
];

// Events

function onBuildStarted(_msg: BuildStartedMessage) {
  spans.value = [];
  appendSpan("success", `Starting new build at ${new Date().toISOString()}\n`);
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

defineExpose({
  onBuildStarted,
  onError,
  onNote,
  onWarning,
});
</script>

<template>
  <p>Build output associated with each source file.</p>

  <n-split direction="horizontal" :default-size="0.2">
    <template #1>
      <n-menu v-model:value="selected" :options="menuItems" />
    </template>
    <template #2>
      <pre class="log"><code ref="code" v-for="s in spans"><span :class="s.cls">{{ s.content }}</span></code></pre>
    </template>
  </n-split>
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
