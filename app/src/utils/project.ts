export function getProjectName(path: string): string {
  const normalized = path.replace(/\\/g, "/").replace(/\/$/, "");
  const name = normalized.split("/").filter(Boolean).pop();
  return name || path;
}

export function shortPath(path: string): string {
  const normalized = path.replace(/\\/g, "/");
  const parts = normalized.split("/").filter(Boolean);

  if (parts.length <= 3) {
    return path;
  }

  return `.../${parts.slice(-3).join("/")}`;
}

export function createId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

