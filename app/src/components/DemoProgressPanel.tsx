import { AlertTriangle, Bot, FileCode2, ListChecks, ShieldCheck, Terminal, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import type { AgentEvent, AgentFileChange, AgentItem, AgentRun, DemoSession } from "../types/app";
import { getAgentItemDurationMs, getAgentOutputPreview } from "../utils/agentProgress";

type DemoProgressPanelProps = {
  session: DemoSession;
  compact?: boolean;
};

export function DemoProgressPanel({ session, compact }: DemoProgressPanelProps) {
  const latestRun = session.runs[session.runs.length - 1];
  const workItems = latestRun
    ? (latestRun.itemOrder ?? [])
        .map((itemId) => latestRun.itemsById?.[itemId])
        .filter((item): item is AgentItem => Boolean(item && (item.type === "fileChange" || item.type === "commandExecution")))
    : [];
  const hasRunningCommand = workItems.some(
    (item) => item.type === "commandExecution" && item.lifecycle === "in_progress"
  );
  const nowMs = useAgentClock(hasRunningCommand);

  if (!latestRun) {
    return (
      <section className={`agent-progress-panel ${compact ? "is-compact" : ""}`} aria-live="polite">
        <div className="agent-progress-header">
          <div>
            <span>Demo 生成</span>
            <strong>{getDemoStatusLabel(session.status)}</strong>
          </div>
        </div>
        <div className="agent-progress-events">
          <div className="agent-progress-event is-waiting">
            <Bot size={15} />
            <p>等待开始生成 demo</p>
          </div>
        </div>
      </section>
    );
  }

  const visibleEvents = summarizeAgentEvents(latestRun.events)
    .filter((event) => !isFileOrCommandItemEvent(event))
    .slice(-5);
  const changedCount = Object.keys(latestRun.filesByPath ?? {}).length || latestRun.changedFiles.length;
  const fileDiffStats = Object.values(latestRun.filesByPath ?? {}).reduce(
    (stats, file) => ({
      additions: stats.additions + file.additions,
      deletions: stats.deletions + file.deletions,
      files: stats.files + 1
    }),
    { additions: 0, deletions: 0, files: 0 }
  );
  const diffStats = latestRun.aggregateDiff
    ? latestRun.aggregateDiffStats ?? fileDiffStats
    : fileDiffStats;

  return (
    <section className={`agent-progress-panel is-${latestRun.status} ${compact ? "is-compact" : ""}`} aria-live="polite">
      <div className="agent-progress-header">
        <div>
          <span>{getAgentRunKindLabel(latestRun.kind)}</span>
          <strong>{getAgentRunStatusLabel(latestRun.status)}</strong>
        </div>
        {changedCount || diffStats.additions || diffStats.deletions ? (
          <small>
            {changedCount} 个文件
            <span className="agent-diff-additions"> +{diffStats.additions}</span>
            <span className="agent-diff-deletions"> -{diffStats.deletions}</span>
          </small>
        ) : null}
      </div>

      {workItems.length ? (
        <div className="agent-work-items">
          {workItems.map((item) => (
            <AgentWorkItemCard
              item={item}
              nowMs={nowMs}
              projectPath={session.projectPath}
              key={item.id}
            />
          ))}
        </div>
      ) : null}

      <div className="agent-progress-events">
        {visibleEvents.length ? (
          visibleEvents.map((event, index) => (
            <div className={`agent-progress-event is-${event.type}`} key={`${event.type}-${event.createdAt}-${index}`}>
              <AgentEventIcon event={event} />
              <p>{formatAgentEvent(event)}</p>
            </div>
          ))
        ) : !workItems.length ? (
          <div className="agent-progress-event is-waiting">
            <Bot size={15} />
            <p>正在启动 Codex thread</p>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function AgentWorkItemCard({
  item,
  nowMs,
  projectPath
}: {
  item: AgentItem;
  nowMs: number;
  projectPath: string;
}) {
  return item.type === "fileChange"
    ? <FileChangeCard item={item} projectPath={projectPath} />
    : <CommandExecutionCard item={item} nowMs={nowMs} />;
}

function FileChangeCard({ item, projectPath }: { item: AgentItem; projectPath: string }) {
  const changes = item.fileChanges ?? [];
  const status = item.status ?? (item.lifecycle === "completed" ? "completed" : "inProgress");

  return (
    <article className={`agent-work-card is-file-change is-${normalizeCssToken(status)}`}>
      <header>
        <FileCode2 size={15} />
        <strong>{getFileChangeStatusLabel(status, item.lifecycle)}</strong>
        <span>{changes.length ? `${changes.length} 个文件` : "等待补丁"}</span>
      </header>
      {changes.length ? (
        <ul className="agent-file-change-list">
          {changes.map((change) => (
            <li key={`${change.path}-${change.movePath ?? ""}`}>
              <span className={`agent-file-kind is-${change.kind}`}>{getFileChangeKindLabel(change.kind)}</span>
              <code>{formatFileChangePath(change, projectPath)}</code>
              <span className="agent-file-stats">
                <span className="agent-diff-additions">+{change.additions}</span>
                <span className="agent-diff-deletions">-{change.deletions}</span>
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="agent-work-placeholder">Codex 正在准备文件修改…</p>
      )}
    </article>
  );
}

function CommandExecutionCard({ item, nowMs }: { item: AgentItem; nowMs: number }) {
  const command = item.command;
  const status = command?.status ?? item.status ?? (item.lifecycle === "completed" ? "completed" : "inProgress");
  const durationMs = getAgentItemDurationMs(
    item.startedAt,
    item.completedAt,
    command?.durationMs,
    nowMs
  );
  const outputPreview = getAgentOutputPreview(command?.outputTail ?? item.output);

  return (
    <article className={`agent-work-card is-command-execution is-${normalizeCssToken(status)}`}>
      <header>
        <Terminal size={15} />
        <strong>{getCommandStatusLabel(status)}</strong>
        {durationMs !== undefined ? <span>{formatDuration(durationMs)}</span> : null}
      </header>
      <code className="agent-command-text">{command?.command || "等待 Codex 提供命令…"}</code>
      {command?.cwd ? <small className="agent-command-cwd">cwd · {command.cwd}</small> : null}
      {outputPreview ? (
        <pre className="agent-command-output">
          {command?.outputTruncated ? "… 仅显示输出末尾\n" : ""}
          {outputPreview}
        </pre>
      ) : item.lifecycle === "in_progress" ? (
        <p className="agent-work-placeholder">命令正在执行，等待输出…</p>
      ) : null}
      {item.lifecycle === "completed" ? (
        <footer>
          {typeof command?.exitCode === "number" ? <span>退出码 {command.exitCode}</span> : null}
          {status === "declined" ? <span>命令未获批准</span> : null}
        </footer>
      ) : null}
    </article>
  );
}

function useAgentClock(enabled: boolean) {
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (!enabled) {
      return;
    }
    setNowMs(Date.now());
    const interval = window.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => window.clearInterval(interval);
  }, [enabled]);

  return nowMs;
}

function isFileOrCommandItemEvent(event: AgentEvent) {
  return (
    (event.type === "item_started" || event.type === "item_delta" || event.type === "item_completed") &&
    (event.itemType === "fileChange" || event.itemType === "commandExecution")
  );
}

function formatFileChangePath(change: AgentFileChange, projectPath: string) {
  const path = compactProjectPath(change.path, projectPath);
  return change.movePath
    ? `${path} → ${compactProjectPath(change.movePath, projectPath)}`
    : path;
}

function compactProjectPath(path: string, projectPath: string) {
  const prefix = `${projectPath.replace(/\/+$/, "")}/`;
  return path.startsWith(prefix) ? path.slice(prefix.length) : path;
}

function getFileChangeStatusLabel(status: string, lifecycle: AgentItem["lifecycle"]) {
  if (status === "failed") {
    return "文件修改失败";
  }
  if (status === "declined") {
    return "文件修改被拒绝";
  }
  return lifecycle === "completed" ? "文件修改完成" : "正在修改文件";
}

function getFileChangeKindLabel(kind: AgentFileChange["kind"]) {
  return {
    add: "新增",
    update: "修改",
    delete: "删除",
    unknown: "变更"
  }[kind];
}

function getCommandStatusLabel(status: string) {
  return {
    inProgress: "正在执行命令",
    completed: "命令执行完成",
    failed: "命令执行失败",
    declined: "命令执行被拒绝"
  }[status] ?? `命令状态：${status}`;
}

function formatDuration(durationMs: number) {
  if (durationMs < 1_000) {
    return `${Math.round(durationMs)}ms`;
  }
  if (durationMs < 60_000) {
    return `${(durationMs / 1_000).toFixed(durationMs < 10_000 ? 1 : 0)}s`;
  }
  const minutes = Math.floor(durationMs / 60_000);
  const seconds = Math.floor((durationMs % 60_000) / 1_000);
  return `${minutes}m ${seconds}s`;
}

function normalizeCssToken(value: string) {
  return value.replace(/([a-z])([A-Z])/g, "$1-$2").toLowerCase();
}

function summarizeAgentEvents(events: AgentEvent[]) {
  const summarized: AgentEvent[] = [];

  for (const event of events) {
    const previous = summarized[summarized.length - 1];
    if (
      previous &&
      event.type === "agent_message" &&
      previous.type === "agent_message"
    ) {
      summarized[summarized.length - 1] = {
        ...event,
        text: compactText(`${previous.text}${event.text}`)
      };
      continue;
    }

    if (
      previous &&
      event.type === "plan_update" &&
      previous.type === "plan_update"
    ) {
      summarized[summarized.length - 1] = {
        ...event,
        text: compactText(`${previous.text} ${event.text}`)
      };
      continue;
    }

    summarized.push(event);
  }

  return summarized;
}

function AgentEventIcon({ event }: { event: AgentEvent }) {
  if (event.type === "command") {
    return <Terminal size={15} />;
  }

  if (event.type === "file_change" || isItemEventOfType(event, "fileChange")) {
    return <FileCode2 size={15} />;
  }

  if (event.type === "plan_update" || event.type === "plan_updated" || isItemEventOfType(event, "plan")) {
    return <ListChecks size={15} />;
  }

  if (isItemEventOfType(event, "commandExecution")) {
    return <Terminal size={15} />;
  }

  if (event.type === "approval_review") {
    return <ShieldCheck size={15} />;
  }

  if (event.type === "error") {
    return <XCircle size={15} />;
  }

  if (event.type === "warning") {
    return <AlertTriangle size={15} />;
  }

  if (event.type === "diagnostic") {
    return event.level === "error" ? <XCircle size={15} /> : <Bot size={15} />;
  }

  return <Bot size={15} />;
}

function formatAgentEvent(event: AgentEvent) {
  if (event.type === "thread_started") {
    return `Thread ${event.threadId}`;
  }

  if (event.type === "turn_started") {
    return event.turnId ? `Turn ${event.turnId}` : "Turn 已启动";
  }

  if (event.type === "agent_message") {
    return compactText(event.text);
  }

  if (event.type === "plan_update") {
    return compactText(event.text);
  }

  if (event.type === "plan_updated") {
    const steps = event.plan.map((step) => `[${step.status}] ${step.step}`).join(" · ");
    return compactText([event.explanation, steps].filter(Boolean).join(" · "));
  }

  if (event.type === "turn_diff_updated") {
    return "整轮文件 diff 已更新";
  }

  if (event.type === "item_started") {
    return formatItemLifecycleEvent(event.itemType, event.status ?? "inProgress", event.item);
  }

  if (event.type === "item_delta") {
    return formatItemDeltaEvent(event);
  }

  if (event.type === "item_completed") {
    return formatItemLifecycleEvent(event.itemType, event.status ?? "completed", event.item);
  }

  if (event.type === "approval_review") {
    const status = getApprovalReviewStatusLabel(event.status);
    const action = event.action ? ` · ${event.action}` : "";
    const rationale = event.rationale ? ` · ${event.rationale}` : "";
    return `自动权限审查${status}${action}${rationale}`;
  }

  if (event.type === "command") {
    return `${event.status} · ${event.command}`;
  }

  if (event.type === "file_change") {
    return `${event.changeType ?? "changed"} · ${event.path}`;
  }

  if (event.type === "turn_completed") {
    return compactText(event.finalMessage ?? `本轮 ${event.status}`);
  }

  if (event.type === "diagnostic") {
    return compactText(event.method ? `${event.message} · ${event.method}` : event.message);
  }

  return event.message;
}

function isItemEventOfType(event: AgentEvent, itemType: string) {
  return (
    (event.type === "item_started" || event.type === "item_delta" || event.type === "item_completed") &&
    event.itemType === itemType
  );
}

function formatItemLifecycleEvent(itemType: string, status: string, item: Record<string, unknown>) {
  if (itemType === "commandExecution") {
    return compactText(`${status} · ${readDisplayString(item.command) ?? "执行命令"}`);
  }
  if (itemType === "fileChange") {
    const changes = Array.isArray(item.changes) ? item.changes : [];
    return `${status} · ${changes.length} 个文件变更`;
  }
  if (itemType === "agentMessage" || itemType === "plan") {
    return compactText(readDisplayString(item.text) ?? `${itemType} · ${status}`);
  }
  return `${itemType} · ${status}`;
}

function formatItemDeltaEvent(event: Extract<AgentEvent, { type: "item_delta" }>) {
  if (typeof event.delta === "string") {
    return compactText(event.delta) || `${event.itemType} 更新中`;
  }
  if (event.method === "item/fileChange/patchUpdated" && Array.isArray(event.delta)) {
    return `正在更新 ${event.delta.length} 个文件`;
  }
  return `${event.itemType} 更新中`;
}

function readDisplayString(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

function compactText(text: string) {
  return text.replace(/\s+/g, " ").trim();
}

function getApprovalReviewStatusLabel(status: string) {
  const labels: Record<string, string> = {
    inProgress: "中",
    approved: "已批准",
    denied: "已拒绝",
    timedOut: "已超时",
    aborted: "已取消"
  };

  return labels[status] ?? `：${status}`;
}

function getAgentRunKindLabel(kind: AgentRun["kind"]) {
  return kind === "initial_build" ? "Demo 生成" : "Demo 修改";
}

function getAgentRunStatusLabel(status: AgentRun["status"]) {
  const labels = {
    queued: "排队中",
    starting: "启动中",
    running: "运行中",
    succeeded: "已完成",
    failed: "失败",
    cancelled: "已取消"
  };

  return labels[status];
}

function getDemoStatusLabel(status: DemoSession["status"]) {
  const labels = {
    idle: "Demo 空闲",
    ready_to_start: "待生成 demo",
    agent_running: "生成 demo 中",
    preview_ready: "Demo 已生成",
    feedback_listening: "等待反馈",
    feedback_processing: "整理反馈中",
    agent_modifying: "修改 demo 中",
    error: "Demo 出错"
  };

  return labels[status];
}
