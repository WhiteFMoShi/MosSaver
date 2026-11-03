<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { check, type Update } from "@tauri-apps/plugin-updater";
  import { cubicOut } from "svelte/easing";
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import FileDiff from "./lib/FileDiff.svelte";
  import {
    buildTimelineEntries,
    buildTimelineGraph,
    CURRENT_FILES_ID,
    measureTimelineRows,
    type TimelineGraph,
  } from "./lib/timeline";

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
    parentId: string | null;
    mergeParentIds: string[];
    summary: CheckpointSummary | null;
    deleteBlockedReason?: "readOnly" | "current" | "recovery" | "hasChildren" | null;
  }

  interface CheckpointSummary {
    fileCount: number;
    added: number;
    modified: number;
    deleted: number;
  }

  interface CheckpointDetails {
    summary: CheckpointSummary;
    files: ChangedFile[];
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
  let dialogMode = $state<"save" | "restore" | "undo" | "delete" | "merge" | null>(null);
  let focusedCheckpointId = $state<string | null>(null);
  let draggedCheckpointId = $state<string | null>(null);
  let dropCheckpointId = $state<string | null>(null);
  let mergeSource = $state<Checkpoint | null>(null);
  let mergeTargetId = $state("");
  let mergeLabel = $state("");
  let operationNotice = $state("");
  let saveLabel = $state("");
  let checkpointDetails = $state<Record<string, CheckpointDetails>>({});
  let expandedCheckpointId = $state<string | null>(null);
  let loadingCheckpointId = $state<string | null>(null);
  let changesPaneWidth = $state(340);
  let timelineHost = $state<HTMLOListElement | null>(null);
  let timelineGraph = $state<TimelineGraph>({ width: 96, height: 0, nodes: [], edges: [] });
  const timelineGraphEdges = $derived([...timelineGraph.edges].sort((left, right) =>
    Number(left.selected) - Number(right.selected) || Number(left.kind === "merge") - Number(right.kind === "merge")));
  let paneResizeCleanup: (() => void) | undefined;
  let managedFilesOpen = $state(false);
  let managedFiles = $state<ManagedFile[]>([]);
  let stopProtectionOpen = $state(false);
  let theme = $state<Theme>("light");
  let availableUpdate = $state<Update | null>(null);
  let updateChecking = $state(false);
  let updateInstalling = $state(false);
  let updateProgress = $state<number | null>(null);
  let updateError = $state("");
  let loadedCheckpointCount = 0;
  let detailsRequest = 0;
  let autoRefreshTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingRefreshRoot: string | undefined;
  let lastFocusRefreshAt = 0;
  const overviewTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const queuedOverviewPaths = new Set<string>();
  let overviewQueue = Promise.resolve();
  let updateCheckTimer: ReturnType<typeof setTimeout> | undefined;
  let updateNoticeTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const storedTheme = localStorage.getItem("mossaver-theme");
    applyTheme(storedTheme === "dark" ? "dark" : "light", false);
    const storedPaneWidth = Number(localStorage.getItem("mossaver-changes-pane-width"));
    if (Number.isFinite(storedPaneWidth)) changesPaneWidth = Math.min(520, Math.max(280, storedPaneWidth));

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
    updateCheckTimer = window.setTimeout(() => {
      void checkForUpdate();
    }, 1500);

    return () => {
      disposed = true;
      window.removeEventListener("focus", refreshOnFocus);
      if (autoRefreshTimer) clearTimeout(autoRefreshTimer);
      for (const timer of overviewTimers.values()) clearTimeout(timer);
      overviewTimers.clear();
      if (updateCheckTimer) clearTimeout(updateCheckTimer);
      if (updateNoticeTimer) clearTimeout(updateNoticeTimer);
      void availableUpdate?.close().catch(() => {});
      unlisten?.();
      void invoke("stop_project_watch").catch(() => {});
      paneResizeCleanup?.();
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
  const timelineEntries = $derived(
    buildTimelineEntries(project?.checkpoints ?? [], project?.currentCheckpointId ?? null),
  );
  const checkpointsById = $derived(new Map((project?.checkpoints ?? []).map((checkpoint) => [checkpoint.id, checkpoint])));
  const mergeTargets = $derived(project?.checkpoints.filter((checkpoint) => checkpoint.kind !== "git" && checkpoint.id !== mergeSource?.id) ?? []);

  $effect(() => {
    timelineEntries;
    showCurrentFilesNode;
    expandedCheckpointId;
    focusedCheckpointId;
    if (!timelineHost) return;
    const frame = requestAnimationFrame(measureTimelineGraph);
    return () => cancelAnimationFrame(frame);
  });

  $effect(() => {
    if (!timelineHost) return;
    const observer = new ResizeObserver(() => measureTimelineGraph());
    observer.observe(timelineHost);
    return () => observer.disconnect();
  });

  function measureTimelineGraph() {
    const host = timelineHost;
    if (!host || !project) return;
    const rows = [...host.querySelectorAll<HTMLElement>("[data-timeline-row]")];
    const measurements = rows.map((row) => {
      const rect = row.getBoundingClientRect();
      const anchor = row.querySelector<HTMLElement>("[data-timeline-anchor]")?.getBoundingClientRect() ?? rect;
      return {
        id: row.dataset.timelineRow!,
        top: rect.top,
        height: rect.height,
        anchorTop: anchor.top,
        anchorHeight: anchor.height,
      };
    });
    const { centers, height } = measureTimelineRows({
      top: host.getBoundingClientRect().top,
      clientTop: host.clientTop,
      scrollTop: host.scrollTop,
      clientHeight: host.clientHeight,
    }, measurements, Number.parseFloat(getComputedStyle(host).paddingBottom) || 0);
    timelineGraph = buildTimelineGraph(timelineEntries, centers, project.currentCheckpointId, height, focusedCheckpointId);
  }

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

  function openSaveDialog() {
    if (!project || project.currentChanges.length === 0 || loading) return;
    saveLabel = "";
    errorMessage = "";
    dialogMode = "save";
  }

  async function saveVersion() {
    if (!project) return;
    const label = saveLabel.trim() || "未命名版本";
    await perform(async () => {
      const saved = await invoke<ProjectState>("save_version", {
        path: project!.root,
        label,
      });
      await adoptProjectState(saved, loadedCheckpointCount);
      updateFolderOverviewFromProject(saved);
      dialogMode = null;
      saveLabel = "";
    });
  }

  async function toggleCheckpointDetails(checkpoint: Checkpoint) {
    const root = project?.root;
    if (!root) return;
    const request = ++detailsRequest;
    loadingCheckpointId = null;
    if (expandedCheckpointId === checkpoint.id) {
      expandedCheckpointId = null;
      return;
    }
    expandedCheckpointId = checkpoint.id;
    if (checkpointDetails[checkpoint.id]) return;
    loadingCheckpointId = checkpoint.id;
    try {
      const details = await invoke<CheckpointDetails>("checkpoint_details", {
        path: root,
        checkpointId: checkpoint.id,
      });
      if (request === detailsRequest && project?.root === root) {
        checkpointDetails = { ...checkpointDetails, [checkpoint.id]: details };
      }
    } catch (error) {
      if (request === detailsRequest && project?.root === root) {
        errorMessage = friendlyError(error);
        expandedCheckpointId = null;
      }
    } finally {
      if (request === detailsRequest) loadingCheckpointId = null;
    }
  }

  function selectNode(id: string) {
    if (loading || id === CURRENT_FILES_ID) return;
    focusedCheckpointId = focusedCheckpointId === id ? null : id;
  }

  function deletionReason(checkpoint: Checkpoint): string {
    const reasons = {
      readOnly: "Git 历史仅供查看",
      current: "当前版本不能删除，请先切换到其他版本",
      recovery: "这是撤销恢复所需的安全点，不能删除",
      hasChildren: "此存档仍被后续版本或恢复安全点使用，不能删除",
    };
    if (checkpoint.deleteBlockedReason) return reasons[checkpoint.deleteBlockedReason];
    if (checkpoint.kind === "git") return "Git 历史仅供查看";
    if (checkpoint.id === project?.currentCheckpointId) return "当前版本不能删除，请先切换到其他版本";
    if (project?.checkpoints.some((child) => child.parentId === checkpoint.id || child.mergeParentIds?.includes(checkpoint.id))) return "已有后续版本，不能删除";
    return "删除存档点";
  }

  function requestDelete(checkpoint: Checkpoint) {
    if (loading) return;
    const reason = deletionReason(checkpoint);
    if (reason !== "删除存档点") { errorMessage = reason; return; }
    selectedCheckpoint = checkpoint;
    errorMessage = "";
    dialogMode = "delete";
  }

  function openMerge(source: Checkpoint, target?: Checkpoint) {
    if (loading || source.kind === "git" || source.id === target?.id || target?.kind === "git") return;
    mergeSource = source;
    mergeTargetId = target?.id ?? (project?.currentCheckpointId !== source.id ? project?.currentCheckpointId ?? "" : "");
    mergeLabel = `合并：${checkpointLabel(source)}`.slice(0, 120);
    errorMessage = "";
    dialogMode = "merge";
  }

  function startNodeDrag(event: DragEvent, checkpoint: Checkpoint) {
    if (loading || checkpoint.kind === "git") { event.preventDefault(); return; }
    draggedCheckpointId = checkpoint.id;
    focusedCheckpointId = checkpoint.id;
    event.dataTransfer?.setData("application/x-mossaver-checkpoint", checkpoint.id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "linkMove";
  }

  function dragOverNode(event: DragEvent, checkpoint: Checkpoint) {
    if (loading || !draggedCheckpointId || draggedCheckpointId === checkpoint.id || checkpoint.kind === "git") return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "link";
    dropCheckpointId = checkpoint.id;
  }

  function endNodeDrag() { draggedCheckpointId = null; dropCheckpointId = null; }

  function dropOnNode(event: DragEvent, checkpoint: Checkpoint) {
    event.preventDefault();
    const source = checkpointsById.get(draggedCheckpointId ?? "");
    endNodeDrag();
    if (source) openMerge(source, checkpoint);
  }

  async function deleteVersion() {
    if (!project || !selectedCheckpoint || loading) return;
    const checkpoint = selectedCheckpoint;
    await perform(async () => {
      const updated = await invoke<ProjectState>("delete_checkpoint", { path: project!.root, checkpointId: checkpoint.id });
      await adoptProjectState(updated, Math.max(0, loadedCheckpointCount - 1));
      updateFolderOverviewFromProject(updated);
      if (expandedCheckpointId === checkpoint.id) expandedCheckpointId = null;
      const { [checkpoint.id]: _deleted, ...details } = checkpointDetails;
      checkpointDetails = details;
      focusedCheckpointId = null; selectedCheckpoint = null; dialogMode = null;
      operationNotice = "已删除存档点";
    });
  }

  async function mergeVersions() {
    if (!project || !mergeSource || !mergeTargetId || loading) return;
    const existingIds = new Set(project.checkpoints.map((checkpoint) => checkpoint.id));
    await perform(async () => {
      const updated = await invoke<ProjectState>("merge_checkpoints", {
        path: project!.root, sourceId: mergeSource!.id, targetId: mergeTargetId, label: mergeLabel,
      });
      await adoptProjectState(updated, loadedCheckpointCount + 1);
      updateFolderOverviewFromProject(updated);
      focusedCheckpointId = project?.checkpoints.find((checkpoint) => !existingIds.has(checkpoint.id) && checkpoint.parentId === mergeTargetId && checkpoint.mergeParentIds?.includes(mergeSource!.id))?.id ?? null;
      dialogMode = null;
      operationNotice = "合并存档已创建 · 当前文件保持原样，可选择合并存档恢复";
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
    detailsRequest++;
    loadingCheckpointId = null;
    project = null;
    openingProjectPath = null;
    errorMessage = "";
    selectedCheckpoint = null;
    dialogMode = null;
    managedFilesOpen = false;
    stopProtectionOpen = false;
    loadedCheckpointCount = 0;
    focusedCheckpointId = null;
    endNodeDrag();
    operationNotice = "";
    checkpointDetails = {};
    expandedCheckpointId = null;
  }

  async function adoptProjectState(next: ProjectState, preserveCheckpointCount: number) {
    if (project?.root !== next.root) {
      detailsRequest++;
      loadingCheckpointId = null;
      checkpointDetails = {};
      expandedCheckpointId = null;
      focusedCheckpointId = null;
      operationNotice = "";
    }
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
    if (loading) return;
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

  function showUpdateNotice(message: string, duration = 4500) {
    operationNotice = message;
    if (updateNoticeTimer) clearTimeout(updateNoticeTimer);
    if (duration > 0) {
      updateNoticeTimer = window.setTimeout(() => {
        updateNoticeTimer = undefined;
        if (!availableUpdate && !updateInstalling) operationNotice = "";
      }, duration);
    }
  }

  async function checkForUpdate(manual = false) {
    if (updateChecking || updateInstalling || availableUpdate) return;
    updateChecking = true;
    if (manual) showUpdateNotice("正在检查更新…", 0);
    try {
      const update = await check({ timeout: 8000 });
      if (update) {
        availableUpdate = update;
        updateError = "";
        updateProgress = null;
        showUpdateNotice(`发现 MosSaver ${update.version} 更新`, 0);
      } else if (manual) {
        showUpdateNotice("已是最新版本");
      }
    } catch (error) {
      if (manual) showUpdateNotice("暂时无法检查更新，请稍后再试");
      console.warn("MosSaver update check failed", error);
    } finally {
      updateChecking = false;
    }
  }

  async function installUpdate() {
    const update = availableUpdate;
    if (!update || updateInstalling) return;
    updateInstalling = true;
    updateError = "";
    updateProgress = 0;
    operationNotice = `正在下载 MosSaver ${update.version}…`;
    try {
      let downloaded = 0;
      let total = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
          updateProgress = total > 0 ? 0 : null;
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          updateProgress = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null;
        } else if (event.event === "Finished") {
          updateProgress = 100;
        }
      });
      operationNotice = "更新已安装，正在重启 MosSaver…";
      await relaunch();
    } catch (error) {
      updateError = "更新没有完成。请检查网络后重试，或稍后从 GitHub Release 手动安装。";
      operationNotice = "更新失败";
      console.warn("MosSaver update installation failed", error);
    } finally {
      updateInstalling = false;
    }
  }

  async function dismissUpdate() {
    if (updateInstalling) return;
    const update = availableUpdate;
    availableUpdate = null;
    updateError = "";
    updateProgress = null;
    if (updateNoticeTimer) clearTimeout(updateNoticeTimer);
    operationNotice = "";
    await update?.close().catch(() => {});
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
        CHECKPOINT_IN_USE: "这个存档是当前版本、其他版本的来源或恢复安全点，暂时不能删除。",
        MERGE_NOT_ALLOWED: "这两个版本无法合并。请选择两个从共同存档派生、且互不包含的版本，并确认 Git 为 2.38 或更新版本。",
        MERGE_CONFLICT: "两个版本修改了同一处内容，暂时无法自动合并。项目文件和原有存档保持原样。",
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

  function checkpointReference(checkpoint: Checkpoint): string {
    const date = new Date(checkpoint.createdAt);
    const shortTime = Number.isNaN(date.getTime())
      ? checkpoint.createdAt
      : new Intl.DateTimeFormat("zh-CN", {
          month: "2-digit",
          day: "2-digit",
          hour: "2-digit",
          minute: "2-digit",
        }).format(date);
    return `${shortTime} · ${checkpointLabel(checkpoint)}`;
  }

  function setPaneWidth(width: number) {
    changesPaneWidth = Math.min(520, Math.max(280, width));
  }

  function adjustPaneWidth(delta: number) {
    setPaneWidth(changesPaneWidth + delta);
    localStorage.setItem("mossaver-changes-pane-width", String(changesPaneWidth));
  }

  function startPaneResize(event: PointerEvent) {
    if (window.innerWidth <= 860) return;
    paneResizeCleanup?.();
    const startX = event.clientX;
    const startWidth = changesPaneWidth;
    const move = (moveEvent: PointerEvent) => setPaneWidth(startWidth + moveEvent.clientX - startX);
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem("mossaver-changes-pane-width", String(changesPaneWidth));
      paneResizeCleanup = undefined;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop, { once: true });
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    paneResizeCleanup = stop;
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
          <h1>我的文件夹</h1>
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
        <p class="list-footnote"><svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="6"/><path d="M8 7.2v3.4M8 5.2h.01"/></svg><span>移出列表不会删除文件，也不会关闭该文件夹的版本保护。</span></p>
      {/if}
    </section>
  {:else}
    <section class:project-main={project.initialized} class="project-view" in:fly={{ x: 14, duration: 230, easing: cubicOut }} out:fly={{ x: 10, duration: 150, easing: cubicOut }}>
      {#if !project.initialized}
        <div class="project-heading">
          <div class="heading-copy">
            <h1>{project.name}</h1>
            <p class="project-path" title={project.root}>{project.root}</p>
          </div>
        </div>
      {/if}

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
        <div class="content-grid" style={`--changes-pane-width: ${changesPaneWidth}px`}>
          <section class="panel changes-panel">
            <div class="panel-heading">
              <div class="panel-heading-copy">
                <h2><span>文件变化</span><span class="panel-context" title={project.name}>{project.name}</span></h2>
              </div>
              <div class="panel-heading-actions">
                <button class="icon-action" data-tooltip="管理文件" aria-label="管理文件" onclick={openManagedFiles} disabled={loading}>
                  <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M2.8 5.5h5l1.5 1.7h7.9v7.5a1.5 1.5 0 0 1-1.5 1.5H4.3a1.5 1.5 0 0 1-1.5-1.5z"/><path d="M2.8 8.2h14.4"/></svg>
                </button>
                {#if project.canUndoRestore}
                  <button class="icon-action" data-tooltip="撤销上次恢复" aria-label="撤销上次恢复" onclick={requestUndoRestore} disabled={loading}>
                    <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 7.5a6.5 6.5 0 1 1-.1 5M4 3.8v3.8h3.8"/><path d="M10 6.5v3.8l2.3 1.4"/></svg>
                  </button>
                {/if}
                <button class="icon-action icon-action-primary" data-tooltip="保存版本" aria-label="保存版本" onclick={openSaveDialog} disabled={loading || project.currentChanges.length === 0}>
                  <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3.5 3.5h10l3 3v10h-13z"/><path d="M6.5 3.5v5h7v-5M6.5 16.5v-5h7v5"/></svg>
                </button>
              </div>
            </div>
            {#if project.currentChanges.length === 0}
              <div class="empty-panel">
                <span class="empty-check">✓</span><strong>暂时没有新的变化</strong>
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

          <button
            class="pane-divider"
            aria-label="调整文件变化区宽度"
            tabindex="0"
            onpointerdown={startPaneResize}
            onkeydown={(event) => {
              if (event.key === "ArrowLeft") { event.preventDefault(); adjustPaneWidth(-16); }
              if (event.key === "ArrowRight") { event.preventDefault(); adjustPaneWidth(16); }
            }}
          ></button>

          <section class="panel history-panel">
            <div class="panel-heading">
              <div><h2>保存记录</h2><p>{#if draggedCheckpointId}拖到另一个节点合并，或拖到垃圾桶删除{:else}<span class="history-legend-item"><i class="history-legend-line"></i>直接来源</span><span class="history-legend-item"><i class="history-legend-line history-legend-merge"></i>合并来源</span>{/if}</p></div>
              {#if draggedCheckpointId}
                <button class="node-trash-target icon-action" aria-label="拖到此处删除存档点" title="拖到此处删除存档点"
                  ondragover={(event) => { event.preventDefault(); if (event.dataTransfer) event.dataTransfer.dropEffect = "move"; }}
                  ondrop={(event) => { event.preventDefault(); const checkpoint = checkpointsById.get(draggedCheckpointId!); endNodeDrag(); if (checkpoint) requestDelete(checkpoint); }}>
                  <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 5h14M7 5V3h6v2M5 5l1 12h8l1-12M8 8v6m4-6v6"/></svg>
                </button>
              {:else}<span class="history-total">{project.checkpoints.length}{project.hasMoreCheckpoints ? "+" : ""} 个</span>{/if}
            </div>
            {#if project.checkpoints.length === 0}
              <div class="empty-panel"><span class="history-empty-icon">◷</span><strong>还没有保存记录</strong><span>保存当前版本，为项目留下一个安全点。</span></div>
            {:else}
              <div class="timeline-scroll">
              <ol class="timeline" bind:this={timelineHost} style={`--timeline-graph-width: ${timelineGraph.width}px`}>
                {#if timelineGraph.height > 0}
                  <svg
                    class="timeline-graph-layer"
                    width={timelineGraph.width}
                    height={timelineGraph.height}
                    viewBox={`0 0 ${timelineGraph.width} ${timelineGraph.height}`}
                    aria-hidden="true"
                  >
                    {#each timelineGraphEdges as edge (edge.id)}
                      <g class:graph-edge-context-dim={focusedCheckpointId !== null && !edge.selected}>
                        <path class="graph-edge-halo" d={edge.path}></path>
                        <path class:graph-edge-active={edge.active} class:graph-edge-merge={edge.kind === "merge"}
                          class:graph-edge-selected={edge.selected} data-kind={edge.kind}
                          data-from={edge.fromId} data-to={edge.toId} d={edge.path}></path>
                      </g>
                    {/each}
                    {#each timelineGraph.nodes as node (node.id)}
                      {#if focusedCheckpointId === node.id}
                        <circle class="graph-node-selection-halo" cx={node.x} cy={node.y} r="11"></circle>
                      {/if}
                      {#if focusedCheckpointId === node.id}
                        <path class="graph-node-selected" class:graph-node-current={node.current} d={`M ${node.x} ${node.y - 8} L ${node.x + 8} ${node.y} L ${node.x} ${node.y + 8} L ${node.x - 8} ${node.y} Z`} data-node-id={node.id}></path>
                      {:else}
                      <circle
                        class:graph-node-active={node.active}
                        class:graph-node-current={node.current}
                        class:graph-node-related={node.relatedToSelection}
                        class:graph-node-context-dim={focusedCheckpointId !== null && !node.relatedToSelection}
                        data-node-id={node.id}
                        cx={node.x}
                        cy={node.y}
                        r={node.current ? 6 : 5}
                      ></circle>
                      {/if}
                      {#if node.current && focusedCheckpointId !== node.id}<circle class="graph-node-dot" cx={node.x} cy={node.y} r="2.2"></circle>{/if}
                    {/each}
                  </svg>
                {/if}
                <li class="graph-controls" aria-label="版本图节点">
                {#each timelineGraph.nodes as node (node.id)}
                  {@const checkpoint = checkpointsById.get(node.id)}
                  {#if checkpoint}
                    <button class="graph-node-control" class:node-selected={focusedCheckpointId === node.id} class:node-drop-target={dropCheckpointId === node.id}
                      style={`left:${node.x - 13}px;top:${node.y - 13}px`}
                      aria-label={`选择版本：${checkpointLabel(checkpoint)}`} aria-pressed={focusedCheckpointId === node.id}
                      title={checkpoint.kind === "git" ? "Git 历史，仅查看" : "点击选择 · 拖到其他节点合并"}
                      draggable={checkpoint.kind !== "git" && !loading} disabled={loading}
                      onclick={() => selectNode(node.id)} ondragstart={(event) => startNodeDrag(event, checkpoint)}
                      ondragend={endNodeDrag} ondragover={(event) => dragOverNode(event, checkpoint)}
                      ondragleave={() => dropCheckpointId = null} ondrop={(event) => dropOnNode(event, checkpoint)}></button>
                  {/if}
                {/each}
                </li>
                {#if showCurrentFilesNode}
                  <li class="timeline-item current-files-item" data-timeline-row={CURRENT_FILES_ID}>
                    <div class="timeline-content">
                      <div class="timeline-title-row" data-timeline-anchor>
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
                {#each timelineEntries as entry (entry.checkpoint.id)}
                  {@const checkpoint = entry.checkpoint}
                  {@const isCurrentCheckpoint = checkpoint.id === project.currentCheckpointId}
                  <li class:current-checkpoint={isCurrentCheckpoint} class:timeline-branch={entry.hasSibling} class:active-lineage={entry.isActiveLineage} class:checkpoint-selected={focusedCheckpointId === checkpoint.id} class:checkpoint-drop-target={dropCheckpointId === checkpoint.id} class="timeline-item" data-timeline-row={checkpoint.id}>
                    <div class="timeline-content">
                      <div class="timeline-title-row" data-timeline-anchor>
                        <h3><button class="checkpoint-title-button" aria-pressed={focusedCheckpointId === checkpoint.id}
                          draggable={checkpoint.kind !== "git" && !loading} disabled={loading}
                          onclick={() => selectNode(checkpoint.id)} ondragstart={(event) => startNodeDrag(event, checkpoint)}
                          ondragend={endNodeDrag} ondragover={(event) => dragOverNode(event, checkpoint)}
                          ondragleave={() => dropCheckpointId = null} ondrop={(event) => dropOnNode(event, checkpoint)}
                          title={checkpoint.kind === "git" ? "选择版本" : "点击选择 · 拖动合并"}>{checkpointLabel(checkpoint)}</button></h3>
                        {#if checkpoint.kind === "git"}
                          <span class="readonly-icon" role="img" title="Git 提交，仅供查看" aria-label="Git 提交，仅供查看">
                            <svg viewBox="0 0 20 20" aria-hidden="true"><rect x="5.2" y="8.5" width="9.6" height="8" rx="1.5"/><path d="M7.2 8.5V6.7a2.8 2.8 0 0 1 5.6 0v1.8"/></svg>
                          </span>
                        {/if}
                        {#if isCurrentCheckpoint}<span class="latest-badge">{project.currentChanges.length > 0 ? "保存基准" : "当前"}</span>{/if}
                        {#if checkpoint.mergeParentIds?.length}<span class="merge-badge" title="此存档连接了直接来源和合并来源">合并点</span>{/if}
                      </div>
                      {#if focusedCheckpointId === checkpoint.id && checkpoint.kind !== "git"}
                        <div class="node-actions" aria-label="版本操作">
                          <button class="icon-action" data-tooltip="恢复到此版本" aria-label="恢复到此版本" disabled={loading}
                            onclick={() => { errorMessage = ""; selectedCheckpoint = checkpoint; dialogMode = "restore"; }}>
                            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M4 8a6 6 0 1 1-.2 3M4 4.5V8h3.5M10 7v3l2 1.2"/></svg>
                          </button>
                          <button class="icon-action" data-tooltip="与其他版本合并" aria-label="与其他版本合并" disabled={loading || project.checkpoints.filter((item) => item.kind !== "git").length < 2} onclick={() => openMerge(checkpoint)}>
                            <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="5" cy="4" r="2"/><circle cx="15" cy="4" r="2"/><circle cx="5" cy="16" r="2"/><path d="M5 6v8M15 6c0 5-10 1-10 6"/></svg>
                          </button>
                          <span title={deletionReason(checkpoint)}>
                            <button class="icon-action node-delete" aria-label={deletionReason(checkpoint)} disabled={loading || deletionReason(checkpoint) !== "删除存档点"} onclick={() => requestDelete(checkpoint)}>
                              <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 5h14M7 5V3h6v2M5 5l1 12h8l1-12M8 8v6m4-6v6"/></svg>
                            </button>
                          </span>
                        </div>
                      {/if}
                      <p class="timeline-time">{formatTime(checkpoint.createdAt)}</p>
                      {#if checkpoint.parentId && (entry.hasSibling || isCurrentCheckpoint)}
                        <span class="timeline-fork">
                          {entry.hasSibling ? "分支自" : "基于"}
                          {entry.parent ? checkpointReference(entry.parent) : checkpoint.parentId.startsWith("git-") ? `Git 历史 ${checkpoint.parentId.slice(4, 10)}` : "较早版本"}
                        </span>
                      {/if}
                      {#if checkpoint.summary}
                        <div class="checkpoint-summary">
                          <span>{checkpoint.summary.fileCount} 个文件</span>
                          {#if checkpoint.summary.added > 0}<span class="summary-added">+{checkpoint.summary.added}</span>{/if}
                          {#if checkpoint.summary.modified > 0}<span class="summary-modified">·{checkpoint.summary.modified}</span>{/if}
                          {#if checkpoint.summary.deleted > 0}<span class="summary-deleted">−{checkpoint.summary.deleted}</span>{/if}
                          <button class="details-link" onclick={() => toggleCheckpointDetails(checkpoint)} disabled={loadingCheckpointId === checkpoint.id}>
                            {loadingCheckpointId === checkpoint.id ? "读取中…" : expandedCheckpointId === checkpoint.id ? "收起变更" : "查看变更"}
                          </button>
                        </div>
                      {:else}
                        <button class="details-link details-link-standalone" onclick={() => toggleCheckpointDetails(checkpoint)} disabled={loadingCheckpointId === checkpoint.id}>
                          {loadingCheckpointId === checkpoint.id ? "读取中…" : expandedCheckpointId === checkpoint.id ? "收起详情" : "查看文件详情"}
                        </button>
                      {/if}
                      {#if expandedCheckpointId === checkpoint.id && checkpointDetails[checkpoint.id]}
                        <div class="checkpoint-details">
                          {#each checkpointDetails[checkpoint.id].files as file (file.path)}
                            <FileDiff root={project.root} checkpointId={checkpoint.id} path={file.path} kind={file.kind} />
                          {:else}
                            <span class="checkpoint-no-changes">与父版本相比没有文件变化</span>
                          {/each}
                        </div>
                      {/if}
                      {#if checkpoint.kind === "automatic"}<span class="automatic-note">自动安全保存</span>{/if}
                    </div>
                  </li>
                {/each}
              </ol>
              </div>
              {#if project.hasMoreCheckpoints}<button class="load-more-button" onclick={loadMoreCheckpoints} disabled={loading}>加载更早的记录</button>{/if}
            {/if}
          </section>
        </div>

        <footer class="local-footer">
          <span class="protection-status">
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M10 2.5 16 5v4.8c0 3.5-2.4 6.1-6 7.7-3.6-1.6-6-4.2-6-7.7V5l6-2.5Z"/><path d="m7 9.8 2 2 4-4"/></svg>
            <strong>此文件夹正在受保护</strong>
          </span>
          <button class="stop-protection-link" onclick={() => stopProtectionOpen = true} disabled={loading}>
            <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M5 5l10 10M15 5 5 15"/></svg>
            停止保护
          </button>
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
      {#if operationNotice}<span class="statusbar-notice" title={operationNotice}>{operationNotice}</span>{/if}
      {#if updateChecking}
        <span class="statusbar-state is-busy"><span class="statusbar-spinner"></span>正在检查更新…</span>
      {:else if updateInstalling}
        <span class="statusbar-state is-busy"><span class="statusbar-spinner"></span>正在更新…</span>
      {:else if loading || refreshingFolders}
        <span class="statusbar-state is-busy"><span class="statusbar-spinner"></span>{loading ? "正在处理…" : "正在同步状态…"}</span>
      {:else if project?.initialized}
        <button class="statusbar-refresh" onclick={refreshProject} disabled={loading} title="刷新当前文件夹状态">
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M16.3 8A6.5 6.5 0 0 0 4.9 5.7L3.5 7.1M3.7 4.2v3.2h3.2M3.7 12A6.5 6.5 0 0 0 15.1 14.3l1.4-1.4m-.2 3v-3.2h-3.2" /></svg>
          刷新
        </button>
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

{#if dialogMode === "save"}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal save-modal" aria-modal="true" aria-labelledby="save-title" aria-describedby="save-description">
      <div class="modal-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 4.5h11l3 3v12H5zM8 4.5v5h8v-5M8 19.5v-6h8v6" /></svg></div>
      <h2 id="save-title">保存当前版本</h2>
      <p id="save-description">给这次修改写一句说明，之后可以更容易地找到和恢复它。</p>
      <input class="save-label-input" bind:value={saveLabel} maxlength="120" placeholder="例如：完成首页布局" autofocus onkeydown={(event) => event.key === "Enter" && saveVersion()} />
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-subtle" onclick={() => dialogMode = null} disabled={loading}>取消</button>
        <button class="button button-primary" onclick={saveVersion} disabled={loading}>{loading ? "正在保存…" : "保存版本"}</button>
      </div>
    </dialog>
  </div>
{/if}

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

{#if dialogMode === "delete" && selectedCheckpoint}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal" aria-modal="true" aria-labelledby="delete-title">
      <div class="modal-icon modal-icon-danger">!</div>
      <h2 id="delete-title">删除这个存档点？</h2>
      <p><strong>{checkpointLabel(selectedCheckpoint)}</strong> 将从保存记录中移除，此操作不能撤销。项目文件保持原样。当前版本、已有后续版本的存档和恢复安全点会受到保护。</p>
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-subtle" onclick={() => { selectedCheckpoint = null; dialogMode = null; }} disabled={loading}>取消</button>
        <button class="button button-danger" onclick={deleteVersion} disabled={loading}>{loading ? "正在删除…" : "删除存档点"}</button>
      </div>
    </dialog>
  </div>
{/if}

{#if dialogMode === "merge" && mergeSource}
  <div class="modal-backdrop" role="presentation">
    <dialog open class="confirm-modal" aria-modal="true" aria-labelledby="merge-title">
      <div class="modal-icon"><svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="5" cy="4" r="2"/><circle cx="15" cy="4" r="2"/><circle cx="5" cy="16" r="2"/><path d="M5 6v8M15 6c0 5-10 1-10 6"/></svg></div>
      <h2 id="merge-title">合并两个存档</h2>
      <p>将 <strong>{checkpointLabel(mergeSource)}</strong> 的变化合入下面的版本，生成一个保留两条来源的新存档。当前文件保持原样；需要时再恢复到合并存档。有冲突时会停止合并。</p>
      <label class="merge-field">合入版本
        <select bind:value={mergeTargetId} disabled={loading}>
          <option value="" disabled>选择另一个版本</option>
          {#each mergeTargets as target (target.id)}<option value={target.id}>{checkpointLabel(target)} · {formatTime(target.createdAt)}</option>{/each}
        </select>
      </label>
      <label class="merge-field">新存档说明<input bind:value={mergeLabel} maxlength="120" disabled={loading} /></label>
      {#if errorMessage}<p class="modal-error" role="alert">{errorMessage}</p>{/if}
      <div class="modal-actions">
        <button class="button button-subtle" onclick={() => dialogMode = null} disabled={loading}>取消</button>
        <button class="button button-primary" onclick={mergeVersions} disabled={loading || !mergeTargetId}>{loading ? "正在合并…" : "创建合并存档"}</button>
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

{#if availableUpdate}
  <div class="modal-backdrop update-backdrop" role="presentation">
    <dialog open class="confirm-modal update-modal" aria-modal="true" aria-labelledby="update-title" aria-describedby="update-description">
      <div class="modal-icon update-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 16V4m0 0L7.5 8.5M12 4l4.5 4.5M5 15.5v2A2.5 2.5 0 0 0 7.5 20h9a2.5 2.5 0 0 0 2.5-2.5v-2" /></svg></div>
      <h2 id="update-title">{updateInstalling ? "正在更新 MosSaver" : "发现新版本"}</h2>
      {#if updateInstalling}
        <p id="update-description">正在下载 MosSaver {availableUpdate.version}，完成后应用会自动重启。</p>
        <div class="update-progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={updateProgress ?? undefined}>
          <span style={`width: ${updateProgress ?? 30}%`}></span>
        </div>
        <p class="update-progress-label">{updateProgress === null ? "正在下载…" : `${updateProgress}%`}</p>
      {:else}
        <p id="update-description">当前版本 {availableUpdate.currentVersion}，可以更新到 MosSaver {availableUpdate.version}。</p>
        {#if availableUpdate.body}<div class="update-notes">{availableUpdate.body}</div>{/if}
        {#if updateError}<p class="modal-error" role="alert">{updateError}</p>{/if}
        <div class="modal-actions">
          <button class="button button-subtle" onclick={dismissUpdate}>稍后</button>
          <button class="button button-primary" onclick={installUpdate}>{updateError ? "重试更新" : "立即更新"}</button>
        </div>
      {/if}
    </dialog>
  </div>
{/if}
