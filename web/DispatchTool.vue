<template>
  <div class="content-aligned">
    <ul class="actions">
      <li>
        <button class="icon-button" type="button" @click="onSearch">
          <FontAwesomeIcon icon="fa-solid fa-magnifying-glass" size="xl" :fixedWidth="true" /> Search
        </button>
      </li>
      <li>
        <a :href="relTop">
          <button class="icon-button" type="button">
            <FontAwesomeIcon icon="fa-solid fa-house" size="xl" :fixedWidth="true" /> Home
          </button>
        </a>
      </li>
      <li>
        <button class="icon-button" type="button" @click="onHelp">
          <FontAwesomeIcon icon="fa-solid fa-life-ring" size="xl" :fixedWidth="true" /> Help
        </button>
      </li>
    </ul>
    <ul class="actions">
      <li>
        <button class="icon-button" type="button" @click="onClose">
          <FontAwesomeIcon icon="fa-solid fa-bars" size="xl" :fixedWidth="true" /> Close toolbox
        </button>
      </li>
    </ul>
  </div>
</template>

<style lang="scss" scoped>
.actions {
  width: 100%;
  padding: 0;
  margin-top: 1rem;
  box-sizing: border-box;

  li {
    width: 100%;
    margin: 5px 0;
    padding: 5px;
    box-sizing: border-box;

    display: block;
    border: 1px solid rgba(0, 0, 0, 0);
    border-radius: 8px;
    background-color: #efefef;

    &:hover {
      border: 1px solid var(--searchbar-border-color);
    }

    a {
      color: inherit;
    }

    button {
      width: 100%;
      height: 100%;
      text-align: left;

      &:hover {
        cursor: pointer;
      }

      :first-child {
        margin-right: 5px;
      }
    }
  }
}
</style>

<script setup lang="ts">
import { ToolKind } from "./base";

defineProps({
  relTop: { type: String, required: true },
});

const emit = defineEmits<{
  (e: "doTool", kind: ToolKind): void;
}>();

function onClose() {
  emit("doTool", ToolKind.None);
}

function onHelp() {
  emit("doTool", ToolKind.Help);
}

function onSearch() {
  emit("doTool", ToolKind.Search);
}
</script>
