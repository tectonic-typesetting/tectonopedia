# The Tectonopedia

The Tectonic reference encyclopedia.

## Repository Structure

- `cls`: shared TeX support files
- `src`: source for the Rust program
- `txt`: encyclopedia TeX content
- `web`: web application frontend code


## Current Workflow

The wrapper program should eventually automate these steps

- `cargo run --release -- build`
- `yarn index`
- `yarn build` or `yarn serve`


## Legalities

The source code underlying the Tectonopedia is licensed under the MIT License
(see the file `LICENSE-MIT`).
