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

export type VoiceSessionStatus = "idle" | "starting" | "requesting-permission" | "recording" | "transcribing" | "error";

export type VoiceTranscriptSegment = {
  id: string;
  sessionId: string;
  speakerId?: string;
  text: string;
  isFinal: boolean;
  startedAtMs?: number;
  endedAtMs?: number;
  createdAt: string;
};

export type VoiceProviderKind = "auto" | "mock" | "tencent" | "iflytek_llm" | "volcengine";

export type VoiceSessionStartedEvent = {
  sessionId: string;
  provider: VoiceProviderKind;
  startedAt: string;
};

export type VoiceTranscriptEvent = VoiceTranscriptSegment;

export type VoiceErrorEvent = {
  sessionId?: string;
  message: string;
  code?: string;
};

export type VoiceStoppedEvent = {
  sessionId?: string;
  reason: "user" | "completed" | "error";
  stoppedAt: string;
};

export type VoiceAudioChunkPayload = {
  sessionId: string;
  sampleRate: number;
  channels: number;
  format: "pcm_s16le";
  sequence: number;
  data: number[];
};

export type VoiceProviderStatus = {
  autoProvider: Exclude<VoiceProviderKind, "auto">;
  providerOverride?: VoiceProviderKind;
  diagnostics: VoiceProviderDiagnostic[];
};

export type VoiceProviderDiagnostic = {
  provider: Exclude<VoiceProviderKind, "auto">;
  configured: boolean;
  missingEnv: string[];
  endpoint?: string;
  details: Record<string, string>;
  error?: string;
};

export type VoiceSessionSnapshot = {
  active: boolean;
  sessionId?: string;
  provider?: Exclude<VoiceProviderKind, "auto">;
  receivedAudioChunks: number;
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
