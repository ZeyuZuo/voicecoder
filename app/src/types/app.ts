export type Project = {
  id: string;
  name: string;
  path: string;
  lastActivity: string;
};

export type Conversation = {
  id: string;
  title: string;
  projectId?: string;
  lastActivity: string;
};

export type WorkspaceMode = "launcher" | "files" | "browser" | "review" | "terminal";

export type ComposerMode = "idle" | "project-menu-open" | "submitting";

export type WorkspaceTabKind = "files" | "browser" | "review" | "terminal";

export type WorkspaceTab = {
  id: string;
  kind: WorkspaceTabKind;
  title: string;
};

export type FileTreeEntry = {
  name: string;
  path: string;
  isDirectory: boolean;
  children?: FileTreeEntry[];
};

export type BrowserDirectoryProject = {
  name: string;
  path: string;
  handle: FileSystemDirectoryHandle;
};

export type BrowserFileSystemEntry = FileSystemFileHandle | FileSystemDirectoryHandle;
