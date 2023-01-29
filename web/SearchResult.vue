<template>
  <a :href="url">
    <h1 v-text="title"></h1>
    <p v-text="snippet"></p>
  </a>
</template>

<style lang="scss" scoped>
a {
  color: var(--fg);
  text-decoration: none;

  &:hover {
    color: var(--fg);
  }

  &:visited {
    color: var(--fg);
  }
}

h1 {
  margin: 0.1rem 0;
  font-size: 100%;
}

p {
  margin: 0.1rem 0 0 0;
}
</style>

<script setup lang="ts">
//import { ref } from "vue";

defineProps({
  title: { type: String, required: true },
  snippet: { type: String, required: true },
  url: { type: String, required: true },
});
</script>