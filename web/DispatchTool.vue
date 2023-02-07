<template>
  <div class="content-aligned">
    <ul class="actions">
      <li>
        <button class="icon-button" type="button" @click="onBack">
          <FontAwesomeIcon icon="fa-solid fa-bars" /> Back to text
        </button>
      </li>
    </ul>

    <ul class="actions">
      <li>
        <button class="icon-button" type="button" @click="onHelp">
          <FontAwesomeIcon icon="fa-solid fa-bars" /> Help
        </button>
      </li>
    </ul>
  </div>
</template>

<style lang="scss" scoped>
.actions {
  width: 100%;
  padding: 0;
  margin-top: 2rem;

  li {
    width: 100%;
    margin: 2px 0;
    border: 1px solid var(--searchbar-border-color);
    border-radius: 3px;
    padding: 2px 6px;
    box-sizing: border-box;

    display: block;

    button {
      width: 100%;
      height: 100%;
      text-align: left;

      &:hover {
        cursor: pointer;
      }
    }
  }
}
</style>

<script setup lang="ts">
import { ToolKind } from "./base";

const emit = defineEmits<{
  (e: "doTool", kind: ToolKind): void;
}>();

function onBack() {
  emit("doTool", ToolKind.None);
}

function onHelp() {
  emit("doTool", ToolKind.Help);
}
</script>
