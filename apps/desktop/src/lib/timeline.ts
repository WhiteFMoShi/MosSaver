export interface TimelineCheckpoint {
  id: string;
  createdAt: string;
  parentId: string | null;
}

export interface TimelineEntry<T extends TimelineCheckpoint = TimelineCheckpoint> {
  checkpoint: T;
  lane: number;
  parent: T | null;
  hasSibling: boolean;
  isActiveLineage: boolean;
}

export interface GraphPoint {
  x: number;
  y: number;
}

export interface TimelineGraphNode extends GraphPoint {
  id: string;
  active: boolean;
  current: boolean;
}

export interface TimelineGraphEdge {
  id: string;
  fromId: string;
  toId: string;
  points: GraphPoint[];
  path: string;
  active: boolean;
}

export interface TimelineGraph {
  width: number;
  height: number;
  nodes: TimelineGraphNode[];
  edges: TimelineGraphEdge[];
}

export const CURRENT_FILES_ID = "__current__";

export function buildTimelineEntries<T extends TimelineCheckpoint>(
  checkpoints: readonly T[],
  currentCheckpointId: string | null,
): TimelineEntry<T>[] {
  const byId = new Map(checkpoints.map((checkpoint) => [checkpoint.id, checkpoint]));
  const children = new Map<string, T[]>();
  for (const checkpoint of checkpoints) {
    if (checkpoint.parentId && byId.has(checkpoint.parentId)) {
      const siblings = children.get(checkpoint.parentId) ?? [];
      siblings.push(checkpoint);
      children.set(checkpoint.parentId, siblings);
    }
  }
  // The backend supplies newest-first save order for equal timestamps. Git
  // timestamps only have second precision, so random IDs cannot break ties.
  const timestamps = new Map(checkpoints.map((checkpoint) => [
    checkpoint.id, Date.parse(checkpoint.createdAt) || 0,
  ]));
  const newestFirst = [...checkpoints].sort((left, right) =>
    timestamps.get(right.id)! - timestamps.get(left.id)!);
  const oldestFirst = [...newestFirst].reverse();
  const oldestIndex = new Map(oldestFirst.map((checkpoint, index) => [checkpoint.id, index]));
  for (const siblings of children.values()) {
    siblings.sort((left, right) => oldestIndex.get(left.id)! - oldestIndex.get(right.id)!);
  }
  const activePath = new Set<string>();
  let active = byId.get(currentCheckpointId ?? "");
  while (active && !activePath.has(active.id)) {
    activePath.add(active.id);
    active = active.parentId ? byId.get(active.parentId) : undefined;
  }
  const laneById = new Map<string, number>();
  const siblingById = new Set<string>();
  let nextLane = 1;
  const visited = new Set<string>();
  const visit = (start: T, startLane: number) => {
    // Avoid a recursive walk: a long, ordinary history must not overflow the
    // JavaScript stack when the user loads more versions.
    const pending = [{ checkpoint: start, lane: startLane }];
    while (pending.length) {
      const { checkpoint, lane } = pending.pop()!;
      if (visited.has(checkpoint.id)) continue;
      visited.add(checkpoint.id);
      laneById.set(checkpoint.id, lane);
      const childList = children.get(checkpoint.id) ?? [];
      if (childList.length > 1) {
        for (const child of childList) siblingById.add(child.id);
      }
      // Restoring a version ends the active lane there. Its existing children
      // must move aside so the working-state edge cannot run through them.
      const mainChild = childList.find((child) => activePath.has(child.id))
        ?? (checkpoint.id === currentCheckpointId ? undefined : childList[0]);
      for (let index = childList.length - 1; index >= 0; index--) {
        const child = childList[index];
        if (child !== mainChild) pending.push({ checkpoint: child, lane: nextLane++ });
      }
      if (mainChild) pending.push({ checkpoint: mainChild, lane });
    }
  };
  // A missing parent is a separate component on this loaded page. Giving all
  // roots lane zero would draw other roots directly on an unrelated edge.
  const roots = oldestFirst.filter((checkpoint) => !checkpoint.parentId || !byId.has(checkpoint.parentId));
  roots.sort((left, right) => Number(activePath.has(right.id)) - Number(activePath.has(left.id)));
  for (const checkpoint of roots) visit(checkpoint, visited.size === 0 ? 0 : nextLane++);
  for (const checkpoint of oldestFirst) {
    if (!visited.has(checkpoint.id)) visit(checkpoint, visited.size === 0 ? 0 : nextLane++);
  }
  const packedLanes = packLanes(newestFirst, laneById);
  return newestFirst.map((checkpoint) => ({
    checkpoint,
    lane: packedLanes.get(laneById.get(checkpoint.id)!) ?? 0,
    parent: checkpoint.parentId ? byId.get(checkpoint.parentId) ?? null : null,
    hasSibling: siblingById.has(checkpoint.id),
    isActiveLineage: activePath.has(checkpoint.id),
  }));
}

// Reuse side lanes only when their complete time ranges do not overlap.
// Including the fork's parent keeps long edges clear of unrelated nodes,
// while independent Git entries can share a narrow column of dots.
function packLanes(
  checkpoints: readonly TimelineCheckpoint[],
  laneById: ReadonlyMap<string, number>,
): Map<number, number> {
  const rowById = new Map(checkpoints.map((checkpoint, row) => [checkpoint.id, row]));
  const spans = new Map<number, { lane: number; start: number; end: number }>();
  for (let row = 0; row < checkpoints.length; row++) {
    const checkpoint = checkpoints[row];
    const lane = laneById.get(checkpoint.id)!;
    if (lane === 0) continue;
    const parentRow = checkpoint.parentId ? rowById.get(checkpoint.parentId) ?? row : row;
    const span = spans.get(lane) ?? { lane, start: row, end: row };
    span.start = Math.min(span.start, row, parentRow);
    span.end = Math.max(span.end, row, parentRow);
    spans.set(lane, span);
  }
  const packed = new Map([[0, 0]]);
  const occupied: { lane: number; end: number }[] = [];
  const available: number[] = [];
  let nextLane = 1;
  // A small min-heap releases finished tracks in O(log tracks) per branch.
  function occupy(item: { lane: number; end: number }) {
    let index = occupied.length;
    occupied.push(item);
    while (index > 0) {
      const parent = (index - 1) >>> 1;
      if (occupied[parent].end <= item.end) break;
      occupied[index] = occupied[parent];
      index = parent;
    }
    occupied[index] = item;
  }
  function release() {
    available.push(occupied[0].lane);
    const last = occupied.pop()!;
    if (!occupied.length) return;
    let index = 0;
    while (index * 2 + 1 < occupied.length) {
      let child = index * 2 + 1;
      if (child + 1 < occupied.length && occupied[child + 1].end < occupied[child].end) child++;
      if (last.end <= occupied[child].end) break;
      occupied[index] = occupied[child];
      index = child;
    }
    occupied[index] = last;
  }
  for (const span of [...spans.values()].sort((left, right) => left.start - right.start || left.lane - right.lane)) {
    while (occupied.length && occupied[0].end < span.start) release();
    const lane = available.pop() ?? nextLane++;
    packed.set(span.lane, lane);
    occupy({ lane, end: span.end });
  }
  return packed;
}

export interface TimelineHostMeasurement {
  top: number;
  clientTop: number;
  scrollTop: number;
  clientHeight: number;
}

export interface TimelineRowMeasurement {
  id: string;
  top: number;
  height: number;
  anchorTop: number;
  anchorHeight: number;
}

export function measureTimelineRows(
  host: TimelineHostMeasurement,
  rows: readonly TimelineRowMeasurement[],
  bottomPadding: number,
): { centers: Map<string, number>; height: number } {
  const centers = new Map<string, number>();
  const origin = host.top + host.clientTop - host.scrollTop;
  let bottom = 0;
  for (const row of rows) {
    // Use the title anchor, not the center of the whole expandable row. Both
    // DOM rectangles share the SVG's coordinate system, even after scrolling.
    centers.set(row.id, row.anchorTop - origin + row.anchorHeight / 2);
    bottom = Math.max(bottom, row.top - origin + row.height);
  }
  // Never read scrollHeight here: the previous SVG can keep it artificially
  // tall after details collapse, causing stale scrollbars and feedback loops.
  return { centers, height: Math.max(host.clientHeight, bottom + bottomPadding) };
}

export function buildTimelineGraph(
  entries: readonly TimelineEntry[],
  rowCenters: ReadonlyMap<string, number>,
  currentCheckpointId: string | null,
  height: number,
): TimelineGraph {
  const entryById = new Map(entries.map((entry) => [entry.checkpoint.id, entry]));
  const nodes: TimelineGraphNode[] = [];
  const edges: TimelineGraphEdge[] = [];
  const nodeX = (lane: number) => 20 + lane * 20;
  let maxLane = 0;
  for (const entry of entries) maxLane = Math.max(maxLane, entry.lane);
  let maxX = nodeX(maxLane);
  const currentY = rowCenters.get(CURRENT_FILES_ID);
  if (currentY !== undefined) {
    nodes.push({ id: CURRENT_FILES_ID, x: nodeX(0), y: currentY, active: true, current: true });
  }
  for (const entry of entries) {
    const y = rowCenters.get(entry.checkpoint.id);
    if (y === undefined) continue;
    nodes.push({
      id: entry.checkpoint.id,
      x: nodeX(entry.lane),
      y,
      active: entry.isActiveLineage,
      current: entry.checkpoint.id === currentCheckpointId,
    });
  }
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const ysByX = new Map<number, number[]>();
  for (const node of nodes) {
    const ys = ysByX.get(node.x) ?? [];
    ys.push(node.y);
    ysByX.set(node.x, ys);
  }
  for (const ys of ysByX.values()) ys.sort((left, right) => left - right);
  function hasIntermediateNode(x: number, firstY: number, lastY: number): boolean {
    const ys = ysByX.get(x) ?? [];
    const minY = Math.min(firstY, lastY) + 0.001;
    const maxY = Math.max(firstY, lastY) - 0.001;
    let low = 0;
    let high = ys.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (ys[mid] < minY) low = mid + 1;
      else high = mid;
    }
    return low < ys.length && ys[low] <= maxY;
  }
  function connect(fromId: string, toId: string, active: boolean) {
    const from = nodeById.get(fromId);
    const to = nodeById.get(toId);
    if (!from || !to) return;
    const direction = Math.sign(to.y - from.y);
    const joinY = to.y - direction * Math.min(24, Math.abs(to.y - from.y) / 2);
    const start = { x: from.x, y: from.y };
    const end = { x: to.x, y: to.y };
    let points: GraphPoint[];
    if (hasIntermediateNode(from.x, from.y, from.x === to.x ? to.y : joinY)) {
      // A clock change can put an ancestor between a child and its parent in
      // time order. Detour rather than visually claiming it is another parent.
      maxX += 20;
      points = [start, { x: maxX, y: from.y }, { x: maxX, y: joinY }, end];
    } else if (from.x === to.x) {
      points = [start, end];
    } else {
      // Stay on the child's lane until the actual parent node. A turn halfway
      // down the edge can run through a sibling and imply a false lineage.
      points = [start, { x: from.x, y: joinY }, end];
    }
    edges.push({
      id: `${fromId}->${toId}`, fromId, toId, active, points,
      path: points.map((point, index) => `${index === 0 ? "M" : "L"} ${point.x} ${point.y}`).join(" "),
    });
  }
  for (const entry of entries) {
    if (entry.parent) {
      connect(entry.checkpoint.id, entry.parent.id,
        entry.isActiveLineage && Boolean(entryById.get(entry.parent.id)?.isActiveLineage));
    }
  }
  if (currentCheckpointId) connect(CURRENT_FILES_ID, currentCheckpointId, true);
  return {
    width: Math.max(76, maxX + 20),
    height, nodes, edges,
  };
}
