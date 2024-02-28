<script setup lang="ts">
import { ref, type Ref } from "vue";
import { NBadge, NButton, NCard, NConfigProvider, NGlobalStyle, NTabPane, NTabs } from "naive-ui";

import { parseMessage, type ServerInfoMessage } from "./messages.js";
import OutputTab from "./OutputTab.vue";
import BuildProgressTab from "./BuildProgressTab.vue";
import YarnServeTab from "./YarnServeTab.vue";

const outputTab: Ref<typeof OutputTab | null> = ref(null);
const buildProgressTab: Ref<typeof BuildProgressTab | null> = ref(null);
const yarnServeTab: Ref<typeof YarnServeTab | null> = ref(null);

const appUrl = ref("");

// Handling the websocket

const socket = new WebSocket(`ws://${window.location.host}/ws`);

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
    } else if (msg.hasOwnProperty("build_complete")) {
      buildProgressTab.value?.onBuildComplete(msg);
    } else if (msg === "build_started") {
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
  kind: "error" | "warning" | "info";
  value: number;
}

const outputBadge = ref<BadgeInfo>({ kind: "info", value: 0 });

function onUpdateOutputBadge(kind: "error" | "warning" | "info", value: number) {
  outputBadge.value.value = value;
  outputBadge.value.kind = kind;
}

// Actions

function onTrigger() {
  socket.send("trigger_build");
}

function onQuit() {
  socket.send("quit");
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
        <n-tab-pane name="output" tab="Build Outputs" display-directive="show">
          <template #tab>
            <n-badge :value="outputBadge.value" :type="outputBadge.kind">
              <span class="tablabel">Build Outputs</span>
            </n-badge>
          </template>
          <output-tab ref="outputTab" @updateBadge="onUpdateOutputBadge" />
        </n-tab-pane>

        <n-tab-pane name="progress" tab="Build Progress" display-directive="show">
          <build-progress-tab ref="buildProgressTab" />
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
