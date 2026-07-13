import type { AgentDiffStats, AgentFileChange, AgentFileChangeKind, AgentRun } from "../types/app";

export const AGENT_COMMAND_OUTPUT_TAIL_LIMIT = 12_000;
export const AGENT_COMMAND_OUTPUT_PREVIEW_LINES = 6;
export const AGENT_TIMELINE_BOTTOM_THRESHOLD_PX = 48;
export const AGENT_WAITING_THRESHOLD_MS = 15_000;
export const EMPTY_AGENT_DIFF_STATS: AgentDiffStats = {
  additions: 0,
  deletions: 0,
  files: 0
};

export function appendAgentOutputTail(
  current: string | undefined,
  delta: string,
  wasTruncated = false
) {
  const combined = `${current ?? ""}${delta}`;
  const truncated = wasTruncated || combined.length > AGENT_COMMAND_OUTPUT_TAIL_LIMIT;
  return {
    outputTail: truncated ? combined.slice(-AGENT_COMMAND_OUTPUT_TAIL_LIMIT) : combined,
    truncated
  };
}

export function limitAgentOutputTail(output: string) {
  const truncated = output.length > AGENT_COMMAND_OUTPUT_TAIL_LIMIT;
  return {
    outputTail: truncated ? output.slice(-AGENT_COMMAND_OUTPUT_TAIL_LIMIT) : output,
    truncated
  };
}

export function getAgentOutputPreview(output: string | undefined) {
  if (!output) {
    return "";
  }

  const lines = output.split(/\r?\n/);
  while (lines[lines.length - 1] === "") {
    lines.pop();
  }
  return lines
    .slice(-AGENT_COMMAND_OUTPUT_PREVIEW_LINES)
    .join("\n")
    .trimEnd();
}

export function parseAgentFileChanges(data: Record<string, unknown>, itemId: string): AgentFileChange[] {
  if (!Array.isArray(data.changes)) {
    return [];
  }

  return data.changes.flatMap((change) => {
    const record = readRecord(change);
    const path = record ? readString(record.path) : undefined;
    if (!record || !path) {
      return [];
    }

    const kindRecord = readRecord(record.kind);
    const kind = normalizeFileChangeKind(readString(kindRecord?.type) ?? readString(record.kind));
    const movePath = readString(kindRecord?.move_path) ?? readString(kindRecord?.movePath);
    const diff = readString(record.diff) ?? "";
    const stats = countUnifiedDiffLines(diff);

    return [{
      itemId,
      path,
      kind,
      movePath,
      diff,
      additions: stats.additions,
      deletions: stats.deletions
    }];
  });
}

export function buildAgentFilesByPath(items: Iterable<{ type: string; fileChanges?: AgentFileChange[] }>) {
  const filesByPath: Record<string, AgentFileChange> = {};
  for (const item of items) {
    if (item.type !== "fileChange") {
      continue;
    }
    for (const change of item.fileChanges ?? []) {
      filesByPath[change.path] = change;
    }
  }
  return filesByPath;
}

export function parseUnifiedDiffStats(diff: string): AgentDiffStats {
  const paths = new Set<string>();
  let additions = 0;
  let deletions = 0;
  let oldPath: string | undefined;

  for (const line of diff.split(/\r?\n/)) {
    if (line.startsWith("diff --git ")) {
      const match = line.match(/^diff --git a\/(.+) b\/(.+)$/);
      if (match) {
        paths.add(match[2]);
      }
      oldPath = undefined;
      continue;
    }
    if (line.startsWith("--- ")) {
      oldPath = normalizeDiffPath(line.slice(4));
      continue;
    }
    if (line.startsWith("+++ ")) {
      const newPath = normalizeDiffPath(line.slice(4));
      const displayPath = newPath === "/dev/null" ? oldPath : newPath;
      if (displayPath && displayPath !== "/dev/null") {
        paths.add(displayPath);
      }
      continue;
    }
    if (line.startsWith("+") && !line.startsWith("+++")) {
      additions += 1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      deletions += 1;
    }
  }

  return {
    additions,
    deletions,
    files: paths.size
  };
}

export function countUnifiedDiffLines(diff: string) {
  let additions = 0;
  let deletions = 0;

  for (const line of diff.split(/\r?\n/)) {
    if (line.startsWith("+") && !line.startsWith("+++")) {
      additions += 1;
    } else if (line.startsWith("-") && !line.startsWith("---")) {
      deletions += 1;
    }
  }

  return { additions, deletions };
}

export function getCompletedFileChangePaths(itemType: string, item: Record<string, unknown>) {
  return itemType === "fileChange"
    ? parseAgentFileChanges(item, readString(item.id) ?? "completed-file-change").flatMap((change) =>
        change.movePath ? [change.movePath, change.path] : [change.path]
      )
    : [];
}

export function getAgentItemDurationMs(
  startedAt: string,
  completedAt: string | undefined,
  protocolDurationMs: number | undefined,
  nowMs: number
) {
  if (typeof protocolDurationMs === "number" && Number.isFinite(protocolDurationMs)) {
    return Math.max(0, protocolDurationMs);
  }

  const startedMs = Date.parse(startedAt);
  const endedMs = completedAt ? Date.parse(completedAt) : nowMs;
  return Number.isFinite(startedMs) && Number.isFinite(endedMs)
    ? Math.max(0, endedMs - startedMs)
    : undefined;
}

export function getAgentLatestProgressAt(run: AgentRun) {
  const candidates = [
    run.startedAt,
    run.completedAt,
    run.aggregateDiffUpdatedAt,
    run.currentPlan?.updatedAt,
    ...run.events.map((event) => event.createdAt),
    ...Object.values(run.itemsById ?? {}).map((item) => item.updatedAt),
    ...(run.warnings ?? []).map((warning) => warning.createdAt),
    ...(run.errors ?? []).map((error) => error.createdAt)
  ].filter((value): value is string => Boolean(value));

  let latest: string | undefined;
  let latestMs = Number.NEGATIVE_INFINITY;
  for (const candidate of candidates) {
    const candidateMs = Date.parse(candidate);
    if (Number.isFinite(candidateMs) && candidateMs >= latestMs) {
      latest = candidate;
      latestMs = candidateMs;
    }
  }
  return latest;
}

export function getAgentProgressAgeMs(progressAt: string | undefined, nowMs: number) {
  if (!progressAt) {
    return undefined;
  }
  const progressMs = Date.parse(progressAt);
  return Number.isFinite(progressMs) ? Math.max(0, nowMs - progressMs) : undefined;
}

export function isAgentTimelineNearBottom(
  scrollHeight: number,
  scrollTop: number,
  clientHeight: number,
  thresholdPx = AGENT_TIMELINE_BOTTOM_THRESHOLD_PX
) {
  return scrollHeight - scrollTop - clientHeight <= thresholdPx;
}

function normalizeFileChangeKind(value: string | undefined): AgentFileChangeKind {
  return value === "add" || value === "update" || value === "delete" ? value : "unknown";
}

function normalizeDiffPath(value: string) {
  const path = value.trim().split("\t", 1)[0];
  return path.startsWith("a/") || path.startsWith("b/") ? path.slice(2) : path;
}

function readRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

function readString(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
