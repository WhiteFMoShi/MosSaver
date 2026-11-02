<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { cubicOut } from "svelte/easing";
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";

  type ChangeKind = "added" | "modified" | "deleted";
  type Theme = "light" | "dark";

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
    currentCheckpointId: string | null;
    canUndoRestore: boolean;
  }

  interface ProjectOverview {
    root: string;
    name: string;
    initialized: boolean;
    isGitRepository: boolean;
    changeCount: number;
  }

  let project = $state<ProjectState | null>(null);
  let openingProjectPath = $state<string | null>(null);
  let managedPaths = $state<string[]>([]);
  let folderOverviews = $state<Record<string, ProjectOverview>>({});
  let folderErrors = $state<Record<string, string>>({});
  let loading = $state(false);
  let refreshingFolders = $state(false);
  let errorMessage = $state("");
  let selectedCheckpoint = $state<Checkpoint | null>(null);
  let dialogMode = $state<"restore" | "undo" | null>(null);
  let managedFilesOpen = $state(false);
  let managedFiles = $state<ManagedFile[]>([]);
  let stopProtectionOpen = $state(false);
  let theme = $state<Theme>("light");
  let loadedCheckpointCount = 0;
  let autoRefreshTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingRefreshRoot: string | undefined;
  let lastFocusRefreshAt = 0;
  const overviewTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const queuedOverviewPaths = new Set<string>();
  let overviewQueue = Promise.resolve();

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const storedTheme = localStorage.getItem("mossaver-theme");
    applyTheme(storedTheme === "dark" ? "dark" : "light", false);

    listen<string>("project-files-changed", (event) => {
      scheduleProjectRefresh(event.payload);
      scheduleOverviewRefresh(event.payload);
    })
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch(() => {
        errorMessage = "自动同步暂不可用，你仍可以点击“刷新”查看最新状态。";
      });

    const refreshOnFocus = () => {
      if (document.visibilityState === "hidden") return;
      const now = Date.now();
      if (now - lastFocusRefreshAt < 5000) return;
      lastFocusRefreshAt = now;
      if (project?.initialized) {
        if (!loading) void refreshProject();
        return;
      }
      void refreshAllFolderOverviews();
    };
    window.addEventListener("focus", refreshOnFocus);
    void loadManagedFolders();

    return () => {
      disposed = true;
      window.removeEventListener("focus", refreshOnFocus);
      if (autoRefreshTimer) clearTimeout(autoRefreshTimer);
      for (const timer of overviewTimers.values()) clearTimeout(timer);
      overviewTimers.clear();
      unlisten?.();
      void invoke("stop_project_watch").catch(() => {});
    };
  });

  const changeLabels: Record<ChangeKind, string> = {
    added: "新增",
    modified: "已修改",
    deleted: "已删除",
  };

  const changedFolderCount = $derived(
    managedPaths.filter((path) => (folderOverviews[path]?.changeCount ?? 0) > 0).length,
  );
  const inaccessibleFolderCount = $derived(
    managedPaths.filter((path) => Boolean(folderErrors[path])).length,
  );
  const currentCheckpointVisible = $derived.by(() => {
    const currentProject = project;
    if (!currentProject?.currentCheckpointId) return false;
    return currentProject.checkpoints.some((checkpoint) => checkpoint.id === currentProject.currentCheckpointId);
  });
  const showCurrentFilesNode = $derived(
    Boolean(project && (project.currentChanges.length > 0 || !currentCheckpointVisible)),
  );

  function applyTheme(nextTheme: Theme, persist = true) {
    theme = nextTheme;
    document.documentElement.dataset.theme = nextTheme;
    if (persist) localStorage.setItem("mossaver-theme", nextTheme);
  }

  async function loadManagedFolders() {
    try {
      managedPaths = await invoke<string[]>("list_managed_projects");
      await syncProjectWatcher();
      await refreshAllFolderOverviews();
    } catch (error) {
      errorMessage = friendlyError(error);
    }
  }

  async function syncProjectWatcher() {
    try {
      await invoke("watch_projects", { paths: managedPaths });
    } catch (error) {
      errorMessage = typeof error === "string" ? error : "文件变化自动同步暂不可用。";
    }
  }

  async function refreshAllFolderOverviews() {
    if (managedPaths.length === 0 || refreshingFolders) return;
    refreshingFolders = true;
    try {
      for (const path of managedPaths) await refreshFolderOverview(path);
    } finally {
      refreshingFolders = false;
    }
  }

  function refreshFolderOverview(path: string): Promise<void> {
    if (queuedOverviewPaths.has(path)) return overviewQueue;
    queuedOverviewPaths.add(path);
    const refresh = overviewQueue.then(async () => {
      queuedOverviewPaths.delete(path);
      await readFolderOverview(path);
    });
    overviewQueue = refresh.then(() => {}, () => {});
    return overviewQueue;
  }

  async function readFolderOverview(path: string) {
    try {
      const overview = await invoke<ProjectOverview>("project_overview", { path });
      folderOverviews = { ...folderOverviews, [path]: overview };
      const { [path]: _removed, ...remainingErrors } = folderErrors;
      folderErrors = remainingErrors;
    } catch (error) {
      folderErrors = { ...folderErrors, [path]: friendlyError(error) };
    }
  }

  function updateFolderOverviewFromProject(state: ProjectState) {
    folderOverviews = {
      ...folderOverviews,
      [state.root]: {
        root: state.root,
        name: state.name,
        initialized: state.initialized,
        isGitRepository: state.isGitRepository,
        changeCount: state.currentChanges.length,
      },
    };
    const { [state.root]: _removed, ...remainingErrors } = folderErrors;
    folderErrors = remainingErrors;
  }

  function scheduleOverviewRefresh(path: string) {
    if (!managedPaths.includes(path)) return;
    const existing = overviewTimers.get(path);
    if (existing) clearTimeout(existing);
    overviewTimers.set(path, setTimeout(() => {
      overviewTimers.delete(path);
      if (project?.root === path && project.initialized) {
        updateFolderOverviewFromProject(project);
      } else {
        void refreshFolderOverview(path);
      }
    }, 450));
  }

  async function chooseFolder() {
    errorMessage = "";
    try {
      const selection = await open({ directory: true, multiple: false, title: "添加要保护的文件夹" });
      if (typeof selection === "string") await openProject(selection);
    } catch (error) {
      errorMessage = friendlyError(error);
    }
  }

  async function openProject(path: string) {
    if (loading) return;
    openingProjectPath = path;
    try {
      await perform(async () => {
        const opened = await invoke<ProjectState>("open_project", { path });
        await adoptProjectState(opened, 0);
        openingProjectPath = null;
        selectedCheckpoint = null;
        dialogMode = null;
        managedFilesOpen = false;
        stopProtectionOpen = false;
        if (opened.initialized) {
          await registerManagedFolder(opened.root);
          updateFolderOverviewFromProject(opened);
        }
      });
    } finally {
      openingProjectPath = null;
    }
  }

  async function registerManagedFolder(path: string) {
    managedPaths = await invoke<string[]>("add_managed_project", { path });
    await syncProjectWatcher();
  }

  async function initializeProject() {
    if (!project) return;
    await perform(async () => {
      const initialized = await invoke<ProjectState>("initialize_project", { path: project!.root });
      await adoptProjectState(initialized, 0);
      await registerManagedFolder(initialized.root);
      updateFolderOverviewFromProject(initialized);
    });
  }

  async function refreshProject() {
    if (!project) return;
    const previousCount = loadedCheckpointCount;
    await perform(async () => {
      const refreshed = await invoke<ProjectState>("refresh_project", { path: project!.root });
      await adoptProjectState(refreshed, previousCount);
      updateFolderOverviewFromProject(refreshed);
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
      updateFolderOverviewFromProject(saved);
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
      updateFolderOverviewFromProject(restored);
      selectedCheckpoint = null;
      dialogMode = null;
    });
  }

  async function undoRestore() {
    if (!project) return;
    await perform(async () => {
      const restored = await invoke<ProjectState>("undo_restore", { path: project!.root });
      await adoptProjectState(restored, loadedCheckpointCount);
      updateFolderOverviewFromProject(restored);
      dialogMode = null;
    });
  }

  async function removeManagedFolder(path: string) {
    await perform(async () => {
      managedPaths = await invoke<string[]>("remove_managed_project", { path });
      const { [path]: _overview, ...remainingOverviews } = folderOverviews;
      const { [path]: _error, ...remainingErrors } = folderErrors;
      folderOverviews = remainingOverviews;
      folderErrors = remainingErrors;
      await syncProjectWatcher();
    });
  }

  async function stopProtection() {
    if (!project) return;
    const root = project.root;
    await perform(async () => {
      await invoke<ProjectState>("disable_protection", { path: root });
      managedPaths = await invoke<string[]>("remove_managed_project", { path: root });
      const { [root]: _overview, ...remainingOverviews } = folderOverviews;
      folderOverviews = remainingOverviews;
      await syncProjectWatcher();
      managedFilesOpen = false;
      stopProtectionOpen = false;
      project = null;
      loadedCheckpointCount = 0;
    });
  }

  function returnHome() {
    project = null;
    openingProjectPath = null;
    errorMessage = "";
    selectedCheckpoint = null;
    dialogMode = null;
    managedFilesOpen = false;
    stopProtectionOpen = false;
    loadedCheckpointCount = 0;
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
      updateFolderOverviewFromProject(updated);
      managedFiles = await invoke<ManagedFile[]>("list_managed_files", { path: project!.root });
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
      if (loading) pendingRefreshRoot = root;
      else if (project?.initialized && project.root === root) void refreshProject();
    }, 600);
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
        CHECKPOINT_READ_ONLY: "这是原有 Git 提交，只能查看，不能通过 MosSaver 恢复。",
        NOTHING_TO_UNDO: "现在没有可以撤销的恢复操作。",
        INVALID_PATH: "项目中有一个文件名无法安全处理。",
        PERMISSION_DENIED: "没有权限访问项目文件，请检查文件夹权限。",
        RESTORE_FAILED: "恢复没有完成。请刷新项目状态后再试。",
        METADATA_ERROR: "MosSaver 的版本记录暂时无法读写，请检查文件夹权限。",
        GIT_ERROR: "保存项目时遇到问题，请检查 Git 和项目文件后重试。",
        INTERNAL_ERROR: "操作没有完成，请重试。",
      };
      return messages[code] ?? "操作没有完成，请重试。";
    }
    if (typeof error === "string") return error;
    return "操作没有完成，请重试。";
  }

  function folderStatus(path: string): { label: string; tone: string; detail: string } {
    const overview = folderOverviews[path];
    if (folderErrors[path]) return { label: "无法访问", tone: "danger", detail: folderErrors[path] };
    if (!overview) return { label: "正在读取", tone: "pending", detail: "正在检查文件夹状态" };
    if (!overview.initialized) return { label: "保护已停止", tone: "muted", detail: "打开文件夹可重新开启保护" };
    if (overview.changeCount > 0) {
      return { label: `${overview.changeCount} 项变化`, tone: "changed", detail: "尚未保存到新版本" };
    }
    return { label: "一切正常", tone: "saved", detail: "与最近保存的版本一致" };
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

  function checkpointLabel(checkpoint: Checkpoint): string {
    return checkpoint.label === "开始使用 EasilyGit" ? "开始使用 MosSaver" : checkpoint.label;
  }

  function fileName(path: string): string {
    return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
  }
</script>

<svelte:head>
  <title>{project ? `${project.name} · MosSaver` : "MosSaver · 轻松保存每一步"}</title>
</svelte:head>

<main class="app-shell">
  <header class="topbar">
    <button class="brand" aria-label="MosSaver 首页" onclick={returnHome} disabled={loading}>
      <span class="brand-mark" aria-hidden="true"><span></span><span></span><span></span></span>
      <span>MosSaver</span>
    </button>
    <div class="topbar-actions">
      {#if project || openingProjectPath}
        <button class="button button-subtle back-button" onclick={returnHome} disabled={loading}>
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m12.5 4.5-5.5 5.5 5.5 5.5" /></svg>
          我的文件夹
        </button>
      {:else}
        <button class="button button-primary add-button" onclick={chooseFolder} disabled={loading}>
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 4v12M4 10h12" /></svg>
          添加文件夹
        </button>
      {/if}
      <div class="theme-switch" role="group" aria-label="选择外观">
        <button class:active={theme === "light"} aria-pressed={theme === "light"} onclick={() => applyTheme("light")} title="浅色主题">
          <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="10" cy="10" r="3.3" /><path d="M10 2.3v1.6m0 12.2v1.6M2.3 10h1.6m12.2 0h1.6M4.55 4.55l1.1 1.1m8.7 8.7 1.1 1.1m0-10.9-1.1 1.1m-8.7 8.7-1.1 1.1" /></svg>
          <span>浅色</span>
        </button>
        <button class:active={theme === "dark"} aria-pressed={theme === "dark"} onclick={() => applyTheme("dark")} title="深色主题">
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M16.5 12.4A7 7 0 0 1 7.6 3.5 7.1 7.1 0 1 0 16.5 12.4Z" /></svg>
          <span>深色</span>
        </button>
      </div>
    </div>
  </header>

  {#if errorMessage}
    <div class="error-banner" role="alert">
      <span class="error-icon">!</span>
      <span>{errorMessage}</span>
      <button class="error-dismiss" onclick={() => errorMessage = ""} aria-label="关闭提示">×</button>
    </div>
  {/if}

  <div class="view-switcher">
  {#if openingProjectPath}
    <section class="project-view project-opening" in:fly={{ x: 14, duration: 230, easing: cubicOut }} out:fly={{ x: 10, duration: 150, easing: cubicOut }} aria-live="polite">
      <div class="project-heading">
        <div class="heading-copy">
          <p class="breadcrumb">我的文件夹 <span>›</span> 正在打开</p>
          <h1>{fileName(openingProjectPath)}</h1>
          <p class="project-path" title={openingProjectPath}>{openingProjectPath}</p>
        </div>
      </div>
      <div class="project-opening-state">
        <span class="project-opening-spinner" aria-hidden="true"></span>
        <div>
          <strong>正在读取文件夹状态…</strong>
          <p>马上就好</p>
        </div>
      </div>
    </section>
  {:else if !project}
    <section class="home-view" in:fly={{ x: -14, duration: 230, easing: cubicOut }} out:fly={{ x: -10, duration: 150, easing: cubicOut }}>
      <div class="home-heading">
        <div>
          <p class="eyebrow">你的工作空间</p>
          <h1>我的文件夹</h1>
          <p class="home-subtitle">在这里查看项目变化，需要时一键保存或回到之前的版本。</p>
        </div>
        {#if managedPaths.length > 0}
          <div class="folder-total"><strong>{managedPaths.length}</strong><span>个文件夹</span></div>
        {/if}
      </div>

      {#if managedPaths.length === 0}
        <section class="empty-home">
          <div class="empty-folder-icon" aria-hidden="true">
            <svg viewBox="0 0 48 48"><path d="M6 13.5A3.5 3.5 0 0 1 9.5 10h9l4 4h16A3.5 3.5 0 0 1 42 17.5v17a3.5 3.5 0 0 1-3.5 3.5h-29A3.5 3.5 0 0 1 6 34.5v-21Z" /><path d="M6.5 19h35" /></svg>
          </div>
          <h2>还没有添加文件夹</h2>
          <p>选择一个项目文件夹，MosSaver 就会帮你记录变化。</p>
          <button class="button button-primary button-large" onclick={chooseFolder} disabled={loading}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 4v12M4 10h12" /></svg>
            添加第一个文件夹
          </button>
          <span class="privacy-note"><span class="privacy-dot"></span>你的文件和版本都只保存在这台电脑上</span>
        </section>
      {:else}
        <div class="folder-grid">
          {#each managedPaths as path (path)}
            {@const state = folderStatus(path)}
            {@const overview = folderOverviews[path]}
            <article class="folder-card">
              <button class="folder-card-open" onclick={() => openProject(path)} disabled={loading}>
                <span class="folder-icon" aria-hidden="true">
                  <svg viewBox="0 0 24 24"><path d="M3 6.5A2.5 2.5 0 0 1 5.5 4h5l2.5 2.5h5A2.5 2.5 0 0 1 20.5 9v8.5a2.5 2.5 0 0 1-2.5 2.5H5.5A2.5 2.5 0 0 1 3 17.5v-11Z" /><path d="M3.5 9h17" /></svg>
                </span>
                <span class="folder-card-copy">
                  <span class="folder-name">{overview?.name ?? fileName(path)}</span>
                  <span class="folder-path" title={path}>{path}</span>
                </span>
                <span class="folder-arrow" aria-hidden="true">›</span>
              </button>
              <div class="folder-card-footer">
                <span class="folder-status status-{state.tone}"><span></span>{state.label}</span>
                <span class="folder-detail">{state.detail}</span>
                <button class="remove-folder" onclick={() => removeManagedFolder(path)} disabled={loading} aria-label={`从列表移除 ${fileName(path)}`} title="从列表移除">移除</button>
              </div>
            </article>
          {/each}
        </div>
        <p class="list-footnote"><span class="privacy-dot"></span>移出列表不会删除文件，也不会关闭该文件夹的版本保护。</p>
      {/if}
    </section>
  {:else}
    <section class="project-view" in:fly={{ x: 14, duration: 230, easing: cubicOut }} out:fly={{ x: 10, duration: 150, easing: cubicOut }}>
      <div class:project-heading-compact={project.initialized} class="project-heading">
        {#if !project.initialized}
          <div class="heading-copy">
            <h1>{project.name}</h1>
            <p class="project-path" title={project.root}>{project.root}</p>
          </div>
        {/if}
        {#if project.initialized}
          <div class="project-heading-actions">
            {#if project.canUndoRestore}
              <button class="button button-subtle" onclick={requestUndoRestore} disabled={loading}>撤销恢复</button>
            {/if}
            <button class="button button-subtle refresh-button" onclick={refreshProject} disabled={loading} aria-label="刷新文件夹状态">
              <svg class:spinning={loading} viewBox="0 0 20 20" aria-hidden="true"><path d="M16.3 8A6.5 6.5 0 0 0 4.9 5.7L3.5 7.1M3.7 4.2v3.2h3.2M3.7 12A6.5 6.5 0 0 0 15.1 14.3l1.4-1.4m-.2 3v-3.2h-3.2" /></svg>
              刷新
            </button>
            <button class="button button-primary" onclick={saveVersion} disabled={loading || project.currentChanges.length === 0}>
              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3.5v9m-3.5-3.5L10 12.5l3.5-3.5M4.5 15.5v1h11v-1" /></svg>
              保存版本
            </button>
          </div>
        {/if}
      </div>

      {#if !project.initialized}
        <section class="setup-card">
          <div class="setup-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.7 20 8v8l-8 4.3L4 16V8l8-4.3Z" /><path d="m4.4 8.2 7.6 4.1 7.6-4.1M12 12.4v7.4" /></svg></div>
          <div class="setup-copy">
            <p class="eyebrow">简单、安全、本地保存</p>
            <h2>{project.isGitRepository ? "检测到已有 Git 仓库" : "开启这个文件夹的版本保护"}</h2>
            {#if project.isGitRepository}
              <p>可以只读查看已有 Git 提交，并保存项目当前状态。MosSaver 不会移动分支或改动暂存区；只有 MosSaver 保存的版本可以恢复。</p>
            {:else}
              <p>开启后，MosSaver 会在本地记录项目版本。不会上传文件，也不会影响你电脑上的其他文件夹。</p>
            {/if}
            <div class="setup-actions">
              <button class="button button-primary" onclick={initializeProject} disabled={loading}>
                {loading ? "正在准备…" : "开启版本保护"}
                <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 10h12m-5-5 5 5-5 5" /></svg>
              </button>
              <button class="button button-subtle" onclick={returnHome} disabled={loading}>返回列表</button>
            </div>
          </div>
        </section>
      {:else}
        <div class="content-grid">
          <section class="panel changes-panel">
            <div class="panel-heading">
              <div><h2>文件变化 <span class="panel-context">{project.name}</span></h2><p>项目中的修改会显示在这里</p></div>
              <div class="panel-heading-actions">
                <button class="text-button" onclick={openManagedFiles} disabled={loading}>管理文件</button>
              </div>
            </div>
            {#if project.currentChanges.length === 0}
              <div class="empty-panel">
                <span class="empty-check">✓</span><strong>暂时没有新的变化</strong><span>有新修改时，会显示在这里。</span>
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
            {/if}
          </section>

          <section class="panel history-panel">
            <div class="panel-heading">
              <div><h2>保存记录</h2><p>MosSaver 版本可恢复 · Git 提交仅查看</p></div>
              <span class="history-total">{project.checkpoints.length}{project.hasMoreCheckpoints ? "+" : ""} 个</span>
            </div>
            {#if project.checkpoints.length === 0}
              <div class="empty-panel"><span class="history-empty-icon">◷</span><strong>还没有保存记录</strong><span>保存当前版本，为项目留下一个安全点。</span></div>
            {:else}
              <ol class="timeline">
                {#if showCurrentFilesNode}
                  <li class="timeline-item current-files-item">
                    <span class="timeline-marker current-files-marker"><span></span></span>
                    <div class="timeline-content">
                      <div class="timeline-title-row">
                        <h3>当前文件</h3>
                        <span class="latest-badge">当前</span>
                      </div>
                      <p class="timeline-time">
                        {project.currentChanges.length > 0
                          ? `${project.currentChanges.length} 项尚未保存的变化`
                          : "当前状态与保存基准一致"}
                      </p>
                    </div>
                  </li>
                {/if}
                {#each project.checkpoints as checkpoint (checkpoint.id)}
                  {@const isCurrentCheckpoint = checkpoint.id === project.currentCheckpointId}
                  <li class:current-checkpoint={isCurrentCheckpoint} class="timeline-item">
                    <span class="timeline-marker">{#if isCurrentCheckpoint}<span></span>{/if}</span>
                    <div class="timeline-content">
                      <div class="timeline-title-row">
                        <h3>{checkpointLabel(checkpoint)}</h3>
                        {#if isCurrentCheckpoint}<span class="latest-badge">{project.currentChanges.length > 0 ? "保存基准" : "当前"}</span>{/if}
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
              {#if project.hasMoreCheckpoints}<button class="load-more-button" onclick={loadMoreCheckpoints} disabled={loading}>加载更早的记录</button>{/if}
            {/if}
          </section>
        </div>

        <footer class="local-footer">
          <button class="stop-protection-link" onclick={() => stopProtectionOpen = true} disabled={loading}>停止此文件夹的保护</button>
        </footer>
      {/if}
    </section>
  {/if}
  </div>

  <footer class="app-statusbar" aria-label="应用状态" aria-live="polite">
    <div class="statusbar-left">
      <strong>MosSaver</strong>
      {#if project?.initialized}
        <span class="statusbar-divider"></span><span class="statusbar-path" title={project.root}>{project.root}</span>
      {:else if openingProjectPath}
        <span class="statusbar-divider"></span><span>{fileName(openingProjectPath)}</span>
        <span class="statusbar-divider"></span><span class="statusbar-path" title={openingProjectPath}>{openingProjectPath}</span>
      {:else if !project && managedPaths.length > 0}
        <span class="statusbar-divider"></span><span>{managedPaths.length} 个文件夹</span>
      {/if}
    </div>
    <div class="statusbar-right">
      {#if loading || refreshingFolders}
        <span class="statusbar-state is-busy"><span class="statusbar-spinner"></span>{loading ? "正在处理…" : "正在同步状态…"}</span>
      {:else if project?.initialized}
        <span class="statusbar-state" class:has-changes={project.currentChanges.length > 0}>
          <span class="statusbar-dot"></span>
          {project.currentChanges.length > 0 ? `${project.currentChanges.length} 项变化，尚未保存` : "已保存"}
        </span>
      {:else if project}
        <span class="statusbar-state is-muted"><span class="statusbar-dot"></span>保护未开启</span>
      {:else if inaccessibleFolderCount > 0}
        <span class="statusbar-state has-warning"><span class="statusbar-dot"></span>{inaccessibleFolderCount} 个文件夹无法读取</span>
      {:else if changedFolderCount > 0}
        <span class="statusbar-state has-changes"><span class="statusbar-dot"></span>{changedFolderCount} 个文件夹有变化</span>
      {:else if managedPaths.length > 0}
        <span class="statusbar-state"><span class="statusbar-dot"></span>文件夹状态正常</span>
      {:else}
        <span class="statusbar-state is-muted"><span class="statusbar-dot"></span>准备就绪</span>
      {/if}
    </div>
  </footer>
</main>

{#if (dialogMode === "restore" && selectedCheckpoint !== null) || dialogMode === "undo"}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal" aria-modal="true" aria-labelledby="restore-title" aria-describedby="restore-description">
      <div class="modal-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 8a7 7 0 1 1-.2 8M5 4.5V8h3.5M12 8v4l2.5 1.5" /></svg></div>
      <h2 id="restore-title">{dialogMode === "undo" ? "撤销上次恢复？" : "恢复到这个版本？"}</h2>
      {#if dialogMode === "undo"}
        <p id="restore-description">项目会回到上次恢复之前的状态。操作前会自动保存现在的文件变化；完成后，这次恢复不能再次撤销。</p>
      {:else if selectedCheckpoint}
        <p id="restore-description">项目文件会回到 <strong>{formatTime(selectedCheckpoint.createdAt)}</strong> 的状态。MosSaver 会先自动保存现在的状态，恢复后也可以撤销。</p>
      {/if}
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-subtle" onclick={() => { selectedCheckpoint = null; dialogMode = null; }} disabled={loading}>取消</button>
        <button class="button button-primary" onclick={dialogMode === "undo" ? undoRestore : restoreVersion} disabled={loading}>{dialogMode === "undo" ? "撤销恢复" : "恢复这个版本"}</button>
      </div>
    </dialog>
  </div>
{/if}

{#if managedFilesOpen}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="file-manager-modal" aria-modal="true" aria-labelledby="file-manager-title">
      <div class="file-manager-heading">
        <div><p class="eyebrow">项目文件</p><h2 id="file-manager-title">选择要保护的文件</h2></div>
        <button class="error-dismiss" onclick={() => managedFilesOpen = false} aria-label="关闭">×</button>
      </div>
      <p class="file-manager-description">停止保护某个文件后，它仍会留在电脑上；MosSaver 不再保存或恢复它。Git 自身的跟踪状态不会改变。</p>
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      {#if managedFiles.length === 0}
        <div class="empty-managed-files">这个项目里还没有可管理的文件。</div>
      {:else}
        <ul class="managed-file-list">
          {#each managedFiles as file (file.path)}
            <li>
              <div class="managed-file-copy"><span class="managed-file-name" title={file.path}>{fileName(file.path)}</span><span class="managed-file-path" title={file.path}>{file.path}</span></div>
              {#if file.managed}<button class="file-rule-button" onclick={() => toggleFileManagement(file)} disabled={loading}>停止保护</button>
              {:else}<button class="file-rule-button file-rule-enable" onclick={() => toggleFileManagement(file)} disabled={loading}>恢复保护</button>{/if}
            </li>
          {/each}
        </ul>
      {/if}
      <div class="modal-actions"><button class="button button-subtle" onclick={() => managedFilesOpen = false}>完成</button></div>
    </dialog>
  </div>
{/if}

{#if stopProtectionOpen}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal" aria-modal="true" aria-labelledby="stop-protection-title" aria-describedby="stop-protection-description">
      <div class="modal-icon modal-icon-danger">!</div>
      <h2 id="stop-protection-title">停止此文件夹的保护？</h2>
      <p id="stop-protection-description">这会删除 MosSaver 的版本记录、文件保护设置和独立存储。项目文件、普通 Git 提交、分支和暂存区会保留。</p>
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-subtle" onclick={() => stopProtectionOpen = false} disabled={loading}>取消</button>
        <button class="button button-danger" onclick={stopProtection} disabled={loading}>停止保护并清理记录</button>
      </div>
    </dialog>
  </div>
{/if}
