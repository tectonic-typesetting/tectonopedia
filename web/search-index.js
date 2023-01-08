// Copyright 2023 the Tectonic Project contributors
// Licensed under the MIT License

// Index the HTML content for elasticlunr.js
//
// As far as I can tell we have to hand-roll something to do this. We walk all
// of the HTML content and ingest it into an elasticlunr index, then write it
// out into the build directory.
//
// The chief tricky thing here is that we need to read the _all.html file to
// know what to index, and we need to read all of the content files that it
// points to, and those are all asynchronous. We have a simple "manager" class
// that keeps track of the number of active read "tasks" and writes out the
// index when all reads -- both the _all.html file and all HTML files that it
// points to -- are finished.

const elasticlunr = require('elasticlunrjs');
const fs = require('fs');
const htmlparser2 = require('htmlparser2');

// Simple manager for our async indexing

class IndexLoadManager {
    constructor() {
        this.index = elasticlunr();
        this.index.addField('title');
        this.index.addField('content');
        this.index.setRef('relpath');
        this.index.saveDocument(false);

        this.depth = 1;
        this.n_tasks = 0;
    }

    start_task() {
        this.depth += 1;
        this.n_tasks += 1;
    }

    finish_task(doc) {
        if (doc !== null) {
            this.index.addDoc(doc);
        }

        this.depth -= 1;

        if (this.depth == 0) {
            console.log(`Writing index of ${this.n_tasks} documents ...`);
            const ser = this.index.toJSON();
            fs.writeFileSync("build/search_index.json", JSON.stringify(ser));
        }
    }
}

// Processing an actual HTML input

class DocLoader {
    constructor(manager, relpath) {
        this.manager = manager;
        this.relpath = relpath;
        this.state = "ignoring";
        this.tag_depth = 0;
        this.title = "";
        this.content = "";
        this.content_tag_depth = 0;

        const self = this;

        this.parser = new htmlparser2.Parser({
            onopentag(name, attributes) {
                self.tag_depth += 1;

                if (self.state == "ignoring") {
                    if (name == "title") {
                        self.state = "title";
                    } else if (name == "div" && attributes.id == "content") {
                        self.state = "content";
                        self.content_tag_depth = self.tag_depth - 1;
                    }
                } else if (self.state == "content") {
                    self.content += " ";
                }
            },

            ontext(text) {
                if (self.state == "content") {
                    self.content += text;
                } else if (self.state == "title") {
                    self.title += text;
                }
            },

            onclosetag(name) {
                self.tag_depth -= 1;

                if (self.state == "content") {
                    if (self.tag_depth == self.content_tag_depth) {
                        self.state = "ignoring";
                    } else {
                        self.content += " ";
                    }
                } else if (self.state == "title") {
                    self.state = "ignoring";
                }
            }
        });
    }

    load(fspath) {
        this.manager.start_task();
        const s = fs.createReadStream(fspath, 'utf-8');

        s.on('error', (error) => {
            console.log(`error: ${fspath}: ${error.message}`);
            process.exit(1);
        });

        s.on('data', (chunk) => {
            this.parser.write(chunk);
        });

        s.on('end', () => {
            this.parser.end();

            if (this.title == "") {
                console.log(`error: ${fspath}: no title extracted`);
                process.exit(1);
            }

            if (this.content == "") {
                console.log(`error: ${fspath}: no content extracted`);
                process.exit(1);
            }

            const doc = {
                "relpath": this.relpath,
                "title": this.title,
                "content": this.content,
            };

            this.manager.finish_task(doc);
        });
    }
}

// Read the index and process everything

const manager = new IndexLoadManager();

const index_parser = new htmlparser2.Parser({
    onopentag(name, attributes) {
        if (name === "a") {
            var relpath = attributes.href;

            if (relpath.endsWith("/index.html")) {
                relpath = relpath.slice(0, -10);
            }

            const fspath = "build/" + attributes.href;

            new DocLoader(manager, relpath).load(fspath);
        }
    },
});

console.log("Scanning and indexing ...");
const file_list = fs.createReadStream('build/_all.html', 'utf-8');

file_list.on('error', (error) => {
    console.log(`error: build/_all.html: ${error.message}`);
    process.exit(1);
});

file_list.on('data', (chunk) => {
    index_parser.write(chunk);
});

file_list.on('end', () => {
    index_parser.end();
    manager.finish_task(null);
});
