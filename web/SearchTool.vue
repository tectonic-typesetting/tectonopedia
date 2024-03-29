<template>
  <div class="content-aligned" @keydown="onKeydown">
    <form @submit.prevent="onSubmit">
      <input ref="input" v-model="text" type="search" id="search-entry" name="search-entry" placeholder="Search ..." />
    </form>

    <ol ref="searchResultsList">
      <li v-for="r in results" class="search-result" tabindex="0">
        <SearchResult :title="r.title" snippet="snippet ..." :url="relTop + r.relpath"></SearchResult>
      </li>
    </ol>
  </div>
</template>

<style lang="scss" scoped>
#search-entry {
  width: 100%;
  margin: 1rem 0;

  font-size: 110%;
}

ol {
  list-style: none;
  margin: 0;
  padding: 0;
}

li {
  border: 2px solid #eee;
  margin: 0.1rem 0;
  padding: 2px;
  border-radius: 4px;

  &:focus {
    border: 2px solid var(--links);
  }
}
</style>

<script setup lang="ts">
import { nextTick, ref } from "vue";
import * as elasticlunr from "elasticlunrjs";

import { buildSpecificSettings } from "./base";
import SearchResult from "./SearchResult.vue";

defineProps({
  relTop: { type: String, required: true },
});

const input = ref<HTMLInputElement | null>(null);
const text = ref("");
const results = ref<IndexDoc[]>([]);
const searchResultsList = ref<HTMLElement | null>(null);

type IndexDoc = {
  relpath: string,
  title: string,
  content: string,
};

var indexPromise: Promise<elasticlunr.Index<IndexDoc>> | null = null;

function ensureIndexPromise() {
  if (indexPromise === null) {
    indexPromise = fetch(buildSpecificSettings.indexUrl).then((resp) => {
      return resp.json() as Promise<elasticlunr.SerialisedIndexData<IndexDoc>>
    }).then((json) => {
      return elasticlunr.Index.load(json);
    });
  }
}

function onSubmit() {
  // It shouldn't be possible to get here without the index promise
  // already having been created, but who knows.
  ensureIndexPromise();

  const query = text.value;

  if (query != "") {
    indexPromise.then((index) => {
      const lunr_results = index.search(query, {
        bool: "AND",
        expand: true,
        fields: {
          title: { boost: 5 },
          content: { boost: 1 }
        }
      });

      results.value.length = 0;

      for (const r of lunr_results) {
        const doc = index.documentStore.getDoc(r.ref);
        results.value.push(doc);
      }
    });
  }
}

function activate() {
  // If needed, start loading the search index.
  ensureIndexPromise();

  // Seems that we need to nextTick() the input field, presumably because this
  // widget is may start out hidden() when this function is called.
  nextTick(() => {
    input.value?.focus();
  });
}

// Keybindings

function noModifiers(event: KeyboardEvent): boolean {
  // NB, currently not checking shiftKey
  return !(event.altKey || event.ctrlKey || event.metaKey);
}

const keydownHandlers = {
  // Make it so that arrow keys can navigate the focus between the search entry
  // and the results.
  "ArrowDown": (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      // If the search entry is focused, navigate to the first result, if it
      // exists.
      if (document.querySelector("#search-entry:focus") !== null) {
        const results = searchResultsList.value?.children;

        if (results.length > 0) {
          (results[0] as HTMLElement).focus();
          event.preventDefault();
        }

        return;
      }

      // Otherwise, if a result is focused, navigate to the next one, if it exists.
      const focusedResult = document.querySelector(".search-result:focus");
      if (focusedResult?.nextElementSibling !== null) {
        (focusedResult.nextElementSibling as HTMLElement).focus();
        event.preventDefault();
      }
    }
  },

  "ArrowUp": (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      const focusedResult = document.querySelector(".search-result:focus");

      if (focusedResult !== null) {
        if (focusedResult.previousElementSibling === null) {
          // If we're on the first result, focus back to the entry
          input.value?.focus();
        } else {
          // Otherwise, we have a previous result to go to.
          (focusedResult.previousElementSibling as HTMLElement).focus();
        }

        event.preventDefault();
      }
    }
  },

  "Enter": (event: KeyboardEvent) => {
    if (noModifiers(event)) {
      const focusedResult = document.querySelector(".search-result:focus");

      if (focusedResult !== null) {
        (focusedResult.firstElementChild as HTMLElement).click();
        event.preventDefault();
      }
    }
  },
};

function onKeydown(event: KeyboardEvent) {
  const handler = keydownHandlers[event.key];
  if (handler !== undefined) {
    handler(event);
  }
}

defineExpose({
  activate,
});
</script>
