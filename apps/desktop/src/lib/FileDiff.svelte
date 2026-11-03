<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy } from "svelte";
  import { parsePatch, previewPosition } from "./diff";

  let { root, checkpointId, path, kind }: {
    root: string; checkpointId: string; path: string; kind: "added" | "modified" | "deleted";
  } = $props();

  interface Preview { path: string; patch: string; unavailableReason: string | null; truncated: boolean }
  let opened = $state(false);
  let pinned = $state(false);
  let expanded = $state(false);
  let busy = $state(false);
  let failure = $state("");
  let preview = $state<Preview | null>(null);
  let position = $state({ left: 12, top: 12, width: 620, height: 420 });
  let trigger: HTMLButtonElement;
  let popover = $state<HTMLDivElement>();
  let showTimer: ReturnType<typeof setTimeout> | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let requestVersion = 0;
  let alive = true;
  const lines = $derived(parsePatch(preview?.patch ?? ""));
  const addedCount = $derived(lines.filter((line) => line.kind === "added").length);
  const deletedCount = $derived(lines.filter((line) => line.kind === "deleted").length);

  // Escape the scrolled file list's clipping without adding a UI dependency.
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  function cancelHide() { clearTimeout(hideTimer); }
  function close() {
    clearTimeout(showTimer);
    clearTimeout(hideTimer);
    opened = false; pinned = false; expanded = false;
  }

  function scheduleClose() {
    clearTimeout(showTimer);
    if (!pinned) hideTimer = setTimeout(close, 240);
  }

  function scheduleOpen(event: PointerEvent) {
    cancelHide();
    if (opened) return;
    const { clientX, clientY } = event;
    clearTimeout(showTimer);
    showTimer = setTimeout(() => void show(clientX, clientY), 550);
  }

  async function show(x?: number, y?: number, pin = false) {
    clearTimeout(showTimer);
    cancelHide();
    const rect = trigger.getBoundingClientRect();
    position = previewPosition(x ?? rect.right, y ?? rect.top, window.innerWidth, window.innerHeight);
    pinned = pin || pinned;
    if (opened) return;
    opened = true;
    if (preview || busy) return;
    busy = true; failure = "";
    const version = ++requestVersion;
    try {
      const result = await invoke<Preview>("file_preview", { path: root, checkpointId, filePath: path });
      if (alive && version === requestVersion) preview = result;
    } catch {
      if (alive && version === requestVersion) failure = "暂时无法读取文件差异，请关闭后重试。";
    } finally {
      if (alive && version === requestVersion) busy = false;
    }
  }

  function scrollOrResize(event: Event) {
    if (event.type === "resize" || !(event.target instanceof Node && popover?.contains(event.target))) close();
  }

  function outsideClick(event: PointerEvent) {
    if (opened && event.target instanceof Node && !popover?.contains(event.target) && !trigger.contains(event.target)) close();
  }

  $effect(() => {
    if (!opened) return;
    const keydown = (event: KeyboardEvent) => { if (event.key === "Escape") close(); };
    const anotherPreview = () => close();
    window.dispatchEvent(new Event("mossaver-preview-open"));
    window.addEventListener("mossaver-preview-open", anotherPreview);
    window.addEventListener("keydown", keydown);
    window.addEventListener("pointerdown", outsideClick);
    window.addEventListener("resize", scrollOrResize);
    window.addEventListener("scroll", scrollOrResize, true);
    return () => {
      window.removeEventListener("mossaver-preview-open", anotherPreview);
      window.removeEventListener("keydown", keydown);
      window.removeEventListener("pointerdown", outsideClick);
      window.removeEventListener("resize", scrollOrResize);
      window.removeEventListener("scroll", scrollOrResize, true);
    };
  });

  onDestroy(() => { alive = false; requestVersion++; close(); });
</script>

<button bind:this={trigger} class="checkpoint-file file-preview-trigger" title="悬停预览差异，点击固定窗口"
  onpointerenter={scheduleOpen} onpointerleave={scheduleClose}
  onfocus={() => { clearTimeout(showTimer); showTimer = setTimeout(() => void show(), 550); }}
  onblur={scheduleClose} onclick={() => void show(undefined, undefined, true)} aria-expanded={opened}>
  <b class="summary-{kind}">{kind === "added" ? "+" : kind === "deleted" ? "−" : "·"}</b>{path}
</button>

{#if opened}
  <div bind:this={popover} use:portal class="diff-popover" class:diff-expanded={expanded}
    style={`left:${position.left}px;top:${position.top}px;width:${position.width}px;height:${position.height}px`}
    role="dialog" aria-label={`${path} 的文件差异`} tabindex="-1"
    onpointerenter={cancelHide} onpointerleave={scheduleClose}>
    <div class="diff-toolbar">
      <div class="diff-heading"><strong title={path}>{path}</strong><span>与父版本比较 · <span class="summary-added">+{addedCount}</span> / <span class="summary-deleted">−{deletedCount}</span></span></div>
      <button class="icon-action" class:is-active={pinned} title={pinned ? "取消固定" : "固定预览"} aria-label="固定预览" aria-pressed={pinned}
        onclick={() => pinned = !pinned}>
        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m7 3 6 0-1 5 3 3H5l3-3zM10 11v6"/></svg>
      </button>
      <button class="icon-action" title={expanded ? "缩小预览" : "放大预览"} aria-label="切换预览大小" onclick={() => { expanded = !expanded; pinned = true; }}>
        <svg viewBox="0 0 20 20" aria-hidden="true"><path d="M7 3H3v4m10-4h4v4M3 13v4h4m10-4v4h-4"/></svg>
      </button>
      <button class="icon-action" title="关闭预览 (Esc)" aria-label="关闭预览" onclick={close}>×</button>
    </div>
    <div class="diff-body" onscroll={(event) => event.stopPropagation()}>
      {#if busy}<p class="diff-message">正在读取差异…</p>
      {:else if failure}<p class="diff-message">{failure}</p>
      {:else if preview?.unavailableReason}<p class="diff-message">{preview.unavailableReason}</p>
      {:else if lines.length === 0}<p class="diff-message">文件内容没有文本差异。</p>
      {:else}
        <div class="diff-code" aria-label="文本差异">
          <div class="diff-line diff-column-heading" aria-hidden="true"><span class="diff-line-number">原行</span><span class="diff-line-number">新行</span><span class="diff-sign"></span><span>文件内容</span></div>
          {#each lines as line, index (index)}
            <div class="diff-line diff-{line.kind}"><span class="diff-line-number">{line.oldNumber ?? ""}</span><span class="diff-line-number">{line.newNumber ?? ""}</span><span class="diff-sign" aria-hidden="true">{line.sign}</span><code>{line.text || " "}</code></div>
          {/each}
        </div>
      {/if}
    </div>
    <div class="diff-footer"><span>只读预览 · <span class="summary-deleted">− 删除</span> / <span class="summary-added">+ 新增</span></span>{#if preview?.truncated}<span>差异过长，仅显示前面部分</span>{/if}</div>
  </div>
{/if}
