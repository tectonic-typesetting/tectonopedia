<script setup lang="ts">
// The tab reporting build output associated with individual source files.
import { computed, h, ref } from "vue";
import { NBadge, NMenu, NSplit } from "naive-ui";

import type {
  AlertMessage,
  BuildStartedMessage,
  ErrorMessage,
  NoteMessage,
  WarningMessage,
} from "./messages.js";

import { SpanSet } from "./spanset.js";

// Managing file selection

const selected = ref<string | null>(null);

class FileData {
  content: SpanSet;
  n_warnings: number;
  n_errors: number;

  constructor() {
    this.content = new SpanSet();
    this.n_warnings = 0;
    this.n_errors = 0;
  }

  renderBadge() {
    let value;
    let btype;

    if (this.n_errors) {
      value = this.n_errors;
      btype = "error";
    } else if (this.n_warnings) {
      value = this.n_warnings;
      btype = "warning";
    } else {
      return h("span");
    }

    return h(NBadge, {
      value,
      "type": btype as any,
    });
  }
}

const files = ref<Map<string, FileData>>(new Map());

const noFileContent = new SpanSet();
noFileContent.append("default", "(no file selected)");

const selectedSpans = computed(() => {
  if (selected.value === null) {
    return noFileContent;
  }

  const fdata = files.value.get(selected.value);
  if (fdata === undefined) {
    return noFileContent;
  }

  return fdata.content;
});

const menuItems = computed(() => {
  const items = Array.from(files.value.keys()).sort();

  if (!items.length) {
    return [{
      label: "(no files yet)",
      key: "",
      disabled: true,
    }];
  }

  return items.map((n) => {
    const fd = files.value.get(n);

    return {
      label: n,
      key: n,
      extra: () => fd?.renderBadge(),
    }
  });
});

// Events

function onBuildStarted(_msg: BuildStartedMessage) {
  files.value.clear();
}

function onAlert(cls: string, prefix: string, msg: AlertMessage) {
  if (!msg.file) {
    return; // Messages not associated with specific files go in the progress tab
  }

  let fdata = files.value.get(msg.file);

  if (fdata === undefined) {
    fdata = new FileData();
    files.value.set(msg.file, fdata);
  }

  let text = `${prefix}: ${msg.message}`;

  if (msg.context.length > 0) {
    text += "\n  " + msg.context.join("\n  ");
  }

  text += "\n";
  fdata.content.append(cls, text);

  // This is a litle hacky ...
  if (cls == "warning") {
    fdata.n_warnings++;
  } else if (cls == "error") {
    fdata.n_errors++;
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
      <n-menu class="filelist" v-model:value="selected" :options="menuItems" :indent="12" />
    </template>
    <template #2>
      <pre
        class="log"><code ref="code" v-for="s in selectedSpans.spans"><span :class="s.cls">{{ s.content }}</span></code></pre>
    </template>
  </n-split>
</template>

<style scoped>
.filelist {
  max-height: 75vh;
  overflow: scroll;
}

.log {
  width: 100%;
  min-height: 10rem;
  max-height: 75vh;
  overflow: scroll;
  margin-left: 8px;
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
