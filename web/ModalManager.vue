<template>
  <div>
    <div :class="{
      'modal-overlay': true,
      'modal-overlay-visible': active != ModalKind.None,
    }"></div>

    <div :class="{
      'modal-wrapper': true,
      'modal-wrapper-visible': active != ModalKind.None,
    }">
      <!-- This has the same layout as the app's menu bar, to provide a nice
        title for the modal that matches the high-level page layout -->
      <div id="modal-menu-bar" class="modal-menu-bar">
        <div class="left-buttons"></div>

        <h1 class="modal-menu-title" v-text="modalTitle"></h1>

        <div class="right-buttons">
          <button type="button" @click="clear" class="close-button" title="Close overlay" aria-label="Close overlay">
            ×
          </button>
        </div>
      </div>

      <!-- The "dispatch" modal is basically the "main menu" of the doc app that
        provides access to all of its functionality. We need to provide a way to
        get at everything without using a keyboard for mobile. -->
      <div v-show="active == ModalKind.Dispatch" class="modal-container page-wrapper">
        <DispatchModal @do-modal="onDoModal"></DispatchModal>
      </div>

      <!-- The "help" modal shows help on using the app. -->
      <div v-show="active == ModalKind.Help" class="modal-container page-wrapper">
        <HelpModal></HelpModal>
      </div>

      <!-- The "search" model provides access to the search UI. -->
      <div v-show="active == ModalKind.Search" class="modal-container page-wrapper">
        <SearchModal ref="search" :relTop="relTop"></SearchModal>
      </div>
    </div>
  </div>
</template>

<style lang="scss" scoped>
// Derived from
// https://rapaccinim.medium.com/how-to-create-a-custom-resizable-modal-with-scrollable-and-fixed-content-21adb2adda28

.modal-overlay {
  display: none;
  position: fixed;
  top: 0;
  left: 0;
  z-index: 200;
  width: 100%;
  height: 100%;
  background-color: rgba(255, 255, 255, 0.9);
  backdrop-filter: blur(1px);

  &.modal-overlay-visible {
    display: block;
  }
}

.modal-wrapper {
  display: none;
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  height: 100%;
  padding: 0 var(--page-padding);

  z-index: 201;

  &.modal-wrapper-visible {
    display: block;
  }
}

.modal-container {
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

#modal-menu-bar {
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

  // Modals "blue pages", with a faint blue pattern
  // in the menu bar.
  background: repeating-linear-gradient(120deg,
      var(--bg),
      var(--bg) 4px,
      #f6fbff 4px,
      #f6fbff 8px);
}

.modal-menu-title {
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
import { ModalKind } from "./base";
import DispatchModal from "./DispatchModal.vue";
import HelpModal from "./HelpModal.vue";
import SearchModal from "./SearchModal.vue";

defineProps({
  relTop: { type: String, required: true },
});

const active = ref(ModalKind.None);
const search = ref();

const modalTitle = computed(() => {
  switch (active.value) {
    case ModalKind.None: return "";
    case ModalKind.Dispatch: return "Main Menu";
    case ModalKind.Help: return "Help";
    case ModalKind.Search: return "Search";
  }
});

function clear() {
  active.value = ModalKind.None;
}

function toggleBasic(kind: ModalKind): boolean {
  if (active.value == kind) {
    active.value = ModalKind.None;
    return false;
  } else {
    active.value = kind;
    return true;
  }
}

function toggleDispatch() {
  toggleBasic(ModalKind.Dispatch);
}

function toggleHelp() {
  toggleBasic(ModalKind.Help);
}

function toggleSearch() {
  if (toggleBasic(ModalKind.Search)) {
    search.value?.activate();
  }
}

function onDoModal(kind: ModalKind) {
  active.value = kind;
}

defineExpose({
  clear,
  toggleDispatch,
  toggleHelp,
  toggleSearch,
});
</script>
