<template>
  <div>
    <ToolManager ref="toolManager" :relTop="relTop"></ToolManager>

    <div class="page-wrapper">
      <div class="page">
        <!-- This element hangs out at the top of the window invisibly, so that we can
          detect when the user hovers the mouse there and reveal the menu bar -->
        <div id="menu-bar-hover-placeholder"></div>

        <div ref="menuBar" id="menu-bar" class="menu-bar sticky bordered">
          <div class="left-buttons">
            <button class="icon-button" type="button" @click="onDispatchClicked">
              <FontAwesomeIcon icon="fa-solid fa-bars" />
            </button>
          </div>

          <h1 class="menu-title" v-text="bookName"></h1>

          <div class="right-buttons">
          </div>
        </div>

        <div id="content" class="content">
          <main id="main" class="main" v-html="content"></main>
        </div>
      </div>
    </div>
  </div>

</template>

<style src="./style.scss">

</style>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import ToolManager from "./ToolManager.vue";

const props = defineProps({
  content: { type: String, required: true },
  title: { type: String, required: true },
  bookName: { type: String, required: true },
  relTop: { type: String, required: true },
});

const menuBar = ref();
const toolManager = ref();

// Local event handlers

function onDispatchClicked() {
  toolManager.value?.toggleDispatch();
}

// Global keybindings

function noModifiers(event: KeyboardEvent): boolean {
  // NB, currently not checking shiftKey
  return !(event.altKey || event.ctrlKey || event.metaKey);
}

const keydownHandlers = {
  "/": (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      event.preventDefault();
      toolManager.value?.toggleSearch();
    }
  },

  "?": (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      event.preventDefault();
      toolManager.value?.toggleHelp();
    }
  },

  Escape: (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      event.preventDefault();
      toolManager.value?.clear();
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

// Scroll monitoring for sticky menu bar
//
// This is all more-or-less copied out of mdBook.

var onScroll = null;

function mountScroll() {
  var scrollTop = document.scrollingElement.scrollTop;
  var prevScrollTop = scrollTop;
  var minMenuY = -menuBar.value?.clientHeight - 50;

  // When the script loads, the page can be at any scroll
  menuBar.value!.style.top = scrollTop + 'px';
  var topCache = menuBar.value?.style.top.slice(0, -2);

  menuBar.value!.classList.remove('sticky');
  var stickyCache = false;

  onScroll = function onScroll(event: Event) {
    const scrollTop = Math.max(document.scrollingElement.scrollTop, 0);

    // `null` means that it doesn't need to be updated
    var nextSticky = null;
    var nextTop = null;
    var scrollDown = scrollTop > prevScrollTop;
    var menuPosAbsoluteY = topCache - scrollTop;

    if (scrollDown) {
      nextSticky = false;
      if (menuPosAbsoluteY > 0) {
        nextTop = prevScrollTop;
      }
    } else {
      if (menuPosAbsoluteY > 0) {
        nextSticky = true;
      } else if (menuPosAbsoluteY < minMenuY) {
        nextTop = prevScrollTop + minMenuY;
      }
    }

    if (nextSticky === true && stickyCache === false) {
      menuBar.value!.classList.add('sticky');
      stickyCache = true;
    } else if (nextSticky === false && stickyCache === true) {
      menuBar.value!.classList.remove('sticky');
      stickyCache = false;
    }

    if (nextTop !== null) {
      menuBar.value!.style.top = nextTop + 'px';
      topCache = nextTop;
    }

    prevScrollTop = scrollTop;
  }

  document.addEventListener("scroll", onScroll, { passive: true });
}

function unmountScroll() {
  document.removeEventListener("scroll", onScroll);
}

// The hooks

onMounted(() => {
  mountKeybindings();
  mountScroll();
});

onUnmounted(() => {
  unmountKeybindings();
  unmountScroll();
});

</script>