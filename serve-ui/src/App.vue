<script setup lang="ts">
import { NButton } from "naive-ui";

const socket = new WebSocket(`ws://${window.location.host}/ws`);

socket.addEventListener("open", (_event) => {
  console.log("connected!");
});

socket.addEventListener("message", (event) => {
  console.log("Message:", event.data);
});

function onQuit() {
  socket.send("quit");
}
</script>

<template>
  <p>Hello world.</p>

  <n-button @click="onQuit">Quit</n-button>
</template>

<style scoped></style>
