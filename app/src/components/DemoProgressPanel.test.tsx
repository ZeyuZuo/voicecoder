import test from "node:test";
import assert from "node:assert/strict";
import { renderToStaticMarkup } from "react-dom/server";
import { DemoProgressPanel } from "./DemoProgressPanel";
import { createDemoSession, demoSessionReducer } from "../utils/demoSession";
import type { AgentEvent, DemoSession } from "../types/app";

function assertMatches(actual: string, pattern: RegExp) {
  assert.ok(pattern.test(actual), `Expected output to match ${pattern}`);
}

function assertDoesNotMatch(actual: string, pattern: RegExp) {
  assert.ok(!pattern.test(actual), `Expected output not to match ${pattern}`);
}

function getUniqueArticleContaining(html: string, text: string) {
  const matches = getArticles(html)
    .filter((article) => article.includes(text));
  assert.equal(matches.length, 1, `Expected one article containing ${JSON.stringify(text)}`);
  return matches[0];
}

function getArticles(html: string) {
  return html.match(/<article\b[\s\S]*?<\/article>/g) ?? [];
}

function getDomainArticles(html: string) {
  return getArticles(html).filter((article) => !article.includes("agent-working-card"));
}

function createRunningTestSession(runId: string): DemoSession {
  const session = createDemoSession({
    projectPath: "/tmp/demo",
    requirementId: "requirement-1",
    initialRequirementDocument: "构建 demo",
    initialCodingPrompt: "实现 demo",
    now: "2026-07-13T01:00:00Z"
  });
  return demoSessionReducer(session, {
    type: "start_agent_run",
    runId,
    kind: "initial_build",
    prompt: "实现 demo",
    now: "2026-07-13T01:00:00Z"
  });
}

function appendTestEvents(session: DemoSession, runId: string, events: AgentEvent[]) {
  return demoSessionReducer(session, {
    type: "append_agent_events",
    runId,
    events,
    now: events[events.length - 1]?.createdAt ?? "2026-07-13T01:00:59Z"
  });
}

function completedItemEvent(
  index: number,
  itemType: string,
  item: Record<string, unknown>,
  status = "completed"
): Extract<AgentEvent, { type: "item_completed" }> {
  const createdAt = `2026-07-13T01:00:${String(index).padStart(2, "0")}Z`;
  return {
    type: "item_completed",
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: typeof item.id === "string" ? item.id : `${itemType}-${index}`,
    itemType,
    lifecycle: "completed",
    status,
    completedAt: createdAt,
    item,
    createdAt
  };
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

test("uses the authoritative turn diff for per-file stats", () => {
  const running = createRunningTestSession("run-file-stats");
  const updated = appendTestEvents(running, "run-file-stats", [
    completedItemEvent(1, "fileChange", {
      id: "file-added",
      type: "fileChange",
      status: "completed",
      changes: [{
        path: "/tmp/demo/src/new.ts",
        kind: { type: "update" },
        diff: "@@ -1 +1 @@\n-old\n+const two = 2;\n"
      }]
    }),
    {
      type: "turn_diff_updated",
      threadId: "thread-1",
      turnId: "turn-1",
      diff: [
        "diff --git a/src/new.ts b/src/new.ts",
        "new file mode 100644",
        "--- /dev/null",
        "+++ b/src/new.ts",
        "@@ -0,0 +1,2 @@",
        "+const one = 1;",
        "+const two = 2;"
      ].join("\n"),
      createdAt: "2026-07-13T01:00:02Z"
    }
  ]);

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assertMatches(html, /src\/new\.ts[\s\S]*\+2[\s\S]*-0/);
  assertMatches(html, /新增[\s\S]*src\/new\.ts/);
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
      type: "command" as const,
      command: `时间线事件 ${index + 1}`,
      status: "completed",
      createdAt: `2026-07-13T00:00:0${index + 1}Z`
    })),
    {
      type: "thread_started" as const,
      threadId: "thread-internal",
      createdAt: "2026-07-13T00:00:07Z"
    },
    {
      type: "turn_started" as const,
      turnId: "turn-internal",
      createdAt: "2026-07-13T00:00:07Z"
    },
    {
      type: "diagnostic" as const,
      level: "warning",
      message: "内部协议版本提醒",
      createdAt: "2026-07-13T00:00:07Z"
    },
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
  assertDoesNotMatch(html, /thread-internal/);
  assertDoesNotMatch(html, /turn-internal/);
  assertDoesNotMatch(html, /内部协议版本提醒/);
  assert.ok(html.lastIndexOf("执行计划") > html.lastIndexOf("构建进程已经终止"));
  assertMatches(html, /<\/section><article class="agent-timeline-card agent-plan-card">/);
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
      type: "command" as const,
      command: `保留事件 ${index + 1}`,
      status: "completed",
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

  const withPlan = appendTestEvents(running, "run-waiting", [{
    type: "plan_updated",
    threadId: "thread-1",
    turnId: "turn-1",
    plan: [{ step: "生成页面代码", status: "inProgress" }],
    createdAt: startedAt
  }]);
  const html = renderToStaticMarkup(<DemoProgressPanel session={withPlan} compact />);

  assertMatches(html, /Working\.\.\./);
  assertDoesNotMatch(html, /正在执行：生成页面代码/);
  assertDoesNotMatch(html, /暂无新的 Codex 事件/);
  assertDoesNotMatch(html, /仍在等待 Codex/);
  assertDoesNotMatch(html.slice(0, html.indexOf('role="log"')), /Working\.\.\./);
  assertMatches(html, /Working\.\.\.[\s\S]*<\/section><article class="agent-timeline-card agent-plan-card">/);
});

test("renders every ThreadItem family once as a bounded domain card", () => {
  const runId = "run-remaining-items";
  const rawReasoning = "M5_RAW_REASONING_MUST_STAY_PRIVATE";
  const secret = "M5_SECRET_MUST_NOT_RENDER";
  const base64 = "data:image/png;base64,M5_BASE64_MUST_NOT_RENDER";
  const running = createRunningTestSession(runId);
  const itemEvents: AgentEvent[] = [
    completedItemEvent(1, "userMessage", {
      id: "user-message-1",
      type: "userMessage",
      content: [{ type: "text", text: "用户协议原文" }]
    }),
    completedItemEvent(2, "hookPrompt", {
      id: "hook-prompt-1",
      type: "hookPrompt",
      fragments: [{ text: "补充测试上下文", apiKey: secret }]
    }),
    completedItemEvent(3, "agentMessage", {
      id: "agent-message-1",
      type: "agentMessage",
      text: "正在覆盖剩余协议事件。",
      phase: "commentary"
    }),
    completedItemEvent(4, "plan", {
      id: "plan-1",
      type: "plan",
      text: "逐项验证剩余 ThreadItem。"
    }),
    completedItemEvent(5, "reasoning", {
      id: "reasoning-1",
      type: "reasoning",
      summary: ["先核对协议类型", "再验证领域卡片"],
      content: [rawReasoning]
    }),
    completedItemEvent(6, "commandExecution", {
      id: "command-1",
      type: "commandExecution",
      command: "npm run test:unit",
      cwd: "/tmp/demo",
      status: "completed",
      exitCode: 0,
      durationMs: 750,
      aggregatedOutput: "all tests passed"
    }),
    completedItemEvent(7, "fileChange", {
      id: "file-1",
      type: "fileChange",
      status: "completed",
      changes: [{
        path: "/tmp/demo/src/App.tsx",
        kind: { type: "update" },
        diff: "@@ -1 +1 @@\n-old\n+new\n"
      }]
    }),
    {
      type: "item_started",
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "mcp-1",
      itemType: "mcpToolCall",
      lifecycle: "in_progress",
      status: "inProgress",
      startedAt: "2026-07-13T01:00:08Z",
      item: {
        id: "mcp-1",
        type: "mcpToolCall",
        server: "browser",
        tool: "open",
        status: "inProgress",
        arguments: { url: "https://example.test", password: secret }
      },
      createdAt: "2026-07-13T01:00:08Z"
    },
    {
      type: "item_delta",
      threadId: "thread-1",
      turnId: "turn-1",
      itemId: "mcp-1",
      itemType: "mcpToolCall",
      lifecycle: "in_progress",
      method: "item/mcpToolCall/progress",
      delta: { message: "正在读取连接器结果" },
      createdAt: "2026-07-13T01:00:09Z"
    },
    completedItemEvent(10, "mcpToolCall", {
      id: "mcp-1",
      type: "mcpToolCall",
      server: "browser",
      tool: "open",
      status: "completed",
      durationMs: 420,
      arguments: { url: "https://example.test", password: secret },
      result: { ok: true }
    }),
    completedItemEvent(11, "dynamicToolCall", {
      id: "dynamic-1",
      type: "dynamicToolCall",
      namespace: "workspace",
      tool: "inspect",
      status: "failed",
      success: false,
      arguments: { apiKey: secret },
      contentItems: [{ type: "inputText", text: "inspection failed" }]
    }, "failed"),
    completedItemEvent(12, "collabAgentToolCall", {
      id: "collab-1",
      type: "collabAgentToolCall",
      tool: "spawnAgent",
      status: "completed",
      receiverThreadIds: ["thread-child-123456789"],
      prompt: "检查 UI 映射",
      agentsStates: {
        "thread-child-123456789": { status: "completed", message: "审计完成" }
      }
    }),
    completedItemEvent(13, "subAgentActivity", {
      id: "subagent-1",
      type: "subAgentActivity",
      kind: "started",
      agentThreadId: "thread-child-123456789",
      agentPath: "/root/ui-audit"
    }),
    completedItemEvent(14, "webSearch", {
      id: "web-1",
      type: "webSearch",
      query: "Codex app-server ThreadItem",
      action: { type: "search", query: "Codex app-server ThreadItem" }
    }),
    completedItemEvent(15, "imageView", {
      id: "image-view-1",
      type: "imageView",
      status: "completed",
      path: "/tmp/demo/reference.png"
    }),
    completedItemEvent(16, "sleep", {
      id: "sleep-1",
      type: "sleep",
      status: "completed",
      durationMs: 1_500
    }),
    completedItemEvent(17, "imageGeneration", {
      id: "image-generation-1",
      type: "imageGeneration",
      status: "completed",
      revisedPrompt: "生成简洁的产品图",
      savedPath: "/tmp/demo/generated.png",
      result: base64
    }),
    completedItemEvent(18, "enteredReviewMode", {
      id: "review-enter-1",
      type: "enteredReviewMode",
      status: "completed",
      review: "检查协议覆盖"
    }),
    completedItemEvent(19, "exitedReviewMode", {
      id: "review-exit-1",
      type: "exitedReviewMode",
      status: "completed",
      review: "协议覆盖通过"
    }),
    completedItemEvent(20, "contextCompaction", {
      id: "compact-1",
      type: "contextCompaction",
      status: "completed"
    }),
    completedItemEvent(21, "futureActivity", {
      id: "future-1",
      type: "futureActivity",
      status: "completed",
      credential: secret,
      payload: base64
    }),
    {
      type: "context_compacted",
      threadId: "thread-1",
      turnId: "turn-1",
      createdAt: "2026-07-13T01:00:22Z"
    }
  ];
  const updated = appendTestEvents(running, runId, itemEvents);

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assert.equal(getDomainArticles(html).length, 18);
  assert.equal((html.match(/长对话上下文整理完成/g) ?? []).length, 1);
  assertDoesNotMatch(html, /用户输入已提交/);
  assertMatches(html, /Hook 已补充上下文/);
  assertMatches(html, /Codex 过程说明/);
  assertMatches(html, /分析完成/);
  assertMatches(html, /查看分析摘要/);
  assertMatches(html, /原始分析仅保留在受限协议日志/);
  assertMatches(html, /aria-expanded="false"/);
  assertDoesNotMatch(html, /先核对协议类型/);
  assertMatches(html, /文件修改完成/);
  assertMatches(html, /命令执行完成/);
  assertMatches(getUniqueArticleContaining(html, "MCP 工具 · browser / open"), />已完成</);
  assertMatches(html, /正在读取连接器结果/);
  assertMatches(html, /页面仅保留安全摘要，完整原始内容见受限协议日志/);
  assertMatches(getUniqueArticleContaining(html, "动态工具 · workspace / inspect"), />失败</);
  assertMatches(html, /正在创建子 Agent/);
  assertMatches(html, /子 Agent 已启动/);
  assertMatches(html, /正在搜索网页/);
  assertMatches(getUniqueArticleContaining(html, "查看图像"), />已完成</);
  assertMatches(html, /等待 1\.5s/);
  assertMatches(getUniqueArticleContaining(html, "图像生成"), />已完成</);
  assertMatches(html, /已进入审查模式/);
  assertMatches(html, /已退出审查模式/);
  assertMatches(html, /协议活动 futureActivity · 已完成/);
  assertMatches(html, /查看协议详情/);
  assertDoesNotMatch(html, new RegExp(rawReasoning));
  assertDoesNotMatch(html, new RegExp(secret));
  assertDoesNotMatch(html, /M5_BASE64_MUST_NOT_RENDER/);
  assertDoesNotMatch(html, /data:image\/png;base64/);
});

test("renders token, hook, model and differentiated warning updates", () => {
  const runId = "run-system-updates";
  const running = createRunningTestSession(runId);
  const updated = appendTestEvents(running, runId, [
    {
      type: "token_usage_updated",
      threadId: "thread-1",
      turnId: "turn-1",
      tokenUsage: {
        total: {
          totalTokens: 1_234,
          inputTokens: 900,
          cachedInputTokens: 120,
          outputTokens: 334,
          reasoningOutputTokens: 80
        },
        last: {
          totalTokens: 321,
          inputTokens: 220,
          cachedInputTokens: 20,
          outputTokens: 101,
          reasoningOutputTokens: 30
        },
        modelContextWindow: 8_192
      },
      createdAt: "2026-07-13T01:01:01Z"
    },
    {
      type: "hook_run_updated",
      threadId: "thread-1",
      turnId: "turn-1",
      hookId: "hook-1",
      lifecycle: "completed",
      run: {
        eventName: "postToolUse",
        status: "failed",
        statusMessage: "lint hook failed",
        durationMs: 230,
        sourcePath: "/tmp/demo/.codex/hooks/lint.sh",
        _uiProjectionTruncated: true,
        entries: [{ kind: "error", text: "lint failed" }]
      },
      createdAt: "2026-07-13T01:01:02Z"
    },
    {
      type: "model_rerouted",
      threadId: "thread-1",
      turnId: "turn-1",
      fromModel: "gpt-primary",
      toModel: "gpt-safety",
      reason: "highRiskCyberActivity",
      createdAt: "2026-07-13T01:01:03Z"
    },
    {
      type: "model_safety_buffering_updated",
      threadId: "thread-1",
      turnId: "turn-1",
      model: "gpt-safety",
      useCases: ["cyber"],
      reasons: ["正在执行安全检查"],
      showBufferingUi: true,
      fasterModel: "gpt-fast",
      createdAt: "2026-07-13T01:01:04Z"
    },
    {
      type: "model_verification_updated",
      threadId: "thread-1",
      turnId: "turn-1",
      verifications: ["trustedAccessForCyber"],
      createdAt: "2026-07-13T01:01:05Z"
    },
    {
      type: "warning",
      message: "网络较慢，但任务仍在运行",
      threadId: "thread-1",
      turnId: "turn-1",
      createdAt: "2026-07-13T01:01:06Z"
    },
    {
      type: "config_warning",
      summary: "配置键已弃用",
      details: "请改用 approval_policy",
      path: "/tmp/demo/.codex/config.toml",
      range: {
        start: { line: 3, column: 1 },
        end: { line: 3, column: 12 }
      },
      createdAt: "2026-07-13T01:01:07Z"
    },
    {
      type: "config_warning",
      summary: "配置键已弃用",
      details: "请改用 approval_policy",
      path: "/tmp/demo/.codex/config.toml",
      range: {
        start: { line: 3, column: 1 },
        end: { line: 3, column: 12 }
      },
      createdAt: "2026-07-13T01:01:08Z"
    },
    {
      type: "guardian_warning",
      message: "命令需要额外安全审查",
      threadId: "thread-1",
      createdAt: "2026-07-13T01:01:09Z"
    }
  ]);

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assertMatches(html, /Token 本轮 321/);
  assertMatches(html, /累计 1,234/);
  assertMatches(html, /窗口 8,192/);
  assertMatches(getUniqueArticleContaining(html, "Hook · 工具调用后"), />失败</);
  assertMatches(html, /lint hook failed/);
  assertMatches(getUniqueArticleContaining(html, "Hook · 工具调用后"), /页面仅保留安全摘要/);
  assertMatches(html, /模型已切换/);
  assertMatches(html, /gpt-primary/);
  assertMatches(html, /gpt-safety/);
  assertMatches(html, /因高风险网络安全活动切换模型/);
  assertMatches(html, /安全缓冲处理中/);
  assertMatches(html, /正在执行安全检查/);
  assertMatches(html, /可切换到 gpt-fast/);
  assertMatches(html, /模型验证已启用/);
  assertMatches(html, /可信网络安全访问/);
  assertMatches(html, /非阻塞提醒/);
  assertMatches(html, /配置提醒/);
  assertMatches(html, /重复 2 次/);
  assertMatches(html, /config\.toml:3:1/);
  assertMatches(html, /安全审查提醒/);
});

test("aggregates routine hooks while keeping failed hooks prominent", () => {
  const runId = "run-hook-aggregation";
  const running = createRunningTestSession(runId);
  const routineHooks: AgentEvent[] = Array.from({ length: 80 }, (_, index) => {
    const createdAt = new Date(Date.parse("2026-07-13T02:00:00Z") + index).toISOString();
    return {
      type: "hook_run_updated",
      threadId: "thread-1",
      turnId: "turn-1",
      hookId: `hook-routine-${index}`,
      lifecycle: "completed",
      run: {
        displayOrder: index,
        eventName: "preToolUse",
        handlerType: "command",
        executionMode: "sync",
        status: "completed",
        startedAt: Date.parse(createdAt),
        completedAt: Date.parse(createdAt),
        entries: []
      },
      createdAt
    };
  });
  const failedHook: AgentEvent = {
    type: "hook_run_updated",
    threadId: "thread-1",
    turnId: "turn-1",
    hookId: "hook-failed",
    lifecycle: "completed",
    run: {
      displayOrder: 81,
      eventName: "postToolUse",
      handlerType: "command",
      executionMode: "sync",
      status: "failed",
      statusMessage: "format hook failed",
      entries: [{ kind: "error", text: "format failed" }]
    },
    createdAt: "2026-07-13T02:00:01Z"
  };
  const updated = appendTestEvents(running, runId, [...routineHooks, failedHook]);

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);

  assert.equal(getDomainArticles(html).length, 2);
  assertMatches(getUniqueArticleContaining(html, "Hooks · 80 次"), /工具调用前 × 80/);
  assertMatches(getUniqueArticleContaining(html, "Hook · 工具调用后"), /format hook failed/);
});

test("renders auto-reviewed approvals as non-blocking status without manual buttons", () => {
  const runId = "run-auto-approval";
  const updated = appendTestEvents(createRunningTestSession(runId), runId, [{
    type: "server_request",
    requestId: 9002,
    requestKey: "number:9002",
    method: "item/commandExecution/requestApproval",
    kind: "command_approval",
    status: "auto_reviewing",
    requiresUserInput: false,
    autoReview: true,
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "item-command",
    details: {
      command: "npm run check",
      cwd: "/tmp/demo",
      reason: "需要执行项目检查"
    },
    expiresAt: "2026-07-13T03:02:00Z",
    createdAt: "2026-07-13T03:00:00Z"
  }]);

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);
  const card = getUniqueArticleContaining(html, "自动审查命令权限");
  assertMatches(card, /自动审批中/);
  assertMatches(card, /无需手动操作/);
  assertMatches(card, /npm run check/);
  assertDoesNotMatch(card, /批准本次/);
  assertDoesNotMatch(card, /本会话允许/);
});

test("renders request_user_input choices with explicit timeout and cancellation", () => {
  const runId = "run-user-input";
  const updated = appendTestEvents(createRunningTestSession(runId), runId, [{
    type: "server_request",
    requestId: 9004,
    requestKey: "number:9004",
    method: "item/tool/requestUserInput",
    kind: "user_input",
    status: "pending",
    requiresUserInput: true,
    autoReview: false,
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "item-user-input",
    details: {
      autoResolutionMs: 60_000,
      questions: [{
        id: "theme",
        header: "主题",
        question: "Demo 应使用哪种主题？",
        isOther: true,
        isSecret: false,
        options: [
          { label: "浅色", description: "推荐，适合演示" },
          { label: "深色", description: "适合暗色工作区" }
        ]
      }]
    },
    expiresAt: "2026-07-13T03:01:00Z",
    createdAt: "2026-07-13T03:00:00Z"
  }]);

  const html = renderToStaticMarkup(<DemoProgressPanel session={updated} compact />);
  const card = getUniqueArticleContaining(html, "Codex 需要你的选择");
  assertMatches(card, /Demo 应使用哪种主题/);
  assertMatches(card, /浅色/);
  assertMatches(card, /深色/);
  assertMatches(card, /超时后使用推荐首选项继续/);
  assertMatches(card, /提交回答/);
  assertMatches(card, /取消请求/);
  assertMatches(card, /输入其他回答/);
});

test("renders MCP elicitation form and removes actions after server resolution", () => {
  const runId = "run-mcp-elicitation";
  const requested = appendTestEvents(createRunningTestSession(runId), runId, [{
    type: "server_request",
    requestId: "mcp-9005",
    requestKey: "string:mcp-9005",
    method: "mcpServer/elicitation/request",
    kind: "mcp_elicitation",
    status: "pending",
    requiresUserInput: true,
    autoReview: false,
    threadId: "thread-1",
    turnId: "turn-1",
    details: {
      serverName: "deployment",
      mode: "form",
      message: "请选择预览环境",
      requestedSchema: {
        type: "object",
        properties: {
          environment: {
            type: "string",
            title: "环境",
            enum: ["staging", "production"],
            default: "staging"
          }
        },
        required: ["environment"]
      }
    },
    expiresAt: "2026-07-13T03:05:00Z",
    createdAt: "2026-07-13T03:00:00Z"
  }]);
  const pendingHtml = renderToStaticMarkup(<DemoProgressPanel session={requested} compact />);
  const pendingCard = getUniqueArticleContaining(pendingHtml, "MCP 请求补充信息");
  assertMatches(pendingCard, /请选择预览环境/);
  assertMatches(pendingCard, /MCP Server · deployment/);
  assertMatches(pendingCard, /staging/);
  assertMatches(pendingCard, /提交给 MCP/);

  const resolved = appendTestEvents(requested, runId, [{
    type: "server_request_resolved",
    requestId: "mcp-9005",
    requestKey: "string:mcp-9005",
    status: "resolved",
    resolution: "submitted",
    message: "已提交回答",
    createdAt: "2026-07-13T03:00:10Z"
  }]);
  const resolvedHtml = renderToStaticMarkup(<DemoProgressPanel session={resolved} compact />);
  const resolvedCard = getUniqueArticleContaining(resolvedHtml, "MCP 请求补充信息");
  assertMatches(resolvedCard, /已提交回答/);
  assertDoesNotMatch(resolvedCard, /提交给 MCP/);
  assertDoesNotMatch(resolvedCard, /超时后安全取消/);
});
