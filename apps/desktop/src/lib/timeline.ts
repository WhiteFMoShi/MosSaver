export interface TimelineCheckpoint {
  id: string;
  createdAt: string;
  parentId: string | null;
  mergeParentIds?: readonly string[];
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
  selected: boolean;
  relatedToSelection: boolean;
}

export interface TimelineGraphEdge {
  id: string;
  fromId: string;
  toId: string;
  points: GraphPoint[];
  path: string;
  active: boolean;
  kind: "direct" | "merge" | "working";
  selected: boolean;
}

export interface TimelineGraph {
  width: number;
  height: number;
  nodes: TimelineGraphNode[];
  edges: TimelineGraphEdge[];
}

export const CURRENT_FILES_ID = "__current__";

function segmentIntersection(
  a: GraphPoint,
  b: GraphPoint,
  c: GraphPoint,
  d: GraphPoint,
): GraphPoint | null {
  const abX = b.x - a.x;
  const abY = b.y - a.y;
  const cdX = d.x - c.x;
  const cdY = d.y - c.y;
  const denominator = abX * cdY - abY * cdX;
  if (Math.abs(denominator) < 0.001) return null;
  const acX = c.x - a.x;
  const acY = c.y - a.y;
  const alongFirst = (acX * cdY - acY * cdX) / denominator;
  const alongSecond = (acX * abY - acY * abX) / denominator;
  // A contact at a bend is also visually ambiguous. Only actual shared nodes
  // are excluded by the caller, rather than every segment endpoint.
  if (alongFirst < -0.001 || alongFirst > 1.001 || alongSecond < -0.001 || alongSecond > 1.001) return null;
  return { x: a.x + alongFirst * abX, y: a.y + alongFirst * abY };
}

function samePoint(left: GraphPoint, right: GraphPoint): boolean {
  return Math.abs(left.x - right.x) < 0.001 && Math.abs(left.y - right.y) < 0.001;
}

function segmentOverlap(a: GraphPoint, b: GraphPoint, c: GraphPoint, d: GraphPoint): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const length = Math.hypot(dx, dy);
  if (!length) return 0;
  if (Math.abs(dx * (c.y - a.y) - dy * (c.x - a.x)) / length > 0.001
    || Math.abs(dx * (d.y - a.y) - dy * (d.x - a.x)) / length > 0.001) return 0;
  const first = ((c.x - a.x) * dx + (c.y - a.y) * dy) / length;
  const last = ((d.x - a.x) * dx + (d.y - a.y) * dy) / length;
  return Math.max(0, Math.min(length, Math.max(first, last)) - Math.max(0, Math.min(first, last)));
}

function pathConflictScore(points: readonly GraphPoint[], edges: readonly TimelineGraphEdge[]): number {
  let crossings = 0;
  let overlap = 0;
  const minY = Math.min(points[0].y, points.at(-1)!.y);
  const maxY = Math.max(points[0].y, points.at(-1)!.y);
  for (const edge of edges) {
    if (Math.max(edge.points[0].y, edge.points.at(-1)!.y) < minY
      || Math.min(edge.points[0].y, edge.points.at(-1)!.y) > maxY) continue;
    const intersections = new Set<string>();
    for (let first = 1; first < points.length; first++) {
      for (let second = 1; second < edge.points.length; second++) {
        const a = points[first - 1];
        const b = points[first];
        const c = edge.points[second - 1];
        const d = edge.points[second];
        overlap += segmentOverlap(a, b, c, d);
        const point = segmentIntersection(a, b, c, d);
        const sharedNode = point && (samePoint(point, points[0]) || samePoint(point, points.at(-1)!))
          && (samePoint(point, edge.points[0]) || samePoint(point, edge.points.at(-1)!));
        if (point && !sharedNode) intersections.add(`${Math.round(point.x * 10)},${Math.round(point.y * 10)}`);
      }
    }
    crossings += intersections.size;
  }
  // A shared stretch can look like a new lineage. Prefer a clear crossing to
  // a long overlap, then prefer short, local routes over outside detours.
  return crossings * 200 + overlap * 30;
}

function simplifyPoints(points: readonly GraphPoint[]): GraphPoint[] {
  const result: GraphPoint[] = [];
  for (const point of points) {
    if (result.length && samePoint(result.at(-1)!, point)) continue;
    while (result.length > 1) {
      const a = result.at(-2)!;
      const b = result.at(-1)!;
      const cross = (b.x - a.x) * (point.y - b.y) - (b.y - a.y) * (point.x - b.x);
      const dot = (b.x - a.x) * (point.x - b.x) + (b.y - a.y) * (point.y - b.y);
      if (Math.abs(cross) > 0.001 || dot < 0) break;
      result.pop();
    }
    result.push(point);
  }
  return result;
}

function roundedPath(points: readonly GraphPoint[]): string {
  let path = `M ${points[0].x} ${points[0].y}`;
  for (let index = 1; index < points.length - 1; index++) {
    const previous = points[index - 1];
    const corner = points[index];
    const next = points[index + 1];
    const incoming = Math.hypot(previous.x - corner.x, previous.y - corner.y);
    const outgoing = Math.hypot(next.x - corner.x, next.y - corner.y);
    const radius = Math.min(5, incoming / 3, outgoing / 3);
    const before = { x: corner.x + (previous.x - corner.x) * radius / incoming,
      y: corner.y + (previous.y - corner.y) * radius / incoming };
    const after = { x: corner.x + (next.x - corner.x) * radius / outgoing,
      y: corner.y + (next.y - corner.y) * radius / outgoing };
    path += ` L ${before.x} ${before.y} Q ${corner.x} ${corner.y} ${after.x} ${after.y}`;
  }
  const end = points.at(-1)!;
  return `${path} L ${end.x} ${end.y}`;
}

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
  const pendingAncestors = [currentCheckpointId ?? ""];
  while (pendingAncestors.length) {
    const active = byId.get(pendingAncestors.pop()!);
    if (!active || activePath.has(active.id)) continue;
    activePath.add(active.id);
    if (active.parentId) pendingAncestors.push(active.parentId);
    pendingAncestors.push(...(active.mergeParentIds ?? []));
  }
  // Highlight every source of a merge, but reserve lane zero for the active
  // version's first-parent chain. Picking any highlighted child could put a
  // merge source on lane zero and move the current version to a side lane.
  const primaryPath = new Set<string>();
  let primary = byId.get(currentCheckpointId ?? "");
  while (primary && !primaryPath.has(primary.id)) {
    primaryPath.add(primary.id);
    primary = primary.parentId ? byId.get(primary.parentId) : undefined;
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
      const mainChild = childList.find((child) => primaryPath.has(child.id))
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
  roots.sort((left, right) => Number(primaryPath.has(right.id)) - Number(primaryPath.has(left.id)));
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
  selectedCheckpointId: string | null = null,
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
    nodes.push({ id: CURRENT_FILES_ID, x: nodeX(0), y: currentY, active: true, current: true,
      selected: false, relatedToSelection: false });
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
      selected: false,
      relatedToSelection: false,
    });
  }
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const connectionSpecs: { fromId: string; toId: string; active: boolean; kind: TimelineGraphEdge["kind"] }[] = [];
  const nodesByX = new Map<number, TimelineGraphNode[]>();
  for (const node of nodes) {
    const column = nodesByX.get(node.x) ?? [];
    column.push(node);
    nodesByX.set(node.x, column);
  }
  const columns = [...nodesByX].sort(([left], [right]) => left - right);
  for (const [, column] of columns) column.sort((left, right) => left.y - right.y);
  function hasNodeNearSegment(from: GraphPoint, to: GraphPoint, fromId: string, toId: string): boolean {
    const dx = to.x - from.x;
    const dy = to.y - from.y;
    const lengthSquared = dx * dx + dy * dy;
    if (lengthSquared === 0) return false;
    const clearance = 9;
    const minX = Math.min(from.x, to.x) - clearance;
    const maxX = Math.max(from.x, to.x) + clearance;
    const minY = Math.min(from.y, to.y) - clearance;
    const maxY = Math.max(from.y, to.y) + clearance;
    // Search only the relevant rows in nearby columns. In particular, a long
    // linear history never scans all nodes for every ordinary vertical edge.
    let firstColumn = 0;
    let lastColumn = columns.length;
    while (firstColumn < lastColumn) {
      const mid = (firstColumn + lastColumn) >>> 1;
      if (columns[mid][0] < minX) firstColumn = mid + 1;
      else lastColumn = mid;
    }
    for (let columnIndex = firstColumn; columnIndex < columns.length; columnIndex++) {
      const [x, column] = columns[columnIndex];
      if (x > maxX) break;
      let low = 0;
      let high = column.length;
      while (low < high) {
        const mid = (low + high) >>> 1;
        if (column[mid].y < minY) low = mid + 1;
        else high = mid;
      }
      for (let index = low; index < column.length && column[index].y <= maxY; index++) {
        const node = column[index];
        if (node.id === fromId || node.id === toId) continue;
        const projection = Math.max(0, Math.min(1, ((node.x - from.x) * dx + (node.y - from.y) * dy) / lengthSquared));
        const nearestX = from.x + projection * dx;
        const nearestY = from.y + projection * dy;
        if ((node.x - nearestX) ** 2 + (node.y - nearestY) ** 2 < clearance ** 2) return true;
      }
    }
    return false;
  }
  function blockedPath(points: readonly GraphPoint[], fromId: string, toId: string): boolean {
    return points.slice(1).some((point, index) => hasNodeNearSegment(points[index], point, fromId, toId));
  }
  function routeViaTrack(start: GraphPoint, end: GraphPoint, fromId: string, toId: string): GraphPoint[] {
    const direction = Math.sign(end.y - start.y);
    const turn = Math.min(20, Math.abs(end.y - start.y) / 3);
    // First try the endpoint columns and the spaces beside them. Outside
    // tracks are fallbacks, and can be reused when their time spans are free.
    const tracks = [...new Set([
      end.x, start.x,
      end.x - 10, end.x + 10, start.x - 10, start.x + 10,
      10, nodeX(maxLane) + 10, maxX + 10,
    ])].filter((x) => x >= 10);
    let best: GraphPoint[] | undefined;
    let bestScore = Infinity;
    for (const x of tracks) {
      const candidate = simplifyPoints([
        start, { x, y: start.y + direction * turn },
        { x, y: end.y - direction * turn }, end,
      ]);
      if (blockedPath(candidate, fromId, toId)) continue;
      const length = candidate.reduce((sum, point, index) => index
        ? sum + Math.hypot(point.x - candidate[index - 1].x, point.y - candidate[index - 1].y) : sum, 0);
      const score = pathConflictScore(candidate, edges) + length * 0.02 + candidate.length;
      if (score < bestScore) {
        best = candidate;
        bestScore = score;
      }
    }
    // The outermost track clears all node columns. Keep the exit/entry within
    // the gaps adjacent to their rows so the fallback cannot cross a node.
    return best ?? simplifyPoints([
      start, { x: maxX + 10, y: start.y + direction * turn },
      { x: maxX + 10, y: end.y - direction * turn }, end,
    ]);
  }
  function connect(
    fromId: string,
    toId: string,
    active: boolean,
    kind: TimelineGraphEdge["kind"],
  ) {
    connectionSpecs.push({ fromId, toId, active, kind });
  }
  function routeConnection({ fromId, toId, active, kind }: typeof connectionSpecs[number]) {
    const from = nodeById.get(fromId);
    const to = nodeById.get(toId);
    if (!from || !to) return;
    const direction = Math.sign(to.y - from.y);
    const joinY = to.y - direction * Math.min(24, Math.abs(to.y - from.y) / 2);
    const start = { x: from.x, y: from.y };
    const end = { x: to.x, y: to.y };
    let points = simplifyPoints(from.x === to.x ? [start, end]
      : [start, { x: from.x, y: joinY }, end]);
    if (kind === "merge") {
      points = routeViaTrack(start, end, fromId, toId);
    } else if (blockedPath(points, fromId, toId)) {
      // A clock change can put an ancestor between a child and its parent in
      // time order. Detour rather than visually claiming it is another parent.
      points = routeViaTrack(start, end, fromId, toId);
    }
    maxX = Math.max(maxX, ...points.map((point) => point.x));
    edges.push({
      id: `${fromId}->${toId}`, fromId, toId, active, kind, selected: false, points,
      path: roundedPath(points),
    });
  }
  for (const entry of entries) {
    if (entry.parent) {
      connect(entry.checkpoint.id, entry.parent.id,
        entry.isActiveLineage && Boolean(entryById.get(entry.parent.id)?.isActiveLineage), "direct");
    }
    for (const parentId of entry.checkpoint.mergeParentIds ?? []) {
      if (parentId !== entry.checkpoint.parentId) {
        connect(entry.checkpoint.id, parentId,
          entry.isActiveLineage && Boolean(entryById.get(parentId)?.isActiveLineage), "merge");
      }
    }
  }
  if (currentCheckpointId) connect(CURRENT_FILES_ID, currentCheckpointId, true, "working");
  const originalOrder = new Map(connectionSpecs.map((connection, index) => [`${connection.fromId}->${connection.toId}`, index]));
  const routingOrder = [...connectionSpecs].sort((left, right) => {
    const rank = (kind: TimelineGraphEdge["kind"]) => kind === "direct" ? 0 : kind === "working" ? 1 : 2;
    const span = (connection: typeof connectionSpecs[number]) => Math.abs(
      (nodeById.get(connection.fromId)?.y ?? 0) - (nodeById.get(connection.toId)?.y ?? 0));
    return rank(left.kind) - rank(right.kind)
      || (left.kind === "merge" ? span(left) - span(right) : 0);
  });
  for (const connection of routingOrder) routeConnection(connection);
  edges.sort((left, right) => originalOrder.get(left.id)! - originalOrder.get(right.id)!);
  const mergeChildren = new Set(edges.filter((edge) => edge.kind === "merge").map((edge) => edge.fromId));
  const relatedMergeChildren = new Set(edges.filter((edge) => edge.kind !== "working"
    && mergeChildren.has(edge.fromId)
    && (edge.fromId === selectedCheckpointId || edge.toId === selectedCheckpointId))
    .map((edge) => edge.fromId));
  for (const edge of edges) {
    const touchesSelection = selectedCheckpointId && (
      (edge.kind === "working" && edge.toId === selectedCheckpointId)
      || (edge.kind !== "working" && (edge.fromId === selectedCheckpointId || edge.toId === selectedCheckpointId))
    );
    edge.selected = Boolean(touchesSelection || (edge.kind !== "working" && relatedMergeChildren.has(edge.fromId)));
  }
  const relatedIds = new Set<string>();
  for (const edge of edges) {
    if (edge.selected) {
      relatedIds.add(edge.fromId);
      relatedIds.add(edge.toId);
    }
  }
  if (selectedCheckpointId) relatedIds.add(selectedCheckpointId);
  for (const node of nodes) {
    node.selected = node.id === selectedCheckpointId;
    node.relatedToSelection = relatedIds.has(node.id);
  }
  return {
    width: Math.max(76, maxX + 20),
    height, nodes, edges,
  };
}
