import {
  AlertTriangle,
  Bot,
  BrainCircuit,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronUp,
  Circle,
  Clipboard,
  Clock,
  FileCode2,
  GitCompare,
  ImageIcon,
  Info,
  Layers3,
  ListChecks,
  LoaderCircle,
  Plug,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Terminal,
  Timer,
  Users,
  Workflow,
  Wrench,
  XCircle
} from "lucide-react";
import { isTauri } from "@tauri-apps/api/core";
import { writeText as writeClipboardText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AgentEvent,
  AgentFileChange,
  AgentHookRun,
  AgentItem,
  AgentPlan,
  AgentRun,
  AgentRunError,
  AgentStructuredPreview,
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

const AGENT_HOOK_DETAILS_LIMIT = 12_000;

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
      kind: "hook";
      id: string;
      occurredAt: string;
      hook: AgentHookRun;
    }
  | {
      kind: "hook_group";
      id: string;
      occurredAt: string;
      hooks: AgentHookRun[];
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
  const latestProgressAt = useMemo(
    () => latestRun ? getAgentLatestProgressAt(latestRun) : undefined,
    [latestRun]
  );
  const progressVersion = useMemo(
    () => latestRun ? getAgentProgressVersion(latestRun, latestProgressAt) : "idle",
    [latestRun, latestProgressAt]
  );
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
        {latestRun.tokenUsage ? (
          <span className="agent-token-usage">
            Token 本轮 {formatCompactNumber(latestRun.tokenUsage.last.totalTokens)}
            <span> · 累计 {formatCompactNumber(latestRun.tokenUsage.total.totalTokens)}</span>
            {latestRun.tokenUsage.modelContextWindow
              ? <span> · 窗口 {formatCompactNumber(latestRun.tokenUsage.modelContextWindow)}</span>
              : null}
          </span>
        ) : null}
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
  if (entry.kind === "hook") {
    return <AgentHookCard hook={entry.hook} />;
  }
  if (entry.kind === "hook_group") {
    return <AgentHookGroupCard hooks={entry.hooks} />;
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
  if (entry.item.type === "agentMessage" || entry.item.type === "plan") {
    return <AgentMessageCard item={entry.item} occurredAt={entry.occurredAt} />;
  }
  if (entry.item.presentation?.kind === "reasoning") {
    return <AgentReasoningCard item={entry.item} occurredAt={entry.occurredAt} />;
  }
  if (entry.item.presentation?.kind === "toolCall") {
    return <AgentToolCallCard item={entry.item} occurredAt={entry.occurredAt} />;
  }
  if (entry.item.presentation?.kind === "collaboration") {
    return <AgentCollaborationCard item={entry.item} occurredAt={entry.occurredAt} />;
  }
  if (entry.item.presentation?.kind === "webSearch") {
    return <AgentWebSearchCard item={entry.item} occurredAt={entry.occurredAt} />;
  }
  if (entry.item.presentation?.kind === "image") {
    return <AgentImageCard item={entry.item} occurredAt={entry.occurredAt} />;
  }
  return <AgentStatusItemCard item={entry.item} occurredAt={entry.occurredAt} />;
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

function AgentReasoningCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const presentation = item.presentation;
  if (!presentation || presentation.kind !== "reasoning") {
    return null;
  }
  const active = item.lifecycle === "in_progress";
  return (
    <article className={`agent-timeline-card agent-reasoning-card ${active ? "is-active" : "is-completed"}`}>
      <header>
        <BrainCircuit size={15} />
        <strong>{active ? "正在分析" : "分析完成"}</strong>
        {active ? <span className="agent-streaming-label">正在更新</span> : null}
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {presentation.summary ? (
        <AgentExpandableDetails label="查看分析摘要" content={presentation.summary} />
      ) : (
        <p className="agent-work-placeholder">Codex 正在整理分析摘要…</p>
      )}
      {presentation.rawTextAvailable ? (
        <small className="agent-restricted-debug-note">原始分析仅保留在受限协议日志，页面不加载正文</small>
      ) : null}
    </article>
  );
}

function AgentToolCallCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const presentation = item.presentation;
  if (!presentation || presentation.kind !== "toolCall") {
    return null;
  }
  const failed = presentation.status === "failed" || presentation.success === false || Boolean(presentation.error);
  const target = presentation.toolKind === "mcp"
    ? [presentation.server, presentation.tool].filter(Boolean).join(" / ")
    : [presentation.namespace, presentation.tool].filter(Boolean).join(" / ");
  const details = formatToolCallDetails(presentation.arguments, presentation.result);

  return (
    <article className={`agent-timeline-card agent-tool-card is-${presentation.toolKind} ${failed ? "is-failed" : ""}`}>
      <header>
        {presentation.toolKind === "mcp" ? <Plug size={15} /> : <Wrench size={15} />}
        <strong>{presentation.toolKind === "mcp" ? "MCP 工具" : "动态工具"} · {target || "unknown"}</strong>
        <span>{getActivityStatusLabel(presentation.status, item.lifecycle)}</span>
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {presentation.progress ? <p className="agent-tool-progress">{presentation.progress}</p> : null}
      {presentation.error ? <p className="agent-tool-error">{presentation.error}</p> : null}
      {presentation.durationMs !== undefined ? <small>耗时 {formatDuration(presentation.durationMs)}</small> : null}
      {details ? (
        <AgentExpandableDetails
          label="查看参数与结果"
          content={details}
          truncated={Boolean(presentation.arguments?.truncated || presentation.result?.truncated)}
        />
      ) : null}
    </article>
  );
}

function AgentCollaborationCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const presentation = item.presentation;
  if (!presentation || presentation.kind !== "collaboration") {
    return null;
  }
  const title = presentation.activityKind === "subAgent"
    ? getSubAgentActivityLabel(presentation.status)
    : getCollabToolLabel(presentation.tool);
  const targetCount = presentation.receiverThreadIds.length || presentation.agentStates.length;

  return (
    <article className={`agent-timeline-card agent-collaboration-card is-${normalizeCssToken(presentation.status)}`}>
      <header>
        <Users size={15} />
        <strong>{title}</strong>
        {targetCount ? <span>{targetCount} 个子 Agent</span> : null}
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {presentation.agentPath ? <code className="agent-inline-path">{presentation.agentPath}</code> : null}
      {presentation.agentStates.length ? (
        <ul className="agent-state-list">
          {presentation.agentStates.map((state) => (
            <li key={state.threadId}>
              <code>{compactAgentId(state.threadId)}</code>
              <span>{getActivityStatusLabel(state.status)}</span>
              {state.message ? <small>{state.message}</small> : null}
            </li>
          ))}
        </ul>
      ) : null}
      {presentation.prompt ? (
        <AgentExpandableDetails
          label="查看协作提示"
          content={presentation.prompt.text}
          truncated={presentation.prompt.truncated}
        />
      ) : null}
    </article>
  );
}

function AgentWebSearchCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const presentation = item.presentation;
  if (!presentation || presentation.kind !== "webSearch") {
    return null;
  }
  const detail = presentation.query ?? presentation.url ?? presentation.pattern;
  return (
    <article className="agent-timeline-card agent-web-search-card">
      <header>
        <Search size={15} />
        <strong>{getWebSearchActionLabel(presentation.action)}</strong>
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {detail ? <code className="agent-inline-path">{detail}</code> : null}
    </article>
  );
}

function AgentImageCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const presentation = item.presentation;
  if (!presentation || presentation.kind !== "image") {
    return null;
  }
  const path = presentation.savedPath ?? presentation.path;
  return (
    <article className={`agent-timeline-card agent-image-card is-${normalizeCssToken(presentation.status)}`}>
      <header>
        {presentation.activityKind === "generation" ? <Sparkles size={15} /> : <ImageIcon size={15} />}
        <strong>{presentation.activityKind === "generation" ? "图像生成" : "查看图像"}</strong>
        <span>{getActivityStatusLabel(presentation.status, item.lifecycle)}</span>
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {path ? <code className="agent-inline-path">{path}</code> : null}
      {presentation.revisedPrompt ? (
        <AgentExpandableDetails
          label="查看修订提示"
          content={presentation.revisedPrompt.text}
          truncated={presentation.revisedPrompt.truncated}
        />
      ) : null}
      {!path && presentation.resultAvailable ? <p className="agent-work-placeholder">图像结果已生成</p> : null}
    </article>
  );
}

function AgentStatusItemCard({ item, occurredAt }: { item: AgentItem; occurredAt: string }) {
  const presentation = item.presentation;
  const activityKind = presentation?.kind === "status" ? presentation.activityKind : "generic";
  const status = presentation?.kind === "status" ? presentation.status : item.status ?? "unknown";
  const label = getStatusItemLabel(item.type, item.lifecycle, presentation?.kind === "status" ? presentation.durationMs : undefined);
  const Icon = activityKind === "contextCompaction"
    ? Layers3
    : activityKind === "sleep"
      ? Timer
      : activityKind === "reviewMode"
        ? ShieldCheck
        : activityKind === "hookPrompt"
          ? Workflow
          : Info;

  return (
    <article className={`agent-timeline-card agent-status-card is-${normalizeCssToken(status)}`}>
      <header>
        <Icon size={15} />
        <strong>{label}</strong>
        <AgentEventTime occurredAt={occurredAt} />
      </header>
      {presentation?.kind === "status" && presentation.details ? (
        <AgentExpandableDetails
          label="查看协议详情"
          content={presentation.details.text}
          truncated={presentation.details.truncated}
        />
      ) : null}
    </article>
  );
}

function AgentHookCard({ hook }: { hook: AgentHookRun }) {
  const failed = hook.status === "failed" || hook.status === "blocked" || hook.status === "stopped";
  const hookDetails = buildBoundedHookDetails([hook]);

  return (
    <article className={`agent-timeline-card agent-hook-card is-${normalizeCssToken(hook.status)} ${failed ? "is-failed" : ""}`}>
      <header>
        <Workflow size={15} />
        <strong>Hook · {getHookEventLabel(hook.eventName)}</strong>
        <span>{getActivityStatusLabel(hook.status, hook.lifecycle)}</span>
        <AgentEventTime occurredAt={hook.updatedAt} />
      </header>
      {hook.statusMessage ? <p>{hook.statusMessage}</p> : null}
      {hook.durationMs !== undefined ? <small>耗时 {formatDuration(hook.durationMs)}</small> : null}
      {hookDetails.text ? (
        <AgentExpandableDetails
          label="查看 Hook 详情"
          content={hookDetails.text}
          truncated={Boolean(hook.restrictedDebugAvailable || hookDetails.truncated)}
        />
      ) : hook.restrictedDebugAvailable ? (
        <small className="agent-detail-truncated">页面仅保留安全摘要，完整原始内容见受限协议日志</small>
      ) : null}
    </article>
  );
}

function AgentHookGroupCard({ hooks }: { hooks: AgentHookRun[] }) {
  const active = hooks.some((hook) => hook.lifecycle === "in_progress" || hook.status === "running");
  const latestHook = hooks.reduce((latest, hook) =>
    Date.parse(hook.updatedAt) > Date.parse(latest.updatedAt) ? hook : latest
  );
  const eventCounts = new Map<string, number>();
  for (const hook of hooks) {
    const label = getHookEventLabel(hook.eventName);
    eventCounts.set(label, (eventCounts.get(label) ?? 0) + 1);
  }
  const summary = [...eventCounts.entries()]
    .slice(0, 3)
    .map(([label, count]) => `${label}${count > 1 ? ` × ${count}` : ""}`)
    .join(" · ");
  const hookDetails = buildBoundedHookDetails(hooks);
  const truncated = hookDetails.truncated || hooks.some((hook) => hook.restrictedDebugAvailable);

  return (
    <article className={`agent-timeline-card agent-hook-card agent-hook-group ${active ? "is-active" : "is-completed"}`}>
      <header>
        <Workflow size={15} />
        <strong>Hooks · {hooks.length} 次</strong>
        <span>{active ? "运行中" : "已完成"}</span>
        <AgentEventTime occurredAt={latestHook.updatedAt} />
      </header>
      {summary ? <p>{summary}</p> : null}
      {hookDetails.text ? (
        <AgentExpandableDetails
          label="查看 Hook 详情"
          content={hookDetails.text}
          truncated={truncated}
        />
      ) : truncated ? (
        <small className="agent-detail-truncated">页面仅保留安全摘要，完整原始内容见受限协议日志</small>
      ) : null}
    </article>
  );
}

function buildBoundedHookDetails(hooks: AgentHookRun[]) {
  let text = "";
  let truncated = false;
  for (const hook of hooks) {
    const lines = [
      `${getHookEventLabel(hook.eventName)} · ${getActivityStatusLabel(hook.status, hook.lifecycle)}`,
      hook.statusMessage,
      hook.sourcePath ? `来源：${hook.sourcePath}` : undefined,
      ...hook.entries.map((entry) => `[${entry.kind}] ${entry.text}`)
    ].filter((value): value is string => Boolean(value));
    const block = lines.join("\n");
    const separator = text ? "\n\n" : "";
    const remaining = AGENT_HOOK_DETAILS_LIMIT - text.length - separator.length;
    if (remaining <= 0) {
      truncated = true;
      break;
    }
    if (block.length > remaining) {
      text = `${text}${separator}${block.slice(0, Math.max(0, remaining - 1))}…`;
      truncated = true;
      break;
    }
    text = `${text}${separator}${block}`;
  }
  return { text, truncated };
}

function AgentExpandableDetails({
  label,
  content,
  truncated = false
}: {
  label: string;
  content: string;
  truncated?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="agent-detail-panel">
      <button
        className="agent-expand-button"
        type="button"
        aria-expanded={expanded}
        onClick={() => setExpanded((value) => !value)}
      >
        {expanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
        {expanded ? "收起详情" : label}
      </button>
      {truncated ? (
        <small className="agent-detail-truncated">页面仅保留安全摘要，完整原始内容见受限协议日志</small>
      ) : null}
      {expanded ? <pre className="agent-detail-content">{content}</pre> : null}
    </div>
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
  const source = warning.source ?? "runtime";
  const title = source === "guardian"
    ? "安全审查提醒"
    : source === "config"
      ? "配置提醒"
      : "非阻塞提醒";
  const location = warning.path
    ? `${warning.path}${warning.range ? `:${warning.range.start.line}:${warning.range.start.column}` : ""}`
    : undefined;
  return (
    <article className={`agent-timeline-card agent-alert-card is-warning is-${source}`} role="status">
      <header>
        {source === "guardian" ? <ShieldAlert size={15} /> : <AlertTriangle size={15} />}
        <strong>{title}</strong>
        {(warning.count ?? 1) > 1 ? <span>重复 {warning.count} 次</span> : null}
        <AgentEventTime occurredAt={warning.updatedAt ?? warning.createdAt} />
      </header>
      <p>{warning.message}</p>
      {warning.details ? <p className="agent-alert-details">{warning.details}</p> : null}
      {location ? <code className="agent-inline-path">{location}</code> : null}
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

  if (event.type === "context_compacted") {
    return (
      <article className="agent-timeline-card agent-status-card is-completed">
        <header>
          <Layers3 size={15} />
          <strong>长对话上下文已整理</strong>
          <AgentEventTime occurredAt={event.createdAt} />
        </header>
      </article>
    );
  }

  if (event.type === "model_rerouted") {
    return (
      <article className="agent-timeline-card agent-model-card is-warning">
        <header>
          <RefreshCw size={15} />
          <strong>模型已切换</strong>
          <AgentEventTime occurredAt={event.createdAt} />
        </header>
        <p><code>{event.fromModel}</code> → <code>{event.toModel}</code></p>
        <small>{getModelRerouteReasonLabel(event.reason)}</small>
      </article>
    );
  }

  if (event.type === "model_safety_buffering_updated") {
    return (
      <article className={`agent-timeline-card agent-model-card ${event.showBufferingUi ? "is-warning" : "is-completed"}`}>
        <header>
          <ShieldAlert size={15} />
          <strong>{event.showBufferingUi ? "安全缓冲处理中" : "安全缓冲已结束"}</strong>
          <AgentEventTime occurredAt={event.createdAt} />
        </header>
        {event.reasons.length ? <p>{event.reasons.join(" · ")}</p> : null}
        {event.fasterModel ? <small>可切换到 {event.fasterModel}</small> : null}
      </article>
    );
  }

  if (event.type === "model_verification_updated") {
    return (
      <article className="agent-timeline-card agent-model-card is-verification">
        <header>
          <ShieldCheck size={15} />
          <strong>{event.verifications.length ? "模型验证已启用" : "模型验证已清除"}</strong>
          <AgentEventTime occurredAt={event.createdAt} />
        </header>
        {event.verifications.length ? <p>{event.verifications.map(getModelVerificationLabel).join(" · ")}</p> : null}
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
    representedItemIds.add(item.id);
    if (item.type === "fileChange") {
      fileItems.push(item);
      continue;
    }
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

  const routineHooks: AgentHookRun[] = [];
  for (const hookId of run.hookOrder ?? []) {
    const hook = run.hooksById?.[hookId];
    if (!hook) {
      continue;
    }
    if (hook.status === "failed" || hook.status === "blocked" || hook.status === "stopped") {
      push({
        kind: "hook",
        id: `hook-${hook.id}`,
        occurredAt: hook.startedAt,
        hook
      });
    } else {
      routineHooks.push(hook);
    }
  }

  if (routineHooks.length === 1) {
    const [hook] = routineHooks;
    push({
      kind: "hook",
      id: `hook-${hook.id}`,
      occurredAt: hook.startedAt,
      hook
    });
  } else if (routineHooks.length > 1) {
    push({
      kind: "hook_group",
      id: "hooks-routine",
      occurredAt: routineHooks[0].startedAt,
      hooks: routineHooks
    });
  }

  if (run.modelSafetyBuffering) {
    push({
      kind: "event",
      id: "model-safety-buffering",
      occurredAt: run.modelSafetyBuffering.createdAt,
      event: {
        type: "model_safety_buffering_updated",
        ...run.modelSafetyBuffering
      }
    });
  }

  if (run.modelVerification) {
    push({
      kind: "event",
      id: "model-verification",
      occurredAt: run.modelVerification.createdAt,
      event: {
        type: "model_verification_updated",
        ...run.modelVerification
      }
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
      (event.type === "context_compacted" && orderedItems.some((item) => item.type === "contextCompaction")) ||
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
    contentSize += (item.text?.length ?? 0)
      + (item.output?.length ?? 0)
      + (item.fileChanges?.length ?? 0)
      + (item.reasoningSummary?.length ?? 0)
      + (item.progressMessage?.length ?? 0)
      + (item.presentation ? JSON.stringify(item.presentation).length : 0);
  }
  for (const hook of Object.values(run.hooksById ?? {})) {
    contentSize += hook.entries.reduce((size, entry) => size + entry.text.length, 0);
  }
  return [
    run.id,
    run.status,
    latestProgressAt,
    contentSize,
    run.currentPlan?.steps.length ?? 0,
    run.hookOrder?.length ?? 0,
    run.modelSafetyBuffering?.createdAt ?? "",
    run.modelVerification?.createdAt ?? "",
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

function formatToolCallDetails(
  argumentsPreview: AgentStructuredPreview | undefined,
  resultPreview: AgentStructuredPreview | undefined
) {
  return [
    argumentsPreview ? `参数\n${argumentsPreview.text}` : undefined,
    resultPreview ? `结果\n${resultPreview.text}` : undefined
  ].filter((value): value is string => Boolean(value)).join("\n\n");
}

function getActivityStatusLabel(status: string, lifecycle?: AgentItem["lifecycle"]) {
  const labels: Record<string, string> = {
    inProgress: "进行中",
    running: "进行中",
    pendingInit: "正在启动",
    completed: "已完成",
    failed: "失败",
    errored: "出错",
    interrupted: "已中断",
    blocked: "已阻止",
    stopped: "已停止",
    shutdown: "已关闭",
    notFound: "未找到",
    started: "已启动",
    interacted: "正在交互"
  };
  return labels[status] ?? (lifecycle === "in_progress" ? "进行中" : status);
}

function getCollabToolLabel(tool: string | undefined) {
  return {
    spawnAgent: "正在创建子 Agent",
    sendInput: "正在向子 Agent 发送消息",
    resumeAgent: "正在恢复子 Agent",
    wait: "正在等待子 Agent",
    closeAgent: "正在关闭子 Agent"
  }[tool ?? ""] ?? "子 Agent 协作";
}

function getSubAgentActivityLabel(kind: string) {
  return {
    started: "子 Agent 已启动",
    interacted: "子 Agent 正在交互",
    interrupted: "子 Agent 已中断"
  }[kind] ?? "子 Agent 活动";
}

function compactAgentId(value: string) {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-5)}` : value;
}

function getWebSearchActionLabel(action: string) {
  return {
    search: "正在搜索网页",
    openPage: "正在打开网页",
    open_page: "正在打开网页",
    findInPage: "正在页内查找",
    find_in_page: "正在页内查找",
    other: "网页操作"
  }[action] ?? "网页操作";
}

function getStatusItemLabel(type: string, lifecycle: AgentItem["lifecycle"], durationMs?: number) {
  if (type === "contextCompaction") {
    return lifecycle === "completed" ? "长对话上下文整理完成" : "正在整理长对话上下文";
  }
  if (type === "sleep") {
    return durationMs !== undefined ? `等待 ${formatDuration(durationMs)}` : "正在等待";
  }
  if (type === "enteredReviewMode") {
    return "已进入审查模式";
  }
  if (type === "exitedReviewMode") {
    return "已退出审查模式";
  }
  if (type === "userMessage") {
    return "用户输入已提交";
  }
  if (type === "hookPrompt") {
    return "Hook 已补充上下文";
  }
  return `${getAgentItemTypeLabel(type)} · ${lifecycle === "completed" ? "已完成" : "进行中"}`;
}

function getAgentItemTypeLabel(type: string) {
  return {
    reasoning: "分析",
    mcpToolCall: "MCP 工具",
    dynamicToolCall: "动态工具",
    collabAgentToolCall: "子 Agent 协作",
    subAgentActivity: "子 Agent 活动",
    webSearch: "网页搜索",
    imageView: "查看图像",
    imageGeneration: "图像生成"
  }[type] ?? `协议活动 ${type}`;
}

function getHookEventLabel(eventName: string) {
  return {
    preToolUse: "工具调用前",
    permissionRequest: "权限请求",
    postToolUse: "工具调用后",
    preCompact: "上下文整理前",
    postCompact: "上下文整理后",
    sessionStart: "会话开始",
    userPromptSubmit: "用户输入提交",
    subagentStart: "子 Agent 启动",
    subagentStop: "子 Agent 停止",
    stop: "停止"
  }[eventName] ?? eventName;
}

function getModelRerouteReasonLabel(reason: string) {
  return reason === "highRiskCyberActivity" ? "因高风险网络安全活动切换模型" : `切换原因：${reason}`;
}

function getModelVerificationLabel(verification: string) {
  return verification === "trustedAccessForCyber" ? "可信网络安全访问" : verification;
}

function formatCompactNumber(value: number) {
  return new Intl.NumberFormat("zh-CN", {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1
  }).format(value);
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
  if (event.type === "warning" || event.type === "error" || event.type === "guardian_warning") {
    return event.message;
  }
  if (event.type === "config_warning") {
    return compactText([event.summary, event.details].filter(Boolean).join(" · "));
  }
  if (event.type === "hook_run_updated") {
    return `Hook ${event.hookId} · ${event.lifecycle}`;
  }
  if (event.type === "context_compacted") {
    return "长对话上下文已整理";
  }
  if (event.type === "token_usage_updated") {
    return `Token 使用已更新 · ${event.tokenUsage.last.totalTokens}`;
  }
  if (event.type === "model_rerouted") {
    return `模型已从 ${event.fromModel} 切换到 ${event.toModel}`;
  }
  if (event.type === "model_safety_buffering_updated") {
    return event.showBufferingUi ? "安全缓冲处理中" : "安全缓冲已结束";
  }
  return event.verifications.length ? "模型验证已启用" : "模型验证已清除";
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
