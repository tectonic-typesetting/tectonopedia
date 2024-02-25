<script setup lang="ts">
import { NButton, NCard, NConfigProvider, NGlobalStyle, NTabPane, NTabs } from "naive-ui";
import { parseMessage } from "./messages.js";

const socket = new WebSocket(`ws://${window.location.host}/ws`);

socket.addEventListener("message", (event) => {
  try {
    const structured = JSON.parse(event.data);
    const msg = parseMessage(structured);
    console.log("parsed OK:", msg);
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
          <p>The output from <code>yarn serve</code>.</p>
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
