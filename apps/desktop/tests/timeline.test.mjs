import assert from "node:assert/strict";
import test from "node:test";
import { buildTimelineEntries, buildTimelineGraph, CURRENT_FILES_ID, measureTimelineRows } from "../src/lib/timeline.ts";

const checkpoint = (id, parentId, time) => ({ id, parentId, createdAt: `2026-11-03T${time}:00+08:00` });
const fork = [
  checkpoint("03:57", "01:19", "03:57"),
  checkpoint("02:45", "01:19", "02:45"),
  checkpoint("01:19", "initial", "01:19"),
  checkpoint("initial", null, "00:00"),
];
function graphFor(checkpoints = fork, currentId = "03:57", extraRows = []) {
  const entries = buildTimelineEntries(checkpoints, currentId);
  const rows = [...extraRows, ...entries.map((entry) => entry.checkpoint.id)];
  return {
    entries,
    graph: buildTimelineGraph(entries, new Map(rows.map((id, index) => [id, 30 + index * 100])), currentId, rows.length * 100),
  };
}
function pointOnSegment(point, from, to) {
  const cross = (point.x - from.x) * (to.y - from.y) - (point.y - from.y) * (to.x - from.x);
  return Math.abs(cross) < 0.001
    && point.x >= Math.min(from.x, to.x) && point.x <= Math.max(from.x, to.x)
    && point.y >= Math.min(from.y, to.y) && point.y <= Math.max(from.y, to.y);
}
function assertNoFalseConnections(graph) {
  for (const edge of graph.edges) {
    for (const node of graph.nodes) {
      if (node.id === edge.fromId || node.id === edge.toId) continue;
      for (let index = 1; index < edge.points.length; index++) {
        assert.equal(pointOnSegment(node, edge.points[index - 1], edge.points[index]), false,
          `${edge.id} must not pass through unrelated node ${node.id}`);
      }
    }
  }
}

test("a merged version has both real parents and activates both source ancestries", () => {
  const checkpoints = [
    { ...checkpoint("merge", "right", "06:00"), mergeParentIds: ["left"] },
    checkpoint("right", "root", "05:00"),
    checkpoint("left", "root", "04:00"),
    checkpoint("root", null, "01:00"),
  ];
  const { entries, graph } = graphFor(checkpoints, "merge", [CURRENT_FILES_ID]);
  assert.ok(entries.every((entry) => entry.isActiveLineage));
  for (const id of ["merge", "right", "root"]) assert.equal(entries.find((entry) => entry.checkpoint.id === id).lane, 0);
  assert.ok(entries.find((entry) => entry.checkpoint.id === "left").lane > 0);
  assert.deepEqual(graph.edges.map(({ fromId, toId }) => [fromId, toId]), [
    ["merge", "right"], ["merge", "left"], ["right", "root"], ["left", "root"], [CURRENT_FILES_ID, "merge"],
  ]);
  assert.ok(graph.edges.every((edge) => edge.active));
  const mergeEdge = graph.edges.find((edge) => edge.fromId === "merge" && edge.toId === "left");
  assert.equal(mergeEdge.kind, "merge");
  assert.equal(mergeEdge.points.length, 2, "an unobstructed merge link should be a single straight line");
  assert.deepEqual(mergeEdge.points[0], { x: graph.nodes.find((node) => node.id === "merge").x,
    y: graph.nodes.find((node) => node.id === "merge").y });
  assert.deepEqual(mergeEdge.points.at(-1), { x: graph.nodes.find((node) => node.id === "left").x,
    y: graph.nodes.find((node) => node.id === "left").y });
  assertNoFalseConnections(graph);
});

test("selecting a merge highlights its exact parent paths without highlighting their ancestors", () => {
  const checkpoints = [
    { ...checkpoint("merge", "right", "06:00"), mergeParentIds: ["left"] },
    checkpoint("right", "root", "05:00"),
    checkpoint("left", "root", "04:00"),
    checkpoint("root", null, "01:00"),
    checkpoint("unrelated", null, "00:30"),
  ];
  const entries = buildTimelineEntries(checkpoints, "merge");
  const rows = new Map(entries.map((entry, index) => [entry.checkpoint.id, 30 + index * 100]));
  const graph = buildTimelineGraph(entries, rows, "merge", 600, "merge");
  assert.deepEqual(graph.edges.filter((edge) => edge.selected).map(({ fromId, toId, kind }) => [fromId, toId, kind]), [
    ["merge", "right", "direct"], ["merge", "left", "merge"],
  ]);
  assert.deepEqual(graph.nodes.filter((node) => node.selected).map((node) => node.id), ["merge"]);
  assert.deepEqual(graph.nodes.filter((node) => node.relatedToSelection).map((node) => node.id).sort(), ["left", "merge", "right"]);
  assert.equal(graph.nodes.find((node) => node.id === "root").relatedToSelection, false);
  assert.equal(graph.nodes.find((node) => node.id === "unrelated").relatedToSelection, false);
});

test("selecting either source parent highlights both sides of its merge relationship", () => {
  const checkpoints = [
    { ...checkpoint("merge", "right", "06:00"), mergeParentIds: ["left"] },
    checkpoint("right", "root", "05:00"),
    checkpoint("left", "root", "04:00"),
    checkpoint("root", null, "01:00"),
  ];
  const entries = buildTimelineEntries(checkpoints, "merge");
  const centers = new Map(entries.map((entry, index) => [entry.checkpoint.id, 30 + index * 100]));
  const graph = buildTimelineGraph(entries, centers, "merge", 600, "left");
  assert.deepEqual(graph.edges.filter((edge) => edge.selected && edge.fromId === "merge").map(({ fromId, toId }) => [fromId, toId]), [
    ["merge", "right"], ["merge", "left"],
  ]);
  assert.equal(graph.nodes.find((node) => node.id === "merge").relatedToSelection, true);
  assert.equal(graph.nodes.find((node) => node.id === "right").relatedToSelection, true);
});

test("selecting the current version also highlights its working-state connection", () => {
  const entries = buildTimelineEntries(fork, "03:57");
  const centers = new Map([[CURRENT_FILES_ID, 20], ...entries.map((entry, index) => [entry.checkpoint.id, 100 + index * 100])]);
  const graph = buildTimelineGraph(entries, centers, "03:57", 600, "03:57");
  assert.deepEqual(graph.edges.filter((edge) => edge.selected).map(({ fromId, toId, kind }) => [fromId, toId, kind]), [
    ["03:57", "01:19", "direct"], [CURRENT_FILES_ID, "03:57", "working"],
  ]);
  assert.equal(graph.nodes.find((node) => node.id === CURRENT_FILES_ID).relatedToSelection, true);
});

test("merge parents outside the loaded page do not produce invented nodes or edges", () => {
  const { graph } = graphFor([
    { ...checkpoint("merge", "right", "06:00"), mergeParentIds: ["unloaded"] },
    checkpoint("right", null, "05:00"),
  ], "merge");
  assert.deepEqual(graph.edges.map(({ fromId, toId }) => [fromId, toId]), [["merge", "right"]]);
  assert.equal(graph.nodes.length, 2);
});

test("repeated merges keep their exact parents after switching to either side", () => {
  const checkpoints = [
    { ...checkpoint("merge2", "next", "09:00"), mergeParentIds: ["third"] },
    checkpoint("next", "merge1", "08:00"),
    checkpoint("third", "root", "07:00"),
    { ...checkpoint("merge1", "right", "06:00"), mergeParentIds: ["left"] },
    checkpoint("right", "root", "05:00"),
    checkpoint("left", "root", "04:00"),
    checkpoint("root", null, "01:00"),
  ];
  for (const current of ["merge2", "merge1", "right", "left", "third", "root"]) {
    const { entries, graph } = graphFor(checkpoints, current, [CURRENT_FILES_ID]);
    assert.equal(entries.find((entry) => entry.checkpoint.id === current).lane, 0);
    assertNoFalseConnections(graph);
    for (const version of checkpoints) {
      assert.deepEqual(graph.edges.filter((edge) => edge.fromId === version.id).map((edge) => edge.toId),
        [version.parentId, ...(version.mergeParentIds ?? [])].filter(Boolean));
    }
    const active = new Set();
    const pending = [current];
    while (pending.length) {
      const id = pending.pop();
      if (active.has(id)) continue;
      active.add(id);
      const version = checkpoints.find((version) => version.id === id);
      pending.push(...[version.parentId, ...(version.mergeParentIds ?? [])].filter(Boolean));
    }
    assert.deepEqual(new Set(entries.filter((entry) => entry.isActiveLineage).map((entry) => entry.checkpoint.id)), active);
  }
});

test("100 reproducible merge graphs preserve all parents with clock changes and expanded rows", () => {
  let seed = 20261104;
  const random = (limit) => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed % limit;
  };
  for (let sample = 0; sample < 100; sample++) {
    const checkpoints = Array.from({ length: 40 }, (_, index) => {
      const parentIndex = index ? random(index) : -1;
      const mergeIndex = index > 1 && random(3) === 0 ? random(index) : -1;
      return {
        id: `v${index}`, parentId: parentIndex >= 0 ? `v${parentIndex}` : null,
        mergeParentIds: mergeIndex >= 0 && mergeIndex !== parentIndex ? [`v${mergeIndex}`] : [],
        createdAt: new Date(1_700_000_000_000 + random(50) * 1000).toISOString(),
      };
    });
    const currentId = `v${random(checkpoints.length)}`;
    const entries = buildTimelineEntries(checkpoints, currentId);
    let y = 30;
    const centers = new Map([[CURRENT_FILES_ID, y]]);
    for (const entry of entries) {
      y += 98 + random(200);
      centers.set(entry.checkpoint.id, y);
    }
    const graph = buildTimelineGraph(entries, centers, currentId, y + 98);
    assertNoFalseConnections(graph);
    assert.equal(graph.edges.length, checkpoints.reduce((count, version) => count + Number(Boolean(version.parentId)) + version.mergeParentIds.length, 1));
    for (const version of checkpoints) {
      assert.deepEqual(graph.edges.filter((edge) => edge.fromId === version.id).map((edge) => edge.toId),
        [version.parentId, ...version.mergeParentIds].filter(Boolean));
    }
  }
});

test("01:19 has two children; 03:57 is never connected to 02:45", () => {
  const { graph, entries } = graphFor();
  assert.deepEqual(graph.edges.map(({ fromId, toId }) => [fromId, toId]), [
    ["03:57", "01:19"], ["02:45", "01:19"], ["01:19", "initial"],
  ]);
  assert.equal(entries.filter((entry) => entry.hasSibling).length, 2);
  assertNoFalseConnections(graph);
});

test("timestamps determine row order independently of parent traversal", () => {
  const { entries } = graphFor([fork[3], fork[1], fork[2], fork[0]]);
  assert.deepEqual(entries.map((entry) => entry.checkpoint.id), ["03:57", "02:45", "01:19", "initial"]);
});

test("equal timestamps preserve the backend's save order instead of sorting random IDs", () => {
  const entries = buildTimelineEntries([
    checkpoint("a-newer-save", "z-older-save", "01:19"),
    checkpoint("z-older-save", null, "01:19"),
  ], "a-newer-save");
  assert.deepEqual(entries.map((entry) => entry.checkpoint.id), ["a-newer-save", "z-older-save"]);
});

test("timezone offsets are compared as instants, not strings", () => {
  const entries = buildTimelineEntries([
    { id: "older", parentId: null, createdAt: "2026-11-03T12:00:00+08:00" },
    { id: "newer", parentId: "older", createdAt: "2026-11-03T05:00:00Z" },
  ], "newer");
  assert.deepEqual(entries.map((entry) => entry.checkpoint.id), ["newer", "older"]);
});

test("switching between siblings moves only the current ancestry onto the main lane", () => {
  for (const currentId of ["03:57", "02:45"]) {
    const { entries, graph } = graphFor(fork, currentId);
    for (const entry of entries) {
      const active = [currentId, "01:19", "initial"].includes(entry.checkpoint.id);
      assert.equal(entry.isActiveLineage, active);
      assert.equal(entry.lane === 0, active);
    }
    assert.deepEqual(graph.edges.filter((edge) => edge.active).map((edge) => edge.fromId).sort(), [currentId, "01:19"].sort());
    assertNoFalseConnections(graph);
  }
});

test("restoring an ancestor reserves its main lane for the unsaved working state", () => {
  const { entries, graph } = graphFor(fork, "01:19", [CURRENT_FILES_ID]);
  assert.equal(entries.find((entry) => entry.checkpoint.id === "01:19").lane, 0);
  assert.ok(entries.filter((entry) => ["03:57", "02:45"].includes(entry.checkpoint.id)).every((entry) => entry.lane > 0));
  assert.equal(graph.edges.find((edge) => edge.fromId === CURRENT_FILES_ID).toId, "01:19");
  assertNoFalseConnections(graph);
});

test("a long side branch joins at its real parent without running through an intervening sibling", () => {
  const { graph } = graphFor([
    checkpoint("b-new", "b", "05:00"),
    checkpoint("a", "parent", "04:00"),
    checkpoint("b", "parent", "03:00"),
    checkpoint("parent", null, "01:00"),
  ], "b-new");
  assertNoFalseConnections(graph);
  const edge = graph.edges.find((edge) => edge.fromId === "a");
  // A fork meets the parent's lane at the parent, never halfway through history.
  assert.ok(edge.points.slice(1, -1).every((point) => point.x !== edge.points.at(-1).x));
});

test("unrelated roots never create a false connection through the active ancestry", () => {
  const { entries, graph } = graphFor([
    checkpoint("new", "root", "04:00"),
    checkpoint("unrelated", null, "03:00"),
    checkpoint("root", null, "01:00"),
  ], "new");
  assert.notEqual(entries.find((entry) => entry.checkpoint.id === "unrelated").lane, 0);
  assertNoFalseConnections(graph);
});

test("a parent outside the loaded page never gets an invented connection", () => {
  const { graph } = graphFor([checkpoint("child", "unloaded", "04:00")], "child");
  assert.equal(graph.nodes.length, 1);
  assert.deepEqual(graph.edges, []);
});

test("every rendered edge starts and ends at its actual nodes", () => {
  const { graph } = graphFor(fork, "03:57", [CURRENT_FILES_ID]);
  const nodes = new Map(graph.nodes.map((node) => [node.id, node]));
  for (const edge of graph.edges) {
    const from = nodes.get(edge.fromId);
    const to = nodes.get(edge.toId);
    assert.deepEqual(edge.points[0], { x: from.x, y: from.y });
    assert.deepEqual(edge.points.at(-1), { x: to.x, y: to.y });
    assert.ok(edge.path.startsWith(`M ${from.x} ${from.y} `));
    assert.ok(edge.path.endsWith(`L ${to.x} ${to.y}`));
  }
});

test("10,000 sequential versions can be laid out without recursive stack overflow", () => {
  const checkpoints = Array.from({ length: 10_000 }, (_, index) => ({
    id: `v${index}`, parentId: index ? `v${index - 1}` : null,
    createdAt: new Date(1_700_000_000_000 + index * 1000).toISOString(),
  })).reverse();
  const entries = buildTimelineEntries(checkpoints, "v9999");
  assert.equal(entries.length, 10_000);
  assert.ok(entries.every((entry) => entry.lane === 0 && entry.isActiveLineage));
  const graph = buildTimelineGraph(entries, new Map(entries.map((entry, index) => [entry.checkpoint.id, 30 + index * 100])), "v9999", 1_000_000);
  assert.equal(graph.nodes.length, 10_000);
  assert.equal(graph.edges.length, 9_999);
  assert.equal(graph.width, 76);
});

test("scrolling and borders do not change coordinates within the SVG", () => {
  const host = { top: 100, clientTop: 2, scrollTop: 0, clientHeight: 300 };
  const rows = [{ id: "a", top: 116, height: 98, anchorTop: 116, anchorHeight: 24 }];
  const before = measureTimelineRows(host, rows, 4);
  const after = measureTimelineRows({ ...host, scrollTop: 80 }, rows.map((row) => ({
    ...row, top: row.top - 80, anchorTop: row.anchorTop - 80,
  })), 4);
  assert.equal(before.centers.get("a"), 26);
  assert.deepEqual(after, before);
});

test("expanding details keeps the title node fixed and shifts subsequent nodes", () => {
  const host = { top: 100, clientTop: 0, scrollTop: 0, clientHeight: 200 };
  const rows = [
    { id: "child", top: 114, height: 100, anchorTop: 114, anchorHeight: 24 },
    { id: "parent", top: 214, height: 100, anchorTop: 214, anchorHeight: 24 },
  ];
  const before = measureTimelineRows(host, rows, 4);
  const expanded = measureTimelineRows(host, [
    { ...rows[0], height: 250 },
    { ...rows[1], top: 364, anchorTop: 364 },
  ], 4);
  assert.equal(expanded.centers.get("child"), before.centers.get("child"));
  assert.equal(expanded.centers.get("parent"), before.centers.get("parent") + 150);
  assert.equal(expanded.height, before.height + 150);
  // Collapsing must shrink the graph, independent of its previous SVG height.
  assert.equal(measureTimelineRows(host, rows, 4).height, before.height);
});

test("loading the missing parent adds only the real edge", () => {
  const child = checkpoint("child", "parent", "04:00");
  assert.equal(graphFor([child], "child").graph.edges.length, 0);
  const { graph } = graphFor([child, checkpoint("parent", null, "01:00")], "child");
  assert.deepEqual(graph.edges.map((edge) => [edge.fromId, edge.toId]), [["child", "parent"]]);
});

test("clock skew does not reorder by ancestry or draw an edge through an unrelated version", () => {
  const { entries, graph } = graphFor([
    checkpoint("parent", "root", "04:00"),
    checkpoint("root", null, "03:00"),
    checkpoint("child", "parent", "02:00"),
  ], "child");
  assert.deepEqual(entries.map((entry) => entry.checkpoint.id), ["parent", "root", "child"]);
  assertNoFalseConnections(graph);
});

test("read-only history with 50 independent entries does not consume 50 lanes", () => {
  const checkpoints = Array.from({ length: 50 }, (_, index) => ({
    id: `git-${index}`, parentId: null, createdAt: new Date(1_700_000_000_000 + index * 1000).toISOString(),
  })).reverse();
  const { graph } = graphFor(checkpoints, null);
  assert.ok(graph.width <= 96, `independent history should remain compact, got ${graph.width}px`);
  assert.equal(graph.edges.length, 0);
});

test("nested forks keep their exact parents across multiple current-version choices", () => {
  const checkpoints = [
    checkpoint("a3", "a2", "09:00"),
    checkpoint("b2", "b1", "08:00"),
    checkpoint("c1", "a1", "07:00"),
    checkpoint("a2", "a1", "06:00"),
    checkpoint("b1", "root", "05:00"),
    checkpoint("a1", "root", "04:00"),
    checkpoint("root", null, "01:00"),
  ];
  for (const currentId of ["a3", "b2", "c1", "a1", "root"]) {
    const { graph } = graphFor(checkpoints, currentId, [CURRENT_FILES_ID]);
    assertNoFalseConnections(graph);
    for (const checkpoint of checkpoints) {
      const edge = graph.edges.find((edge) => edge.fromId === checkpoint.id);
      assert.equal(edge?.toId ?? null, checkpoint.parentId);
    }
  }
});

test("100 reproducible trees preserve every parent and keep edges clear of unrelated nodes", () => {
  let seed = 20261103;
  const random = (limit) => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed % limit;
  };
  for (let sample = 0; sample < 100; sample++) {
    const checkpoints = Array.from({ length: 40 }, (_, index) => ({
      id: `version-${index}`,
      parentId: index && random(5) ? `version-${random(index)}` : null,
      // Include equal timestamps and clock changes as well as ordinary forks.
      createdAt: new Date(1_700_000_000_000 + random(50) * 1000).toISOString(),
    }));
    const currentId = `version-${random(checkpoints.length)}`;
    const entries = buildTimelineEntries(checkpoints, currentId);
    let y = 30;
    const centers = new Map([[CURRENT_FILES_ID, y]]);
    for (const entry of entries) {
      y += 98 + random(200);
      centers.set(entry.checkpoint.id, y);
    }
    const graph = buildTimelineGraph(entries, centers, currentId, y + 98);
    assertNoFalseConnections(graph);
    assert.equal(graph.edges.length, checkpoints.filter((checkpoint) => checkpoint.parentId).length + 1);
    for (const checkpoint of checkpoints) {
      assert.equal(graph.edges.find((edge) => edge.fromId === checkpoint.id)?.toId ?? null, checkpoint.parentId);
    }
  }
});
