# MosSaver

MosSaver is a simple desktop app for people who want safe, easy-to-understand versions of their project without learning Git commands. Add folders once, then see their current state from one home screen. Save a version and restore it when you need to go back. Existing Git commits are read-only in MosSaver; the app keeps branches and the staging area untouched, and watches managed folders for file changes.

The interface offers two built-in appearances: a light theme inspired by the default VS Code light theme, and a dark theme inspired by One Dark Pro.

MosSaver uses Tauri's native system WebView rather than bundling Chromium. macOS uses WKWebView, Windows uses the installed Evergreen WebView2 runtime, and Linux uses the system WebKitGTK runtime. The Windows installer uses the small WebView2 bootstrapper by default; it downloads the runtime only when the computer does not already have it. A fixed WebView2 runtime is intentionally not embedded, so MosSaver does not add a roughly 180 MB browser runtime to the installer.

## Run locally

Requirements: Rust (stable), Node.js 22.12 or newer, Git, and the platform dependencies required by Tauri 2.

```sh
cd apps/desktop
npm install
npm run tauri dev
```

On macOS, install the Xcode command line tools first. On Windows, install the Visual Studio C++ Build Tools and WebView2 for development. On Ubuntu, install the Tauri/WebKit development packages described in the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

## Verify changes

```sh
source "$HOME/.cargo/env" # zsh/bash on macOS and Linux, if Rust's bin directory is not on PATH
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/desktop
npm test
npm run check
npm run build
```

The Rust workspace is divided by responsibility: `easilygit-domain` defines the data and errors, `easilygit-app` holds the user-facing workflows, `easilygit-git` performs local Git operations, and `apps/desktop/src-tauri` connects those workflows to the window. The Svelte UI is intentionally a small Vite app rather than a server-rendered web app.

See [the architecture notes](docs/architecture.md) for the local storage and restore behavior.
