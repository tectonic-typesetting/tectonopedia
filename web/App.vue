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
          <main id="main" v-html="content"></main>
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
import { ref } from "vue";

const props = defineProps({
  content: { type: String, required: true },
  title: { type: String, required: true },
})
</script>