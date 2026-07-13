import test from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { DemoProgressPanel } from "./DemoProgressPanel";
import { createDemoSession, demoSessionReducer } from "../utils/demoSession";

function assertMatches(actual: string, pattern: RegExp) {
  assert.ok(pattern.test(actual), `Expected output to match ${pattern}`);
}

function assertDoesNotMatch(actual: string, pattern: RegExp) {
  assert.ok(!pattern.test(actual), `Expected output not to match ${pattern}`);
}

test("renders live file stats and completed command metadata from Agent items", () => {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-1",
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:02Z",
    events: [
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "file-1",
        itemType: "fileChange",
        lifecycle: "completed",
        status: "completed",
        completedAt: "2026-07-13T00:00:01Z",
        item: {
          id: "file-1",
          type: "fileChange",
          status: "completed",
          changes: [{
            path: "/tmp/demo/src/App.tsx",
            kind: { type: "update", move_path: "/tmp/demo/src/Main.tsx" },
            diff: "@@ -1 +1,2 @@\n-old\n+new\n+extra\n"
          }]
        },
        createdAt: "2026-07-13T00:00:01Z"
      },
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "completed",
        status: "completed",
        completedAt: "2026-07-13T00:00:02Z",
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run check",
          cwd: "/tmp/demo",
          status: "completed",
          exitCode: 0,
          durationMs: 1_200,
          aggregatedOutput: "tests passed"
        },
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "turn_diff_updated",
        threadId: "thread-1",
        turnId: "turn-1",
        diff: "diff --git a/src/App.tsx b/src/Main.tsx\n--- a/src/App.tsx\n+++ b/src/Main.tsx\n@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
        createdAt: "2026-07-13T00:00:02Z"
      },
      {
        type: "item_completed",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-2",
        itemType: "commandExecution",
        lifecycle: "completed",
        status: "declined",
        completedAt: "2026-07-13T00:00:02Z",
        item: {
          id: "command-2",
          type: "commandExecution",
          command: "sudo dangerous-command",
          cwd: "/tmp/demo",
          status: "declined",
          exitCode: null,
          durationMs: 100,
          aggregatedOutput: null
        },
        createdAt: "2026-07-13T00:00:02Z"
      }
    ]
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assert.ok(/文件修改完成/.test(html));
  assert.ok(/src\/App\.tsx → src\/Main\.tsx/.test(html));
  assert.ok(/\+2/.test(html));
  assert.ok(/-1/.test(html));
  assert.ok(/命令执行完成/.test(html));
  assert.ok(/npm run check/.test(html));
  assert.ok(/cwd · \/tmp\/demo/.test(html));
  assert.ok(/tests passed/.test(html));
  assert.ok(/退出码 0/.test(html));
  assert.ok(/1\.2s/.test(html));
  assert.ok(/命令执行被拒绝/.test(html));
  assert.ok(/命令未获批准/.test(html));
});

test("renders file and command cards as soon as items start", () => {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-1",
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const started = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-1",
    now: "2026-07-13T00:00:01Z",
    events: [
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "file-1",
        itemType: "fileChange",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: "2026-07-13T00:00:00Z",
        item: { id: "file-1", type: "fileChange", status: "inProgress", changes: [] },
        createdAt: "2026-07-13T00:00:00Z"
      },
      {
        type: "item_started",
        threadId: "thread-1",
        turnId: "turn-1",
        itemId: "command-1",
        itemType: "commandExecution",
        lifecycle: "in_progress",
        status: "inProgress",
        startedAt: new Date().toISOString(),
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "npm run build",
          cwd: "/tmp/demo",
          status: "inProgress"
        },
        createdAt: "2026-07-13T00:00:01Z"
      }
    ]
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={started} compact />);

  assert.ok(/正在修改文件/.test(html));
  assert.ok(/Codex 正在准备文件修改/.test(html));
  assert.ok(/正在执行命令/.test(html));
  assert.ok(/npm run build/.test(html));
  assert.ok(/命令正在执行，等待输出/.test(html));
});

test("renders a continuous structured timeline with aggregated files and actionable details", () => {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-timeline",
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const events = [
    ...Array.from({ length: 7 }, (_, index) => ({
      type: "diagnostic" as const,
      level: "info",
      message: `时间线事件 ${index + 1}`,
      createdAt: `2026-07-13T00:00:0${index + 1}Z`
    })),
    {
      type: "plan_updated" as const,
      threadId: "thread-1",
      turnId: "turn-1",
      explanation: "按步骤构建并验证",
      plan: [
        { step: "读取项目", status: "completed" as const },
        { step: "修改界面", status: "inProgress" as const },
        { step: "运行测试", status: "pending" as const }
      ],
      createdAt: "2026-07-13T00:00:08Z"
    },
    {
      type: "item_completed" as const,
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "message-1",
      itemType: "agentMessage",
      lifecycle: "completed" as const,
      completedAt: "2026-07-13T00:00:09Z",
      item: {
        id: "message-1",
        type: "agentMessage",
        phase: "commentary",
        text: "正在梳理组件并持续更新页面。"
      },
      createdAt: "2026-07-13T00:00:09Z"
    },
    {
      type: "item_completed" as const,
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "file-1",
      itemType: "fileChange",
      lifecycle: "completed" as const,
      status: "completed",
      completedAt: "2026-07-13T00:00:10Z",
      item: {
        id: "file-1",
        type: "fileChange",
        status: "completed",
        changes: [{
          path: "/tmp/demo/src/App.tsx",
          kind: { type: "update" },
          diff: "@@ -1 +1 @@\n-old\n+interim\n"
        }]
      },
      createdAt: "2026-07-13T00:00:10Z"
    },
    {
      type: "item_completed" as const,
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "file-2",
      itemType: "fileChange",
      lifecycle: "completed" as const,
      status: "completed",
      completedAt: "2026-07-13T00:00:11Z",
      item: {
        id: "file-2",
        type: "fileChange",
        status: "completed",
        changes: [{
          path: "/tmp/demo/src/App.tsx",
          kind: { type: "update" },
          diff: "@@ -1 +1,2 @@\n-old\n+new\n+extra\n"
        }]
      },
      createdAt: "2026-07-13T00:00:11Z"
    },
    {
      type: "turn_diff_updated" as const,
      threadId: "thread-1",
      turnId: "turn-1",
      diff: "diff --git a/src/App.tsx b/src/App.tsx\n--- a/src/App.tsx\n+++ b/src/App.tsx\n@@ -1 +1,2 @@\n-old\n+new\n+extra\n",
      createdAt: "2026-07-13T00:00:12Z"
    },
    {
      type: "item_completed" as const,
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "command-1",
      itemType: "commandExecution",
      lifecycle: "completed" as const,
      status: "failed",
      completedAt: "2026-07-13T00:00:13Z",
      item: {
        id: "command-1",
        type: "commandExecution",
        command: "npm run check",
        cwd: "/tmp/demo",
        status: "failed",
        exitCode: 1,
        durationMs: 1_300,
        aggregatedOutput: "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8"
      },
      createdAt: "2026-07-13T00:00:13Z"
    },
    {
      type: "warning" as const,
      message: "网络较慢，但运行仍在继续",
      threadId: "thread-1",
      turnId: "turn-1",
      createdAt: "2026-07-13T00:00:14Z"
    },
    {
      type: "error" as const,
      message: "构建进程已经终止",
      retryable: false,
      terminal: true,
      threadId: "thread-1",
      turnId: "turn-1",
      createdAt: "2026-07-13T00:00:15Z"
    }
  ];
  const updated = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-timeline",
    events,
    now: "2026-07-13T00:00:15Z"
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assertMatches(html, /role="log"/);
  assertMatches(html, /时间线事件 1/);
  assertMatches(html, /时间线事件 7/);
  assertMatches(html, /执行计划/);
  assertMatches(html, /读取项目/);
  assertMatches(html, /已完成/);
  assertMatches(html, /修改界面/);
  assertMatches(html, /进行中/);
  assertMatches(html, /运行测试/);
  assertMatches(html, /待处理/);
  assertMatches(html, /Codex 过程说明/);
  assertMatches(html, /正在梳理组件并持续更新页面/);
  assert.equal((html.match(/src\/App\.tsx/g) ?? []).length, 1);
  assertMatches(html, /当前 diff · 1 个文件/);
  assertMatches(html, /展开当前 diff/);
  assertDoesNotMatch(html, /@@ -1 \+1,2 @@/);
  assertMatches(html, /展开完整命令输出/);
  assertMatches(html, /复制命令/);
  assertMatches(html, /复制错误/);
  assertMatches(html, /line 3/);
  assertDoesNotMatch(html, /line 1/);
  assertMatches(html, /非阻塞提醒/);
  assertMatches(html, /网络较慢，但运行仍在继续/);
  assertMatches(html, /运行失败/);
  assertMatches(html, /构建进程已经终止/);
  assertMatches(html, /1 个文件/);
  assertMatches(html, /\+2/);
  assertMatches(html, /-1/);
});

test("keeps the full timeline visible after the preview becomes ready", () => {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-complete",
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T00:00:00Z"
  });
  const withEvents = demoSessionReducer(running, {
    type: "append_agent_events",
    runId: "run-complete",
    now: "2026-07-13T00:00:08Z",
    events: Array.from({ length: 8 }, (_, index) => ({
      type: "diagnostic" as const,
      level: "info",
      message: `保留事件 ${index + 1}`,
      createdAt: `2026-07-13T00:00:0${index + 1}Z`
    }))
  });
  const completed = demoSessionReducer(withEvents, {
    type: "complete_agent_run",
    runId: "run-complete",
    status: "completed",
    finalMessage: "Demo 已完成",
    now: "2026-07-13T00:00:09Z"
  });
  const previewReady = demoSessionReducer(completed, {
    type: "set_preview_url",
    currentPreviewUrl: "http://localhost:5173",
    now: "2026-07-13T00:00:10Z"
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={previewReady} compact />);

  assertMatches(html, /已完成/);
  assertMatches(html, /Demo 已完成/);
  assertMatches(html, /耗时 9\.0s/);
  assertMatches(html, /保留事件 1/);
  assertMatches(html, /保留事件 8/);
});

test("shows a waiting hint when an active run has no progress beyond the threshold", () => {
  const startedAt = new Date(Date.now() - 20_000).toISOString();
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: startedAt
  });
  const running = demoSessionReducer(session, {
    type: "start_agent_run",
    runId: "run-waiting",
    kind: "initial_build",
    prompt: "实现 demo",
    now: startedAt
  });

  const html = renderToStaticMarkup(<DemoProgressPanel session={running} compact />);

  assertMatches(html, /最后进展于 \d+ 秒前/);
  assertMatches(html, /仍在等待 Codex/);
});
