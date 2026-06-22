import { useCallback, useEffect, useMemo, useReducer } from "react";
import type {
  AgentEvent,
  AgentRun,
  AgentRunKind,
  DemoFeedbackTurn,
  DemoSession,
  RequirementState,
  RequirementUtterance
} from "../types/app";
import { createId } from "./project";

export type CreateDemoSessionInput = {
  projectPath: string;
  requirementId: string;
  initialRequirementDocument: string;
  initialCodingPrompt: string;
  currentPreviewUrl?: string;
  now: string;
};

export type DemoSessionAction =
  | {
      type: "start_agent_run";
      kind: AgentRunKind;
      prompt: string;
      now: string;
      feedbackTurnId?: string;
    }
  | {
      type: "append_agent_event";
      runId: string;
      event: AgentEvent;
      now: string;
    }
  | {
      type: "complete_agent_run";
      runId: string;
      now: string;
      finalMessage?: string;
      changedFiles?: string[];
      currentPreviewUrl?: string;
    }
  | {
      type: "fail_agent_run";
      runId: string;
      error: string;
      now: string;
    }
  | {
      type: "cancel_agent_run";
      runId: string;
      now: string;
    }
  | {
      type: "start_feedback_listening";
      now: string;
    }
  | {
      type: "apply_feedback_result";
      utterances: RequirementUtterance[];
      summary: string;
      modificationPrompt: string;
      now: string;
    };

export type DemoSessionStoreAction =
  | {
      type: "create_demo_session";
      input: CreateDemoSessionInput;
    }
  | DemoSessionAction;

export type DemoSessionController = {
  session?: DemoSession;
  active: boolean;
  canStartInitialRun: boolean;
  startInitialRun: () => void;
};

export function createDemoSession(input: CreateDemoSessionInput): DemoSession {
  return {
    id: createId("demo_session"),
    projectPath: input.projectPath,
    requirementId: input.requirementId,
    initialRequirementDocument: input.initialRequirementDocument,
    initialCodingPrompt: input.initialCodingPrompt,
    runs: [],
    feedbackTurns: [],
    currentPreviewUrl: input.currentPreviewUrl,
    status: "ready_to_start",
    createdAt: input.now,
    updatedAt: input.now
  };
}

export function demoSessionStoreReducer(
  session: DemoSession | undefined,
  action: DemoSessionStoreAction
): DemoSession | undefined {
  if (action.type === "create_demo_session") {
    if (
      session &&
      session.projectPath === action.input.projectPath &&
      session.requirementId === action.input.requirementId
    ) {
      return session;
    }

    return createDemoSession(action.input);
  }

  if (!session) {
    return undefined;
  }

  return demoSessionReducer(session, action);
}

export function demoSessionReducer(session: DemoSession, action: DemoSessionAction): DemoSession {
  if (action.type === "start_agent_run") {
    if (!canStartAgentRun(session, action.kind)) {
      return session;
    }

    const run: AgentRun = {
      id: createId("agent_run"),
      kind: action.kind,
      prompt: action.prompt,
      status: "running",
      codexThreadId: session.codexThreadId,
      events: [],
      changedFiles: [],
      startedAt: action.now
    };

    return {
      ...session,
      runs: [...session.runs, run],
      feedbackTurns: linkFeedbackTurnToRun(session.feedbackTurns, action.feedbackTurnId, run.id),
      status: action.kind === "initial_build" ? "agent_running" : "agent_modifying",
      error: undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "append_agent_event") {
    const run = findRun(session, action.runId);
    if (!run || isTerminalRunStatus(run.status)) {
      return session;
    }

    const changedFile = action.event.type === "file_change" ? action.event.path : undefined;
    const nextCodexThreadId = action.event.type === "thread_started" ? action.event.threadId : session.codexThreadId;
    const nextCodexTurnId = action.event.type === "turn_started" ? action.event.turnId : run.codexTurnId;

    return {
      ...session,
      codexThreadId: nextCodexThreadId,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? {
              ...candidate,
              codexThreadId: nextCodexThreadId ?? candidate.codexThreadId,
              codexTurnId: nextCodexTurnId,
              events: [...candidate.events, action.event],
              changedFiles: changedFile ? appendUnique(candidate.changedFiles, changedFile) : candidate.changedFiles
            }
          : candidate
      ),
      updatedAt: action.now
    };
  }

  if (action.type === "complete_agent_run") {
    const run = findRun(session, action.runId);
    if (!run || isTerminalRunStatus(run.status)) {
      return session;
    }

    return {
      ...session,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? {
              ...candidate,
              status: "succeeded",
              finalMessage: action.finalMessage ?? candidate.finalMessage,
              changedFiles: mergeUnique(candidate.changedFiles, action.changedFiles ?? []),
              completedAt: action.now
            }
          : candidate
      ),
      currentPreviewUrl: action.currentPreviewUrl ?? session.currentPreviewUrl,
      status: "preview_ready",
      error: undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "fail_agent_run") {
    const run = findRun(session, action.runId);
    if (!run || isTerminalRunStatus(run.status)) {
      return session;
    }

    return {
      ...session,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? {
              ...candidate,
              status: "failed",
              error: action.error,
              completedAt: action.now
            }
          : candidate
      ),
      status: "error",
      error: action.error,
      updatedAt: action.now
    };
  }

  if (action.type === "cancel_agent_run") {
    const run = findRun(session, action.runId);
    if (!run || isTerminalRunStatus(run.status)) {
      return session;
    }

    return {
      ...session,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? {
              ...candidate,
              status: "cancelled",
              completedAt: action.now
            }
          : candidate
      ),
      status: "preview_ready",
      updatedAt: action.now
    };
  }

  if (action.type === "start_feedback_listening") {
    if (session.status !== "preview_ready") {
      return session;
    }

    return {
      ...session,
      status: "feedback_listening",
      error: undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "apply_feedback_result") {
    if (session.status !== "feedback_listening" || !action.modificationPrompt.trim()) {
      return session;
    }

    const feedbackTurn: DemoFeedbackTurn = {
      id: createId("demo_feedback"),
      utterances: action.utterances,
      summary: action.summary,
      modificationPrompt: action.modificationPrompt,
      createdAt: action.now
    };

    return {
      ...session,
      feedbackTurns: [...session.feedbackTurns, feedbackTurn],
      status: "feedback_processing",
      error: undefined,
      updatedAt: action.now
    };
  }

  return session;
}

export function useDemoSession(requirementState: RequirementState | undefined, projectPath: string | undefined): DemoSessionController {
  const [session, dispatch] = useReducer(demoSessionStoreReducer, undefined);

  useEffect(() => {
    if (
      !projectPath ||
      requirementState?.status !== "confirmed" ||
      !requirementState.requirementDocument.trim()
    ) {
      return;
    }

    dispatch({
      type: "create_demo_session",
      input: {
        projectPath,
        requirementId: requirementState.id,
        initialRequirementDocument: requirementState.requirementDocument,
        initialCodingPrompt: requirementState.codingPrompt || createInitialCodingPrompt(requirementState),
        now: nowString()
      }
    });
  }, [projectPath, requirementState]);

  const startInitialRun = useCallback(() => {
    if (!session || session.status !== "ready_to_start") {
      return;
    }

    dispatch({
      type: "start_agent_run",
      kind: "initial_build",
      prompt: createInitialDemoPrompt(session),
      now: nowString()
    });
  }, [session]);

  return useMemo(
    () => ({
      session,
      active: Boolean(session),
      canStartInitialRun: session?.status === "ready_to_start",
      startInitialRun
    }),
    [session, startInitialRun]
  );
}

function canStartAgentRun(session: DemoSession, kind: AgentRunKind) {
  if (hasActiveRun(session)) {
    return false;
  }

  if (kind === "initial_build") {
    return session.status === "ready_to_start" && session.runs.length === 0;
  }

  return session.status === "feedback_processing" && session.feedbackTurns.some((turn) => !turn.linkedAgentRunId);
}

function hasActiveRun(session: DemoSession) {
  return session.runs.some((run) => !isTerminalRunStatus(run.status));
}

function isTerminalRunStatus(status: AgentRun["status"]) {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}

function findRun(session: DemoSession, runId: string) {
  return session.runs.find((run) => run.id === runId);
}

function linkFeedbackTurnToRun(feedbackTurns: DemoFeedbackTurn[], feedbackTurnId: string | undefined, runId: string) {
  if (!feedbackTurnId) {
    return feedbackTurns;
  }

  return feedbackTurns.map((turn) =>
    turn.id === feedbackTurnId && !turn.linkedAgentRunId
      ? {
          ...turn,
          linkedAgentRunId: runId
        }
      : turn
  );
}

function mergeUnique(left: string[], right: string[]) {
  return right.reduce((values, value) => appendUnique(values, value), left);
}

function appendUnique(values: string[], value: string) {
  return values.includes(value) ? values : [...values, value];
}

function createInitialDemoPrompt(session: DemoSession) {
  return [
    "你正在为 VoiceCoder 生成第一版可运行 demo。",
    "",
    "目标项目路径：",
    session.projectPath,
    "",
    "已确认需求文档：",
    session.initialRequirementDocument,
    "",
    "Coding Prompt：",
    session.initialCodingPrompt,
    "",
    "请基于当前项目实现第一版 demo。优先保证可运行、可展示、交互完整。",
    "完成后给出简短变更摘要和后续可改进点。"
  ].join("\n");
}

function createInitialCodingPrompt(requirementState: RequirementState) {
  const document = requirementState.requirementDocument || requirementState.summary;
  return `请根据以下已确认需求进行实现：\n\n${document}`;
}

function nowString() {
  return Date.now().toString();
}
