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
        <SearchModal></SearchModal>
      </div>

      <button type="button" @click="clear" class="close-button" title="Close overlay" aria-label="Close overlay">
        ×
      </button>
    </div>
  </div>
</template>

<style lang="scss">
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
  position: fixed;
  top: 0.5rem;
  right: 0.5rem;
  font-size: 2em;
  width: 3rem;
  height: 3rem;
  border: none;
  border-radius: 5px;

  background-color: #fff;
  color: var(--icons);

  &:hover {
    cursor: pointer;
    color: var(--icons-hover);
  }
}
</style>

<script setup lang="ts">
import { ref } from "vue";
import { ModalKind } from "./base";
import DispatchModal from "./DispatchModal.vue";
import HelpModal from "./HelpModal.vue";
import SearchModal from "./SearchModal.vue";

const active = ref(ModalKind.None);

function clear() {
  active.value = ModalKind.None;
}


function toggleBasic(kind: ModalKind) {
  if (active.value == kind) {
    active.value = ModalKind.None;
  } else {
    active.value = kind;
  }
}


function toggleDispatch() {
  toggleBasic(ModalKind.Dispatch);
}

function toggleHelp() {
  toggleBasic(ModalKind.Help);
}

function toggleSearch() {
  toggleBasic(ModalKind.Search);
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
