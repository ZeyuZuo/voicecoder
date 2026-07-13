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

export type WorkspaceMode = "launcher" | "demo" | "files" | "browser" | "review" | "terminal";

export type ComposerMode = "idle" | "project-menu-open" | "submitting";

export type WorkspaceTabKind = "demo" | "files" | "browser" | "review" | "terminal";

export type WorkspaceTab = {
  id: string;
  kind: WorkspaceTabKind;
  title: string;
};

export type BrowserPreviewState = {
  url?: string;
  updatedAt?: string;
};

export type DevServerSessionStatus = "idle" | "starting" | "running" | "ready" | "stopped" | "error";

export type DevServerOutputStream = "stdout" | "stderr";

export type DevServerStoppedReason = "exited" | "user" | "replaced" | "error";

export type DevServerLifecycleEvent =
  | {
      type: "starting";
      command: string[];
    }
  | {
      type: "output";
      stream: DevServerOutputStream;
      text: string;
    }
  | {
      type: "ready";
      url: string;
    }
  | {
      type: "stopped";
      reason: DevServerStoppedReason;
      exitCode?: number;
    }
  | {
      type: "error";
      message: string;
    };

export type DevServerLifecycleEventEnvelope = {
  sessionId: string;
  projectPath: string;
  event: DevServerLifecycleEvent;
  occurredAt: string;
};

export type DevServerSessionSnapshot = {
  id: string;
  projectPath: string;
  command: string[];
  status: DevServerSessionStatus;
  previewUrl?: string;
  startedAt?: string;
  updatedAt: string;
  error?: string;
};

export type DevServerDiagnostic = {
  configured: boolean;
  command: string[];
  executable?: string;
  version?: string;
  missingDependencies: string[];
  details: Record<string, string>;
  error?: string;
};

export type StartDevServerRequest = {
  projectPath: string;
  sessionId?: string;
  command?: string[];
};

export type StopDevServerRequest = {
  sessionId?: string;
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

export type CodingAgentRuntimeMetadata = {
  provider: Exclude<CodingAgentProviderKind, "auto">;
  version: string;
  transport: string;
  sandbox: string;
  approvalPolicy?: string;
  approvalsReviewer?: string;
  transportLogPath?: string;
};

export type AgentMessagePhase = "commentary" | "final_answer" | "unknown";

export type AgentItemLifecycle = "in_progress" | "completed";

export type AgentFileChangeKind = "add" | "update" | "delete" | "unknown";

export type AgentFileChange = {
  itemId: string;
  path: string;
  kind: AgentFileChangeKind;
  movePath?: string;
  diff: string;
  additions: number;
  deletions: number;
};

export type AgentCommandState = {
  command: string;
  cwd?: string;
  status: string;
  exitCode?: number;
  durationMs?: number;
  outputTail: string;
  outputTruncated: boolean;
};

export type AgentStructuredPreview = {
  text: string;
  truncated: boolean;
};

export type AgentItemPresentation =
  | {
      kind: "reasoning";
      summary: string;
      rawTextAvailable: boolean;
    }
  | {
      kind: "toolCall";
      toolKind: "mcp" | "dynamic";
      server?: string;
      namespace?: string;
      tool: string;
      status: string;
      durationMs?: number;
      progress?: string;
      success?: boolean;
      arguments?: AgentStructuredPreview;
      result?: AgentStructuredPreview;
      error?: string;
    }
  | {
      kind: "collaboration";
      activityKind: "toolCall" | "subAgent";
      tool?: string;
      status: string;
      receiverThreadIds: string[];
      agentThreadId?: string;
      agentPath?: string;
      prompt?: AgentStructuredPreview;
      agentStates: Array<{ threadId: string; status: string; message?: string }>;
    }
  | {
      kind: "webSearch";
      action: string;
      query?: string;
      url?: string;
      pattern?: string;
    }
  | {
      kind: "image";
      activityKind: "view" | "generation";
      status: string;
      path?: string;
      savedPath?: string;
      revisedPrompt?: AgentStructuredPreview;
      resultAvailable: boolean;
    }
  | {
      kind: "status";
      activityKind: "contextCompaction" | "sleep" | "reviewMode" | "userMessage" | "hookPrompt" | "generic";
      status: string;
      label?: string;
      durationMs?: number;
      details?: AgentStructuredPreview;
    };

export type AgentDiffStats = {
  additions: number;
  deletions: number;
  files: number;
};

export type AgentItem = {
  id: string;
  type: string;
  threadId: string;
  turnId: string;
  lifecycle: AgentItemLifecycle;
  status?: string;
  startedAt: string;
  updatedAt: string;
  completedAt?: string;
  data: Record<string, unknown>;
  text?: string;
  phase?: AgentMessagePhase;
  output?: string;
  outputTruncated?: boolean;
  reasoningSummary?: string;
  reasoningSummaryParts?: string[];
  restrictedDebugAvailable?: boolean;
  progressMessage?: string;
  terminalInteractionCount?: number;
  fileChanges?: AgentFileChange[];
  command?: AgentCommandState;
  presentation?: AgentItemPresentation;
};

export type AgentPlanStepStatus = "pending" | "inProgress" | "completed";

export type AgentPlanStep = {
  step: string;
  status: AgentPlanStepStatus;
};

export type AgentPlan = {
  threadId: string;
  turnId: string;
  explanation?: string;
  steps: AgentPlanStep[];
  updatedAt: string;
};

export type AgentHookEntry = {
  kind: string;
  text: string;
};

export type AgentHookRun = {
  id: string;
  threadId: string;
  turnId?: string;
  lifecycle: AgentItemLifecycle;
  displayOrder?: number;
  eventName: string;
  handlerType?: string;
  executionMode?: string;
  scope?: string;
  source?: string;
  sourcePath?: string;
  status: string;
  statusMessage?: string;
  durationMs?: number;
  entries: AgentHookEntry[];
  restrictedDebugAvailable?: boolean;
  startedAt: string;
  updatedAt: string;
  completedAt?: string;
};

export type AgentTokenUsageBreakdown = {
  totalTokens: number;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
};

export type AgentTokenUsage = {
  threadId: string;
  turnId: string;
  total: AgentTokenUsageBreakdown;
  last: AgentTokenUsageBreakdown;
  modelContextWindow: number | null;
  updatedAt: string;
};

export type AgentModelSafetyBuffering = {
  threadId: string;
  turnId: string;
  model: string;
  useCases: string[];
  reasons: string[];
  showBufferingUi: boolean;
  fasterModel?: string;
  createdAt: string;
};

export type AgentModelVerification = {
  threadId: string;
  turnId: string;
  verifications: string[];
  createdAt: string;
};

export type AgentTextRange = {
  start: { line: number; column: number };
  end: { line: number; column: number };
};

export type AgentWarning = {
  message: string;
  source?: "runtime" | "config" | "guardian";
  severity?: "warning" | "important";
  details?: string;
  path?: string;
  range?: AgentTextRange;
  count?: number;
  updatedAt?: string;
  threadId?: string;
  turnId?: string;
  createdAt: string;
};

export type AgentRunError = {
  message: string;
  retryable: boolean;
  terminal: boolean;
  threadId?: string;
  turnId?: string;
  createdAt: string;
};

export type AgentTurnStatus = "completed" | "interrupted" | "failed" | "inProgress";

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
      type: "item_started";
      threadId: string;
      turnId: string;
      itemId: string;
      itemType: string;
      lifecycle: "in_progress";
      status?: string;
      startedAt: string;
      item: Record<string, unknown>;
      createdAt: string;
    }
  | {
      type: "item_delta";
      threadId: string;
      turnId: string;
      itemId: string;
      itemType: string;
      lifecycle: "in_progress";
      method: string;
      delta: unknown;
      createdAt: string;
    }
  | {
      type: "item_completed";
      threadId: string;
      turnId: string;
      itemId: string;
      itemType: string;
      lifecycle: "completed";
      status?: string;
      completedAt: string;
      item: Record<string, unknown>;
      createdAt: string;
    }
  | {
      type: "plan_updated";
      threadId: string;
      turnId: string;
      explanation?: string;
      plan: AgentPlanStep[];
      createdAt: string;
    }
  | {
      type: "turn_diff_updated";
      threadId: string;
      turnId: string;
      diff: string;
      createdAt: string;
    }
  | {
      type: "approval_review";
      status: string;
      action?: string;
      rationale?: string;
      createdAt: string;
    }
  | {
      type: "diagnostic";
      level: string;
      message: string;
      method?: string;
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
      threadId?: string;
      turnId?: string;
      status: AgentTurnStatus;
      finalMessage?: string;
      createdAt: string;
    }
  | {
      type: "warning";
      message: string;
      threadId?: string;
      turnId?: string;
      createdAt: string;
    }
  | {
      type: "config_warning";
      summary: string;
      details?: string;
      path?: string;
      range?: AgentTextRange;
      createdAt: string;
    }
  | {
      type: "guardian_warning";
      message: string;
      threadId: string;
      createdAt: string;
    }
  | {
      type: "hook_run_updated";
      threadId: string;
      turnId?: string;
      hookId: string;
      lifecycle: AgentItemLifecycle;
      run: Record<string, unknown>;
      createdAt: string;
    }
  | {
      type: "context_compacted";
      threadId: string;
      turnId: string;
      createdAt: string;
    }
  | {
      type: "token_usage_updated";
      threadId: string;
      turnId: string;
      tokenUsage: {
        total: AgentTokenUsageBreakdown;
        last: AgentTokenUsageBreakdown;
        modelContextWindow: number | null;
      };
      createdAt: string;
    }
  | {
      type: "model_rerouted";
      threadId: string;
      turnId: string;
      fromModel: string;
      toModel: string;
      reason: string;
      createdAt: string;
    }
  | {
      type: "model_safety_buffering_updated";
      threadId: string;
      turnId: string;
      model: string;
      useCases: string[];
      reasons: string[];
      showBufferingUi: boolean;
      fasterModel?: string;
      createdAt: string;
    }
  | {
      type: "model_verification_updated";
      threadId: string;
      turnId: string;
      verifications: string[];
      createdAt: string;
    }
  | {
      type: "error";
      message: string;
      retryable: boolean;
      terminal: boolean;
      threadId?: string;
      turnId?: string;
      createdAt: string;
    };

export type AgentRun = {
  id: string;
  kind: AgentRunKind;
  prompt: string;
  status: AgentRunStatus;
  codexThreadId?: string;
  codexTurnId?: string;
  runtime?: CodingAgentRuntimeMetadata;
  events: AgentEvent[];
  itemsById: Record<string, AgentItem>;
  itemOrder: string[];
  messagesByItemId: Record<string, AgentItem>;
  filesByPath: Record<string, AgentFileChange>;
  aggregateDiff: string;
  aggregateDiffStats: AgentDiffStats;
  aggregateDiffUpdatedAt?: string;
  currentPlan?: AgentPlan;
  hooksById: Record<string, AgentHookRun>;
  hookOrder: string[];
  tokenUsage?: AgentTokenUsage;
  modelSafetyBuffering?: AgentModelSafetyBuffering;
  modelVerification?: AgentModelVerification;
  warnings: AgentWarning[];
  errors: AgentRunError[];
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
