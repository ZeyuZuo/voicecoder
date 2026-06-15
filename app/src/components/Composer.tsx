import {
  ArrowUp,
  ChevronDown,
  Folder,
  GitBranch,
  Mic,
  Plus,
  Search,
  ShieldCheck,
  Square
} from "lucide-react";
import { useMemo, useState } from "react";
import { useGitBranch } from "../hooks/useGitBranch";
import type { VoiceSessionController } from "../hooks/useVoiceSession";
import { useAppState } from "../providers/AppStateProvider";
import type { RequirementStatus } from "../types/app";
import { shortPath } from "../utils/project";
import { getVoiceInputPermission, type VoiceRequirementController } from "../utils/requirementState";

type ComposerProps = {
  requirement: VoiceRequirementController;
  voice: VoiceSessionController;
  voiceMode: boolean;
};

export function Composer({ requirement, voice, voiceMode }: ComposerProps) {
  const {
    projects,
    currentProject,
    projectPickerMessage,
    prompt,
    addProjectFromPicker,
    selectProject,
    setPrompt
  } = useAppState();
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);

  const visibleProjects = useMemo(() => projects.slice(0, 6), [projects]);
  const gitBranch = useGitBranch(currentProject);

  const submitDisabled = voiceMode || prompt.trim().length === 0;
  const voiceButtonLabel = voice.recording || voice.busy ? "停止语音输入" : "语音输入";
  const requirementState = requirement.session?.requirementState;
  const voiceInputPermission = getVoiceInputPermission(requirementState);
  const voiceInputMode = getVoiceInputMode(voice.status, requirementState?.status);
  const canFinishRequirement = voiceMode && voiceInputPermission.canFinishTurn && voiceInputMode.canFinishTurn;
  const voiceButtonDisabled = voice.status === "transcribing" || !voiceInputPermission.canUseMic;

  const finishRequirement = async () => {
    if (voice.recording || voice.busy) {
      await voice.stop();
    }

    requirement.finishUserTurn();
  };

  return (
    <div className={`composer-shell ${voiceMode ? "is-voice-mode" : ""}`}>
      <div className={`composer-card ${voiceMode ? "is-voice-mode" : ""}`}>
        {voiceMode ? (
          <div className="voice-composer-status">
            <span className={`voice-dot ${voice.recording ? "is-recording" : ""}`} />
            <div>
              <strong>{voiceInputMode.title}</strong>
              <small>{voiceInputMode.hint}</small>
            </div>
          </div>
        ) : (
          <textarea
            className="composer-input"
            placeholder="尽管问"
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
          />
        )}
        <div className="composer-toolbar">
          <div className="composer-actions-left">
            {!voiceMode ? (
              <>
                <button className="icon-button quiet" aria-label="添加上下文">
                  <Plus size={20} />
                </button>
                <button className="tool-button accent">
                  <ShieldCheck size={17} />
                  <span>自动审查</span>
                  <ChevronDown size={14} />
                </button>
              </>
            ) : (
              <button className="tool-button" disabled={!canFinishRequirement} onClick={finishRequirement}>
                <Square size={14} />
                <span>{voiceInputMode.finishLabel}</span>
              </button>
            )}
          </div>
          <div className="composer-actions-right">
            <button
              className={`icon-button quiet voice-button ${voice.recording ? "is-recording" : ""}`}
              aria-label={voiceButtonLabel}
              disabled={voiceButtonDisabled}
              onClick={voice.toggle}
            >
              <Mic size={18} />
            </button>
            <button className="send-button" disabled={submitDisabled} aria-label={voiceMode ? "语音模式下不可发送文本" : "发送需求"}>
              <ArrowUp size={22} />
            </button>
          </div>
        </div>
      </div>

      <div className="context-bar">
        <div className="project-menu-anchor">
          <button className="context-chip" onClick={() => setProjectMenuOpen((open) => !open)}>
            <Folder size={16} />
            <span>{currentProject?.name ?? "不使用项目"}</span>
            <ChevronDown size={14} />
          </button>
          {projectMenuOpen ? (
            <div className={`project-menu ${voiceMode ? "opens-up" : ""}`}>
              {visibleProjects.length ? (
                <>
                  <label className="project-search">
                    <Search size={16} />
                    <input placeholder="搜索项目" />
                  </label>
                  <div className="project-options">
                    {visibleProjects.map((project) => (
                      <button
                        className={`project-option ${project.id === currentProject?.id ? "is-selected" : ""}`}
                        key={project.id}
                        onClick={() => {
                          selectProject(project.id);
                          setProjectMenuOpen(false);
                        }}
                      >
                        <Folder size={17} />
                        <span>
                          <strong>{project.name}</strong>
                          <small>{shortPath(project.path)}</small>
                        </span>
                      </button>
                    ))}
                  </div>
                  <div className="project-menu-separator" />
                </>
              ) : null}
              <button
                className="project-option"
                onClick={() => {
                  void addProjectFromPicker();
                  setProjectMenuOpen(false);
                }}
              >
                <Folder size={17} />
                <span>
                  <strong>使用现有文件夹</strong>
                  <small>选择一个本地前端项目</small>
                </span>
              </button>
              <button
                className="project-option"
                onClick={() => {
                  selectProject(undefined);
                  setProjectMenuOpen(false);
                }}
              >
                <Folder size={17} />
                <span>
                  <strong>不使用项目</strong>
                  <small>只进行需求讨论</small>
                </span>
              </button>
              {projectPickerMessage ? <p className="project-menu-message">{projectPickerMessage}</p> : null}
            </div>
          ) : null}
        </div>
        {gitBranch ? (
          <button className="context-chip">
            <GitBranch size={16} />
            <span>{gitBranch}</span>
            <ChevronDown size={14} />
          </button>
        ) : null}
      </div>
    </div>
  );
}

function getVoiceInputMode(status: VoiceSessionController["status"], requirementStatus: RequirementStatus | undefined) {
  if (requirementStatus === "finalizing" || requirementStatus === "processing") {
    return {
      title: "正在生成需求文档",
      hint: "正在整理完整需求，稍等一下。",
      finishLabel: "生成中",
      canFinishTurn: false,
      disableMic: true
    };
  }

  if (requirementStatus === "document_ready") {
    return {
      title: "需求文档已生成",
      hint: "可以展开查看文档，确认后进入后续编码阶段。",
      finishLabel: "已生成",
      canFinishTurn: false,
      disableMic: true
    };
  }

  if (requirementStatus === "confirmed") {
    return {
      title: "需求已确认",
      hint: "需求文档已生成，后续可交给编码阶段。",
      finishLabel: "已确认",
      canFinishTurn: false,
      disableMic: true
    };
  }

  if (status === "transcribing") {
    return {
      title: "正在等待最后的转写",
      hint: "收到最后一段语音后会继续整理。",
      finishLabel: "我说完了",
      canFinishTurn: false,
      disableMic: true
    };
  }

  if (status === "error") {
    return {
      title: "语音输入出错",
      hint: "检查语音 provider 后，可以点击麦克风重试。",
      finishLabel: "我说完了",
      canFinishTurn: false,
      disableMic: false
    };
  }

  if (status === "recording") {
    return {
      title: "正在听你说需求",
      hint: "继续自然描述，系统会实时整理当前理解和缺口。",
      finishLabel: "我说完了",
      canFinishTurn: true,
      disableMic: false
    };
  }

  return {
    title: "语音需求模式",
    hint: "点击麦克风持续说需求，或点“我说完了”生成文档。",
    finishLabel: "我说完了",
    canFinishTurn: true,
    disableMic: false
  };
}
