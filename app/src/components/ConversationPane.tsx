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
import { useDemoSession, type DemoSessionController } from "../utils/demoSession";
import { useVoiceRequirementSession, type VoiceRequirementController } from "../utils/requirementState";
import { Composer } from "./Composer";

export function ConversationPane() {
  const {
    currentProject,
    maximizedPane,
    openBrowserPreview,
    sidebarCollapsed,
    workspaceCollapsed,
    toggleMaximizedPane,
    toggleSidebar,
    toggleWorkspace
  } = useAppState();
  const maximized = maximizedPane === "conversation";
  const voice = useVoiceSession();
  const requirement = useVoiceRequirementSession(voice, currentProject?.path);
  const demo = useDemoSession(requirement.session?.requirementState, currentProject?.path, {
    onPreviewReady: openBrowserPreview
  });
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
            <VoiceRequirementWorkspace demo={demo} requirement={requirement} voice={voice} />
          ) : (
            <h2>{currentProject ? `我们应该在 ${currentProject.name} 中构建什么？` : "我们应该聊些什么？"}</h2>
          )}
          <Composer demo={demo} requirement={requirement} voice={voice} voiceMode={voiceMode} />
        </div>
      </div>
    </section>
  );
}

function VoiceRequirementWorkspace({
  demo,
  requirement,
  voice
}: {
  demo: DemoSessionController;
  requirement: VoiceRequirementController;
  voice: VoiceSessionController;
}) {
  const state = requirement.session?.requirementState;
  const utterances = state?.utterances ?? [];
  const voiceProvider = voice.sessionSnapshot?.provider ?? voice.providerStatus?.autoProvider ?? voice.provider;
  const providerDiagnostic = voice.providerStatus?.diagnostics.find((diagnostic) => diagnostic.provider === voiceProvider);
  const missingProviderEnv = providerDiagnostic?.missingEnv ?? [];
  const canConfirm = state?.status === "document_ready" && !state.pendingAction;
  const [documentExpanded, setDocumentExpanded] = useState(false);
  const openGaps = state?.openGaps.filter((gap) => gap.status === "open").slice(0, 3) ?? [];
  const showRequirementConfirm = state?.status === "document_ready";
  const showRequirementDocument = state?.status === "document_ready" && Boolean(state.requirementDocument);
  const hasBottomStack = showRequirementConfirm || showRequirementDocument;

  return (
    <div className={`voice-workspace ${hasBottomStack ? "has-document-stack" : ""}`}>
      <div className="voice-workspace-main">
        <div className="voice-workspace-topline">
          <StatusPill active={voice.recording}>{getVoiceStatusLabel(voice.status)}</StatusPill>
          {voiceProvider ? <StatusPill>{getProviderLabel(voiceProvider)}</StatusPill> : null}
          {state ? <StatusPill>{getRequirementStatusLabel(state.status)}</StatusPill> : null}
          {demo.session ? <StatusPill active={demo.session.status === "agent_running" || demo.session.status === "agent_modifying"}>{getDemoStatusLabel(demo.session.status)}</StatusPill> : null}
        </div>

        {voice.error ? <p className="voice-workspace-error">{voice.error}</p> : null}
        {state?.error ? <p className="voice-workspace-error">{state.error}</p> : null}
        {demo.session?.error ? <p className="voice-workspace-error">{demo.session.error}</p> : null}
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
                <span>{utterance.speakerId ?? "语音"}</span>
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
          <span>{state?.pendingAction === "summarize" ? "理解中" : "当前理解"}</span>
          <p>{state?.summary || "先听你完整描述几句。"}</p>
          {openGaps.length ? (
            <ul>
              {openGaps.map((gap) => (
                <li key={gap.id}>{gap.question}</li>
              ))}
            </ul>
          ) : null}
        </div>
      </aside>

      {hasBottomStack ? (
        <div className="requirement-bottom-stack">
          {showRequirementConfirm ? (
            <section className="requirement-action-card is-confirm is-document-ready">
              <div>
                <span>需求文档已生成</span>
                <p>已根据本轮语音整理需求文档，确认后可交给后续编码阶段。</p>
              </div>
              <button className="tool-button accent" disabled={!canConfirm} onClick={requirement.confirmRequirement}>
                <Check size={15} />
                <span>确认需求</span>
              </button>
            </section>
          ) : null}

          {showRequirementDocument ? (
            <section className={`requirement-document-preview ${documentExpanded ? "is-expanded" : ""}`}>
              <div>
                <span>需求文档</span>
                <p>{state.requirementDocument}</p>
                {state.savedRequirementDocumentPath ? (
                  <small className="requirement-document-path">已写入 {state.savedRequirementDocumentPath}</small>
                ) : null}
              </div>
              <button className="tool-button document-expand-button" onClick={() => setDocumentExpanded((expanded) => !expanded)}>
                {documentExpanded ? <ChevronDown size={15} /> : <ChevronUp size={15} />}
                <span>{documentExpanded ? "收起" : "展开"}</span>
              </button>
            </section>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function getDemoStatusLabel(status: NonNullable<DemoSessionController["session"]>["status"]) {
  const labels = {
    idle: "Demo 空闲",
    ready_to_start: "待生成 demo",
    agent_running: "生成 demo 中",
    preview_ready: "Demo 已生成",
    feedback_listening: "等待反馈",
    feedback_processing: "整理反馈中",
    agent_modifying: "修改 demo 中",
    error: "Demo 出错"
  };

  return labels[status];
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
    listening: "持续监听",
    finalizing: "生成文档中",
    document_ready: "文档已生成",
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
