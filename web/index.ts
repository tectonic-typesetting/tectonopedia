import { createApp } from "vue";
import { ResizeObserver } from "vue-resize";
import App from "./App.vue";

// FontAwesome setup:

import { library } from "@fortawesome/fontawesome-svg-core";
import { FontAwesomeIcon } from "@fortawesome/vue-fontawesome";
import { faAngleLeft, faAngleRight, faBars, faLifeRing, faMagnifyingGlass } from "@fortawesome/free-solid-svg-icons";

library.add(faAngleLeft);
library.add(faAngleRight);
library.add(faBars);
library.add(faLifeRing);
library.add(faMagnifyingGlass);

// We want our HTML outputs to contain their actual content so that our content
// can be served with a simple static webserve and so that search engines can
// crawl it. But we also want to use Vue to get all of the web-app construction
// benefits that it provides, and Vue wants to control the entire DOM.
//
// Our solution is a sort of "poor man's server-side rendering": the Tectonic
// processing creates HTML files containing "actual content", but when our app
// initializes (right here) we extract that content and then re-insert it into
// the Vue app after construction, using its `v-html` mechanism. This feels
// gross and terrible, and is certainly inefficient, but I really don't want to
// rely on JS-based server-side rendering.

const title = document.getElementById("title").innerText;
const bookName = document.getElementById("bookname").innerText;
const content = document.getElementById("content").innerHTML;

const metadata_el = document.getElementById("metadata");
const relTop = metadata_el.dataset.reltop;

const app = createApp(App, { content, title, bookName, relTop });
app.component("ResizeObserver", ResizeObserver);
app.component("FontAwesomeIcon", FontAwesomeIcon);
app.mount("#app");
