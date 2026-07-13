import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
  AgentEvent,
  AgentItem,
  AgentMessagePhase,
  AgentRun,
  AgentRunKind,
  AgentTurnStatus,
  CodingAgentRuntimeMetadata,
  CodingAgentProviderKind,
  DevServerLifecycleEventEnvelope,
  DemoFeedbackTurn,
  DemoSession,
  RequirementState,
  RequirementUtterance,
  StartDevServerRequest,
  StopDevServerRequest
} from "../types/app";
import { createId } from "./project";

const DEV_SERVER_START_TIMEOUT_MS = 45_000;
export const DEMO_SESSION_UPDATED_EVENT = "voicecoder:demo-session-updated";

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
      runId?: string;
      kind: AgentRunKind;
      prompt: string;
      now: string;
      feedbackTurnId?: string;
    }
  | {
      type: "mark_agent_run_started";
      runId: string;
      codexThreadId: string;
      codexTurnId: string;
      runtime: CodingAgentRuntimeMetadata;
      now: string;
    }
  | {
      type: "append_agent_event";
      runId: string;
      event: AgentEvent;
      now: string;
    }
  | {
      type: "append_agent_events";
      runId: string;
      events: AgentEvent[];
      now: string;
    }
  | {
      type: "complete_agent_run";
      runId: string;
      now: string;
      status?: Exclude<AgentTurnStatus, "inProgress">;
      error?: string;
      finalMessage?: string;
      changedFiles?: string[];
      currentPreviewUrl?: string;
    }
  | {
      type: "set_preview_url";
      currentPreviewUrl: string;
      now: string;
    }
  | {
      type: "fail_preview";
      error: string;
      now: string;
    }
  | {
      type: "stop_preview";
      now: string;
    }
  | {
      type: "fail_stop_preview";
      error: string;
      now: string;
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
  canStartFeedbackListening: boolean;
  canStopPreview: boolean;
  startInitialRun: () => void;
  startFeedbackListening: () => void;
  stopPreview: () => void;
};

export type UseDemoSessionOptions = {
  onPreviewReady?: (url: string) => void;
  onPreviewStopped?: () => void;
};

type StartInitialDemoRunRequest = {
  demoSessionId: string;
  runId: string;
  projectPath: string;
  prompt: string;
  sandbox?: "read-only" | "workspace-write" | "danger-full-access";
  provider?: CodingAgentProviderKind;
};

type SaveDemoSessionLogRequest = {
  projectPath: string;
  demoSession: DemoSession;
};

type AgentRunStartedPayload = {
  demoSessionId: string;
  runId: string;
  projectPath: string;
  provider: CodingAgentProviderKind;
  codexThreadId: string;
  codexTurnId: string;
  runtime: CodingAgentRuntimeMetadata;
  startedAt: string;
};

type AgentEventPayload = {
  demoSessionId: string;
  runId: string;
  event: AgentEvent;
};

type AgentRunCompletedPayload = {
  demoSessionId: string;
  runId: string;
  finalMessage?: string;
  changedFiles: string[];
  status: Exclude<AgentTurnStatus, "inProgress">;
  error?: string;
  completedAt: string;
};

export const AGENT_EVENT_BATCH_INTERVAL_MS = 75;

type AgentErrorPayload = {
  demoSessionId?: string;
  runId?: string;
  message: string;
  occurredAt: string;
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

    const runId = action.runId ?? createId("agent_run");
    const run: AgentRun = {
      id: runId,
      kind: action.kind,
      prompt: action.prompt,
      status: "running",
      codexThreadId: session.codexThreadId,
      events: [],
      itemsById: {},
      itemOrder: [],
      messagesByItemId: {},
      warnings: [],
      errors: [],
      changedFiles: [],
      startedAt: action.now
    };

    return {
      ...session,
      runs: [...session.runs, run],
      feedbackTurns: linkFeedbackTurnToRun(session.feedbackTurns, action.feedbackTurnId, runId),
      status: action.kind === "initial_build" ? "agent_running" : "agent_modifying",
      error: undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "mark_agent_run_started") {
    const run = findRun(session, action.runId);
    if (!run || isTerminalRunStatus(run.status)) {
      return session;
    }

    return {
      ...session,
      codexThreadId: action.codexThreadId,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? {
              ...candidate,
              codexThreadId: action.codexThreadId,
              codexTurnId: action.codexTurnId,
              runtime: action.runtime,
              status: "running"
            }
          : candidate
      ),
      updatedAt: action.now
    };
  }

  if (action.type === "append_agent_event" || action.type === "append_agent_events") {
    const run = findRun(session, action.runId);
    if (!run || isTerminalRunStatus(run.status)) {
      return session;
    }

    const events = action.type === "append_agent_event" ? [action.event] : action.events;
    if (!events.length) {
      return session;
    }
    const nextRun = events.reduce(applyAgentEvent, run);

    return {
      ...session,
      codexThreadId: nextRun.codexThreadId ?? session.codexThreadId,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? nextRun
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

    const outcome = action.status ?? "completed";
    const runStatus: AgentRun["status"] =
      outcome === "completed" ? "succeeded" : outcome === "interrupted" ? "cancelled" : "failed";
    const sessionStatus: DemoSession["status"] =
      runStatus === "succeeded" ? "preview_ready" : runStatus === "cancelled" ? "preview_ready" : "error";

    return {
      ...session,
      runs: session.runs.map((candidate) =>
        candidate.id === action.runId
          ? {
              ...candidate,
              status: runStatus,
              finalMessage: action.finalMessage ?? candidate.finalMessage,
              changedFiles: mergeUnique(candidate.changedFiles, action.changedFiles ?? []),
              error: action.error ?? candidate.error,
              completedAt: action.now
            }
          : candidate
      ),
      currentPreviewUrl: action.currentPreviewUrl ?? session.currentPreviewUrl,
      status: sessionStatus,
      error: runStatus === "failed" ? action.error ?? run.error ?? "Codex turn failed." : undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "set_preview_url") {
    if (!canAttachPreviewUrl(session)) {
      return session;
    }

    return {
      ...session,
      currentPreviewUrl: action.currentPreviewUrl,
      status: "preview_ready",
      error: undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "fail_preview") {
    if (!canFailPreview(session)) {
      return session;
    }

    return {
      ...session,
      status: "error",
      error: action.error,
      updatedAt: action.now
    };
  }

  if (action.type === "stop_preview") {
    if (!session.currentPreviewUrl) {
      return session;
    }

    return {
      ...session,
      currentPreviewUrl: undefined,
      error: undefined,
      updatedAt: action.now
    };
  }

  if (action.type === "fail_stop_preview") {
    return {
      ...session,
      error: action.error,
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

export function useDemoSession(
  requirementState: RequirementState | undefined,
  projectPath: string | undefined,
  options: UseDemoSessionOptions = {}
): DemoSessionController {
  const [session, dispatch] = useReducer(demoSessionStoreReducer, undefined);
  const onPreviewReadyRef = useRef(options.onPreviewReady);
  const onPreviewStoppedRef = useRef(options.onPreviewStopped);
  const devServerTimeoutRef = useRef<ReturnType<typeof setTimeout> | undefined>();

  useEffect(() => {
    onPreviewReadyRef.current = options.onPreviewReady;
  }, [options.onPreviewReady]);

  useEffect(() => {
    onPreviewStoppedRef.current = options.onPreviewStopped;
  }, [options.onPreviewStopped]);

  useEffect(() => {
    return () => {
      clearDevServerStartTimeout(devServerTimeoutRef);
    };
  }, []);

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

  useEffect(() => {
    if (!isTauri() || !session) {
      return;
    }

    persistDemoSessionLog(session);
  }, [session]);

  useEffect(() => {
    if (!session || typeof window === "undefined") {
      return;
    }

    window.dispatchEvent(new CustomEvent(DEMO_SESSION_UPDATED_EVENT, {
      detail: {
        session
      }
    }));
  }, [session]);

  const startInitialRun = useCallback(() => {
    if (!session || session.status !== "ready_to_start") {
      return;
    }

    const runId = createId("agent_run");
    const prompt = createInitialDemoPrompt(session);
    const now = nowString();
    dispatch({
      type: "start_agent_run",
      runId,
      kind: "initial_build",
      prompt,
      now
    });

    if (!isTauri()) {
      dispatch({
        type: "fail_agent_run",
        runId,
        error: "Demo 生成需要在 Tauri 客户端中使用。",
        now: nowString()
      });
      return;
    }

    const request: StartInitialDemoRunRequest = {
      demoSessionId: session.id,
      runId,
      projectPath: session.projectPath,
      prompt,
      sandbox: "workspace-write"
    };

    void invoke("start_initial_demo_run", { request }).catch((error) => {
      dispatch({
        type: "fail_agent_run",
        runId,
        error: stringifyError(error),
        now: nowString()
      });
    });
  }, [session]);

  const startFeedbackListening = useCallback(() => {
    if (!session || session.status !== "preview_ready") {
      return;
    }

    dispatch({
      type: "start_feedback_listening",
      now: nowString()
    });
  }, [session]);

  const stopPreview = useCallback(() => {
    if (!session?.currentPreviewUrl) {
      return;
    }

    clearDevServerStartTimeout(devServerTimeoutRef);

    if (!isTauri()) {
      dispatch({
        type: "stop_preview",
        now: nowString()
      });
      onPreviewStoppedRef.current?.();
      return;
    }

    void stopDemoDevServer(session.id)
      .then(() => {
        dispatch({
          type: "stop_preview",
          now: nowString()
        });
        onPreviewStoppedRef.current?.();
      })
      .catch((error) => {
        dispatch({
          type: "fail_stop_preview",
          error: `停止 dev server 失败：${stringifyError(error)}`,
          now: nowString()
        });
      });
  }, [session]);

  useEffect(() => {
    if (!isTauri() || !session) {
      return;
    }

    const sessionId = session.id;
    const sessionProjectPath = session.projectPath;
    let agentEventBatch: AgentEventPayload[] = [];
    let agentEventBatchTimer: ReturnType<typeof setTimeout> | undefined;
    const flushAgentEventBatch = () => {
      if (agentEventBatchTimer) {
        clearTimeout(agentEventBatchTimer);
        agentEventBatchTimer = undefined;
      }
      if (!agentEventBatch.length) {
        return;
      }

      const batchesByRun = new Map<string, AgentEvent[]>();
      for (const payload of agentEventBatch) {
        const events = batchesByRun.get(payload.runId) ?? [];
        events.push(payload.event);
        batchesByRun.set(payload.runId, events);
      }
      agentEventBatch = [];

      for (const [runId, events] of batchesByRun) {
        dispatch({
          type: "append_agent_events",
          runId,
          events,
          now: events[events.length - 1].createdAt
        });
      }
    };
    const unlistenPromises = [
      listen<AgentRunStartedPayload>("agent://run-started", (event) => {
        if (event.payload.demoSessionId !== sessionId) {
          return;
        }

        dispatch({
          type: "mark_agent_run_started",
          runId: event.payload.runId,
          codexThreadId: event.payload.codexThreadId,
          codexTurnId: event.payload.codexTurnId,
          runtime: event.payload.runtime,
          now: event.payload.startedAt
        });
      }),
      listen<AgentEventPayload>("agent://event", (event) => {
        if (event.payload.demoSessionId !== sessionId) {
          return;
        }

        agentEventBatch.push(event.payload);
        agentEventBatchTimer ??= setTimeout(flushAgentEventBatch, AGENT_EVENT_BATCH_INTERVAL_MS);
      }),
      listen<AgentRunCompletedPayload>("agent://run-completed", (event) => {
        if (event.payload.demoSessionId !== sessionId) {
          return;
        }

        flushAgentEventBatch();
        dispatch({
          type: "complete_agent_run",
          runId: event.payload.runId,
          status: event.payload.status,
          error: event.payload.error,
          finalMessage: event.payload.finalMessage,
          changedFiles: event.payload.changedFiles,
          now: event.payload.completedAt
        });
        if (event.payload.status !== "completed") {
          return;
        }
        dispatchProjectFilesChanged(sessionProjectPath, event.payload.changedFiles);
        startDevServerTimeout(devServerTimeoutRef, (message) => {
          dispatch({
            type: "fail_preview",
            error: message,
            now: nowString()
          });
        });
        startDemoDevServer(sessionProjectPath, sessionId, (message) => {
          clearDevServerStartTimeout(devServerTimeoutRef);
          dispatch({
            type: "fail_preview",
            error: message,
            now: nowString()
          });
        });
      }),
      listen<DevServerLifecycleEventEnvelope>("dev-server://event", (event) => {
        if (event.payload.projectPath !== sessionProjectPath) {
          return;
        }

        if (event.payload.event.type === "ready") {
          clearDevServerStartTimeout(devServerTimeoutRef);
          dispatch({
            type: "set_preview_url",
            currentPreviewUrl: event.payload.event.url,
            now: event.payload.occurredAt
          });
          onPreviewReadyRef.current?.(event.payload.event.url);
          return;
        }

        const previewError = formatDevServerPreviewError(event.payload);
        if (!previewError) {
          return;
        }

        clearDevServerStartTimeout(devServerTimeoutRef);
        dispatch({
          type: "fail_preview",
          error: previewError,
          now: event.payload.occurredAt
        });
      }),
      listen<AgentErrorPayload>("agent://error", (event) => {
        if (event.payload.demoSessionId && event.payload.demoSessionId !== sessionId) {
          return;
        }
        if (!event.payload.runId) {
          return;
        }

        flushAgentEventBatch();
        dispatch({
          type: "fail_agent_run",
          runId: event.payload.runId,
          error: event.payload.message,
          now: event.payload.occurredAt
        });
      })
    ];

    return () => {
      if (agentEventBatchTimer) {
        clearTimeout(agentEventBatchTimer);
      }
      for (const unlistenPromise of unlistenPromises) {
        void unlistenPromise.then((unlisten) => unlisten());
      }
    };
  }, [session?.id]);

  return useMemo(
    () => ({
      session,
      active: Boolean(session),
      canStartInitialRun: session?.status === "ready_to_start",
      canStartFeedbackListening: session?.status === "preview_ready",
      canStopPreview: canStopPreview(session),
      startInitialRun,
      startFeedbackListening,
      stopPreview
    }),
    [session, startInitialRun, startFeedbackListening, stopPreview]
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

function canAttachPreviewUrl(session: DemoSession) {
  return (
    session.status === "preview_ready" ||
    session.status === "feedback_listening" ||
    session.status === "feedback_processing"
  );
}

function canFailPreview(session: DemoSession) {
  return canAttachPreviewUrl(session) && !session.currentPreviewUrl;
}

function canStopPreview(session: DemoSession | undefined) {
  return Boolean(
    session?.currentPreviewUrl &&
      (session.status === "preview_ready" ||
        session.status === "feedback_listening" ||
        session.status === "feedback_processing")
  );
}

function isTerminalRunStatus(status: AgentRun["status"]) {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}

function applyAgentEvent(run: AgentRun, event: AgentEvent): AgentRun {
  let nextRun: AgentRun = {
    ...run,
    events: [...run.events, event]
  };

  if (event.type === "thread_started") {
    return { ...nextRun, codexThreadId: event.threadId };
  }
  if (event.type === "turn_started") {
    return { ...nextRun, codexTurnId: event.turnId ?? nextRun.codexTurnId };
  }
  if (event.type === "file_change") {
    return { ...nextRun, changedFiles: appendUnique(nextRun.changedFiles, event.path) };
  }
  if (event.type === "plan_updated") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId,
      currentPlan: {
        threadId: event.threadId,
        turnId: event.turnId,
        explanation: event.explanation,
        steps: event.plan,
        updatedAt: event.createdAt
      }
    };
  }
  if (event.type === "warning") {
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      warnings: [...(nextRun.warnings ?? []), event]
    };
  }
  if (event.type === "error") {
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      errors: [...(nextRun.errors ?? []), event],
      error: event.terminal ? event.message : nextRun.error
    };
  }
  if (event.type === "turn_completed") {
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      finalMessage: event.finalMessage ?? nextRun.finalMessage
    };
  }
  if (event.type !== "item_started" && event.type !== "item_delta" && event.type !== "item_completed") {
    return nextRun;
  }

  const item = applyAgentItemEvent((nextRun.itemsById ?? {})[event.itemId], event);
  const itemsById = {
    ...(nextRun.itemsById ?? {}),
    [item.id]: item
  };
  const messagesByItemId =
    item.type === "agentMessage"
      ? {
          ...(nextRun.messagesByItemId ?? {}),
          [item.id]: item
        }
      : nextRun.messagesByItemId ?? {};
  const changedFiles = extractChangedFilePaths(item.data).reduce(
    (paths, path) => appendUnique(paths, path),
    nextRun.changedFiles
  );

  return {
    ...nextRun,
    codexThreadId: event.threadId,
    codexTurnId: event.turnId,
    itemsById,
    itemOrder: appendUnique(nextRun.itemOrder ?? [], item.id),
    messagesByItemId,
    changedFiles
  };
}

function applyAgentItemEvent(
  existing: AgentItem | undefined,
  event: Extract<AgentEvent, { type: "item_started" | "item_delta" | "item_completed" }>
): AgentItem {
  if (event.type === "item_started") {
    const data = existing?.lifecycle === "completed"
      ? existing.data
      : { ...(existing?.data ?? {}), ...event.item };
    return hydrateAgentItem({
      id: event.itemId,
      type: existing?.lifecycle === "completed" ? existing.type : event.itemType,
      threadId: event.threadId,
      turnId: event.turnId,
      lifecycle: existing?.lifecycle ?? event.lifecycle,
      status: existing?.lifecycle === "completed" ? existing.status : event.status,
      startedAt: event.startedAt,
      updatedAt: event.createdAt,
      completedAt: existing?.completedAt,
      data,
      text: existing?.text,
      phase: existing?.phase,
      output: existing?.output,
      reasoningSummary: existing?.reasoningSummary
    });
  }

  if (event.type === "item_completed") {
    const aggregatedOutput = readString(event.item.aggregatedOutput);
    return hydrateAgentItem({
      id: event.itemId,
      type: event.itemType,
      threadId: event.threadId,
      turnId: event.turnId,
      lifecycle: event.lifecycle,
      status: event.status,
      startedAt: existing?.startedAt ?? event.completedAt,
      updatedAt: event.createdAt,
      completedAt: event.completedAt,
      data: event.item,
      text: readString(event.item.text) ?? existing?.text,
      phase: event.itemType === "agentMessage"
        ? readAgentMessagePhase(event.item.phase) ?? "unknown"
        : existing?.phase,
      output: aggregatedOutput ?? existing?.output,
      reasoningSummary: existing?.reasoningSummary
    });
  }

  if (existing?.lifecycle === "completed") {
    return existing;
  }

  const item: AgentItem = existing ?? {
    id: event.itemId,
    type: event.itemType,
    threadId: event.threadId,
    turnId: event.turnId,
    lifecycle: event.lifecycle,
    startedAt: event.createdAt,
    updatedAt: event.createdAt,
    data: {}
  };
  const deltaText = readString(event.delta);
  const next: AgentItem = {
    ...item,
    threadId: event.threadId,
    turnId: event.turnId,
    type: item.type === "unknown" ? event.itemType : item.type,
    updatedAt: event.createdAt,
    data: {
      ...item.data,
      lastDelta: event.delta
    }
  };

  if (event.method === "item/agentMessage/delta" || event.method === "item/plan/delta") {
    next.text = `${item.text ?? ""}${deltaText ?? ""}`;
  } else if (
    event.method === "item/commandExecution/outputDelta" ||
    event.method === "item/fileChange/outputDelta"
  ) {
    next.output = `${item.output ?? ""}${deltaText ?? ""}`;
  } else if (event.method === "item/fileChange/patchUpdated") {
    next.data = {
      ...next.data,
      changes: Array.isArray(event.delta) ? event.delta : readRecord(event.delta)?.changes ?? []
    };
  } else if (event.method === "item/reasoning/summaryTextDelta") {
    next.reasoningSummary = `${item.reasoningSummary ?? ""}${deltaText ?? ""}`;
  }

  return hydrateAgentItem(next);
}

function hydrateAgentItem(item: AgentItem): AgentItem {
  if (item.type !== "agentMessage") {
    return item;
  }

  const dataText = readString(item.data.text);
  const text = item.lifecycle === "completed"
    ? dataText ?? item.text
    : (item.text?.length ?? 0) > (dataText?.length ?? 0)
      ? item.text
      : dataText ?? item.text;

  return {
    ...item,
    text,
    phase: readAgentMessagePhase(item.data.phase) ?? item.phase ?? "unknown"
  };
}

function extractChangedFilePaths(data: Record<string, unknown>) {
  if (!Array.isArray(data.changes)) {
    return [];
  }

  return data.changes.flatMap((change) => {
    const record = readRecord(change);
    const path = record ? readString(record.path) : undefined;
    return path ? [path] : [];
  });
}

function readAgentMessagePhase(value: unknown): AgentMessagePhase | undefined {
  if (value === "commentary" || value === "final_answer") {
    return value;
  }
  return value === null || value === undefined ? undefined : "unknown";
}

function readRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function readString(value: unknown) {
  return typeof value === "string" ? value : undefined;
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

export function createInitialDemoPrompt(session: DemoSession) {
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
    "运行约束：",
    "- 必须生成或保持一个 Node.js 前端项目。",
    "- 项目根目录必须有 package.json。",
    "- package.json 必须包含可用的 scripts.dev。",
    "- 用户会在项目根目录通过 npm run dev 启动预览。",
    "- 不要只生成裸 index.html / script.js / styles.css 静态文件项目，除非同时补齐 package.json 和 npm run dev 启动链路。",
    "- 不要启动 dev server，不要运行 npm run dev / npm start / vite --host / node server 等长驻服务。",
    "- 可以运行不会长驻的本地静态检查，例如 npm run build、npm test、node --check 或 tsc --noEmit；如果依赖缺失，不要联网安装，说明即可。",
    "- 生成完成后，VoiceCoder 会在后台统一启动 npm run dev 并打开预览。",
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

function stringifyError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function startDemoDevServer(projectPath: string, demoSessionId: string, onError: (message: string) => void) {
  const request: StartDevServerRequest = {
    projectPath,
    sessionId: getDemoDevServerSessionId(demoSessionId)
  };

  void invoke("start_demo_dev_server", { request }).catch((error) => {
    onError(`启动 dev server 失败：${stringifyError(error)}`);
  });
}

function stopDemoDevServer(demoSessionId: string) {
  const request: StopDevServerRequest = {
    sessionId: getDemoDevServerSessionId(demoSessionId)
  };

  return invoke("stop_demo_dev_server", { request });
}

function getDemoDevServerSessionId(demoSessionId: string) {
  return `dev_server_${demoSessionId}`;
}

function persistDemoSessionLog(session: DemoSession) {
  const request: SaveDemoSessionLogRequest = {
    projectPath: session.projectPath,
    demoSession: session
  };

  void invoke("save_demo_session_log", { request }).catch((error) => {
    console.warn("Failed to save DemoSession log.", error);
  });
}

function startDevServerTimeout(timeoutRef: { current: ReturnType<typeof setTimeout> | undefined }, onTimeout: (message: string) => void) {
  clearDevServerStartTimeout(timeoutRef);
  timeoutRef.current = setTimeout(() => {
    timeoutRef.current = undefined;
    onTimeout("dev server 启动超时：45 秒内没有检测到本地预览 URL。");
  }, DEV_SERVER_START_TIMEOUT_MS);
}

function clearDevServerStartTimeout(timeoutRef: { current: ReturnType<typeof setTimeout> | undefined }) {
  if (!timeoutRef.current) {
    return;
  }

  clearTimeout(timeoutRef.current);
  timeoutRef.current = undefined;
}

export function formatDevServerPreviewError(envelope: DevServerLifecycleEventEnvelope) {
  const { event } = envelope;

  if (event.type === "error") {
    return `dev server 出错：${event.message}`;
  }

  if (event.type === "stopped") {
    if (event.reason === "user") {
      return undefined;
    }

    return `dev server 在预览 URL 就绪前退出${formatExitCode(event.exitCode)}。`;
  }

  if (event.type === "output") {
    return detectDevServerOutputIssue(event.text);
  }

  return undefined;
}

function formatExitCode(exitCode: number | undefined) {
  return typeof exitCode === "number" ? `，退出码 ${exitCode}` : "";
}

export function detectDevServerOutputIssue(text: string) {
  const normalizedText = text.toLowerCase();
  if (
    normalizedText.includes("eaddrinuse") ||
    normalizedText.includes("address already in use") ||
    normalizedText.includes("port is already in use") ||
    normalizedText.includes("port already in use")
  ) {
    return "dev server 启动失败：端口已被占用。";
  }

  return undefined;
}

function dispatchProjectFilesChanged(projectPath: string, changedFiles: string[]) {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(new CustomEvent("voicecoder:project-files-changed", {
    detail: {
      projectPath,
      changedPath: resolveChangedPath(projectPath, changedFiles[0])
    }
  }));
}

function resolveChangedPath(projectPath: string, changedPath: string | undefined) {
  if (!changedPath) {
    return undefined;
  }

  if (isAbsoluteOrVirtualPath(changedPath)) {
    return changedPath;
  }

  const cleanProjectPath = projectPath.replace(/\/+$/, "");
  const cleanChangedPath = changedPath.replace(/^\.?\//, "");
  return `${cleanProjectPath}/${cleanChangedPath}`;
}

function isAbsoluteOrVirtualPath(path: string) {
  return path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("browser://");
}
