<template>
  <div>
    <div id="page-wrapper" class="page-wrapper">
      <div class="page">
        <div id="menu-bar-hover-placeholder"></div>
        <div id="menu-bar" class="menu-bar sticky bordered">
          <div class="left-buttons">
            <button class="icon-button" type="button">
              <FontAwesomeIcon icon="fa-solid fa-bars" />
            </button>
          </div>

          <h1 class="menu-title" v-text="title"></h1>
        </div>

        <div id="content" class="content">
          <div class="main" v-show="searchActivated">
            <SearchWidget ref="searchWidget"></SearchWidget>
          </div>

          <main id="main" class="main" v-html="content"></main>
        </div>
      </div>
    </div>

    <!-- <ModalManager ref="modalManager" @gotoModule="onGotoModule" @startPaging="onStartPaging"
      @showCurrentModInfo="onShowCurrentModInfo"></ModalManager> -->
  </div>

</template>

<style src="./style.scss">

</style>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import SearchWidget from "./SearchWidget.vue";

const props = defineProps({
  content: { type: String, required: true },
  title: { type: String, required: true },
});

const searchWidget = ref();
const searchActivated = ref(false);

// Global keybindings

function noModifiers(event: KeyboardEvent): boolean {
  // NB, currently not checking shiftKey
  return !(event.altKey || event.ctrlKey || event.metaKey);
}

const keydownHandlers = {
  "/": (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      event.preventDefault();
      searchActivated.value = true;
      searchWidget.value?.activate();
    }
  },
};

function onKeydown(event: KeyboardEvent) {
  const handler = keydownHandlers[event.key];
  if (handler !== undefined) {
    handler(event);
  }
}

function mountKeybindings() {
  window.addEventListener("keydown", onKeydown);
}

function unmountKeybindings() {
  window.removeEventListener("keydown", onKeydown);
}

// The hooks

onMounted(() => {
  mountKeybindings();
});

onUnmounted(() => {
  unmountKeybindings();
});

</script>