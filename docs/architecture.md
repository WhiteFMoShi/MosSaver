# MosSaver architecture

MosSaver keeps the UI, product workflow, and Git implementation separate so each part can be changed and debugged on its own.

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

When protection is enabled in an existing repository, MosSaver reads commits reachable from `HEAD`, local branches, remote-tracking branches, and tags into a paged, read-only timeline. These entries cannot be restored through MosSaver. Loading the timeline starts with 50 entries and retrieves older pages only when requested. It reads local Git data only; it does not fetch from remotes. New commits and moved refs are discovered automatically when Git metadata changes.

MosSaver stores a small list of managed folder paths in its application data directory. This lets the home screen restore the user's folder list after restart; removing a folder from that list does not change its protection or files.

For compatibility with existing MosSaver data, the app keeps its small metadata file at `.git/easilygit/metadata.json` and stores its own Git data in the isolated `.git/easilygit/store.git/` repository. It reads existing Git objects through Git's alternate-object mechanism, but writes MosSaver objects and references only to its isolated store. Existing Git commits do not receive MosSaver references. Nothing is uploaded. The “停止此文件夹的保护” action removes the metadata and isolated store while preserving project files, normal Git references, and the Git index. If an older version wrote objects into the original Git object database, those now-unreachable objects may remain until normal Git maintenance reclaims them.

Restoring a MosSaver version writes its files into the working folder. It does not check out a branch or change the user's Git index. Existing Git commits are visible for context only. Individual files can be excluded from MosSaver saves and restores; the files remain on disk and their Git tracking state is unchanged. The timeline is a simple chronological list; it does not display branch lanes or provide branch and merge controls.

Ignored files are excluded from versions and left alone during restore. Before restoring, MosSaver saves the current contents as an automatic safety version. The “撤销恢复” action returns to that saved state. Automatic safety versions stay out of the normal version list.

Version data grows with project changes. Git reuses identical file objects, but large binary files that change often can take substantial space. The first version does not yet have an in-app storage meter or expiration policy. Stopping protection deletes the isolated MosSaver store; legacy objects left in the original Git database are reclaimed by Git's later maintenance.

## Dependencies and cleanup

The app runs the system Git executable. Its Rust dependencies are Tauri 2, its dialog plugin, the cross-platform `notify` file watcher, and small support crates for JSON, error reporting, temporary files, and checkpoint IDs. The UI uses Svelte, TypeScript, and Vite. npm packages and Rust crates are developer/build dependencies; the running app does not need Node.js.

The desktop shell uses Tauri's system WebView. The frontend is a local bundled interface, and the released application does not ship a Chromium browser. The operating system supplies the WebView runtime: WKWebView on macOS, Evergreen WebView2 on Windows, and WebKitGTK on Linux. The Windows bundle downloads WebView2 through its bootstrapper when needed instead of embedding a fixed runtime.

From the repository root, the generated development files can be removed with:

```sh
rm -rf apps/desktop/node_modules apps/desktop/dist target
```

They are rebuilt by npm and Cargo when needed. The npm and Cargo package caches in the user account are shared by other projects, so clearing those caches is optional and causes future builds to download packages again. The installed Rust toolchain is separate from this repository and should stay installed if Rust is used for other projects.

## First version boundaries

- The app uses the Git executable installed on the computer. It does not bundle Git.
- There is no remote hosting, account, branch browser, merge UI, or AI integration.
- The app watches all listed folders and relevant Git refs for changes, then refreshes their status and timeline automatically; the refresh button remains available as a manual fallback.
- The first supported workflow is to view existing Git history, save local project versions, restore a MosSaver version, undo that restore, exclude files, and stop protection.
