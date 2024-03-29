<script setup lang="ts">
import { ref, watch, type Ref } from "vue";
import { NBadge, NButton, NCard, NConfigProvider, NGlobalStyle, NTabPane, NTabs } from "naive-ui";

import { parseMessage, type ServerInfoMessage } from "./messages.js";
import OutputTab from "./OutputTab.vue";
import BuildProgressTab from "./BuildProgressTab.vue";
import YarnServeTab from "./YarnServeTab.vue";

const outputTab: Ref<typeof OutputTab | null> = ref(null);
const buildProgressTab: Ref<typeof BuildProgressTab | null> = ref(null);
const yarnServeTab: Ref<typeof YarnServeTab | null> = ref(null);

const appUrl = ref("");


// Favicon management

const faviconLink = document.getElementById("favicon")! as HTMLLinkElement;
const faviconMode = ref<"error" | "neutral" | "success" | "warning" | "working">("neutral");

import faviconUrlError from "./assets/favicon_error.ico";
import faviconUrlNeutral from "./assets/favicon_neutral.ico";
import faviconUrlSuccess from "./assets/favicon_success.ico";
import faviconUrlWarning from "./assets/favicon_warning.ico";
import faviconUrlWorking from "./assets/favicon_working.ico";

const faviconUrls = {
  error: faviconUrlError,
  neutral: faviconUrlNeutral,
  success: faviconUrlSuccess,
  warning: faviconUrlWarning,
  working: faviconUrlWorking,
};

watch(faviconMode, (newMode) => {
  faviconLink.href = faviconUrls[newMode];
});


// Handling the websocket

const wsproto = document.location.protocol == "http:" ? "ws:" : "wss:";
const socket = new WebSocket(`${wsproto}//${document.location.host}/ws`);

socket.addEventListener("message", (event) => {
  try {
    const structured = JSON.parse(event.data);
    const msg = parseMessage(structured);

    if (msg.hasOwnProperty("phase_started")) {
      buildProgressTab.value?.onPhaseStarted(msg);
    } else if (msg.hasOwnProperty("command_launched")) {
      buildProgressTab.value?.onCommandLaunched(msg);
    } else if (msg.hasOwnProperty("tool_output")) {
      buildProgressTab.value?.onToolOutput(msg);
    } else if (msg.hasOwnProperty("note")) {
      outputTab.value?.onNote(msg);
      buildProgressTab.value?.onNote(msg);
    } else if (msg.hasOwnProperty("warning")) {
      outputTab.value?.onWarning(msg);
      buildProgressTab.value?.onWarning(msg);
    } else if (msg.hasOwnProperty("error")) {
      outputTab.value?.onError(msg);
      buildProgressTab.value?.onError(msg);
    } else if (msg.hasOwnProperty("yarn_serve_output")) {
      yarnServeTab.value?.onYarnServeOutput(msg);
    } else if (msg.hasOwnProperty("input_debug_output")) {
      outputTab.value?.onInputDebugOutput(msg);
    } else if (msg.hasOwnProperty("build_complete")) {
      outputTab.value?.onBuildComplete(msg);
      buildProgressTab.value?.onBuildComplete(msg);
    } else if (msg.hasOwnProperty("build_started")) {
      outputTab.value?.onBuildStarted(msg);
      buildProgressTab.value?.onBuildStarted(msg);
    } else if (msg === "server_quitting") {
      buildProgressTab.value?.onServerQuitting(msg);
      yarnServeTab.value?.onServerQuitting(msg);
    } else if (msg.hasOwnProperty("server_info")) {
      onServerInfo(msg as ServerInfoMessage);
    } else {
      console.warn("recognized but unhandled message:", msg);
    }
  } catch (e) {
    console.warn(e);
  }
});


// Error/warning summaries. It appears that the "tab-pane" components need to be
// located in this file, so we have to propagate the information up from *Tab
// components.

export interface BadgeInfo {
  kind: "error" | "warning" | "info" | "success";
  value: number | string;
  processing: boolean;
}

const outputBadge = ref<BadgeInfo>({ kind: "info", value: 0, processing: false });
const progressBadge = ref<BadgeInfo>({ kind: "info", value: 0, processing: false });

function onUpdateOutputBadge(kind: "error" | "warning" | "info" | "success", value: number | string, processing: boolean) {
  outputBadge.value.value = value;
  outputBadge.value.kind = kind;
  outputBadge.value.processing = processing;
}

function onUpdateProgressBadge(kind: "error" | "warning" | "info" | "success", value: number | string, processing: boolean) {
  progressBadge.value.value = value;
  progressBadge.value.kind = kind;
  progressBadge.value.processing = processing;

  if (kind == "error") {
    faviconMode.value = "error";
  } else if (kind == "warning") {
    faviconMode.value = "warning";
  } else if (kind == "success" && !processing) {
    faviconMode.value = "success";
  } else if (processing) {
    faviconMode.value = "working";
  }
}

// Actions

function onTrigger() {
  socket.send("trigger_build");
}

function onQuit() {
  socket.send("quit");
}

function onDebugInput(path: string) {
  socket.send(`debug_input:${path}`);
}

function onServerInfo(msg: ServerInfoMessage) {
  const addr = new URL(document.location.toString());
  addr.port = `${msg.server_info.app_port}`;
  appUrl.value = addr.href;
}
</script>

<template>
  <n-config-provider>
    <n-global-style />
    <n-card title="Tectonopedia Build UI">
      <template #header-extra>
        <nav>
          <div class="app-link" v-if="appUrl">
            <b>App URL:</b> <a :href="appUrl" target="_blank">{{ appUrl }}</a>
          </div>
          <n-button @click="onTrigger" strong secondary type="info">Trigger Build</n-button>
          <n-button @click="onQuit" strong secondary type="error">Quit</n-button>
        </nav>
      </template>

      <n-tabs type="card" size="large">
        <n-tab-pane name="output" display-directive="show">
          <template #tab>
            <n-badge :value="outputBadge.value" :type="outputBadge.kind" :processing="progressBadge.processing">
              <span class="tablabel">Outputs</span>
            </n-badge>
          </template>
          <output-tab ref="outputTab" @updateBadge="onUpdateOutputBadge" @debugInput="onDebugInput"/>
        </n-tab-pane>

        <n-tab-pane name="progress" tab="Build Progress" display-directive="show">
          <template #tab>
            <n-badge :value="progressBadge.value" :type="progressBadge.kind" :processing="progressBadge.processing">
              <span class="tablabel">Progress</span>
            </n-badge>
          </template>
          <build-progress-tab ref="buildProgressTab" @updateBadge="onUpdateProgressBadge" />
        </n-tab-pane>

        <n-tab-pane name="yarn-serve" tab="yarn serve" display-directive="show">
          <yarn-serve-tab ref="yarnServeTab" />
        </n-tab-pane>
      </n-tabs>
    </n-card>
  </n-config-provider>
</template>

<style scoped>
nav {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 1rem;
}

.app-link {
  font-size: larger;
}

.tablabel {
  padding-right: 8px;
}
</style>
