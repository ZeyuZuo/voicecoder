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

