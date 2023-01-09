<template>
  <div>
    <form @submit.prevent="onSubmit">
      <input ref="input" v-model="text" type="search" id="search-entry" name="search-entry" placeholder="Search ..." />
    </form>
  </div>
</template>

<style lang="scss" scoped>
#search-entry {
  width: 100%;
  margin-top: 5px;
  margin-bottom: 0.2rem;

  font-size: 110%;
}
</style>

<script setup lang="ts">
import { nextTick, ref } from "vue";
import * as elasticlunr from "elasticlunrjs";

const input = ref<HTMLInputElement | null>(null);
const text = ref("");

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
      return elasticlunr.Index.load(json)
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
      const results = index.search(query, {
        fields: {
          title: { boost: 2 },
          content: { boost: 1 }
        }
      });

      console.log(results);
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
