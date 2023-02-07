<template>
  <div>
    <div :class="{
      'tool-overlay': true,
      'tool-overlay-visible': active != ToolKind.None,
    }"></div>

    <div :class="{
      'tool-wrapper': true,
      'tool-wrapper-visible': active != ToolKind.None,
    }">
      <!-- This has the same layout as the app's menu bar, to provide a nice
        title for the tool that matches the high-level page layout -->
      <div id="tool-menu-bar" class="tool-menu-bar">
        <div class="left-buttons"></div>

        <h1 class="tool-menu-title" v-text="toolTitle"></h1>

        <div class="right-buttons">
          <button type="button" @click="clear" class="close-button" title="Close overlay" aria-label="Close overlay">
            ×
          </button>
        </div>
      </div>

      <!-- The "dispatch" tool is basically the "main menu" of the doc app that
        provides access to all of its functionality. We need to provide a way to
        get at everything without using a keyboard for mobile. -->
      <div v-show="active == ToolKind.Dispatch" class="tool-container page-wrapper">
        <DispatchTool @do-tool="onDoTool"></DispatchTool>
      </div>

      <!-- The "help" tool shows help on using the app. -->
      <div v-show="active == ToolKind.Help" class="tool-container page-wrapper">
        <HelpTool></HelpTool>
      </div>

      <!-- The "search" model provides access to the search UI. -->
      <div v-show="active == ToolKind.Search" class="tool-container page-wrapper">
        <SearchTool ref="search" :relTop="relTop"></SearchTool>
      </div>
    </div>
  </div>
</template>

<style lang="scss" scoped>
// Derived from
// https://rapaccinim.medium.com/how-to-create-a-custom-resizable-tool-with-scrollable-and-fixed-content-21adb2adda28

.tool-overlay {
  display: none;
  position: fixed;
  top: 0;
  left: 0;
  z-index: 200;
  width: 100%;
  height: 100%;
  background-color: rgba(255, 255, 255, 0.9);
  backdrop-filter: blur(1px);

  &.tool-overlay-visible {
    display: block;
  }
}

.tool-wrapper {
  display: none;
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 100%;
  padding: 0 var(--page-padding);

  z-index: 201;

  &.tool-wrapper-visible {
    display: block;
  }
}

.tool-container {
  background-color: #fff;

  // Standardize this for content scrollbox height computation.
  h1 {
    margin: 2rem 0;
    line-height: 2rem;
  }

  .content-aligned {
    margin-left: auto;
    margin-right: auto;
    max-width: var(--content-max-width);
  }
}

.close-button {
  font-size: 2em;
  border: none;
  border-radius: 5px;
  height: var(--menu-bar-height);

  background: none;
  color: var(--icons);

  &:hover {
    cursor: pointer;
    color: var(--icons-hover);
  }
}

// Not great -- duplicating the main app menu bar as specified in style.scss

#tool-menu-bar {
  margin: auto calc(0px - var(--page-padding));

  position: relative;
  display: flex;
  flex-wrap: wrap;
  background-color: var(--bg);
  border-bottom-color: var(--bg);
  border-bottom-width: 1px;
  border-bottom-style: solid;

  border-bottom-color: var(--table-border-color);

  color: var(--icons);

  // Tools "blue pages", with a faint blue pattern
  // in the menu bar.
  background: repeating-linear-gradient(120deg,
      var(--bg),
      var(--bg) 4px,
      #f6fbff 4px,
      #f6fbff 8px);
}

.tool-menu-title {
  display: inline-block;
  font-family: tduxSans;
  font-weight: 400;
  font-size: 1.3rem;
  line-height: var(--menu-bar-height);
  text-align: center;
  margin: 0;
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>

<script setup lang="ts">
import { computed, ref } from "vue";
import { ToolKind } from "./base";
import DispatchTool from "./DispatchTool.vue";
import HelpTool from "./HelpTool.vue";
import SearchTool from "./SearchTool.vue";

defineProps({
  relTop: { type: String, required: true },
});

const active = ref(ToolKind.None);
const search = ref();

const toolTitle = computed(() => {
  switch (active.value) {
    case ToolKind.None: return "";
    case ToolKind.Dispatch: return "Main Menu";
    case ToolKind.Help: return "Help";
    case ToolKind.Search: return "Search";
  }
});

function clear() {
  active.value = ToolKind.None;
}

function toggleBasic(kind: ToolKind): boolean {
  if (active.value == kind) {
    active.value = ToolKind.None;
    return false;
  } else {
    active.value = kind;
    return true;
  }
}

function toggleDispatch() {
  toggleBasic(ToolKind.Dispatch);
}

function toggleHelp() {
  toggleBasic(ToolKind.Help);
}

function toggleSearch() {
  if (toggleBasic(ToolKind.Search)) {
    search.value?.activate();
  }
}

function onDoTool(kind: ToolKind) {
  active.value = kind;
}

defineExpose({
  clear,
  toggleDispatch,
  toggleHelp,
  toggleSearch,
});
</script>
