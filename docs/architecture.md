# First version architecture

EasilyGit keeps the UI, product workflow, and Git implementation separate so each part can be changed and debugged on its own.

```text
Svelte UI
  -> Tauri commands (apps/desktop/src-tauri)
  -> ProjectService (crates/easilygit-app)
  -> ProjectPort interface
  -> SystemGitBackend (crates/easilygit-git)
  -> installed Git executable
```

`easilygit-domain` contains the shared data types and user-readable errors. The app layer defines the actions the product supports. The Git crate implements those actions, and the Tauri crate exposes only the commands the screen needs. Git work runs in a blocking background task so it does not freeze the window.

## Where versions live

The app creates ordinary Git objects without moving the user's current branch or changing the user's staging area. Each version points to a tree containing tracked files and non-ignored files in the folder. A separate temporary Git index is used while making or comparing a version.

EasilyGit keeps its small metadata file at `.git/easilygit/metadata.json` and version references under `.git/refs/easilygit/checkpoints/`. The project files are not copied into a separate database, and nothing is uploaded.

Ignored files are excluded from versions and left alone during restore. Before restoring, EasilyGit saves the current contents as an automatic safety version. The “撤销恢复” action returns to that saved state. Automatic safety versions stay out of the normal version list.

Version data grows with project changes. Git reuses identical file objects, but large binary files that change often can take substantial space. The first version does not yet have an in-app storage meter, expiration policy, or history deletion screen; those should be added before relying on it for long-running projects.

## Dependencies and cleanup

The app runs the system Git executable. Its Rust dependencies are Tauri 2 and its dialog plugin, plus small support crates for JSON, error reporting, temporary files, and checkpoint IDs. The UI uses Svelte, TypeScript, and Vite. npm packages and Rust crates are developer/build dependencies; the running app does not need Node.js.

From the repository root, the generated development files can be removed with:

```sh
rm -rf apps/desktop/node_modules apps/desktop/dist target
```

They are rebuilt by npm and Cargo when needed. The npm and Cargo package caches in the user account are shared by other projects, so clearing those caches is optional and causes future builds to download packages again. The installed Rust toolchain is separate from this repository and should stay installed if Rust is used for other projects.

## First version boundaries

- The app uses the Git executable installed on the computer. It does not bundle Git.
- There is no remote hosting, account, branch browser, merge UI, AI integration, or background file watcher yet.
- The screen refresh button reads the current folder state again.
- The first supported workflow is to save local project versions, restore a selected version, and undo that restore.
