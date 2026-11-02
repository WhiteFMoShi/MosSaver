# EasilyGit

EasilyGit is a small desktop app for people who want safe, easy-to-understand versions of a project without learning Git commands. Choose a folder to view its existing Git history, save a version, and restore it when you need to go back. The app keeps existing branches and the staging area untouched, and watches the selected folder for file changes.

## Run locally

Requirements: Rust (stable), Node.js 22.12 or newer, Git, and the platform dependencies required by Tauri 2.

```sh
cd apps/desktop
npm install
npm run tauri dev
```

On macOS, install the Xcode command line tools first. On Windows, install the Visual Studio C++ Build Tools and WebView2. On Ubuntu, install the Tauri/WebKit development packages described in the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

## Verify changes

```sh
source "$HOME/.cargo/env" # zsh/bash on macOS and Linux, if Rust's bin directory is not on PATH
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop
npm run check
npm run build
```

The Rust workspace is divided by responsibility: `easilygit-domain` defines the data and errors, `easilygit-app` holds the user-facing workflows, `easilygit-git` performs local Git operations, and `apps/desktop/src-tauri` connects those workflows to the window. The Svelte UI is intentionally a small Vite app rather than a server-rendered web app.

See [the architecture notes](docs/architecture.md) for the local storage and restore behavior.
