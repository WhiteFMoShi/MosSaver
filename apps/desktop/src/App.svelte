<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";

  type ChangeKind = "added" | "modified" | "deleted";

  interface ChangedFile {
    path: string;
    kind: ChangeKind;
  }

  interface Checkpoint {
    id: string;
    label: string;
    createdAt: string;
    kind: "user" | "git" | "automatic";
  }

  interface CheckpointPage {
    checkpoints: Checkpoint[];
    hasMore: boolean;
  }

  interface ManagedFile {
    path: string;
    managed: boolean;
  }

  interface ProjectState {
    root: string;
    name: string;
    initialized: boolean;
    isGitRepository: boolean;
    currentChanges: ChangedFile[];
    checkpoints: Checkpoint[];
    hasMoreCheckpoints: boolean;
    canUndoRestore: boolean;
  }

  let project = $state<ProjectState | null>(null);
  let loading = $state(false);
  let errorMessage = $state("");
  let selectedCheckpoint = $state<Checkpoint | null>(null);
  let dialogMode = $state<"restore" | "undo" | null>(null);
  let managedFilesOpen = $state(false);
  let managedFiles = $state<ManagedFile[]>([]);
  let stopProtectionOpen = $state(false);
  let loadedCheckpointCount = 0;
  let autoRefreshTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingRefreshRoot: string | undefined;

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    listen<string>("project-files-changed", (event) => {
      scheduleProjectRefresh(event.payload);
    })
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch(() => {
        errorMessage = "自动刷新暂不可用，你仍可以点击“刷新”查看最新状态。";
      });

    return () => {
      disposed = true;
      if (autoRefreshTimer) clearTimeout(autoRefreshTimer);
      unlisten?.();
      void invoke("stop_project_watch").catch(() => {});
    };
  });

  const changeLabels: Record<ChangeKind, string> = {
    added: "新增",
    modified: "已修改",
    deleted: "已删除",
  };

  const changeCounts = $derived.by(() => {
    const changes = project?.currentChanges ?? [];
    return {
      added: changes.filter((file) => file.kind === "added").length,
      modified: changes.filter((file) => file.kind === "modified").length,
      deleted: changes.filter((file) => file.kind === "deleted").length,
    };
  });

  async function chooseFolder() {
    errorMessage = "";
    try {
      const selection = await open({ directory: true, multiple: false, title: "选择要保护的文件夹" });
      if (typeof selection === "string") {
        await openProject(selection);
      }
    } catch (error) {
      errorMessage = friendlyError(error);
    }
  }

  async function openProject(path: string) {
    await perform(async () => {
      const opened = await invoke<ProjectState>("open_project", { path });
      await adoptProjectState(opened, 0);
      selectedCheckpoint = null;
      dialogMode = null;
      managedFilesOpen = false;
      stopProtectionOpen = false;
      if (opened.initialized) {
        await invoke("watch_project", { path: opened.root });
      } else {
        await invoke("stop_project_watch");
      }
    });
  }

  async function initializeProject() {
    if (!project) return;
    await perform(async () => {
      const initialized = await invoke<ProjectState>("initialize_project", { path: project!.root });
      await adoptProjectState(initialized, 0);
      await invoke("watch_project", { path: initialized.root });
    });
  }

  async function refreshProject() {
    if (!project) return;
    const previousCount = loadedCheckpointCount;
    await perform(async () => {
      const refreshed = await invoke<ProjectState>("refresh_project", { path: project!.root });
      await adoptProjectState(refreshed, previousCount);
    });
  }

  async function saveVersion() {
    if (!project) return;
    await perform(async () => {
      const saved = await invoke<ProjectState>("save_version", {
        path: project!.root,
        label: "手动保存的版本",
      });
      await adoptProjectState(saved, loadedCheckpointCount);
    });
  }

  async function restoreVersion() {
    if (!project || !selectedCheckpoint) return;
    const checkpoint = selectedCheckpoint;
    await perform(async () => {
      const restored = await invoke<ProjectState>("restore_checkpoint", {
        path: project!.root,
        checkpointId: checkpoint.id,
      });
      await adoptProjectState(restored, loadedCheckpointCount);
      selectedCheckpoint = null;
      dialogMode = null;
    });
  }

  async function undoRestore() {
    if (!project) return;
    await perform(async () => {
      const restored = await invoke<ProjectState>("undo_restore", { path: project!.root });
      await adoptProjectState(restored, loadedCheckpointCount);
      dialogMode = null;
    });
  }

  function returnHome() {
    project = null;
    errorMessage = "";
    selectedCheckpoint = null;
    dialogMode = null;
    managedFilesOpen = false;
    stopProtectionOpen = false;
    loadedCheckpointCount = 0;
    pendingRefreshRoot = undefined;
    void invoke("stop_project_watch").catch(() => {});
  }

  async function adoptProjectState(next: ProjectState, preserveCheckpointCount: number) {
    let checkpoints = next.checkpoints;
    let hasMoreCheckpoints = next.hasMoreCheckpoints;
    while (checkpoints.length < preserveCheckpointCount && hasMoreCheckpoints) {
      const page = await invoke<CheckpointPage>("load_more_checkpoints", {
        path: next.root,
        offset: checkpoints.length,
      });
      checkpoints = [...checkpoints, ...page.checkpoints];
      hasMoreCheckpoints = page.hasMore;
      if (page.checkpoints.length === 0) break;
    }
    project = { ...next, checkpoints, hasMoreCheckpoints };
    loadedCheckpointCount = checkpoints.length;
  }

  async function loadMoreCheckpoints() {
    if (!project?.hasMoreCheckpoints) return;
    await perform(async () => {
      const page = await invoke<CheckpointPage>("load_more_checkpoints", {
        path: project!.root,
        offset: project!.checkpoints.length,
      });
      project = {
        ...project!,
        checkpoints: [...project!.checkpoints, ...page.checkpoints],
        hasMoreCheckpoints: page.hasMore,
      };
      loadedCheckpointCount = project.checkpoints.length;
    });
  }

  async function openManagedFiles() {
    if (!project) return;
    await perform(async () => {
      managedFiles = await invoke<ManagedFile[]>("list_managed_files", { path: project!.root });
      managedFilesOpen = true;
    });
  }

  async function toggleFileManagement(file: ManagedFile) {
    if (!project) return;
    await perform(async () => {
      const updated = await invoke<ProjectState>("set_file_managed", {
        path: project!.root,
        filePath: file.path,
        managed: !file.managed,
      });
      await adoptProjectState(updated, loadedCheckpointCount);
      managedFiles = await invoke<ManagedFile[]>("list_managed_files", { path: project!.root });
    });
  }

  async function stopProtection() {
    if (!project) return;
    const root = project.root;
    await perform(async () => {
      const stopped = await invoke<ProjectState>("disable_protection", { path: root });
      await adoptProjectState(stopped, 0);
      managedFilesOpen = false;
      stopProtectionOpen = false;
      await invoke("stop_project_watch").catch(() => {});
    });
  }

  function scheduleProjectRefresh(root: string) {
    if (!project?.initialized || project.root !== root) return;
    if (loading) {
      pendingRefreshRoot = root;
      return;
    }
    if (autoRefreshTimer) clearTimeout(autoRefreshTimer);
    autoRefreshTimer = setTimeout(() => {
      autoRefreshTimer = undefined;
      if (loading) {
        pendingRefreshRoot = root;
      } else if (project?.initialized && project.root === root) {
        void refreshProject();
      }
    }, 200);
  }

  function requestUndoRestore() {
    selectedCheckpoint = null;
    dialogMode = "undo";
  }

  async function perform(action: () => Promise<void>) {
    loading = true;
    errorMessage = "";
    try {
      await action();
    } catch (error) {
      errorMessage = friendlyError(error);
    } finally {
      loading = false;
      if (pendingRefreshRoot) {
        const root = pendingRefreshRoot;
        pendingRefreshRoot = undefined;
        scheduleProjectRefresh(root);
      }
    }
  }

  function friendlyError(error: unknown): string {
    if (error && typeof error === "object" && "code" in error) {
      const code = String(error.code);
      const messages: Record<string, string> = {
        PROJECT_NOT_FOUND: "找不到这个文件夹，请确认它还在这台电脑上。",
        GIT_NOT_INSTALLED: "没有找到 Git。请先安装 Git 后重试。",
        NOT_A_REPOSITORY: "这个文件夹暂时无法开启版本保护。",
        NOT_INITIALIZED: "请先为这个文件夹开启版本保护。",
        CHECKPOINT_NOT_FOUND: "找不到这个版本，它可能已经被移除。",
        CHECKPOINT_READ_ONLY: "这是原有 Git 提交，只能查看，不能通过 EasilyGit 恢复。",
        NOTHING_TO_UNDO: "现在没有可以撤销的恢复操作。",
        INVALID_PATH: "项目中有一个文件名无法安全处理。",
        PERMISSION_DENIED: "没有权限访问项目文件，请检查文件夹权限。",
        RESTORE_FAILED: "恢复没有完成。请刷新项目状态后再试。",
        METADATA_ERROR: "EasilyGit 的版本记录暂时无法读写，请检查文件夹权限。",
        GIT_ERROR: "保存项目时遇到问题，请检查 Git 和项目文件后重试。",
        INTERNAL_ERROR: "操作没有完成，请重试。",
      };
      return messages[code] ?? "操作没有完成，请重试。";
    }
    if (typeof error === "string") return error;
    return "操作没有完成，请重试。";
  }

  function formatTime(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return new Intl.DateTimeFormat("zh-CN", {
      year: "numeric",
      month: "long",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  }

  function fileName(path: string): string {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
  }
</script>

<svelte:head>
  <title>{project ? `${project.name} · EasilyGit` : "EasilyGit · 轻松保存每一步"}</title>
</svelte:head>

<main class="app-shell">
  <header class="topbar">
    <a class="brand" href="/" aria-label="EasilyGit 首页" onclick={(event) => { event.preventDefault(); returnHome(); }}>
      <span class="brand-mark" aria-hidden="true"><span></span><span></span><span></span></span>
      <span>Easily<span class="brand-light">Git</span></span>
    </a>
    {#if project}
      <button class="button button-quiet topbar-folder" onclick={chooseFolder} disabled={loading}>
        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M2.8 5.4a1.6 1.6 0 0 1 1.6-1.6h4l1.5 1.7h5.7a1.6 1.6 0 0 1 1.6 1.6v7.5a1.6 1.6 0 0 1-1.6 1.6H4.4a1.6 1.6 0 0 1-1.6-1.6V5.4Z" /></svg>
        打开其他文件夹
      </button>
    {/if}
  </header>

  {#if !project}
    <section class="welcome-view">
      <div class="welcome-art" aria-hidden="true">
        <div class="art-orbit orbit-one"></div>
        <div class="art-orbit orbit-two"></div>
        <div class="art-card art-card-back"><span class="art-line short"></span><span class="art-line"></span><span class="art-line medium"></span></div>
        <div class="art-card art-card-front">
          <span class="art-dot"></span><span class="art-line short"></span><span class="art-line"></span><span class="art-line medium"></span>
          <span class="art-check">✓</span>
        </div>
        <span class="art-spark spark-one">✳</span><span class="art-spark spark-two">✦</span>
      </div>
      <p class="eyebrow">给每一次 AI 创作，一份安心</p>
      <h1>放心尝试，<span>随时回到之前。</span></h1>
      <p class="welcome-copy">把你的项目交给 EasilyGit。每次重要修改，<br class="desktop-break" />都可以轻松保存，也能随时恢复。</p>
      <button class="button button-primary button-large" onclick={chooseFolder} disabled={loading}>
        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M2.8 5.4a1.6 1.6 0 0 1 1.6-1.6h4l1.5 1.7h5.7a1.6 1.6 0 0 1 1.6 1.6v7.5a1.6 1.6 0 0 1-1.6 1.6H4.4a1.6 1.6 0 0 1-1.6-1.6V5.4Z" /></svg>
        选择一个项目文件夹
      </button>
      <p class="privacy-note"><span class="privacy-dot"></span>你的文件和版本记录都保存在这台电脑上</p>
    </section>
  {:else}
    <section class="project-view">
      <div class="project-heading">
        <div class="heading-copy">
          <div class="breadcrumb"><span>我的项目</span><span class="breadcrumb-slash">/</span><span>{project.name}</span></div>
          <h1>{project.name}</h1>
          <p class="project-path" title={project.root}>{project.root}</p>
        </div>
        <button class="button button-quiet refresh-button" onclick={refreshProject} disabled={loading} aria-label="刷新项目状态">
          <svg class:spinning={loading} viewBox="0 0 20 20" aria-hidden="true"><path d="M16.3 8A6.5 6.5 0 0 0 4.9 5.7L3.5 7.1M3.7 4.2v3.2h3.2M3.7 12A6.5 6.5 0 0 0 15.1 14.3l1.4-1.4m-.2 3v-3.2h-3.2" /></svg>
          刷新
        </button>
      </div>

      {#if errorMessage}
        <div class="error-banner" role="alert">
          <span class="error-icon">!</span>
          <span>{errorMessage}</span>
          <button class="error-dismiss" onclick={() => errorMessage = ""} aria-label="关闭提示">×</button>
        </div>
      {/if}

      {#if !project.initialized}
        <section class="setup-card">
          <div class="setup-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.7 20 8v8l-8 4.3L4 16V8l8-4.3Z" /><path d="m4.4 8.2 7.6 4.1 7.6-4.1M12 12.4v7.4M8 5.9l8 4.3" /></svg></div>
          <div class="setup-copy">
            <p class="eyebrow">只需一步</p>
            <h2>{project.isGitRepository ? "检测到已有 Git 仓库" : "为这个文件夹开启版本保护"}</h2>
            {#if project.isGitRepository}
              <p>开启后，可以只读查看已有 Git 提交，并保存项目当前状态。EasilyGit 不会移动分支或改动暂存区；恢复仅适用于 EasilyGit 保存的版本。</p>
            {:else}
              <p>开启后，你可以保存项目当前状态，并在需要时恢复。EasilyGit 会在项目中创建本地 Git 记录，不会上传任何文件。</p>
            {/if}
          </div>
          <button class="button button-primary" onclick={initializeProject} disabled={loading}>
            {loading ? "正在准备…" : project.isGitRepository ? "读取 Git 历史并开启保护" : "开启版本保护"}
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10h12m-5-5 5 5-5 5" /></svg>
          </button>
        </section>
      {:else}
        <section class="status-card">
          <div class="status-indicator"><span></span></div>
          <div class="status-copy">
            <h2>{project.currentChanges.length === 0 ? "项目已保存" : `有 ${project.currentChanges.length} 项尚未保存`}</h2>
            <p>{project.currentChanges.length === 0 ? "当前文件与最近保存的版本一致。" : "保存一个版本，记录现在的项目状态。"}</p>
          </div>
          <div class="status-actions">
            {#if project.canUndoRestore}
              <button class="button button-quiet undo-button" onclick={requestUndoRestore} disabled={loading}>
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M7 6 3.5 9.5 7 13M4 9.5h7a5 5 0 0 1 5 5" /></svg>
                撤销恢复
              </button>
            {/if}
            <button class="button button-primary" onclick={saveVersion} disabled={loading || project.currentChanges.length === 0}>
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3.5v9m-3.5-3.5L10 12.5l3.5-3.5M4.5 15.5v1h11v-1" /></svg>
              保存当前版本
            </button>
          </div>
        </section>

        <div class="content-grid">
          <section class="panel changes-panel">
            <div class="panel-heading">
              <div>
                <h2>当前变化</h2>
                <p>自上次保存后发生的修改</p>
              </div>
              <div class="panel-heading-actions">
                {#if project.currentChanges.length > 0}
                  <span class="count-badge">{project.currentChanges.length}</span>
                {/if}
                <button class="text-button" onclick={openManagedFiles} disabled={loading}>管理文件</button>
              </div>
            </div>

            {#if project.currentChanges.length === 0}
              <div class="empty-changes">
                <span class="empty-check">✓</span>
                <p>一切都很平静</p>
                <span>有新修改时，会显示在这里。</span>
              </div>
            {:else}
              <ul class="file-list">
                {#each project.currentChanges as file (file.path)}
                  <li class="file-row">
                    <span class="file-kind kind-{file.kind}">{file.kind === "added" ? "+" : file.kind === "deleted" ? "−" : "·"}</span>
                    <span class="file-name" title={file.path}>{fileName(file.path)}</span>
                    <span class="file-path" title={file.path}>{file.path}</span>
                    <span class="change-label label-{file.kind}">{changeLabels[file.kind]}</span>
                  </li>
                {/each}
              </ul>
              <div class="change-legend">
                {#if changeCounts.added > 0}<span class="legend-added">{changeCounts.added} 新增</span>{/if}
                {#if changeCounts.modified > 0}<span class="legend-modified">{changeCounts.modified} 修改</span>{/if}
                {#if changeCounts.deleted > 0}<span class="legend-deleted">{changeCounts.deleted} 删除</span>{/if}
              </div>
            {/if}
          </section>

          <section class="panel history-panel">
            <div class="panel-heading">
              <div>
                <h2>已保存的版本</h2>
                <p>EasilyGit 版本可恢复；已有 Git 提交只读查看</p>
              </div>
              <span class="history-total">已加载 {project.checkpoints.length} 个版本</span>
            </div>

            {#if project.checkpoints.length === 0}
              <div class="empty-history">
                <div class="empty-history-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7.5h16M6.5 4.5h11v15h-11zM9 11h6m-6 3h4" /></svg></div>
                <p>还没有保存过版本</p>
                <span>点击“保存当前版本”，为项目留下一个安全点。</span>
              </div>
            {:else}
              <ol class="timeline">
                {#each project.checkpoints as checkpoint, index (checkpoint.id)}
                  <li class:latest={index === 0} class="timeline-item">
                    <span class="timeline-marker">{#if index === 0}<span></span>{/if}</span>
                    <div class="timeline-content">
                      <div class="timeline-title-row">
                        <h3>{checkpoint.label}</h3>
                        {#if index === 0}<span class="latest-badge">{checkpoint.label === "开始使用 EasilyGit" ? "保护起点" : "最近"}</span>{/if}
                      </div>
                      <p class="timeline-time">{formatTime(checkpoint.createdAt)}</p>
                      {#if checkpoint.kind === "git"}<span class="readonly-note">Git 提交 · 仅查看</span>{/if}
                      {#if checkpoint.kind === "automatic"}<span class="automatic-note">自动安全保存</span>{/if}
                      {#if checkpoint.kind !== "git"}
                        <button class="restore-link" onclick={() => { selectedCheckpoint = checkpoint; dialogMode = "restore"; }} disabled={loading}>
                          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 8a6 6 0 1 1-.2 3M4 4.5V8h3.5M10 7v3l2 1.2" /></svg>
                          恢复到此版本
                        </button>
                      {/if}
                    </div>
                  </li>
                {/each}
              </ol>
              {#if project.hasMoreCheckpoints}
                <button class="load-more-button" onclick={loadMoreCheckpoints} disabled={loading}>加载更早的版本</button>
              {/if}
            {/if}
          </section>
        </div>

        <footer class="local-footer">
          <span><span class="privacy-dot"></span>所有版本都保存在本地电脑</span>
          <button class="stop-protection-link" onclick={() => stopProtectionOpen = true} disabled={loading}>停止 EasilyGit 管理</button>
        </footer>
      {/if}
    </section>
  {/if}

  {#if loading}
    <div class="loading-indicator" aria-live="polite"><span></span>正在处理…</div>
  {/if}
</main>

{#if (dialogMode === "restore" && selectedCheckpoint !== null) || dialogMode === "undo"}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal" aria-modal="true" aria-labelledby="restore-title" aria-describedby="restore-description">
      <div class="modal-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 8a7 7 0 1 1-.2 8M5 4.5V8h3.5M12 8v4l2.5 1.5" /></svg></div>
      <h2 id="restore-title">{dialogMode === "undo" ? "撤销上次恢复？" : "恢复到这个版本？"}</h2>
      {#if dialogMode === "undo"}
        <p id="restore-description">项目会回到上次恢复之前的状态。操作前会自动保存现在的文件变化；完成后，这次恢复不能再次撤销。</p>
      {:else if selectedCheckpoint}
        <p id="restore-description">项目文件会回到 <strong>{formatTime(selectedCheckpoint.createdAt)}</strong> 的状态。EasilyGit 会先自动保存你现在的状态，恢复后也可以撤销。</p>
      {/if}
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-quiet" onclick={() => { selectedCheckpoint = null; dialogMode = null; }} disabled={loading}>再想一想</button>
        <button class="button button-primary" onclick={dialogMode === "undo" ? undoRestore : restoreVersion} disabled={loading}>{dialogMode === "undo" ? "撤销恢复" : "恢复这个版本"}</button>
      </div>
    </dialog>
  </div>
{/if}

{#if managedFilesOpen}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="file-manager-modal" aria-modal="true" aria-labelledby="file-manager-title">
      <div class="file-manager-heading">
        <div>
          <p class="eyebrow">项目文件</p>
          <h2 id="file-manager-title">管理哪些文件</h2>
        </div>
        <button class="error-dismiss" onclick={() => managedFilesOpen = false} aria-label="关闭">×</button>
      </div>
      <p class="file-manager-description">停止管理后，文件仍保留在电脑上；EasilyGit 不再保存或恢复它。Git 自身的跟踪状态不会改变。</p>
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      {#if managedFiles.length === 0}
        <div class="empty-managed-files">项目里还没有可管理的文件。</div>
      {:else}
        <ul class="managed-file-list">
          {#each managedFiles as file (file.path)}
            <li>
              <div class="managed-file-copy">
                <span class="managed-file-name" title={file.path}>{fileName(file.path)}</span>
                <span class="managed-file-path" title={file.path}>{file.path}</span>
              </div>
              {#if file.managed}
                <button class="file-rule-button" onclick={() => toggleFileManagement(file)} disabled={loading}>停止管理</button>
              {:else}
                <button class="file-rule-button file-rule-enable" onclick={() => toggleFileManagement(file)} disabled={loading}>恢复管理</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      <div class="modal-actions"><button class="button button-quiet" onclick={() => managedFilesOpen = false}>完成</button></div>
    </dialog>
  </div>
{/if}

{#if stopProtectionOpen}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal" aria-modal="true" aria-labelledby="stop-protection-title" aria-describedby="stop-protection-description">
      <div class="modal-icon modal-icon-danger">!</div>
      <h2 id="stop-protection-title">停止 EasilyGit 管理？</h2>
      <p id="stop-protection-description">这会删除 EasilyGit 的版本记录、文件管理设置和独立存储。项目文件、普通 Git 提交、分支和暂存区会保留。旧版本写入原 Git 存储的数据，可能需要 Git 后续整理时回收。</p>
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-quiet" onclick={() => stopProtectionOpen = false} disabled={loading}>取消</button>
        <button class="button button-danger" onclick={stopProtection} disabled={loading}>停止管理并清理记录</button>
      </div>
    </dialog>
  </div>
{/if}
