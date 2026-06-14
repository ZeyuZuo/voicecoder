import {
  Check,
  ChevronDown,
  ChevronUp,
  Cloud,
  Maximize2,
  Minimize2,
  MoreHorizontal,
  PanelLeft,
  PanelRight,
  ScrollText
} from "lucide-react";
import { useState, type ReactNode } from "react";
import type { VoiceSessionController } from "../hooks/useVoiceSession";
import { useVoiceSession } from "../hooks/useVoiceSession";
import { useAppState } from "../providers/AppStateProvider";
import { useVoiceRequirementSession, type VoiceRequirementController } from "../utils/requirementState";
import { Composer } from "./Composer";

export function ConversationPane() {
  const { currentProject, maximizedPane, sidebarCollapsed, workspaceCollapsed, toggleMaximizedPane, toggleSidebar, toggleWorkspace } = useAppState();
  const maximized = maximizedPane === "conversation";
  const voice = useVoiceSession();
  const requirement = useVoiceRequirementSession(voice);
  const voiceMode = voice.status !== "idle" || voice.segments.length > 0 || requirement.active;

  return (
    <section className={`conversation-pane ${voiceMode ? "is-voice-mode" : ""}`}>
      <header className="pane-header conversation-header">
        <div className="conversation-header-left">
          {sidebarCollapsed ? (
            <button className="icon-button edge-toggle" aria-label="展开左侧边栏" onClick={toggleSidebar}>
              <PanelLeft size={18} />
            </button>
          ) : null}
          <div className="conversation-title-block">
            <h1>{currentProject ? `我们应该在 ${currentProject.name} 中构建什么？` : "我们应该聊些什么？"}</h1>
          </div>
        </div>
        <div className="header-actions">
          <button className="icon-button" aria-label="更多">
            <MoreHorizontal size={19} />
          </button>
          <button className="icon-button" aria-label={maximized ? "还原对话区域" : "放大对话区域"} onClick={() => toggleMaximizedPane("conversation")}>
            {maximized ? <Minimize2 size={17} /> : <Maximize2 size={17} />}
          </button>
          {workspaceCollapsed ? (
            <button className="icon-button edge-toggle" aria-label="展开右侧边栏" onClick={toggleWorkspace}>
              <PanelRight size={18} />
            </button>
          ) : null}
        </div>
      </header>

      <div className={`empty-conversation ${voiceMode ? "is-voice-mode" : ""}`}>
        <div className={`prompt-stage ${voiceMode ? "is-voice-mode" : ""}`}>
          {voiceMode ? (
            <VoiceRequirementWorkspace requirement={requirement} voice={voice} />
          ) : (
            <h2>{currentProject ? `我们应该在 ${currentProject.name} 中构建什么？` : "我们应该聊些什么？"}</h2>
          )}
          <Composer requirement={requirement} voice={voice} voiceMode={voiceMode} />
        </div>
      </div>
    </section>
  );
}

function VoiceRequirementWorkspace({
  requirement,
  voice
}: {
  requirement: VoiceRequirementController;
  voice: VoiceSessionController;
}) {
  const state = requirement.session?.requirementState;
  const utterances = state?.utterances ?? [];
  const voiceProvider = voice.sessionSnapshot?.provider ?? voice.providerStatus?.autoProvider ?? voice.provider;
  const providerDiagnostic = voice.providerStatus?.diagnostics.find((diagnostic) => diagnostic.provider === voiceProvider);
  const missingProviderEnv = providerDiagnostic?.missingEnv ?? [];
  const canConfirm = state?.status === "ready_to_confirm" && !state.openQuestions.some((question) => question.blocksCoding);
  const [documentExpanded, setDocumentExpanded] = useState(false);
  const showClarification = state?.status === "clarifying" && state.openQuestions.length > 0;
  const showRequirementConfirm = state?.status === "ready_to_confirm";
  const showRequirementDocument = state?.status === "confirmed" && Boolean(state.requirementDocument);

  return (
    <div className="voice-workspace">
      <div className="voice-workspace-main">
        <div className="voice-workspace-topline">
          <StatusPill active={voice.recording}>{getVoiceStatusLabel(voice.status)}</StatusPill>
          {voiceProvider ? <StatusPill>{getProviderLabel(voiceProvider)}</StatusPill> : null}
          {state ? <StatusPill>{getRequirementStatusLabel(state.status)}</StatusPill> : null}
        </div>

        {voice.error ? <p className="voice-workspace-error">{voice.error}</p> : null}
        {state?.error ? <p className="voice-workspace-error">{state.error}</p> : null}
        {missingProviderEnv.length ? (
          <p className="voice-workspace-note">缺少{getProviderLabel(voiceProvider)}配置：{missingProviderEnv.join("、")}</p>
        ) : null}
        {providerDiagnostic?.error && !missingProviderEnv.length ? (
          <p className="voice-workspace-note">{providerDiagnostic.error}</p>
        ) : null}

        <div className="voice-transcript-canvas" aria-live="polite">
          {utterances.length ? (
            utterances.map((utterance) => (
              <article className="voice-transcript-line" key={utterance.id}>
                <span>{utterance.speakerId ?? (utterance.source === "clarification_answer" ? "补充" : "语音")}</span>
                <p>{utterance.text}</p>
              </article>
            ))
          ) : (
            <div className="voice-empty-listening">
              <ScrollText size={22} />
              <p>{voice.recording ? "正在等待第一句转写" : "点击麦克风开始描述需求"}</p>
            </div>
          )}

          {voice.partialText ? (
            <article className="voice-transcript-line is-partial">
              <span>实时</span>
              <p>{voice.partialText}<i /></p>
            </article>
          ) : null}
        </div>
      </div>

      <aside className="voice-thought-cloud" aria-label="当前理解">
        <div className="voice-thought-cloud-icon">
          <Cloud size={22} />
        </div>
        <div className="voice-thought-cloud-copy">
          <span>{state?.pendingAction ? "整理中" : "当前理解"}</span>
          <p>{state?.summary || "先听你完整描述几句。"}</p>
        </div>
      </aside>

      {showClarification ? (
        <section className="requirement-action-card is-clarification">
          <span>需要补充</span>
          <div>
            {state.openQuestions.slice(0, 3).map((question) => (
              <p key={question.id}>{question.question}</p>
            ))}
          </div>
        </section>
      ) : null}

      {showRequirementConfirm ? (
        <section className="requirement-action-card is-confirm">
          <div>
            <span>需求确认</span>
            <p>信息已经足够整理成需求文档。确认后，系统会生成完整需求文档并展示在这里。</p>
          </div>
          <button className="tool-button accent" disabled={!canConfirm} onClick={requirement.confirmRequirement}>
            <Check size={15} />
            <span>确认并生成文档</span>
          </button>
        </section>
      ) : null}

      {showRequirementDocument ? (
        <section className={`requirement-document-preview ${documentExpanded ? "is-expanded" : ""}`}>
          <div>
            <span>需求文档</span>
            <p>{state.requirementDocument}</p>
          </div>
          <button className="tool-button document-expand-button" onClick={() => setDocumentExpanded((expanded) => !expanded)}>
            {documentExpanded ? <ChevronDown size={15} /> : <ChevronUp size={15} />}
            <span>{documentExpanded ? "收起" : "展开"}</span>
          </button>
        </section>
      ) : null}
    </div>
  );
}

function StatusPill({ active, children }: { active?: boolean; children: ReactNode }) {
  return (
    <span className={`voice-status-pill ${active ? "is-active" : ""}`}>
      <i />
      {children}
    </span>
  );
}

function getVoiceStatusLabel(status: VoiceSessionController["status"]) {
  const labels = {
    idle: "语音已停止",
    starting: "启动中",
    "requesting-permission": "请求麦克风",
    recording: "正在录音",
    transcribing: "等待转写",
    error: "语音出错"
  };

  return labels[status];
}

function getRequirementStatusLabel(status: NonNullable<VoiceRequirementController["session"]>["requirementState"]["status"]) {
  const labels = {
    idle: "空闲",
    collecting: "收集中",
    processing: "整理中",
    clarifying: "待补充",
    ready_to_confirm: "待确认",
    confirmed: "已确认"
  };

  return labels[status];
}

function getProviderLabel(provider: string) {
  if (provider === "tencent") {
    return "腾讯云";
  }

  if (provider === "iflytek_llm") {
    return "讯飞大模型";
  }

  if (provider === "volcengine") {
    return "火山引擎";
  }

  if (provider === "mock") {
    return "Mock ASR";
  }

  return provider;
}
