<template>
  <div>
    <form @submit.prevent="onSubmit">
      <input ref="input" v-model="text" type="search" id="search-entry" name="search-entry" placeholder="Search ..." />
    </form>

    <ol>
      <li v-for="r in results">
        <SearchResult :title="r.title" snippet="snippet ..." url="zz"></SearchResult>
      </li>
    </ol>
  </div>
</template>

<style lang="scss" scoped>
#search-entry {
  width: 100%;
  margin-top: 5px;
  margin-bottom: 0.2rem;

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
}
</style>

<script setup lang="ts">
import { nextTick, ref } from "vue";
import * as elasticlunr from "elasticlunrjs";

import SearchResult from "./SearchResult.vue";

const input = ref<HTMLInputElement | null>(null);
const text = ref("");
const results = ref<IndexDoc[]>([]);

type IndexDoc = {
  relpath: String,
  title: String,
  content: String,
};

// This construct gives us the URL of the search index data, which we'll load on
// the fly if needed. We have to give it an extension that isn't `JSON` because
// otherwise Parcel will try to be smart and preprocess the data for us, making
// it so that we can't use the `url:` loader to fetch the data on-demand.
const INDEX_URL = require("url:../build/search_index.json.data");

var indexPromise: Promise<elasticlunr.Index<IndexDoc>> | null = null;

function ensureIndexPromise() {
  if (indexPromise === null) {
    indexPromise = fetch(INDEX_URL).then((resp) => {
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
          title: { boost: 2 },
          content: { boost: 1 }
        }
      });

      results.value.length = 0;

      for (const r of lunr_results) {
        const doc = index.documentStore.getDoc(r.ref);
        console.log(doc);
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

defineExpose({
  activate,
});
</script>
