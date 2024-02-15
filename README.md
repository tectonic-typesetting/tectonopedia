# The Tectonopedia

The Tectonic reference encyclopedia. The Tectonopedia is a web application
whose content is primarily technical documentation that is written using the TeX
typesetting language.


## Quick Start

To set up and launch the webserver locally in “watch” mode:

```sh
yarn install
cargo run --release -- watch
```

The key directories for editing the encyclopedia are:

- `cls`: shared TeX support files
- `idx`: encyclopedia index definitions
- `src`: source for the Rust program
- `txt`: encyclopedia TeX content
- `web`: web application frontend code

To add a new article, create a new file somewhere in the `txt` tree.

You can open this repository in a [GitHub Codespace][ghcs], edit
files in these directories, and immediately see changes in a running
version of the website. (Although, right now it takes a fairly
long time to initialize each Codespace container, since there is a
lot of fetching and compiling to do.)

[ghcs]: https://github.com/features/codespaces


## End-to-End Workflow

The Tectonopedia is created in three main stages:

1. Rust code in `src` is compiled into the `tectonopedia` program.
1. The `tectonopedia` program compiles TeX source in `txt` into
   raw HTML (+CSS, etc.) outputs in `build`.
1. [Parcel.js] combines those HTML outputs with frontend
   code in `web` to create full web app: either a set of
   production-ready files, or a hot-reloading development web
   server.

[Parcel.js]: https://parceljs.org/

The command `tectonopedia watch`, runnable locally as `cargo run --release -- watch`,
will both watch the TeX inputs and manage a Parcel.js development server,
automatically rerunning all processing steps except for any recompilations
of the `tectonopedia` program itself.

Some key support directories in this end-to-end process are:

- `build`: raw HTML+ outputs from the TeX compilation step
- `cache`: cached intermediate files for the TeX compilation step
- `dist`: compiled HTML+ outputs from the Parcel bundling step
- `node_modules`: NPM support modules for the Yarn/Parcel build steps
- `staging`: temporary version of `build`
- `target`: compiled executable outputs from the Rust compilation step

To create a production version of the encylopedia, run: `cargo run --release -- build`.
(In a devcontainer/Codespace, add the flag `--features=external-harfbuzz` to save
rebuild time.)


## Legalities

The source code underlying the Tectonopedia is licensed under the MIT License
(see the file `LICENSE-MIT`).
