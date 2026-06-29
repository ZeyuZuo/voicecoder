import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useReducer, useRef } from "react";
import type {
  AgentEvent,
  AgentRun,
  AgentRunKind,
  CodingAgentProviderKind,
  DevServerLifecycleEventEnvelope,
  DemoFeedbackTurn,
  DemoSession,
  RequirementState,
  RequirementUtterance,
  StartDevServerRequest
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
      now: string;
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
  startInitialRun: () => void;
  startFeedbackListening: () => void;
};

export type UseDemoSessionOptions = {
  onPreviewReady?: (url: string) => void;
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
  completedAt: string;
};

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
              status: "running"
            }
          : candidate
      ),
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
  const devServerTimeoutRef = useRef<ReturnType<typeof setTimeout> | undefined>();

  useEffect(() => {
    onPreviewReadyRef.current = options.onPreviewReady;
  }, [options.onPreviewReady]);

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

  useEffect(() => {
    if (!isTauri() || !session) {
      return;
    }

    const sessionId = session.id;
    const sessionProjectPath = session.projectPath;
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
          now: event.payload.startedAt
        });
      }),
      listen<AgentEventPayload>("agent://event", (event) => {
        if (event.payload.demoSessionId !== sessionId) {
          return;
        }

        dispatch({
          type: "append_agent_event",
          runId: event.payload.runId,
          event: event.payload.event,
          now: event.payload.event.createdAt
        });
      }),
      listen<AgentRunCompletedPayload>("agent://run-completed", (event) => {
        if (event.payload.demoSessionId !== sessionId) {
          return;
        }

        dispatch({
          type: "complete_agent_run",
          runId: event.payload.runId,
          finalMessage: event.payload.finalMessage,
          changedFiles: event.payload.changedFiles,
          now: event.payload.completedAt
        });
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

        dispatch({
          type: "fail_agent_run",
          runId: event.payload.runId,
          error: event.payload.message,
          now: event.payload.occurredAt
        });
      })
    ];

    return () => {
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
      startInitialRun,
      startFeedbackListening
    }),
    [session, startInitialRun, startFeedbackListening]
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
    sessionId: `dev_server_${demoSessionId}`
  };

  void invoke("start_demo_dev_server", { request }).catch((error) => {
    onError(`启动 dev server 失败：${stringifyError(error)}`);
  });
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
