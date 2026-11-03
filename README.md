# MosSaver

MosSaver is a simple desktop app for people who want safe, easy-to-understand versions of their project without learning Git commands. Add folders once, then see their current state from one home screen. Save a version and restore it when you need to go back. Existing Git commits are read-only in MosSaver; the app keeps branches and the staging area untouched, and watches managed folders for file changes.

The interface offers two built-in appearances: a light theme inspired by the default VS Code light theme, and a dark theme inspired by One Dark Pro.

Click a saved version's node or title to reveal its restore, merge, and delete actions. Drag a MosSaver node onto another node to merge them, or onto the trash icon that appears in the history header to request deletion. Both operations ask for confirmation. Only inactive leaf versions that are not referenced by other versions or recovery points can be deleted; existing Git commits stay read-only.

A merge creates a new saved version with both source versions as real Git parents. It leaves the active version and working files in place; restore the new merged version when you want to use its contents. Merges need Git 2.38 or newer and stop when changes conflict. Manual conflict resolution is not included yet.

Expand **查看变更**, then hover over a file for about half a second to preview its saved changes with old/new line numbers. Click to pin the preview, enlarge it with the toolbar, or press Escape to close it. Previews are read-only, load on demand, and compare a version against its first parent. Binary files, non-UTF-8 text, and files larger than 512 KiB show an explanatory message; long patches are truncated.

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

## Build and release installers

Build an installer for the computer you are currently using:

```sh
cd apps/desktop
npm ci
npm run tauri build
```

Tauri writes platform-specific installers under `target/release/bundle/` at the repository root. Windows produces MSI and NSIS installers, macOS produces a DMG, and Linux can produce DEB, RPM, and AppImage packages. You need to build on each operating system; this command does not cross-compile Windows or macOS installers from another platform.

The GitHub Release workflow builds Windows x64, macOS Intel and Apple silicon, and Linux x64 packages when a version tag is pushed. It also creates signed Tauri updater artifacts and publishes `latest.json`; MosSaver checks that manifest after startup and asks before downloading an update. Add the generated updater private key as the repository secret `TAURI_SIGNING_PRIVATE_KEY` (and add `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` when the key has a password). Keep the private key out of the repository. The updater public key is stored in `apps/desktop/src-tauri/tauri.conf.json`.

For the first build, use the current version shown by `node scripts/version.mjs check`; commit the release setup, then create and push the matching tag:

```sh
node scripts/version.mjs check
git tag v0.1.0
git push origin main
git push origin v0.1.0
```

For later releases, update every app version field together, commit it, then create and push the matching tag:

```sh
cd apps/desktop
npm run version:set -- 0.1.1
npm run version:check
```

After committing the version update, from the repository root:

```sh
git tag v0.1.1
git push origin main
git push origin v0.1.1
```

The workflow checks that the tag matches the app version, then creates a draft GitHub Release with the installers, signatures, and updater manifest attached. Review it and publish it when ready; the in-app updater only sees published releases. Replace `0.1.1` with the next version. The version check runs in CI so mismatched Cargo, Tauri, and npm versions are caught before release. Windows SmartScreen and macOS Gatekeeper may show first-run warnings until code-signing certificates and notarization are configured.

The tag must point to the commit that contains the matching version fields. If a tag was pushed before that commit, the release workflow will stop before building; move the tag to the corrected commit and push it again, or use a new version tag.

The Rust workspace is divided by responsibility: `easilygit-domain` defines the data and errors, `easilygit-app` holds the user-facing workflows, `easilygit-git` performs local Git operations, and `apps/desktop/src-tauri` connects those workflows to the window. The Svelte UI is intentionally a small Vite app rather than a server-rendered web app.

See [the architecture notes](docs/architecture.md) for the local storage and restore behavior.
