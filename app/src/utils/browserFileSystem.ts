import type { BrowserDirectoryProject, BrowserFileSystemEntry, FileTreeEntry } from "../types/app";

const browserDirectoryHandles = new Map<string, FileSystemDirectoryHandle>();
const MAX_DEPTH = 3;
const MAX_ENTRIES_PER_DIR = 160;
const IGNORED_DIRECTORIES = new Set(["node_modules", ".git", "target", "dist", ".next"]);

export function registerBrowserDirectory(project: BrowserDirectoryProject) {
  browserDirectoryHandles.set(project.path, project.handle);
}

export function getBrowserDirectoryHandle(path: string) {
  return browserDirectoryHandles.get(path);
}

export async function readBrowserProjectTree(path: string): Promise<FileTreeEntry[] | undefined> {
  const handle = getBrowserDirectoryHandle(path);

  if (!handle) {
    return undefined;
  }

  return readBrowserDirectory(handle, path, 0);
}

export async function readBrowserGitBranch(path: string): Promise<string | undefined> {
  const handle = getBrowserDirectoryHandle(path);

  if (!handle) {
    return undefined;
  }

  try {
    const gitHandle = await handle.getDirectoryHandle(".git");
    const head = await readFileText(gitHandle, "HEAD");
    return parseGitHead(head);
  } catch {
    return undefined;
  }
}

async function readBrowserDirectory(handle: FileSystemDirectoryHandle, basePath: string, depth: number): Promise<FileTreeEntry[]> {
  const entries: FileTreeEntry[] = [];
  let count = 0;

  for await (const entry of readDirectoryEntries(handle)) {
    if (count >= MAX_ENTRIES_PER_DIR) {
      break;
    }

    if (entry.kind === "directory" && IGNORED_DIRECTORIES.has(entry.name)) {
      continue;
    }

    count += 1;

    const entryPath = `${basePath}/${entry.name}`;
    const isDirectory = entry.kind === "directory";
    const children = isDirectory && depth < MAX_DEPTH ? await readBrowserDirectory(entry, entryPath, depth + 1) : undefined;

    entries.push({
      name: entry.name,
      path: entryPath,
      isDirectory,
      children
    });
  }

  return entries.sort((left, right) => Number(right.isDirectory) - Number(left.isDirectory) || left.name.localeCompare(right.name));
}

async function readFileText(directory: FileSystemDirectoryHandle, name: string) {
  const fileHandle = await directory.getFileHandle(name);
  const file = await fileHandle.getFile();
  return file.text();
}

async function* readDirectoryEntries(directory: FileSystemDirectoryHandle): AsyncIterable<BrowserFileSystemEntry> {
  const iterableDirectory = directory as FileSystemDirectoryHandle & {
    values?: () => AsyncIterable<BrowserFileSystemEntry>;
  };

  if (!iterableDirectory.values) {
    return;
  }

  for await (const entry of iterableDirectory.values()) {
    yield entry;
  }
}

function parseGitHead(head: string) {
  const trimmedHead = head.trim();
  const branch = trimmedHead.replace(/^ref: refs\/heads\//, "");

  if (!trimmedHead) {
    return undefined;
  }

  if (branch !== trimmedHead) {
    return branch;
  }

  return trimmedHead.slice(0, 7);
}
