//! Recovery and compatibility normalization for persisted DemoSession snapshots.

import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type {
  AgentEvent,
  AgentItem,
  AgentRun,
  AgentRunKind,
  DemoFeedbackTurn,
  DemoSession
} from "../types/app";
import { EMPTY_AGENT_DIFF_STATS } from "./agentProgress";

type LoadedDemoSessionLog = {
  schemaVersion: number;
  savedAt: string;
  path: string;
  demoSession: unknown;
};

type RestoreDemoSession = (action: {
  type: "restore_demo_session";
  session: DemoSession;
}) => void;

export function usePersistedDemoSessionRecovery(
  projectPath: string | undefined,
  restore: RestoreDemoSession,
  applyAgentEvent: (run: AgentRun, event: AgentEvent) => AgentRun
) {
  const tauriRuntime = isTauri();
  const [recoveryState, setRecoveryState] = useState<{
    projectPath?: string;
    loaded: boolean;
  }>({ loaded: !tauriRuntime });

  useEffect(() => {
    let cancelled = false;
    if (!projectPath || !tauriRuntime) {
      setRecoveryState({ projectPath, loaded: true });
      return () => {
        cancelled = true;
      };
    }

    setRecoveryState({ projectPath, loaded: false });
    void invoke<LoadedDemoSessionLog | null>("load_latest_demo_session_log", { projectPath })
      .then(async (loaded) => {
        if (!loaded || cancelled) {
          return;
        }
        const activeRunIds = new Set<string>();
        await Promise.all(
          getRecoverableActiveRunIds(loaded.demoSession).map(async (runId) => {
            const active = await invoke<boolean>("is_coding_agent_run_active", { runId })
              .catch(() => false);
            if (active) {
              activeRunIds.add(runId);
            }
          })
        );
        if (cancelled) {
          return;
        }
        const recovered = recoverPersistedDemoSessionSnapshot(
          loaded.demoSession,
          activeRunIds,
          new Date().toISOString(),
          applyAgentEvent
        );
        if (recovered?.projectPath === projectPath) {
          restore({ type: "restore_demo_session", session: recovered });
        }
      })
      .catch((error) => {
        console.warn("Failed to recover the latest DemoSession log.", error);
      })
      .finally(() => {
        if (!cancelled) {
          setRecoveryState({ projectPath, loaded: true });
        }
      });

    return () => {
      cancelled = true;
    };
  }, [applyAgentEvent, projectPath, restore, tauriRuntime]);

  return !tauriRuntime ||
    (recoveryState.projectPath === projectPath && recoveryState.loaded);
}

const RECOVERED_RUN_INTERRUPTED_MESSAGE =
  "VoiceCoder 已从本地日志恢复时间线，但原 AgentRun 后端进程已结束，无法继续接收事件。";

export function getRecoverableActiveRunIds(snapshot: unknown): string[] {
  const session = readRecord(snapshot);
  if (!session || !Array.isArray(session.runs)) {
    return [];
  }
  return session.runs.flatMap((candidate) => {
    const run = readRecord(candidate);
    if (
      !run ||
      typeof run.id !== "string" ||
      typeof run.status !== "string" ||
      ["succeeded", "failed", "cancelled"].includes(run.status)
    ) {
      return [];
    }
    return [run.id];
  });
}

export function recoverPersistedDemoSessionSnapshot(
  snapshot: unknown,
  activeRunIds: ReadonlySet<string>,
  now: string,
  applyAgentEvent: (run: AgentRun, event: AgentEvent) => AgentRun
): DemoSession | undefined {
  const rawSession = readRecord(snapshot);
  if (
    !rawSession ||
    typeof rawSession.id !== "string" ||
    typeof rawSession.projectPath !== "string" ||
    typeof rawSession.requirementId !== "string" ||
    !Array.isArray(rawSession.runs)
  ) {
    return undefined;
  }

  let interruptedRun = false;
  const runs = rawSession.runs
    .map((candidate) => normalizeRecoveredAgentRun(candidate, applyAgentEvent))
    .filter((candidate): candidate is AgentRun => Boolean(candidate))
    .map((run) => {
      if (isTerminalRunStatus(run.status) || activeRunIds.has(run.id)) {
        return run;
      }
      interruptedRun = true;
      return settlePendingServerRequests(
        {
          ...run,
          status: "failed",
          error: RECOVERED_RUN_INTERRUPTED_MESSAGE,
          completedAt: now
        },
        now,
        "页面恢复时 AgentRun 已结束，未决请求已取消。"
      );
    });
  const lastThreadId = [...runs]
    .reverse()
    .find((run) => run.codexThreadId)?.codexThreadId;
  const recovered = {
    ...(rawSession as unknown as DemoSession),
    id: rawSession.id,
    projectPath: rawSession.projectPath,
    requirementId: rawSession.requirementId,
    initialRequirementDocument:
      typeof rawSession.initialRequirementDocument === "string"
        ? rawSession.initialRequirementDocument
        : "",
    initialCodingPrompt:
      typeof rawSession.initialCodingPrompt === "string" ? rawSession.initialCodingPrompt : "",
    codexThreadId:
      typeof rawSession.codexThreadId === "string" ? rawSession.codexThreadId : lastThreadId,
    runs,
    feedbackTurns: Array.isArray(rawSession.feedbackTurns) ? rawSession.feedbackTurns as DemoFeedbackTurn[] : [],
    status: typeof rawSession.status === "string" ? rawSession.status as DemoSession["status"] : "error",
    createdAt: typeof rawSession.createdAt === "string" ? rawSession.createdAt : now,
    updatedAt: typeof rawSession.updatedAt === "string" ? rawSession.updatedAt : now,
    recoveredAt: now
  } satisfies DemoSession;

  if (!interruptedRun) {
    return recovered;
  }
  return {
    ...recovered,
    status: "error",
    error: RECOVERED_RUN_INTERRUPTED_MESSAGE,
    updatedAt: now
  };
}

function normalizeRecoveredAgentRun(
  snapshot: unknown,
  applyAgentEvent: (run: AgentRun, event: AgentEvent) => AgentRun
): AgentRun | undefined {
  const rawRun = readRecord(snapshot);
  if (
    !rawRun ||
    typeof rawRun.id !== "string" ||
    typeof rawRun.kind !== "string" ||
    typeof rawRun.status !== "string"
  ) {
    return undefined;
  }

  const events = Array.isArray(rawRun.events)
    ? rawRun.events.filter(isRecoverableAgentEvent) as AgentEvent[]
    : [];
  const itemsById = (readRecord(rawRun.itemsById) ?? {}) as Record<string, AgentItem>;
  let run: AgentRun = {
    ...(rawRun as unknown as AgentRun),
    id: rawRun.id,
    kind: rawRun.kind as AgentRunKind,
    prompt: typeof rawRun.prompt === "string" ? rawRun.prompt : "",
    status: rawRun.status as AgentRun["status"],
    events,
    itemsById,
    itemOrder: Array.isArray(rawRun.itemOrder) ? rawRun.itemOrder.filter(isString) : [],
    messagesByItemId: (readRecord(rawRun.messagesByItemId) ?? {}) as Record<string, AgentItem>,
    filesByPath: (readRecord(rawRun.filesByPath) ?? {}) as AgentRun["filesByPath"],
    aggregateDiff: typeof rawRun.aggregateDiff === "string" ? rawRun.aggregateDiff : "",
    aggregateDiffStats: readAgentDiffStats(rawRun.aggregateDiffStats),
    hooksById: (readRecord(rawRun.hooksById) ?? {}) as AgentRun["hooksById"],
    hookOrder: Array.isArray(rawRun.hookOrder) ? rawRun.hookOrder.filter(isString) : [],
    serverRequestsById: (readRecord(rawRun.serverRequestsById) ?? {}) as NonNullable<AgentRun["serverRequestsById"]>,
    serverRequestOrder: Array.isArray(rawRun.serverRequestOrder)
      ? rawRun.serverRequestOrder.filter(isString)
      : [],
    pendingServerRequestIds: Array.isArray(rawRun.pendingServerRequestIds)
      ? rawRun.pendingServerRequestIds.filter(isString)
      : [],
    warnings: Array.isArray(rawRun.warnings) ? rawRun.warnings as AgentRun["warnings"] : [],
    errors: Array.isArray(rawRun.errors) ? rawRun.errors as AgentRun["errors"] : [],
    changedFiles: Array.isArray(rawRun.changedFiles) ? rawRun.changedFiles.filter(isString) : []
  };

  if (!Object.keys(itemsById).length && events.length) {
    run = events.reduce(applyAgentEvent, {
      ...run,
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
      changedFiles: []
    });
  }

  return run;
}

function isRecoverableAgentEvent(value: unknown): value is AgentEvent {
  const event = readRecord(value);
  return Boolean(event && typeof event.type === "string" && typeof event.createdAt === "string");
}

function readAgentDiffStats(value: unknown): AgentRun["aggregateDiffStats"] {
  const stats = readRecord(value);
  return stats &&
    typeof stats.additions === "number" &&
    typeof stats.deletions === "number" &&
    typeof stats.files === "number"
    ? {
        additions: stats.additions,
        deletions: stats.deletions,
        files: stats.files
      }
    : EMPTY_AGENT_DIFF_STATS;
}

function isString(value: unknown): value is string {
  return typeof value === "string";
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

function isTerminalRunStatus(status: AgentRun["status"]) {
  return status === "succeeded" || status === "failed" || status === "cancelled";
}

function readRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}
