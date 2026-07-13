import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
  AgentEvent,
  AgentHookRun,
  AgentItem,
  AgentItemPresentation,
  AgentMessagePhase,
  AgentRun,
  AgentRunKind,
  AgentServerRequest,
  AgentStructuredPreview,
  AgentTokenUsageBreakdown,
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
import {
  appendAgentOutputTail,
  buildAgentFilesByPath,
  EMPTY_AGENT_DIFF_STATS,
  getCompletedFileChangePaths,
  limitAgentOutputTail,
  parseAgentFileChanges,
  parseUnifiedDiffStats
} from "./agentProgress";
import { createId } from "./project";
import {
  recoverPersistedDemoSessionSnapshot,
  usePersistedDemoSessionRecovery
} from "./demoSessionRecovery";

const DEV_SERVER_START_TIMEOUT_MS = 45_000;
const AGENT_STRUCTURED_PREVIEW_LIMIT = 4_000;
const AGENT_TEXT_PREVIEW_LIMIT = 1_200;
const AGENT_MESSAGE_TEXT_LIMIT = 32_000;
const AGENT_HOOK_ENTRY_LIMIT = 2_000;
const AGENT_STRUCTURED_PREVIEW_MAX_NODES = 240;
const AGENT_STRUCTURED_PREVIEW_MAX_SOURCE_CHARS = 16_000;
const AGENT_STRUCTURED_PREVIEW_MAX_COLLECTION_ENTRIES = 50;
const AGENT_STRUCTURED_PREVIEW_MAX_KEY_LENGTH = 200;
const AGENT_REASONING_SUMMARY_PART_LIMIT = 64;
const SENSITIVE_AGENT_FIELD_PATTERN = /(?:token|secret|password|authorization|cookie|api[_-]?key|credential|private[_-]?key|signature|stdin)/i;
const ENCODED_AGENT_FIELD_PATTERN = /(?:base64|binary|blob|image(?:data|url)?|payload|result)/i;
const CREDENTIAL_AGENT_TEXT_PATTERN = /(?:\b(?:authorization|api[_-]?key|(?:access[_-]?)?token|password|secret|signature|x-amz-(?:credential|signature)|x-goog-signature)\s*[:=]\s*\S+|\bbearer\s+[A-Za-z0-9._~+/=-]{8,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|\bsk-[A-Za-z0-9_-]{12,})/i;
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
  | {
      type: "restore_demo_session";
      session: DemoSession;
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
  if (action.type === "restore_demo_session") {
    return action.session;
  }

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

export function recoverDemoSessionSnapshot(
  snapshot: unknown,
  activeRunIds: ReadonlySet<string>,
  now: string
): DemoSession | undefined {
  return recoverPersistedDemoSessionSnapshot(snapshot, activeRunIds, now, applyAgentEvent);
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
      filesByPath: {},
      aggregateDiff: "",
      aggregateDiffStats: EMPTY_AGENT_DIFF_STATS,
      hooksById: {},
      hookOrder: [],
      serverRequestsById: {},
      serverRequestOrder: [],
      pendingServerRequestIds: [],
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
      runs: session.runs.map((candidate) => {
        if (candidate.id !== action.runId) {
          return candidate;
        }
        const settled = settlePendingServerRequests(
          candidate,
          action.now,
          outcome === "completed" ? "AgentRun 已结束" : "AgentRun 已中断"
        );
        return {
          ...settled,
          status: runStatus,
          finalMessage: action.finalMessage ?? candidate.finalMessage,
          changedFiles: mergeUnique(candidate.changedFiles, action.changedFiles ?? []),
          error: action.error ?? candidate.error,
          completedAt: action.now
        };
      }),
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
      runs: session.runs.map((candidate) => candidate.id === action.runId
        ? {
            ...settlePendingServerRequests(candidate, action.now, "AgentRun 失败，请求已取消"),
            status: "failed",
            error: action.error,
            completedAt: action.now
          }
        : candidate),
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
      runs: session.runs.map((candidate) => candidate.id === action.runId
        ? {
            ...settlePendingServerRequests(candidate, action.now, "AgentRun 已取消"),
            status: "cancelled",
            completedAt: action.now
          }
        : candidate),
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
  const recoveryReady = usePersistedDemoSessionRecovery(projectPath, dispatch, applyAgentEvent);
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
      !recoveryReady ||
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
  }, [projectPath, recoveryReady, requirementState]);

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

        if (event.payload.event.type === "item_completed") {
          const changedPaths = getCompletedFileChangePaths(
            event.payload.event.itemType,
            event.payload.event.item
          );
          if (changedPaths.length) {
            dispatchProjectFilesChanged(sessionProjectPath, changedPaths, "incremental");
          }
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
        dispatchProjectFilesChanged(sessionProjectPath, event.payload.changedFiles, "full");
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

function settlePendingServerRequests(run: AgentRun, now: string, message: string): AgentRun {
  const pendingIds = run.pendingServerRequestIds ?? [];
  if (!pendingIds.length) {
    return run;
  }
  const serverRequestsById = { ...(run.serverRequestsById ?? {}) };
  for (const requestKey of pendingIds) {
    const request = serverRequestsById[requestKey];
    if (!request) {
      continue;
    }
    serverRequestsById[requestKey] = {
      ...request,
      status: "cancelled",
      resolution: "run_finished",
      statusMessage: message,
      updatedAt: now
    };
  }
  return {
    ...run,
    serverRequestsById,
    pendingServerRequestIds: []
  };
}

function applyAgentEvent(run: AgentRun, event: AgentEvent): AgentRun {
  const nextRun: AgentRun = {
    ...run,
    events: appendRetainedAgentEvent(run.events, event)
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
        explanation: limitAgentText(event.explanation),
        steps: event.plan.slice(0, 100).map((step) => ({
          step: limitAgentText(step.step) ?? "",
          status: step.status
        })),
        updatedAt: event.createdAt
      }
    };
  }
  if (event.type === "turn_diff_updated") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId,
      aggregateDiff: event.diff,
      aggregateDiffStats: parseUnifiedDiffStats(event.diff),
      aggregateDiffUpdatedAt: event.createdAt
    };
  }
  if (event.type === "hook_run_updated") {
    const hook = applyAgentHookEvent((nextRun.hooksById ?? {})[event.hookId], event);
    const hooksById = {
      ...(nextRun.hooksById ?? {}),
      [hook.id]: hook
    };
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      hooksById,
      hookOrder: [...appendUnique(nextRun.hookOrder ?? [], hook.id)]
        .sort((left, right) => compareAgentHooks(hooksById[left], hooksById[right]))
    };
  }
  if (event.type === "token_usage_updated") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId,
      tokenUsage: {
        threadId: event.threadId,
        turnId: event.turnId,
        total: normalizeTokenUsageBreakdown(event.tokenUsage.total),
        last: normalizeTokenUsageBreakdown(event.tokenUsage.last),
        modelContextWindow: readNumber(event.tokenUsage.modelContextWindow) ?? null,
        updatedAt: event.createdAt
      }
    };
  }
  if (event.type === "model_safety_buffering_updated") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId,
      modelSafetyBuffering: {
        threadId: event.threadId,
        turnId: event.turnId,
        model: limitAgentText(event.model, AGENT_TEXT_PREVIEW_LIMIT) ?? "unknown",
        useCases: event.useCases.slice(0, 50).map((value) => limitAgentText(value) ?? ""),
        reasons: event.reasons.slice(0, 50).map((value) => limitAgentText(value) ?? ""),
        showBufferingUi: event.showBufferingUi,
        fasterModel: limitAgentText(event.fasterModel, AGENT_TEXT_PREVIEW_LIMIT),
        createdAt: event.createdAt
      }
    };
  }
  if (event.type === "model_verification_updated") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId,
      modelVerification: {
        threadId: event.threadId,
        turnId: event.turnId,
        verifications: event.verifications.slice(0, 50).map((value) => limitAgentText(value) ?? ""),
        createdAt: event.createdAt
      }
    };
  }
  if (event.type === "model_rerouted" || event.type === "context_compacted") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      codexTurnId: event.turnId
    };
  }
  if (event.type === "server_request") {
    const request: AgentServerRequest = {
      requestId: event.requestId,
      requestKey: event.requestKey,
      method: limitAgentText(event.method, 300) ?? "unknown",
      kind: event.kind,
      status: event.status,
      requiresUserInput: event.requiresUserInput,
      autoReview: event.autoReview,
      threadId: event.threadId,
      turnId: event.turnId,
      itemId: event.itemId,
      details: sanitizeAgentServerRequestDetails(event.details),
      expiresAt: event.expiresAt,
      createdAt: event.createdAt,
      updatedAt: event.createdAt
    };
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      serverRequestsById: {
        ...(nextRun.serverRequestsById ?? {}),
        [event.requestKey]: request
      },
      serverRequestOrder: appendUnique(nextRun.serverRequestOrder ?? [], event.requestKey),
      pendingServerRequestIds: appendUnique(
        nextRun.pendingServerRequestIds ?? [],
        event.requestKey
      )
    };
  }
  if (event.type === "server_request_resolved") {
    const existing = nextRun.serverRequestsById?.[event.requestKey];
    const serverRequestsById = existing
      ? {
          ...(nextRun.serverRequestsById ?? {}),
          [event.requestKey]: {
            ...existing,
            status: event.status,
            resolution: limitAgentText(event.resolution, 200),
            statusMessage: limitAgentSensitiveText(event.message, AGENT_TEXT_PREVIEW_LIMIT),
            updatedAt: event.createdAt
          }
        }
      : nextRun.serverRequestsById ?? {};
    return {
      ...nextRun,
      serverRequestsById,
      pendingServerRequestIds: (nextRun.pendingServerRequestIds ?? []).filter(
        (requestKey) => requestKey !== event.requestKey
      )
    };
  }
  if (event.type === "warning") {
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      warnings: upsertAgentWarning(nextRun.warnings ?? [], {
        message: limitAgentSensitiveText(event.message) ?? "Codex 运行提醒",
        source: "runtime",
        severity: "warning",
        threadId: event.threadId,
        turnId: event.turnId,
        createdAt: event.createdAt
      })
    };
  }
  if (event.type === "config_warning") {
    return {
      ...nextRun,
      warnings: upsertAgentWarning(nextRun.warnings ?? [], {
        message: limitAgentSensitiveText(event.summary) ?? "Codex 配置提醒",
        source: "config",
        severity: "warning",
        details: limitAgentSensitiveText(event.details),
        path: limitAgentText(event.path, AGENT_TEXT_PREVIEW_LIMIT),
        range: event.range,
        createdAt: event.createdAt
      })
    };
  }
  if (event.type === "guardian_warning") {
    return {
      ...nextRun,
      codexThreadId: event.threadId,
      warnings: upsertAgentWarning(nextRun.warnings ?? [], {
        message: limitAgentSensitiveText(event.message) ?? "Codex 安全审查提醒",
        source: "guardian",
        severity: "important",
        threadId: event.threadId,
        createdAt: event.createdAt
      })
    };
  }
  if (event.type === "error") {
    const limitedError = {
      ...event,
      message: limitAgentSensitiveText(event.message, AGENT_MESSAGE_TEXT_LIMIT) ?? "Codex 运行失败"
    };
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      errors: [...(nextRun.errors ?? []), limitedError],
      error: event.terminal ? limitedError.message : nextRun.error
    };
  }
  if (event.type === "turn_completed") {
    return {
      ...nextRun,
      codexThreadId: event.threadId ?? nextRun.codexThreadId,
      codexTurnId: event.turnId ?? nextRun.codexTurnId,
      finalMessage: limitAgentText(event.finalMessage, AGENT_MESSAGE_TEXT_LIMIT) ?? nextRun.finalMessage
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
  let filesByPath = nextRun.filesByPath ?? {};
  let changedFiles = nextRun.changedFiles;
  if (item.type === "fileChange") {
    filesByPath = buildAgentFilesByPath(Object.values(itemsById));
    const legacyChangedFiles = nextRun.events.flatMap((candidate) =>
      candidate.type === "file_change" ? [candidate.path] : []
    );
    changedFiles = mergeUnique(legacyChangedFiles, Object.keys(filesByPath));
  }

  return {
    ...nextRun,
    codexThreadId: event.threadId,
    codexTurnId: event.turnId,
    itemsById,
    itemOrder: appendUnique(nextRun.itemOrder ?? [], item.id),
    messagesByItemId,
    filesByPath,
    changedFiles
  };
}

function appendRetainedAgentEvent(events: AgentEvent[], event: AgentEvent) {
  if (!shouldRetainAgentEvent(event)) {
    return events;
  }

  const previous = events[events.length - 1];
  if (previous?.type === "agent_message" && event.type === "agent_message") {
    return [
      ...events.slice(0, -1),
      {
        ...event,
        text: limitAgentText(
          `${previous.text}${event.text}`,
          AGENT_MESSAGE_TEXT_LIMIT
        ) ?? ""
      }
    ];
  }
  if (previous?.type === "plan_update" && event.type === "plan_update") {
    return [
      ...events.slice(0, -1),
      {
        ...event,
        text: limitAgentText(
          [previous.text, event.text].filter(Boolean).join(" "),
          AGENT_MESSAGE_TEXT_LIMIT
        ) ?? ""
      }
    ];
  }

  return [...events, event];
}

function shouldRetainAgentEvent(event: AgentEvent) {
  if (
    (event.type === "diagnostic" && event.level === "debug") ||
    event.type === "turn_diff_updated" ||
    event.type === "item_delta" ||
    event.type === "hook_run_updated" ||
    event.type === "token_usage_updated" ||
    event.type === "model_safety_buffering_updated" ||
    event.type === "model_verification_updated" ||
    event.type === "server_request" ||
    event.type === "server_request_resolved" ||
    event.type === "warning" ||
    event.type === "config_warning" ||
    event.type === "guardian_warning" ||
    event.type === "error"
  ) {
    return false;
  }
  if (event.type === "item_started" || event.type === "item_completed") {
    return false;
  }
  return true;
}

function applyAgentItemEvent(
  existing: AgentItem | undefined,
  event: Extract<AgentEvent, { type: "item_started" | "item_delta" | "item_completed" }>
): AgentItem {
  if (event.type === "item_started") {
    const mergedData = existing?.lifecycle === "completed"
      ? existing.data
      : { ...(existing?.data ?? {}), ...event.item };
    const data = existing?.lifecycle === "completed"
      ? mergedData
      : sanitizeAgentItemData(mergedData, event.itemType);
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
      outputTruncated: existing?.outputTruncated,
      reasoningSummary: existing?.reasoningSummary,
      reasoningSummaryParts: existing?.reasoningSummaryParts,
      restrictedDebugAvailable: existing?.restrictedDebugAvailable
        || hasRestrictedAgentData(event.item, event.itemType),
      progressMessage: existing?.progressMessage,
      terminalInteractionCount: existing?.terminalInteractionCount,
      presentation: existing?.presentation
    });
  }

  if (event.type === "item_completed") {
    const aggregatedOutput = readString(event.item.aggregatedOutput);
    const limitedOutput = aggregatedOutput === undefined ? undefined : limitAgentOutputTail(aggregatedOutput);
    const completedOutputTruncated = limitedOutput
      ? limitedOutput.truncated || (readBoolean(event.item.aggregatedOutputTruncated) ?? false)
      : existing?.outputTruncated;
    const completedSummaryParts = event.itemType === "reasoning"
      ? readStringArray(event.item.summary).slice(0, AGENT_REASONING_SUMMARY_PART_LIMIT)
      : undefined;
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
      data: sanitizeAgentItemData(event.item, event.itemType, limitedOutput),
      text: limitAgentText(readString(event.item.text), AGENT_MESSAGE_TEXT_LIMIT) ?? existing?.text,
      phase: event.itemType === "agentMessage"
        ? readAgentMessagePhase(event.item.phase) ?? "unknown"
        : existing?.phase,
      output: limitedOutput?.outputTail ?? existing?.output,
      outputTruncated: completedOutputTruncated,
      reasoningSummary: completedSummaryParts?.length
        ? joinReasoningSummary(completedSummaryParts)
        : existing?.reasoningSummary,
      reasoningSummaryParts: completedSummaryParts?.length
        ? completedSummaryParts
        : existing?.reasoningSummaryParts,
      restrictedDebugAvailable: existing?.restrictedDebugAvailable
        || hasRestrictedAgentData(event.item, event.itemType),
      progressMessage: existing?.progressMessage,
      terminalInteractionCount: existing?.terminalInteractionCount,
      presentation: existing?.presentation
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
  const deltaRecord = readRecord(event.delta);
  const deltaText = readString(event.delta) ?? readString(deltaRecord?.text) ?? readString(deltaRecord?.delta);
  const next: AgentItem = {
    ...item,
    threadId: event.threadId,
    turnId: event.turnId,
    type: item.type === "unknown" ? event.itemType : item.type,
    updatedAt: event.createdAt,
    data: item.data
  };

  if (event.method === "item/agentMessage/delta" || event.method === "item/plan/delta") {
    next.text = limitAgentText(
      `${item.text ?? ""}${deltaText ?? ""}`,
      AGENT_MESSAGE_TEXT_LIMIT
    );
  } else if (
    event.method === "item/commandExecution/outputDelta" ||
    event.method === "item/fileChange/outputDelta"
  ) {
    const output = appendAgentOutputTail(
      item.output,
      deltaText ?? "",
      item.outputTruncated
    );
    next.output = output.outputTail;
    next.outputTruncated = output.truncated;
  } else if (event.method === "item/commandExecution/terminalInteraction") {
    next.terminalInteractionCount = (item.terminalInteractionCount ?? 0) + 1;
  } else if (event.method === "item/fileChange/patchUpdated") {
    next.data = {
      ...next.data,
      changes: Array.isArray(event.delta) ? event.delta : readRecord(event.delta)?.changes ?? []
    };
  } else if (event.method === "item/reasoning/summaryPartAdded") {
    const parts = [...(item.reasoningSummaryParts ?? [])];
    const summaryIndex = readInteger(deltaRecord?.summaryIndex) ?? parts.length;
    if (summaryIndex >= AGENT_REASONING_SUMMARY_PART_LIMIT) {
      return hydrateAgentItem(next);
    }
    parts[summaryIndex] ??= "";
    next.reasoningSummaryParts = parts;
    next.reasoningSummary = joinReasoningSummary(parts);
  } else if (event.method === "item/reasoning/summaryTextDelta") {
    const parts = [...(item.reasoningSummaryParts ?? [])];
    const summaryIndex = readInteger(deltaRecord?.summaryIndex) ?? 0;
    if (summaryIndex >= AGENT_REASONING_SUMMARY_PART_LIMIT) {
      return hydrateAgentItem(next);
    }
    parts[summaryIndex] = limitAgentText(
      `${parts[summaryIndex] ?? ""}${deltaText ?? ""}`,
      AGENT_TEXT_PREVIEW_LIMIT
    ) ?? "";
    next.reasoningSummaryParts = parts;
    next.reasoningSummary = joinReasoningSummary(parts);
  } else if (event.method === "item/reasoning/textDelta") {
    next.restrictedDebugAvailable = true;
  } else if (event.method === "item/mcpToolCall/progress") {
    next.progressMessage = limitAgentSensitiveText(
      readString(deltaRecord?.message) ?? deltaText,
      AGENT_TEXT_PREVIEW_LIMIT
    ) ?? item.progressMessage;
  } else {
    next.data = {
      ...item.data,
      lastDeltaPreview: buildStructuredPreview(event.delta)
    };
  }

  return hydrateAgentItem(next);
}

function hydrateAgentItem(item: AgentItem): AgentItem {
  if (item.type === "fileChange") {
    return {
      ...item,
      fileChanges: parseAgentFileChanges(item.data, item.id)
    };
  }

  if (item.type === "commandExecution") {
    return {
      ...item,
      command: {
        command: readString(item.data.command) ?? "",
        cwd: readString(item.data.cwd),
        status: readString(item.data.status) ?? item.status ?? "unknown",
        exitCode: readNumber(item.data.exitCode),
        durationMs: readNumber(item.data.durationMs),
        outputTail: item.output ?? "",
        outputTruncated: item.outputTruncated ?? false
      }
    };
  }

  if (item.type === "reasoning") {
    const completedParts = readStringArray(item.data.summary);
    const reasoningSummaryParts = item.lifecycle === "completed" && completedParts.length
      ? completedParts
      : item.reasoningSummaryParts ?? completedParts;
    const reasoningSummary = reasoningSummaryParts.length
      ? joinReasoningSummary(reasoningSummaryParts)
      : item.reasoningSummary ?? "";
    const restrictedDebugAvailable = item.restrictedDebugAvailable
      || readBoolean(item.data.rawTextAvailable)
      || false;
    return {
      ...item,
      reasoningSummary,
      reasoningSummaryParts,
      restrictedDebugAvailable,
      presentation: {
        kind: "reasoning",
        summary: reasoningSummary,
        rawTextAvailable: restrictedDebugAvailable
      }
    };
  }

  if (item.type === "agentMessage") {
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

  return {
    ...item,
    presentation: buildAgentItemPresentation(item)
  };
}

function sanitizeAgentItemData(
  data: Record<string, unknown>,
  itemType: string,
  limitedOutput?: ReturnType<typeof limitAgentOutputTail>
) {
  const base = {
    id: readString(data.id),
    type: readString(data.type) ?? itemType,
    status: readString(data.status)
  };

  if (itemType === "agentMessage") {
    return {
      ...base,
      text: limitAgentText(readString(data.text), AGENT_MESSAGE_TEXT_LIMIT),
      phase: data.phase
    };
  }
  if (itemType === "plan") {
    return { ...base, text: limitAgentText(readString(data.text), AGENT_MESSAGE_TEXT_LIMIT) };
  }
  if (itemType === "reasoning") {
    const summary = readStringArray(data.summary)
      .slice(0, AGENT_REASONING_SUMMARY_PART_LIMIT)
      .map((part) => limitAgentText(part, AGENT_TEXT_PREVIEW_LIMIT) ?? "");
    const contentCount = readNumber(data.contentCount)
      ?? (Array.isArray(data.content) ? data.content.length : 0);
    return {
      ...base,
      summary,
      rawTextAvailable: readBoolean(data.rawTextAvailable)
        ?? contentCount > 0,
      contentCount
    };
  }
  if (itemType === "commandExecution") {
    return {
      ...base,
      command: limitAgentText(readString(data.command), AGENT_TEXT_PREVIEW_LIMIT),
      cwd: limitAgentText(readString(data.cwd), AGENT_TEXT_PREVIEW_LIMIT),
      exitCode: readNumber(data.exitCode),
      durationMs: readNumber(data.durationMs),
      aggregatedOutput: limitedOutput?.outputTail,
      aggregatedOutputTruncated: (limitedOutput?.truncated ?? false)
        || (readBoolean(data.aggregatedOutputTruncated) ?? false)
    };
  }
  if (itemType === "fileChange") {
    return { ...base, changes: Array.isArray(data.changes) ? data.changes : [] };
  }
  if (itemType === "mcpToolCall") {
    return {
      ...base,
      server: limitAgentText(readString(data.server)),
      tool: limitAgentText(readString(data.tool)),
      durationMs: readNumber(data.durationMs),
      argumentsPreview: buildStructuredPreview(data.arguments),
      resultPreview: buildStructuredPreview(data.result),
      errorMessage: limitAgentSensitiveText(readString(readRecord(data.error)?.message)),
      appName: limitAgentText(readString(readRecord(data.appContext)?.appName))
    };
  }
  if (itemType === "dynamicToolCall") {
    return {
      ...base,
      namespace: limitAgentText(readString(data.namespace)),
      tool: limitAgentText(readString(data.tool)),
      durationMs: readNumber(data.durationMs),
      success: readBoolean(data.success),
      argumentsPreview: buildStructuredPreview(data.arguments),
      resultPreview: buildStructuredPreview(data.contentItems)
    };
  }
  if (itemType === "collabAgentToolCall") {
    return {
      ...base,
      tool: limitAgentText(readString(data.tool)),
      senderThreadId: limitAgentText(readString(data.senderThreadId)),
      receiverThreadIds: readStringArray(data.receiverThreadIds)
        .slice(0, 30)
        .map((value) => limitAgentText(value) ?? ""),
      promptPreview: buildTextPreview(readString(data.prompt)),
      model: limitAgentText(readString(data.model)),
      reasoningEffort: limitAgentText(readString(data.reasoningEffort)),
      agentsStates: sanitizeAgentStates(data.agentsStates)
    };
  }
  if (itemType === "subAgentActivity") {
    return {
      ...base,
      kind: limitAgentText(readString(data.kind)),
      agentThreadId: limitAgentText(readString(data.agentThreadId)),
      agentPath: limitAgentText(readString(data.agentPath))
    };
  }
  if (itemType === "webSearch") {
    return {
      ...base,
      query: limitAgentText(readString(data.query)),
      action: sanitizeWebSearchAction(data.action)
    };
  }
  if (itemType === "imageView") {
    return { ...base, path: limitAgentText(readString(data.path), AGENT_TEXT_PREVIEW_LIMIT) };
  }
  if (itemType === "imageGeneration") {
    const resultLength = readNumber(data.resultLength)
      ?? (typeof data.result === "string" ? data.result.length : 0);
    return {
      ...base,
      revisedPromptPreview: buildTextPreview(readString(data.revisedPrompt)),
      savedPath: limitAgentText(readString(data.savedPath), AGENT_TEXT_PREVIEW_LIMIT),
      resultAvailable: readBoolean(data.resultAvailable)
        ?? resultLength > 0,
      resultLength
    };
  }
  if (itemType === "sleep") {
    return { ...base, durationMs: readNumber(data.durationMs) };
  }
  if (itemType === "enteredReviewMode" || itemType === "exitedReviewMode") {
    return { ...base, reviewPreview: buildTextPreview(readString(data.review)) };
  }
  if (itemType === "contextCompaction" || itemType === "userMessage") {
    return base;
  }
  if (itemType === "hookPrompt") {
    return { ...base, fragmentsPreview: buildStructuredPreview(data.fragments) };
  }

  return {
    ...base,
    detailPreview: buildStructuredPreview(data)
  };
}

function buildAgentItemPresentation(item: AgentItem): AgentItemPresentation | undefined {
  const status = readString(item.data.status)
    ?? item.status
    ?? (item.lifecycle === "completed" ? "completed" : "inProgress");

  if (item.type === "mcpToolCall" || item.type === "dynamicToolCall") {
    return {
      kind: "toolCall",
      toolKind: item.type === "mcpToolCall" ? "mcp" : "dynamic",
      server: readString(item.data.server),
      namespace: readString(item.data.namespace),
      tool: readString(item.data.tool) ?? "unknown",
      status,
      durationMs: readNumber(item.data.durationMs),
      progress: item.progressMessage,
      success: readBoolean(item.data.success),
      arguments: readStructuredPreview(item.data.argumentsPreview),
      result: readStructuredPreview(item.data.resultPreview),
      error: readString(item.data.errorMessage)
    };
  }
  if (item.type === "collabAgentToolCall") {
    return {
      kind: "collaboration",
      activityKind: "toolCall",
      tool: readString(item.data.tool),
      status,
      receiverThreadIds: readStringArray(item.data.receiverThreadIds),
      prompt: readStructuredPreview(item.data.promptPreview),
      agentStates: readAgentStates(item.data.agentsStates)
    };
  }
  if (item.type === "subAgentActivity") {
    return {
      kind: "collaboration",
      activityKind: "subAgent",
      status: readString(item.data.kind) ?? status,
      receiverThreadIds: [],
      agentThreadId: readString(item.data.agentThreadId),
      agentPath: readString(item.data.agentPath),
      agentStates: []
    };
  }
  if (item.type === "webSearch") {
    const action = readRecord(item.data.action);
    return {
      kind: "webSearch",
      action: readString(action?.type) ?? "search",
      query: readString(action?.query)
        ?? readStringArray(action?.queries)[0]
        ?? readString(item.data.query),
      url: readString(action?.url),
      pattern: readString(action?.pattern)
    };
  }
  if (item.type === "imageView" || item.type === "imageGeneration") {
    return {
      kind: "image",
      activityKind: item.type === "imageView" ? "view" : "generation",
      status,
      path: readString(item.data.path),
      savedPath: readString(item.data.savedPath),
      revisedPrompt: readStructuredPreview(item.data.revisedPromptPreview),
      resultAvailable: readBoolean(item.data.resultAvailable) ?? false
    };
  }

  const statusActivityKinds: Record<string, Extract<AgentItemPresentation, { kind: "status" }>["activityKind"]> = {
    contextCompaction: "contextCompaction",
    sleep: "sleep",
    enteredReviewMode: "reviewMode",
    exitedReviewMode: "reviewMode",
    userMessage: "userMessage",
    hookPrompt: "hookPrompt"
  };
  const activityKind = statusActivityKinds[item.type] ?? "generic";
  const details = readStructuredPreview(item.data.detailPreview)
    ?? readStructuredPreview(item.data.fragmentsPreview)
    ?? readStructuredPreview(item.data.reviewPreview);
  return {
    kind: "status",
    activityKind,
    status,
    label: item.type,
    durationMs: readNumber(item.data.durationMs),
    details
  };
}

function applyAgentHookEvent(
  existing: AgentHookRun | undefined,
  event: Extract<AgentEvent, { type: "hook_run_updated" }>
): AgentHookRun {
  if (existing?.lifecycle === "completed" && event.lifecycle === "in_progress") {
    return existing;
  }

  const run = event.run;
  const completed = event.lifecycle === "completed";
  return {
    id: event.hookId,
    threadId: event.threadId,
    turnId: event.turnId,
    lifecycle: event.lifecycle,
    displayOrder: readNumber(run.displayOrder) ?? existing?.displayOrder,
    eventName: limitAgentText(readString(run.eventName), 100) ?? existing?.eventName ?? "unknown",
    handlerType: limitAgentText(readString(run.handlerType), 100) ?? existing?.handlerType,
    executionMode: limitAgentText(readString(run.executionMode), 100) ?? existing?.executionMode,
    scope: limitAgentText(readString(run.scope), 100) ?? existing?.scope,
    source: limitAgentText(readString(run.source), 100) ?? existing?.source,
    sourcePath: limitAgentText(readString(run.sourcePath), AGENT_TEXT_PREVIEW_LIMIT) ?? existing?.sourcePath,
    status: readString(run.status) ?? existing?.status ?? (completed ? "completed" : "running"),
    statusMessage: limitAgentSensitiveText(readString(run.statusMessage), AGENT_TEXT_PREVIEW_LIMIT) ?? existing?.statusMessage,
    durationMs: readNumber(run.durationMs) ?? existing?.durationMs,
    entries: readAgentHookEntries(run.entries, completed ? [] : existing?.entries ?? []),
    restrictedDebugAvailable: readBoolean(run._uiProjectionTruncated)
      || existing?.restrictedDebugAvailable
      || false,
    startedAt: readAgentProtocolTimestamp(run.startedAt) ?? existing?.startedAt ?? event.createdAt,
    updatedAt: event.createdAt,
    completedAt: completed
      ? readAgentProtocolTimestamp(run.completedAt) ?? event.createdAt
      : existing?.completedAt
  };
}

function compareAgentHooks(left: AgentHookRun | undefined, right: AgentHookRun | undefined) {
  if (!left || !right) {
    return left ? -1 : right ? 1 : 0;
  }
  const orderDifference = (left.displayOrder ?? Number.MAX_SAFE_INTEGER)
    - (right.displayOrder ?? Number.MAX_SAFE_INTEGER);
  if (orderDifference !== 0) {
    return orderDifference;
  }
  const timeDifference = Date.parse(left.startedAt) - Date.parse(right.startedAt);
  return Number.isFinite(timeDifference) && timeDifference !== 0
    ? timeDifference
    : left.id.localeCompare(right.id);
}

function readAgentProtocolTimestamp(value: unknown) {
  if (typeof value === "number" && Number.isFinite(value)) {
    const timestamp = new Date(value);
    return Number.isFinite(timestamp.getTime()) ? timestamp.toISOString() : undefined;
  }
  if (typeof value === "string") {
    const parsed = Date.parse(value);
    return Number.isFinite(parsed) ? new Date(parsed).toISOString() : undefined;
  }
  return undefined;
}

function readAgentHookEntries(value: unknown, fallback: AgentHookRun["entries"]) {
  if (!Array.isArray(value)) {
    return fallback;
  }
  return value.slice(0, 50).flatMap((entry) => {
    const record = readRecord(entry);
    const text = limitAgentSensitiveText(readString(record?.text), AGENT_HOOK_ENTRY_LIMIT);
    if (!record || !text) {
      return [];
    }
    return [{
      kind: limitAgentText(readString(record.kind), 100) ?? "context",
      text
    }];
  });
}

function normalizeTokenUsageBreakdown(value: AgentTokenUsageBreakdown): AgentTokenUsageBreakdown {
  return {
    totalTokens: readNumber(value.totalTokens) ?? 0,
    inputTokens: readNumber(value.inputTokens) ?? 0,
    cachedInputTokens: readNumber(value.cachedInputTokens) ?? 0,
    outputTokens: readNumber(value.outputTokens) ?? 0,
    reasoningOutputTokens: readNumber(value.reasoningOutputTokens) ?? 0
  };
}

function upsertAgentWarning(
  warnings: AgentRun["warnings"],
  warning: AgentRun["warnings"][number]
) {
  const index = warnings.findIndex((candidate) =>
    (candidate.source ?? "runtime") === (warning.source ?? "runtime") &&
    candidate.message === warning.message &&
    candidate.path === warning.path
  );
  if (index < 0) {
    return [...warnings, { ...warning, count: warning.count ?? 1 }];
  }

  const next = [...warnings];
  next[index] = {
    ...next[index],
    ...warning,
    count: (next[index].count ?? 1) + 1,
    updatedAt: warning.createdAt
  };
  return next;
}

function buildStructuredPreview(
  value: unknown,
  limit = AGENT_STRUCTURED_PREVIEW_LIMIT
): AgentStructuredPreview | undefined {
  if (value === undefined || value === null) {
    return undefined;
  }
  const serialized = JSON.stringify(redactAgentValue(value, 0, {
    nodesRemaining: AGENT_STRUCTURED_PREVIEW_MAX_NODES,
    sourceCharsRemaining: AGENT_STRUCTURED_PREVIEW_MAX_SOURCE_CHARS
  }), null, 2);
  if (!serialized) {
    return undefined;
  }
  const sourceRestricted = /(?:_uiProjectionTruncated|\[truncated\]|omitted|redacted|additional (?:items|fields))/i.test(serialized);
  return {
    text: serialized.length > limit ? `${serialized.slice(0, limit)}\n…` : serialized,
    truncated: serialized.length > limit || sourceRestricted
  };
}

function buildTextPreview(
  value: string | undefined,
  limit = AGENT_TEXT_PREVIEW_LIMIT
): AgentStructuredPreview | undefined {
  if (!value) {
    return undefined;
  }
  const safeValue = redactCredentialLikeAgentText(value);
  const restricted = safeValue !== value || /redacted credential or binary data/i.test(value);
  return {
    text: safeValue.length > limit ? `${safeValue.slice(0, limit)}…` : safeValue,
    truncated: restricted || safeValue.length > limit
  };
}

function redactAgentValue(
  value: unknown,
  depth: number,
  budget: { nodesRemaining: number; sourceCharsRemaining: number }
): unknown {
  if (budget.nodesRemaining <= 0) {
    return "[preview budget exhausted]";
  }
  budget.nodesRemaining -= 1;
  if (depth >= 6) {
    return "[nested data omitted]";
  }
  if (typeof value === "string") {
    if (CREDENTIAL_AGENT_TEXT_PATTERN.test(value)) {
      return "[credential-like text redacted]";
    }
    if (value.startsWith("data:image/") || value.length > 20_000) {
      return `[large string omitted: ${value.length} chars]`;
    }
    const retainedLength = Math.min(value.length, Math.max(0, budget.sourceCharsRemaining));
    budget.sourceCharsRemaining -= retainedLength;
    return retainedLength < value.length
      ? `${value.slice(0, retainedLength)}…`
      : value;
  }
  if (Array.isArray(value)) {
    const items: unknown[] = [];
    const limit = Math.min(value.length, AGENT_STRUCTURED_PREVIEW_MAX_COLLECTION_ENTRIES);
    for (let index = 0; index < limit && budget.nodesRemaining > 0; index += 1) {
      items.push(redactAgentValue(value[index], depth + 1, budget));
    }
    return value.length > items.length ? [...items, `[${value.length - items.length} more items]`] : items;
  }
  const record = readRecord(value);
  if (!record) {
    return value;
  }

  const redacted: Record<string, unknown> = {};
  let entryCount = 0;
  let omitted = false;
  for (const key in record) {
    if (!Object.prototype.hasOwnProperty.call(record, key)) {
      continue;
    }
    if (
      entryCount >= AGENT_STRUCTURED_PREVIEW_MAX_COLLECTION_ENTRIES ||
      budget.nodesRemaining <= 0
    ) {
      omitted = true;
      break;
    }
    const outputKey = key.length > AGENT_STRUCTURED_PREVIEW_MAX_KEY_LENGTH
      ? `${key.slice(0, AGENT_STRUCTURED_PREVIEW_MAX_KEY_LENGTH)}…`
      : key;
    const entryValue = record[key];
    redacted[outputKey] = SENSITIVE_AGENT_FIELD_PATTERN.test(key)
      ? "[REDACTED]"
      : typeof entryValue === "string" && ENCODED_AGENT_FIELD_PATTERN.test(key) && looksLikeBase64(entryValue)
        ? `[encoded payload omitted: ${entryValue.length} chars]`
        : redactAgentValue(entryValue, depth + 1, budget);
    entryCount += 1;
  }
  if (omitted) {
    redacted._omitted = "additional fields omitted";
  }
  return redacted;
}

function looksLikeBase64(value: string) {
  const compact = value.trim();
  return compact.length >= 64
    && compact.length % 4 === 0
    && /^[A-Za-z0-9+/]+={0,2}$/.test(compact);
}

function sanitizeAgentStates(value: unknown) {
  const record = readRecord(value);
  if (!record) {
    return {};
  }
  return Object.fromEntries(Object.entries(record).slice(0, 30).map(([threadId, state]) => {
    const stateRecord = readRecord(state);
    return [limitAgentText(threadId, 200) ?? "unknown", {
      status: limitAgentText(readString(stateRecord?.status), 100) ?? "unknown",
      message: limitAgentSensitiveText(readString(stateRecord?.message), AGENT_TEXT_PREVIEW_LIMIT)
    }];
  }));
}

function sanitizeAgentServerRequestDetails(
  value: unknown
): AgentServerRequest["details"] {
  const details = readRecord(value) ?? {};
  const questions = Array.isArray(details.questions)
    ? details.questions.slice(0, 10).flatMap((candidate) => {
        const question = readRecord(candidate);
        const id = limitAgentText(readString(question?.id), 200);
        const prompt = limitAgentSensitiveText(
          readString(question?.question),
          AGENT_TEXT_PREVIEW_LIMIT
        );
        if (!id || !prompt) {
          return [];
        }
        const options = Array.isArray(question?.options)
          ? question.options.slice(0, 10).flatMap((candidateOption) => {
              const option = readRecord(candidateOption);
              const label = limitAgentSensitiveText(readString(option?.label), 300);
              if (!label) {
                return [];
              }
              return [{
                label,
                description: limitAgentSensitiveText(
                  readString(option?.description),
                  AGENT_TEXT_PREVIEW_LIMIT
                ) ?? ""
              }];
            })
          : undefined;
        return [{
          id,
          header: limitAgentSensitiveText(readString(question?.header), 200) ?? "需要确认",
          question: prompt,
          isOther: readBoolean(question?.isOther) ?? false,
          isSecret: readBoolean(question?.isSecret) ?? false,
          options: options?.length ? options : undefined
        }];
      })
    : undefined;
  const sanitizeRecord = (record: unknown) => readRecord(redactAgentValue(record, 0, {
    nodesRemaining: AGENT_STRUCTURED_PREVIEW_MAX_NODES,
    sourceCharsRemaining: AGENT_STRUCTURED_PREVIEW_MAX_SOURCE_CHARS
  }));

  return {
    command: limitAgentSensitiveText(readString(details.command), AGENT_TEXT_PREVIEW_LIMIT),
    cwd: limitAgentText(readString(details.cwd), AGENT_TEXT_PREVIEW_LIMIT),
    reason: limitAgentSensitiveText(readString(details.reason), AGENT_TEXT_PREVIEW_LIMIT),
    grantRoot: limitAgentText(readString(details.grantRoot), AGENT_TEXT_PREVIEW_LIMIT),
    permissions: sanitizeRecord(details.permissions),
    questions,
    autoResolutionMs: readNumber(details.autoResolutionMs),
    serverName: limitAgentText(readString(details.serverName), 300),
    mode: limitAgentText(readString(details.mode), 100),
    message: limitAgentSensitiveText(readString(details.message), AGENT_TEXT_PREVIEW_LIMIT),
    url: limitAgentText(readString(details.url), AGENT_TEXT_PREVIEW_LIMIT),
    elicitationId: limitAgentText(readString(details.elicitationId), 300),
    requestedSchema: sanitizeRecord(details.requestedSchema)
  };
}

function readAgentStates(value: unknown) {
  const record = readRecord(value);
  if (!record) {
    return [];
  }
  return Object.entries(record).map(([threadId, state]) => {
    const stateRecord = readRecord(state);
    return {
      threadId,
      status: readString(stateRecord?.status) ?? "unknown",
      message: readString(stateRecord?.message)
    };
  });
}

function sanitizeWebSearchAction(value: unknown) {
  const action = readRecord(value);
  if (!action) {
    return undefined;
  }
  return {
    type: readString(action.type) ?? "other",
    query: limitAgentText(readString(action.query)),
    queries: readStringArray(action.queries).slice(0, 20).map((query) => limitAgentText(query) ?? ""),
    url: limitAgentText(readString(action.url), AGENT_TEXT_PREVIEW_LIMIT),
    pattern: limitAgentText(readString(action.pattern))
  };
}

function readStructuredPreview(value: unknown): AgentStructuredPreview | undefined {
  const record = readRecord(value);
  const text = readString(record?.text);
  return text ? { text, truncated: readBoolean(record?.truncated) ?? false } : undefined;
}

function hasRestrictedAgentData(data: Record<string, unknown>, itemType: string) {
  return itemType === "reasoning" && (
    readBoolean(data.rawTextAvailable)
      ?? (readNumber(data.contentCount) ?? (Array.isArray(data.content) ? data.content.length : 0)) > 0
  );
}

function joinReasoningSummary(parts: string[]) {
  return parts.filter(Boolean).join("\n\n").trim();
}

function limitAgentText(value: string | undefined, limit = AGENT_STRUCTURED_PREVIEW_LIMIT) {
  return value && value.length > limit ? `${value.slice(0, limit)}…` : value;
}

function limitAgentSensitiveText(value: string | undefined, limit = AGENT_STRUCTURED_PREVIEW_LIMIT) {
  return value === undefined
    ? undefined
    : limitAgentText(redactCredentialLikeAgentText(value), limit);
}

function redactCredentialLikeAgentText(value: string) {
  return CREDENTIAL_AGENT_TEXT_PATTERN.test(value)
    ? "[credential-like text redacted]"
    : value;
}

function readStringArray(value: unknown) {
  return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === "string") : [];
}

function readBoolean(value: unknown) {
  return typeof value === "boolean" ? value : undefined;
}

function readInteger(value: unknown) {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : undefined;
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

function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
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

function dispatchProjectFilesChanged(
  projectPath: string,
  changedFiles: string[],
  refreshMode: "incremental" | "full"
) {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(new CustomEvent("voicecoder:project-files-changed", {
    detail: {
      projectPath,
      changedPath: resolveChangedPath(projectPath, changedFiles[0]),
      changedPaths: changedFiles.map((path) => resolveChangedPath(projectPath, path)),
      refreshMode
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
