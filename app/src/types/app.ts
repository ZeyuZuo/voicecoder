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

export type LlmProviderKind = "auto" | "openai_compatible";

export type LlmProviderStatus = {
  autoProvider: Exclude<LlmProviderKind, "auto">;
  providerOverride?: LlmProviderKind;
  activeProviderConfigured: boolean;
  activeProviderError?: string;
  diagnostics: LlmProviderDiagnostic[];
};

export type LlmProviderDiagnostic = {
  provider: Exclude<LlmProviderKind, "auto">;
  configured: boolean;
  missingEnv: string[];
  endpoint?: string;
  model?: string;
  details: Record<string, string>;
  error?: string;
};

export type LlmConnectionTestResult = {
  ok: boolean;
  provider: Exclude<LlmProviderKind, "auto">;
  model?: string;
  endpoint?: string;
  durationMs: number;
  response?: unknown;
  error?: string;
};

export type VoiceSessionSnapshot = {
  active: boolean;
  sessionId?: string;
  provider?: Exclude<VoiceProviderKind, "auto">;
  receivedAudioChunks: number;
};

export type RequirementStatus =
  | "idle"
  | "collecting"
  | "processing"
  | "clarifying"
  | "ready_to_confirm"
  | "confirmed";

export type RequirementPendingAction = "summarize" | "process" | "finalize";

export type RequirementUtteranceSource = "voice" | "clarification_answer";

export type RequirementUtterance = {
  id: string;
  source: RequirementUtteranceSource;
  speakerId?: string;
  text: string;
  createdAt: string;
  transcriptId?: string;
};

export type RequirementQuestion = {
  id: string;
  question: string;
  reason: string;
  blocksCoding: boolean;
  answer?: string;
};

export type RequirementProcessingResult = {
  summary: string;
  requirementDocumentDraft: string;
  confirmedFacts: string[];
  constraints: string[];
  acceptanceCriteria: string[];
  outOfScope: string[];
  risks: string[];
  questions: Array<Omit<RequirementQuestion, "id"> & { id?: string }>;
  readyToConfirm: boolean;
};

export type RequirementSummaryResult = {
  summary: string;
  uncertainties: string[];
};

export type RequirementState = {
  id: string;
  status: RequirementStatus;
  utterances: RequirementUtterance[];
  summary: string;
  requirementDocument: string;
  confirmedFacts: string[];
  constraints: string[];
  openQuestions: RequirementQuestion[];
  answeredQuestions: RequirementQuestion[];
  activeQuestionId?: string;
  acceptanceCriteria: string[];
  outOfScope: string[];
  risks: string[];
  codingPrompt?: string;
  savedRequirementDocumentPath?: string;
  pendingAction?: RequirementPendingAction;
  error?: string;
  updatedAt: string;
};

export type SavedRequirementDocument = {
  path: string;
};

export type VoiceRequirementSession = {
  id: string;
  voiceSessionIds: string[];
  requirementState: RequirementState;
  startedAt: string;
  endedAt?: string;
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
