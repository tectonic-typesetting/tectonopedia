<script setup lang="ts">
import { onMounted } from "vue";

onMounted(() => {
  const socket = new WebSocket(`ws://${window.location.host}/ws`);

  socket.addEventListener("open", (_event) => {
    console.log("connected!");
  });

  socket.addEventListener("message", (event) => {
    console.log("Message:", event.data);
  });
});
</script>

<template>
  <p>Hello world.</p>
</template>

<style scoped></style>
