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
  | "listening"
  | "finalizing"
  | "document_ready"
  | "collecting"
  | "processing"
  | "clarifying"
  | "ready_to_confirm"
  | "confirmed";

export type RequirementPendingAction = "summarize" | "process" | "finalize" | "save";

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

export type RequirementGap = {
  id: string;
  question: string;
  reason: string;
  severity: "blocking" | "helpful";
  status: "open" | "resolved";
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
  confirmedFacts: string[];
  constraints: string[];
  acceptanceCriteria: string[];
  outOfScope: string[];
  risks: string[];
  openGaps: Array<Omit<RequirementGap, "id" | "status"> & { id?: string; status?: RequirementGap["status"] }>;
  uncertainties?: string[];
};

export type RequirementState = {
  id: string;
  status: RequirementStatus;
  utterances: RequirementUtterance[];
  summary: string;
  requirementDocument: string;
  confirmedFacts: string[];
  constraints: string[];
  openGaps: RequirementGap[];
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

export type CodingAgentProviderKind = "auto" | "codex_app_server" | "codex_exec_json";

export type CodingAgentProviderStatus = {
  autoProvider: Exclude<CodingAgentProviderKind, "auto">;
  providerOverride?: CodingAgentProviderKind;
  activeProviderConfigured: boolean;
  activeProviderError?: string;
  diagnostics: CodingAgentProviderDiagnostic[];
};

export type CodingAgentProviderDiagnostic = {
  provider: Exclude<CodingAgentProviderKind, "auto">;
  configured: boolean;
  missingDependencies: string[];
  executable?: string;
  version?: string;
  details: Record<string, string>;
  error?: string;
};

export type DemoSessionStatus =
  | "idle"
  | "ready_to_start"
  | "agent_running"
  | "preview_ready"
  | "feedback_listening"
  | "feedback_processing"
  | "agent_modifying"
  | "error";

export type AgentRunKind = "initial_build" | "feedback_change";

export type AgentRunStatus = "queued" | "starting" | "running" | "succeeded" | "failed" | "cancelled";

export type AgentEvent =
  | {
      type: "thread_started";
      threadId: string;
      createdAt: string;
    }
  | {
      type: "turn_started";
      turnId?: string;
      createdAt: string;
    }
  | {
      type: "agent_message";
      text: string;
      createdAt: string;
    }
  | {
      type: "plan_update";
      text: string;
      createdAt: string;
    }
  | {
      type: "command";
      command: string;
      status: string;
      createdAt: string;
    }
  | {
      type: "file_change";
      path: string;
      changeType?: string;
      createdAt: string;
    }
  | {
      type: "turn_completed";
      finalMessage?: string;
      createdAt: string;
    }
  | {
      type: "error";
      message: string;
      createdAt: string;
    };

export type AgentRun = {
  id: string;
  kind: AgentRunKind;
  prompt: string;
  status: AgentRunStatus;
  codexThreadId?: string;
  codexTurnId?: string;
  events: AgentEvent[];
  changedFiles: string[];
  finalMessage?: string;
  error?: string;
  startedAt?: string;
  completedAt?: string;
};

export type DemoFeedbackTurn = {
  id: string;
  utterances: RequirementUtterance[];
  summary: string;
  modificationPrompt: string;
  linkedAgentRunId?: string;
  createdAt: string;
};

export type DemoSession = {
  id: string;
  projectPath: string;
  requirementId: string;
  initialRequirementDocument: string;
  initialCodingPrompt: string;
  codexThreadId?: string;
  runs: AgentRun[];
  feedbackTurns: DemoFeedbackTurn[];
  currentPreviewUrl?: string;
  status: DemoSessionStatus;
  error?: string;
  createdAt: string;
  updatedAt: string;
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
