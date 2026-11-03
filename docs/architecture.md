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

Restoring a MosSaver version writes its files into the working folder. It does not check out a branch or change the user's Git index. Existing Git commits are visible for context only. Individual files can be excluded from MosSaver saves and restores; the files remain on disk and their Git tracking state is unchanged. Each MosSaver version stores its parent version and a compact file summary. Saving after restoring an older version therefore creates a child of that older version instead of silently continuing one linear chain. Existing metadata without parent links is hydrated from the actual parent commit in the isolated MosSaver object store. The UI uses an IDE-style open workspace rather than framed cards: the file-change pane and history pane share the window surface, the divider can be dragged, and the history pane takes the remaining space. The graph follows real parent links; divergent children receive separate lanes and identify their fork point. Version details include file count and changed paths.

The graph's nodes and titles can be selected or dragged. Dropping one MosSaver version onto another opens a merge confirmation. `operations.rs` uses `git merge-tree --write-tree` in the isolated store, then `commit-tree` with two parents to create a new saved version. `parentId` is the target and the comparison base; `mergeParentIds` lists the additional source. Existing metadata defaults to an empty additional-parent list. The active checkpoint and working files stay unchanged until the user explicitly restores the merged version. Merging needs Git 2.38 or newer; identical versions, ancestor/descendant pairs, native Git entries, unrelated histories, and conflicting changes are rejected. Conflict detection may write unreachable objects in the isolated store, but does not change checkpoint references, metadata, the native Git index, or working files. Manual conflict resolution is not implemented.

Deleting a checkpoint removes its isolated reference and metadata entry, with reference rollback if saving metadata fails. Only inactive leaves may be deleted. Both first-parent and additional-parent references, including hidden safety checkpoints, protect their source version. The backend returns deletion restrictions computed from the complete metadata rather than only the loaded page. Deletion does not immediately garbage-collect unreachable Git objects. Service clones share a mutex so refreshes and mutations cannot overwrite each other's metadata.

`preview.rs` reads saved file blobs and first-parent patches without reading or changing working files. Source blobs are limited to 512 KiB per side; binary and non-UTF-8 content is not rendered. Patches are limited to 1,200 lines and 160 KiB. Literal Git pathspecs, disabled external diff helpers, and disabled text conversion keep preview requests scoped to the selected path. `FileDiff.svelte` waits 550 ms before requesting a preview, caches it for the expanded file entry, and portals the popup outside the scrolled list. The read-only popup supports pinning, enlargement, scrolling, and Escape; it registers global handlers only while open. Diff parsing and viewport placement live in `diff.ts`.

Ignored files are excluded from versions and left alone during restore. Before restoring, MosSaver saves the current contents as an automatic safety version. The “撤销恢复” action returns to that saved state. Automatic safety versions stay out of the normal version list.

Timeline layout lives in `apps/desktop/src/lib/timeline.ts`, separate from Svelte and Git access. Rows are sorted by parsed timestamps; equal timestamps preserve the backend's save order rather than ordering random version IDs. The backend applies the same ordering before pagination. Edges come only from stored first and additional parent IDs. Both source ancestries of the active merged version are highlighted; first-parent links determine the main lane and side lanes. Restoring an older version reserves its lane for further work while its existing descendants move to side lanes. Side lanes are reused when their complete primary-edge ranges do not overlap. Edges detour around unrelated nodes when merges or clock changes require it. Node positions use title anchors in the SVG's coordinate system, so scrolling and expanding file details do not detach lines from their versions. The graph sits in the scrolling content, and its height comes from the rows rather than its own previous SVG size. Run `npm test` in `apps/desktop` for the dependency-free graph and diff regression tests, including generated trees and merge graphs. Rust integration tests cover real merge parents, conflict handling, deletion protection, concurrent commands, native Git isolation, and preview limits as well as save order and pagination.

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
- There is no remote hosting, account, native Git branch editor, manual conflict editor, or AI integration. MosSaver versions expose their own parent-child and merge relationships for local history.
- The app watches all listed folders and relevant Git refs for changes, then refreshes their status and timeline automatically; the refresh button remains available as a manual fallback.
- Supported workflows include viewing existing Git history, saving local versions, restoring or undoing a restore, merging MosSaver versions, deleting unreferenced inactive leaves, previewing saved file changes, excluding files, and stopping protection.
