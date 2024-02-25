<script setup lang="ts">
import { ref, type Ref } from "vue";
import { NButton, NCard, NConfigProvider, NGlobalStyle, NTabPane, NTabs } from "naive-ui";

import { parseMessage } from "./messages.js";
import YarnServeTab from "./YarnServeTab.vue";

const yarnServeTab: Ref<typeof YarnServeTab | null> = ref(null);

const socket = new WebSocket(`ws://${window.location.host}/ws`);

socket.addEventListener("message", (event) => {
  try {
    const structured = JSON.parse(event.data);
    const msg = parseMessage(structured);

    if (msg.hasOwnProperty("yarn_output")) {
      yarnServeTab.value?.onYarnOutput(msg);
    } else {
      console.warn("recognized but unhandled message:", msg);
    }
  } catch (e) {
    console.warn(e);
  }
});

function onQuit() {
  socket.send("quit");
}

</script>

<template>
  <n-config-provider>
    <n-global-style />
    <n-card title="Tectonopedia Build UI">
      <template #header-extra>
        <nav>
          <n-button @click="onQuit">Quit</n-button>
        </nav>
      </template>

      <n-tabs type="card" size="large">
        <n-tab-pane name="build" tab="Build">
          <p>The build!</p>
        </n-tab-pane>

        <n-tab-pane name="yarn-serve" tab="yarn serve">
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
}
</style>
