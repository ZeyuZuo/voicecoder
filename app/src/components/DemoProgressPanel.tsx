import { Bot, FileCode2, ListChecks, ShieldCheck, Terminal, XCircle } from "lucide-react";
import type { AgentEvent, AgentRun, DemoSession } from "../types/app";

type DemoProgressPanelProps = {
  session: DemoSession;
  compact?: boolean;
};

export function DemoProgressPanel({ session, compact }: DemoProgressPanelProps) {
  const latestRun = session.runs[session.runs.length - 1];

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

  const visibleEvents = summarizeAgentEvents(latestRun.events).slice(-5);
  const changedCount = latestRun.changedFiles.length;

  return (
    <section className={`agent-progress-panel is-${latestRun.status} ${compact ? "is-compact" : ""}`} aria-live="polite">
      <div className="agent-progress-header">
        <div>
          <span>{getAgentRunKindLabel(latestRun.kind)}</span>
          <strong>{getAgentRunStatusLabel(latestRun.status)}</strong>
        </div>
        {changedCount ? <small>{changedCount} 个文件变更</small> : null}
      </div>

      <div className="agent-progress-events">
        {visibleEvents.length ? (
          visibleEvents.map((event, index) => (
            <div className={`agent-progress-event is-${event.type}`} key={`${event.type}-${event.createdAt}-${index}`}>
              <AgentEventIcon event={event} />
              <p>{formatAgentEvent(event)}</p>
            </div>
          ))
        ) : (
          <div className="agent-progress-event is-waiting">
            <Bot size={15} />
            <p>正在启动 Codex thread</p>
          </div>
        )}
      </div>
    </section>
  );
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

  if (event.type === "file_change") {
    return <FileCode2 size={15} />;
  }

  if (event.type === "plan_update") {
    return <ListChecks size={15} />;
  }

  if (event.type === "approval_review") {
    return <ShieldCheck size={15} />;
  }

  if (event.type === "error") {
    return <XCircle size={15} />;
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
    return compactText(event.finalMessage ?? "本轮已完成");
  }

  if (event.type === "diagnostic") {
    return compactText(event.method ? `${event.message} · ${event.method}` : event.message);
  }

  return event.message;
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
