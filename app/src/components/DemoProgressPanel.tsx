import {
  AlertTriangle,
  Bot,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Circle,
  Clipboard,
  Clock,
  FileCode2,
  GitCompare,
  ListChecks,
  LoaderCircle,
  ShieldCheck,
  Terminal,
  XCircle
} from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { writeText as writeClipboardText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentEvent,
  AgentFileChange,
  AgentItem,
  AgentPlan,
  AgentRun,
  AgentRunError,
  AgentWarning,
  DemoSession
} from "../types/app";
import {
  AGENT_WAITING_THRESHOLD_MS,
  getAgentItemDurationMs,
  getAgentLatestProgressAt,
  getAgentOutputPreview,
  getAgentProgressAgeMs,
  isAgentTimelineNearBottom
} from "../utils/agentProgress";

type DemoProgressPanelProps = {
  session: DemoSession;
  compact?: boolean;
};

type AgentTimelineEntry =
  | {
      kind: "item";
      id: string;
      occurredAt: string;
      item: AgentItem;
    }
  | {
      kind: "files";
      id: string;
      occurredAt: string;
      items: AgentItem[];
    }
  | {
      kind: "plan";
      id: string;
      occurredAt: string;
      plan: AgentPlan;
    }
  | {
      kind: "warning";
      id: string;
      occurredAt: string;
      warning: AgentWarning;
    }
  | {
      kind: "error";
      id: string;
      occurredAt: string;
      error: AgentRunError;
    }
  | {
      kind: "event";
      id: string;
      occurredAt: string;
      event: AgentEvent;
    };

type IndexedTimelineEntry = AgentTimelineEntry & {
  sequence: number;
  sortMs: number;
};

export function DemoProgressPanel({ session, compact }: DemoProgressPanelProps) {
  const latestRun = session.runs[session.runs.length - 1];
  const runActive = Boolean(latestRun && !isTerminalAgentRun(latestRun));
  const nowMs = useAgentClock(runActive);
  const timelineEntries = useMemo(
    () => latestRun ? buildAgentTimelineEntries(latestRun) : [],
    [latestRun]
  );
  const latestProgressAt = latestRun ? getAgentLatestProgressAt(latestRun) : undefined;
  const progressVersion = latestRun ? getAgentProgressVersion(latestRun, latestProgressAt) : "idle";
  const timeline = useAgentTimelineAutoScroll(latestRun?.id, progressVersion);

  if (!latestRun) {
    return (
      <section className={`agent-progress-panel ${compact ? "is-compact" : ""}`} aria-live="polite">
        <div className="agent-progress-header">
          <div className="agent-progress-title">
            <span>Demo 生成</span>
            <strong>{getDemoStatusLabel(session.status)}</strong>
          </div>
        </div>
        <div className="agent-progress-timeline is-empty">
          <div className="agent-progress-event is-waiting">
            <Bot size={15} />
            <p>等待开始生成 demo</p>
          </div>
        </div>
      </section>
    );
  }

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
  const changedCount = Math.max(
    new Set([
      ...Object.keys(latestRun.filesByPath ?? {}),
      ...latestRun.changedFiles
    ]).size,
    diffStats.files
  );
  const durationMs = latestRun.startedAt
    ? getAgentItemDurationMs(latestRun.startedAt, latestRun.completedAt, undefined, nowMs)
    : undefined;
  const progressAgeMs = getAgentProgressAgeMs(latestProgressAt, nowMs);
  const waitingForCodex = runActive && progressAgeMs !== undefined && progressAgeMs >= AGENT_WAITING_THRESHOLD_MS;

  return (
    <section className={`agent-progress-panel is-${latestRun.status} ${compact ? "is-compact" : ""}`}>
      <div className="agent-progress-header">
        <div className="agent-progress-title">
          <span>{getAgentRunKindLabel(latestRun.kind)}</span>
          <strong>{getAgentRunStatusLabel(latestRun.status)}</strong>
          {durationMs !== undefined ? <small>耗时 {formatDuration(durationMs)}</small> : null}
        </div>
        <div className="agent-progress-stats" aria-label="文件变更统计">
          <span>{changedCount} 个文件</span>
          <span className="agent-diff-additions">+{diffStats.additions}</span>
          <span className="agent-diff-deletions">-{diffStats.deletions}</span>
        </div>
      </div>

      <div className={`agent-progress-freshness ${waitingForCodex ? "is-waiting" : ""}`}>
        <Clock size={13} />
        <span>{formatLatestProgress(progressAgeMs)}</span>
        {waitingForCodex ? <strong>仍在等待 Codex</strong> : null}
      </div>

      <div className="agent-progress-timeline-shell">
        <div
          className="agent-progress-timeline"
          ref={timeline.ref}
          onScroll={timeline.onScroll}
          role="log"
          aria-label="Codex 生成时间线"
          aria-live="polite"
          aria-relevant="additions text"
        >
          {timelineEntries.length ? (
            timelineEntries.map((entry) => (
              <AgentTimelineEntryView
                entry={entry}
                run={latestRun}
                nowMs={nowMs}
                projectPath={session.projectPath}
                key={entry.id}
              />
            ))
          ) : (
            <div className="agent-progress-event is-waiting">
              <Bot size={15} />
              <p>正在启动 Codex thread</p>
            </div>
          )}
        </div>
        {!timeline.autoFollow ? (
          <button className="agent-timeline-follow" type="button" onClick={timeline.scrollToLatest}>
            回到最新进展
          </button>
        ) : null}
      </div>
    </section>
  );
}

function AgentTimelineEntryView({
  entry,
  run,
  nowMs,
  projectPath
}: {
  entry: AgentTimelineEntry;
  run: AgentRun;
  nowMs: number;
  projectPath: string;
}) {
  if (entry.kind === "plan") {
    return <AgentPlanCard plan={entry.plan} occurredAt={entry.occurredAt} />;
  }
  if (entry.kind === "files") {
    return (
      <FileChangesCard
        items={entry.items}
        run={run}
        projectPath={projectPath}
        occurredAt={entry.occurredAt}
      />
    );
  }
  if (entry.kind === "warning") {
    return <AgentWarningCard warning={entry.warning} />;
  }
  if (entry.kind === "error") {
    return <AgentErrorCard error={entry.error} />;
  }
  if (entry.kind === "event") {
    return <AgentEventCard event={entry.event} />;
  }
  if (entry.item.type === "commandExecution") {
    return <CommandExecutionCard item={entry.item} nowMs={nowMs} occurredAt={entry.occurredAt} />;
  }
  return <AgentMessageCard item={entry.item} occurredAt={entry.occurredAt} />;
}

function AgentPlanCard({ plan, occurredAt }: { plan: AgentPlan; occurredAt: string }) {
  return (
    <article className="agent-timeline-card agent-plan-card">
      <header>
        <ListChecks size={15} />
        <strong>执行计划</strong>
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {plan.explanation ? <p className="agent-plan-explanation">{plan.explanation}</p> : null}
      {plan.steps.length ? (
        <ol className="agent-plan-steps">
          {plan.steps.map((step, index) => (
            <li className={`is-${normalizeCssToken(step.status)}`} key={`${index}-${step.step}`}>
              <AgentPlanStepIcon status={step.status} />
              <span>{step.step}</span>
              <small>{getPlanStepStatusLabel(step.status)}</small>
            </li>
          ))}
        </ol>
      ) : (
        <p className="agent-work-placeholder">Codex 正在制定计划…</p>
      )}
    </article>
  );
}

function AgentPlanStepIcon({ status }: { status: AgentPlan["steps"][number]["status"] }) {
  if (status === "completed") {
    return <CheckCircle2 size={14} />;
  }
  if (status === "inProgress") {
    return <LoaderCircle className="agent-spin" size={14} />;
  }
  return <Circle size={14} />;
}

function AgentMessageCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const streaming = item.lifecycle === "in_progress";
  const phase = item.phase ?? "unknown";
  const title = phase === "final_answer" ? "Codex 最终回复" : phase === "commentary" ? "Codex 过程说明" : "Codex 消息";

  return (
    <article className={`agent-timeline-card agent-message-card is-${normalizeCssToken(phase)} ${streaming ? "is-streaming" : ""}`}>
      <header>
        <Bot size={15} />
        <strong>{title}</strong>
        {streaming ? <span className="agent-streaming-label">正在更新</span> : null}
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      <p>{item.text || (streaming ? "Codex 正在组织说明…" : "Codex 已完成这条消息。")}</p>
    </article>
  );
}

function FileChangesCard({
  items,
  run,
  projectPath,
  occurredAt
}: {
  items: AgentItem[];
  run: AgentRun;
  projectPath: string;
  occurredAt: string;
}) {
  const [diffExpanded, setDiffExpanded] = useState(false);
  const changes = Object.values(run.filesByPath ?? {});
  const status = getAggregateFileChangeStatus(items, run);
  const currentDiff = run.aggregateDiff || buildFileChangeDiff(changes, projectPath);
  const fallbackStats = changes.reduce(
    (stats, change) => ({
      additions: stats.additions + change.additions,
      deletions: stats.deletions + change.deletions,
      files: stats.files + 1
    }),
    { additions: 0, deletions: 0, files: 0 }
  );
  const diffStats = run.aggregateDiff ? run.aggregateDiffStats : fallbackStats;

  return (
    <article className={`agent-timeline-card agent-work-card is-file-change is-${normalizeCssToken(status)}`}>
      <header>
        <FileCode2 size={15} />
        <strong>{getAggregateFileChangeStatusLabel(status)}</strong>
        <span>{changes.length ? `${changes.length} 个文件` : "等待补丁"}</span>
        <AgentEventTime occurredAt={occurredAt} />
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
      {(changes.length || currentDiff) ? (
        <div className="agent-diff-panel">
          <div className="agent-diff-summary">
            <GitCompare size={13} />
            <span>当前 diff · {diffStats.files || changes.length} 个文件</span>
            <span className="agent-diff-additions">+{diffStats.additions}</span>
            <span className="agent-diff-deletions">-{diffStats.deletions}</span>
          </div>
          {currentDiff ? (
            <>
              <button
                className="agent-expand-button"
                type="button"
                aria-expanded={diffExpanded}
                onClick={() => setDiffExpanded((expanded) => !expanded)}
              >
                {diffExpanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
                {diffExpanded ? "收起当前 diff" : "展开当前 diff"}
              </button>
              {diffExpanded ? <pre className="agent-diff-content">{currentDiff}</pre> : null}
            </>
          ) : null}
        </div>
      ) : null}
    </article>
  );
}

function CommandExecutionCard({
  item,
  nowMs,
  occurredAt
}: {
  item: AgentItem;
  nowMs: number;
  occurredAt: string;
}) {
  const [outputExpanded, setOutputExpanded] = useState(false);
  const command = item.command;
  const status = command?.status ?? item.status ?? (item.lifecycle === "completed" ? "completed" : "inProgress");
  const durationMs = getAgentItemDurationMs(
    item.startedAt,
    item.completedAt,
    command?.durationMs,
    nowMs
  );
  const fullOutput = command?.outputTail ?? item.output ?? "";
  const outputPreview = getAgentOutputPreview(fullOutput);
  const canExpandOutput = Boolean(
    fullOutput && (command?.outputTruncated || fullOutput.trimEnd() !== outputPreview)
  );
  const failed = status === "failed" || status === "declined";

  return (
    <article className={`agent-timeline-card agent-work-card is-command-execution is-${normalizeCssToken(status)}`}>
      <header>
        <Terminal size={15} />
        <strong>{getCommandStatusLabel(status)}</strong>
        {durationMs !== undefined ? <span>{formatDuration(durationMs)}</span> : null}
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      <code className="agent-command-text">{command?.command || "等待 Codex 提供命令…"}</code>
      {command?.cwd ? <small className="agent-command-cwd">cwd · {command.cwd}</small> : null}
      {outputPreview ? (
        <pre className="agent-command-output is-preview">
          {command?.outputTruncated ? "… 仅显示已保留输出的末尾预览\n" : ""}
          {outputPreview}
        </pre>
      ) : item.lifecycle === "in_progress" ? (
        <p className="agent-work-placeholder">命令正在执行，等待输出…</p>
      ) : null}
      {canExpandOutput ? (
        <>
          <button
            className="agent-expand-button"
            type="button"
            aria-expanded={outputExpanded}
            onClick={() => setOutputExpanded((expanded) => !expanded)}
          >
            {outputExpanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            {outputExpanded ? "收起命令输出" : command?.outputTruncated ? "展开保留的命令输出" : "展开完整命令输出"}
          </button>
          {outputExpanded ? (
            <pre className="agent-command-output is-full">
              {command?.outputTruncated ? "… 页面仅保留最近 12,000 个字符；原始输出可在运行 JSONL 中查看。\n" : ""}
              {fullOutput}
            </pre>
          ) : null}
        </>
      ) : null}
      <footer className="agent-work-footer">
        <div className="agent-work-metadata">
          {item.lifecycle === "completed" && typeof command?.exitCode === "number" ? <span>退出码 {command.exitCode}</span> : null}
          {item.lifecycle === "completed" && status === "declined" ? <span>命令未获批准</span> : null}
        </div>
        <div className="agent-copy-actions">
          {command?.command ? <CopyButton text={command.command} label="复制命令" /> : null}
          {failed ? <CopyButton text={getCommandErrorText(item)} label="复制错误" /> : null}
        </div>
      </footer>
    </article>
  );
}

function AgentWarningCard({ warning }: { warning: AgentWarning }) {
  return (
    <article className="agent-timeline-card agent-alert-card is-warning" role="status">
      <header>
        <AlertTriangle size={15} />
        <strong>非阻塞提醒</strong>
        <AgentEventTime occurredAt={warning.createdAt} />
      </header>
      <p>{warning.message}</p>
    </article>
  );
}

function AgentErrorCard({ error }: { error: AgentRunError }) {
  const terminalClass = error.terminal ? "is-terminal" : "is-retryable";
  return (
    <article className={`agent-timeline-card agent-alert-card is-error ${terminalClass}`} role={error.terminal ? "alert" : "status"}>
      <header>
        <XCircle size={15} />
        <strong>{error.terminal ? "运行失败" : error.retryable ? "发生错误，Codex 可重试" : "运行错误"}</strong>
        <AgentEventTime occurredAt={error.createdAt} />
      </header>
      <p>{error.message}</p>
    </article>
  );
}

function AgentEventCard({ event }: { event: AgentEvent }) {
  if (event.type === "agent_message") {
    return (
      <article className="agent-timeline-card agent-message-card is-commentary">
        <header>
          <Bot size={15} />
          <strong>Codex 过程说明</strong>
          <AgentEventTime occurredAt={event.createdAt} />
        </header>
        <p>{event.text}</p>
      </article>
    );
  }

  const diagnosticSeverity = event.type === "diagnostic"
    ? event.level === "error" ? " is-error" : event.level === "warning" ? " is-warning" : ""
    : "";
  return (
    <div className={`agent-progress-event is-${event.type}${diagnosticSeverity}`}>
      <AgentEventIcon event={event} />
      <p>{formatAgentEvent(event)}</p>
      <AgentEventTime occurredAt={event.createdAt} />
    </div>
  );
}

function AgentEventTime({ occurredAt }: { occurredAt: string }) {
  const timestamp = Date.parse(occurredAt);
  return Number.isFinite(timestamp) ? (
    <time dateTime={occurredAt}>{formatClockTime(timestamp)}</time>
  ) : null;
}

function CopyButton({ text, label }: { text: string; label: string }) {
  const [state, setState] = useState<"idle" | "copied" | "failed">("idle");
  const resetTimerRef = useRef<number>();

  useEffect(() => () => {
    if (resetTimerRef.current !== undefined) {
      window.clearTimeout(resetTimerRef.current);
    }
  }, []);

  const copy = async () => {
    try {
      await copyTextToClipboard(text);
      setState("copied");
    } catch {
      setState("failed");
    }
    if (resetTimerRef.current !== undefined) {
      window.clearTimeout(resetTimerRef.current);
    }
    resetTimerRef.current = window.setTimeout(() => setState("idle"), 1_600);
  };

  return (
    <button className={`agent-copy-button is-${state}`} type="button" onClick={() => void copy()}>
      {state === "copied" ? <Check size={12} /> : <Clipboard size={12} />}
      {state === "copied" ? "已复制" : state === "failed" ? "复制失败" : label}
    </button>
  );
}

function useAgentClock(enabled: boolean) {
  const [nowMs, setNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (!enabled) {
      setNowMs(Date.now());
      return;
    }
    setNowMs(Date.now());
    const interval = window.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => window.clearInterval(interval);
  }, [enabled]);

  return nowMs;
}

function useAgentTimelineAutoScroll(runId: string | undefined, progressVersion: string) {
  const ref = useRef<HTMLDivElement>(null);
  const [autoFollow, setAutoFollow] = useState(true);

  useEffect(() => {
    setAutoFollow(true);
  }, [runId]);

  useEffect(() => {
    if (!autoFollow) {
      return;
    }
    const frame = window.requestAnimationFrame(() => {
      const element = ref.current;
      if (element) {
        element.scrollTop = element.scrollHeight;
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [autoFollow, progressVersion, runId]);

  const onScroll = () => {
    const element = ref.current;
    if (!element) {
      return;
    }
    setAutoFollow(isAgentTimelineNearBottom(element.scrollHeight, element.scrollTop, element.clientHeight));
  };

  const scrollToLatest = () => {
    const element = ref.current;
    setAutoFollow(true);
    element?.scrollTo({ top: element.scrollHeight, behavior: "smooth" });
  };

  return { ref, autoFollow, onScroll, scrollToLatest };
}

function buildAgentTimelineEntries(run: AgentRun): AgentTimelineEntry[] {
  const entries: IndexedTimelineEntry[] = [];
  let sequence = 0;
  const push = (entry: AgentTimelineEntry) => {
    const parsedTime = Date.parse(entry.occurredAt);
    entries.push({
      ...entry,
      sequence,
      sortMs: Number.isFinite(parsedTime) ? parsedTime : sequence
    });
    sequence += 1;
  };

  const itemOrder = run.itemOrder ?? [];
  const orderedItems = itemOrder
    .map((itemId) => run.itemsById?.[itemId])
    .filter((item): item is AgentItem => Boolean(item));
  const fileItems: AgentItem[] = [];
  const representedItemIds = new Set<string>();

  for (const item of orderedItems) {
    if (item.type === "fileChange") {
      fileItems.push(item);
      representedItemIds.add(item.id);
      continue;
    }
    if (item.type === "commandExecution" || item.type === "agentMessage" || item.type === "plan") {
      representedItemIds.add(item.id);
      if (item.type === "plan" && run.currentPlan) {
        continue;
      }
      push({
        kind: "item",
        id: `item-${item.id}`,
        occurredAt: item.startedAt || item.updatedAt,
        item
      });
    }
  }

  if (fileItems.length || Object.keys(run.filesByPath ?? {}).length || run.aggregateDiff) {
    const firstFileItem = fileItems[0];
    push({
      kind: "files",
      id: "files",
      occurredAt: firstFileItem?.startedAt
        ?? firstFileItem?.updatedAt
        ?? run.aggregateDiffUpdatedAt
        ?? run.startedAt
        ?? new Date(0).toISOString(),
      items: fileItems
    });
  }

  if (run.currentPlan) {
    push({
      kind: "plan",
      id: `plan-${run.currentPlan.turnId}`,
      occurredAt: run.currentPlan.updatedAt,
      plan: run.currentPlan
    });
  }

  for (const [index, warning] of (run.warnings ?? []).entries()) {
    push({
      kind: "warning",
      id: `warning-${index}-${warning.createdAt}`,
      occurredAt: warning.createdAt,
      warning
    });
  }

  for (const [index, error] of (run.errors ?? []).entries()) {
    push({
      kind: "error",
      id: `error-${index}-${error.createdAt}`,
      occurredAt: error.createdAt,
      error
    });
  }

  for (const [index, event] of run.events.entries()) {
    if (
      event.type === "warning" ||
      event.type === "error" ||
      event.type === "plan_updated" ||
      event.type === "turn_diff_updated" ||
      event.type === "item_delta" ||
      (isAgentItemLifecycleEvent(event) && representedItemIds.has(event.itemId))
    ) {
      continue;
    }
    push({
      kind: "event",
      id: `event-${index}-${event.type}-${event.createdAt}`,
      occurredAt: event.createdAt,
      event
    });
  }

  const finalMessageRepresented = run.finalMessage && (
    run.events.some((event) => event.type === "turn_completed" && event.finalMessage === run.finalMessage) ||
    orderedItems.some((item) => item.type === "agentMessage" && item.text === run.finalMessage)
  );
  if (run.finalMessage && !finalMessageRepresented) {
    const occurredAt = run.completedAt ?? getAgentLatestProgressAt(run) ?? run.startedAt ?? new Date(0).toISOString();
    push({
      kind: "event",
      id: "run-final-message",
      occurredAt,
      event: {
        type: "turn_completed",
        threadId: run.codexThreadId,
        turnId: run.codexTurnId,
        status: run.status === "succeeded"
          ? "completed"
          : run.status === "cancelled"
            ? "interrupted"
            : run.status === "failed"
              ? "failed"
              : "inProgress",
        finalMessage: run.finalMessage,
        createdAt: occurredAt
      }
    });
  }

  if (run.error && !(run.errors ?? []).some((error) => error.message === run.error)) {
    const occurredAt = run.completedAt ?? getAgentLatestProgressAt(run) ?? run.startedAt ?? new Date(0).toISOString();
    push({
      kind: "error",
      id: "terminal-run-error",
      occurredAt,
      error: {
        message: run.error,
        retryable: false,
        terminal: true,
        createdAt: occurredAt
      }
    });
  }

  return entries
    .sort((left, right) => left.sortMs - right.sortMs || left.sequence - right.sequence)
    .map(({ sequence: _sequence, sortMs: _sortMs, ...entry }) => entry);
}

function getAgentProgressVersion(run: AgentRun, latestProgressAt: string | undefined) {
  let contentSize = run.aggregateDiff.length + run.events.length + run.itemOrder.length;
  for (const item of Object.values(run.itemsById ?? {})) {
    contentSize += (item.text?.length ?? 0) + (item.output?.length ?? 0) + (item.fileChanges?.length ?? 0);
  }
  return [
    run.id,
    run.status,
    latestProgressAt,
    contentSize,
    run.currentPlan?.steps.length ?? 0,
    run.warnings?.length ?? 0,
    run.errors?.length ?? 0
  ].join(":");
}

function isAgentItemLifecycleEvent(
  event: AgentEvent
): event is Extract<AgentEvent, { type: "item_started" | "item_completed" }> {
  return event.type === "item_started" || event.type === "item_completed";
}

function isTerminalAgentRun(run: AgentRun) {
  return run.status === "succeeded" || run.status === "failed" || run.status === "cancelled";
}

function getAggregateFileChangeStatus(items: AgentItem[], run: AgentRun) {
  if (items.some((item) => item.status === "failed")) {
    return "failed";
  }
  if (items.some((item) => item.status === "declined")) {
    return "declined";
  }
  if (items.some((item) => item.lifecycle === "in_progress")) {
    return "inProgress";
  }
  if (items.length || isTerminalAgentRun(run)) {
    return "completed";
  }
  return "inProgress";
}

function getAggregateFileChangeStatusLabel(status: string) {
  return {
    inProgress: "正在修改文件",
    completed: "文件修改完成",
    failed: "文件修改失败",
    declined: "文件修改被拒绝"
  }[status] ?? `文件状态：${status}`;
}

function buildFileChangeDiff(changes: AgentFileChange[], projectPath: string) {
  return changes
    .filter((change) => change.diff)
    .map((change) => `# ${formatFileChangePath(change, projectPath)}\n${change.diff}`)
    .join("\n");
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

function getCommandErrorText(item: AgentItem) {
  const command = item.command;
  return [
    command?.command ? `命令：${command.command}` : undefined,
    command?.cwd ? `cwd：${command.cwd}` : undefined,
    `状态：${command?.status ?? item.status ?? "failed"}`,
    typeof command?.exitCode === "number" ? `退出码：${command.exitCode}` : undefined,
    command?.outputTail || item.output
  ].filter((value): value is string => Boolean(value)).join("\n");
}

async function copyTextToClipboard(text: string) {
  if (isTauri()) {
    await writeClipboardText(text);
    return;
  }
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.style.position = "fixed";
  textarea.style.top = "0";
  textarea.style.left = "0";
  textarea.style.opacity = "0";
  textarea.setAttribute("readonly", "");
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();
  textarea.setSelectionRange(0, textarea.value.length);
  const copied = document.execCommand("copy");
  textarea.remove();
  if (!copied) {
    throw new Error("Clipboard unavailable");
  }
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
  if (event.type === "agent_message" || event.type === "plan_update") {
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

function getPlanStepStatusLabel(status: AgentPlan["steps"][number]["status"]) {
  return {
    pending: "待处理",
    inProgress: "进行中",
    completed: "已完成"
  }[status];
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

function formatLatestProgress(ageMs: number | undefined) {
  if (ageMs === undefined) {
    return "等待首个真实进展";
  }
  if (ageMs < 1_000) {
    return "最后进展于刚刚";
  }
  if (ageMs < 60_000) {
    return `最后进展于 ${Math.floor(ageMs / 1_000)} 秒前`;
  }
  const minutes = Math.floor(ageMs / 60_000);
  const seconds = Math.floor((ageMs % 60_000) / 1_000);
  return `最后进展于 ${minutes} 分 ${seconds} 秒前`;
}

function formatClockTime(timestamp: number) {
  return new Intl.DateTimeFormat("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false
  }).format(timestamp);
}

function normalizeCssToken(value: string) {
  return value.replace(/([a-z])([A-Z])/g, "$1-$2").toLowerCase();
}
