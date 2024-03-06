<script setup lang="ts">
// The tab reporting build output associated with individual source files.
import { computed, h, ref, watch } from "vue";
import { NBadge, NMenu, NSplit } from "naive-ui";

import type {
  AlertMessage,
  BuildCompleteMessage,
  BuildStartedMessage,
  ErrorMessage,
  NoteMessage,
  WarningMessage,
} from "./messages.js";

import { SpanSet } from "./spanset.js";

// Per-file state

const ProcState = {
  Initial: 0,
  Processing: 1,
  Complete: 2,
} as const;

type ProcState = typeof ProcState[keyof typeof ProcState];

class FileData {
  content: SpanSet;
  proc_state: ProcState;
  n_warnings: number;
  n_errors: number;

  constructor() {
    this.content = new SpanSet();
    this.proc_state = ProcState.Initial;
    this.n_warnings = 0;
    this.n_errors = 0;
  }

  render(name: string) {
    let badge_value;
    let badge_type;
    let badge_dot = false;
    let badge_processing = false;
    let badge_show = true;
    let badge_color = undefined;

    if (this.n_errors) {
      badge_value = this.n_errors;
      badge_type = "error";
    } else if (this.n_warnings) {
      badge_value = this.n_warnings;
      badge_type = "warning";
    } else if (this.proc_state == ProcState.Complete) {
      badge_value = "";
      badge_type = "success";
      badge_dot = true;
    } else if (this.proc_state == ProcState.Processing) {
      badge_value = "";
      badge_type = "info";
      badge_dot = true;
      badge_processing = true;
    } else {
      badge_value = "";
      badge_type = "info";
      badge_show = false;
      badge_color = "gray";
    }

    return h(
      NBadge,
      {
        value: badge_value,
        "type": badge_type as any,
        processing: badge_processing,
        dot: badge_dot,
        show: badge_show,
        color: badge_color,
        offset: [12, 12],
      },
      [
        h("span", {}, name),
      ]
    );
  }
}

const files = ref<Map<string, FileData>>(new Map());

const noFileContent = new SpanSet();
noFileContent.append("default", "(no file selected)");

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
      key: n,
      label: () => fd?.render(n),
    }
  });
});


// Managing file selection

const selected = ref<string | null>(null);

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


// Summary stats for the tab-level badge -- we need to propagate this info via
// an event.

const totalWarnings = ref(0);
const totalErrors = ref(0);
const totalProcState = ref<ProcState>(ProcState.Initial);

const emit = defineEmits<{
  updateBadge: [kind: "error" | "warning" | "info" | "success", value: number | string, processing: boolean]
}>();

watch([totalWarnings, totalErrors, totalProcState], ([totWarn, totErr, procState]) => {
  const isProc = (procState == ProcState.Processing);

  if (totErr > 0) {
    emit("updateBadge", "error", totErr, isProc);
  } else if (totWarn > 0) {
    emit("updateBadge", "warning", totWarn, isProc);
  } else if (procState == ProcState.Complete) {
    emit("updateBadge", "success", "✓", isProc);
  } else {
    emit("updateBadge", "info", 0, isProc);
  }
}, { immediate: true })


// Events

function onBuildStarted(_msg: BuildStartedMessage) {
  files.value.clear();
  totalWarnings.value = 0;
  totalErrors.value = 0;
  totalProcState.value = ProcState.Processing;
}

function onBuildComplete(_msg: BuildCompleteMessage) {
  totalProcState.value = ProcState.Complete;
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
    totalWarnings.value++;
  } else if (cls == "error") {
    fdata.n_errors++;
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

defineExpose({
  onBuildComplete,
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
