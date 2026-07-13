import test from "node:test";
import assert from "node:assert/strict";
import {
  AGENT_COMMAND_OUTPUT_TAIL_LIMIT,
  appendAgentOutputTail,
  getAgentItemDurationMs,
  getAgentLatestProgressAt,
  getAgentOutputPreview,
  getAgentProgressAgeMs,
  getCompletedFileChangePaths,
  isAgentTimelineNearBottom,
  parseAgentFileChanges,
  parseUnifiedDiffStats
} from "./agentProgress";
import type { AgentRun } from "../types/app";

test("parses file changes, kinds, move paths, and per-file diff stats", () => {
  const changes = parseAgentFileChanges({
    changes: [
      {
        path: "/tmp/demo/src/new.ts",
        kind: { type: "add" },
        diff: "@@ -0,0 +1,2 @@\n+one\n+two\n"
      },
      {
        path: "/tmp/demo/src/old.ts",
        kind: { type: "delete" },
        diff: "@@ -1,2 +0,0 @@\n-one\n-two\n"
      },
      {
        path: "/tmp/demo/src/name.ts",
        kind: { type: "update", move_path: "/tmp/demo/src/renamed.ts" },
        diff: "--- a/src/name.ts\n+++ b/src/renamed.ts\n@@ -1 +1 @@\n-old\n+new\n"
      }
    ]
  }, "file-item-1");

  assert.deepEqual(changes.map(({ kind, movePath, additions, deletions }) => ({
    kind,
    movePath,
    additions,
    deletions
  })), [
    { kind: "add", movePath: undefined, additions: 2, deletions: 0 },
    { kind: "delete", movePath: undefined, additions: 0, deletions: 2 },
    { kind: "update", movePath: "/tmp/demo/src/renamed.ts", additions: 1, deletions: 1 }
  ]);
});

test("parses aggregate unified diff without counting file headers", () => {
  const stats = parseUnifiedDiffStats([
    "diff --git a/src/App.tsx b/src/App.tsx",
    "--- a/src/App.tsx",
    "+++ b/src/App.tsx",
    "@@ -1,2 +1,3 @@",
    "-old",
    "+new",
    "+extra",
    "diff --git a/src/old.css b/src/old.css",
    "--- a/src/old.css",
    "+++ /dev/null",
    "@@ -1 +0,0 @@",
    "-body {}"
  ].join("\n"));

  assert.deepEqual(stats, {
    additions: 2,
    deletions: 2,
    files: 2
  });
});

test("keeps only a bounded command output tail and a six-line preview", () => {
  const first = appendAgentOutputTail(undefined, "a".repeat(AGENT_COMMAND_OUTPUT_TAIL_LIMIT));
  const second = appendAgentOutputTail(first.outputTail, "tail", first.truncated);

  assert.equal(second.outputTail.length, AGENT_COMMAND_OUTPUT_TAIL_LIMIT);
  assert.equal(second.outputTail.endsWith("tail"), true);
  assert.equal(second.truncated, true);
  assert.equal(getAgentOutputPreview("1\n2\n3\n4\n5\n6\n7\n8"), "3\n4\n5\n6\n7\n8");
});

test("uses protocol duration when present and timestamps for active commands", () => {
  assert.equal(
    getAgentItemDurationMs("2026-07-13T00:00:00Z", undefined, 1_250, Date.parse("2026-07-13T00:00:10Z")),
    1_250
  );
  assert.equal(
    getAgentItemDurationMs("2026-07-13T00:00:00Z", undefined, undefined, Date.parse("2026-07-13T00:00:10Z")),
    10_000
  );
});

test("extracts completed file paths only for file change items", () => {
  const item = {
    id: "file-1",
    changes: [
      { path: "src/App.tsx", kind: { type: "update" }, diff: "@@" },
      { path: "src/old.css", kind: { type: "update", move_path: "src/new.css" }, diff: "@@" }
    ]
  };

  assert.deepEqual(
    getCompletedFileChangePaths("fileChange", item),
    ["src/App.tsx", "src/new.css", "src/old.css"]
  );
  assert.deepEqual(getCompletedFileChangePaths("commandExecution", item), []);
});

test("finds the latest real progress across domain items and run snapshots", () => {
  const run: AgentRun = {
    id: "run-1",
    kind: "initial_build",
    prompt: "build",
    status: "running",
    events: [{
      type: "thread_started",
      threadId: "thread-1",
      createdAt: "2026-07-13T00:00:01Z"
    }],
    itemsById: {
      "message-1": {
        id: "message-1",
        type: "agentMessage",
        threadId: "thread-1",
        turnId: "turn-1",
        lifecycle: "in_progress",
        startedAt: "2026-07-13T00:00:02Z",
        updatedAt: "2026-07-13T00:00:05Z",
        data: {},
        text: "working"
      }
    },
    itemOrder: ["message-1"],
    messagesByItemId: {},
    filesByPath: {},
    aggregateDiff: "diff",
    aggregateDiffStats: { additions: 1, deletions: 0, files: 1 },
    aggregateDiffUpdatedAt: "2026-07-13T00:00:04Z",
    warnings: [],
    errors: [],
    changedFiles: [],
    startedAt: "2026-07-13T00:00:00Z"
  };

  assert.equal(getAgentLatestProgressAt(run), "2026-07-13T00:00:05Z");
  assert.equal(
    getAgentProgressAgeMs("2026-07-13T00:00:05Z", Date.parse("2026-07-13T00:00:20Z")),
    15_000
  );
});

test("timeline follow pauses away from the bottom and resumes near it", () => {
  assert.equal(isAgentTimelineNearBottom(1_000, 500, 400), false);
  assert.equal(isAgentTimelineNearBottom(1_000, 555, 400), true);
  assert.equal(isAgentTimelineNearBottom(1_000, 600, 400), true);
});
