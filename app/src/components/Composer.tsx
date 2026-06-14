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
import { shortPath } from "../utils/project";
import type { VoiceRequirementController } from "../utils/requirementState";

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
  const canFinishRequirement = voiceMode && requirement.session?.requirementState.utterances.length;
  const requirementStatus = requirement.session?.requirementState.status;

  const finishRequirement = async () => {
    if (voice.recording || voice.busy) {
      await voice.stop();
    }

    requirement.finishCollection();
  };

  return (
    <div className={`composer-shell ${voiceMode ? "is-voice-mode" : ""}`}>
      <div className={`composer-card ${voiceMode ? "is-voice-mode" : ""}`}>
        {voiceMode ? (
          <div className="voice-composer-status">
            <span className={`voice-dot ${voice.recording ? "is-recording" : ""}`} />
            <div>
              <strong>{getVoiceModeTitle(voice.status, requirementStatus)}</strong>
              <small>{getVoiceModeHint(voice.status, requirementStatus)}</small>
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
                <span>我说完了</span>
              </button>
            )}
          </div>
          <div className="composer-actions-right">
            <button
              className={`icon-button quiet voice-button ${voice.recording ? "is-recording" : ""}`}
              aria-label={voiceButtonLabel}
              disabled={voice.status === "transcribing"}
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
            <div className="project-menu">
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

function getVoiceModeTitle(status: VoiceSessionController["status"], requirementStatus: string | undefined) {
  if (status === "recording") {
    return "正在听你说需求";
  }

  if (status === "transcribing") {
    return "正在等待最后的转写";
  }

  if (requirementStatus === "requirement_ready") {
    return "需求草稿已整理";
  }

  if (requirementStatus === "confirmed") {
    return "需求已确认";
  }

  if (status === "error") {
    return "语音输入出错";
  }

  return "语音需求模式";
}

function getVoiceModeHint(status: VoiceSessionController["status"], requirementStatus: string | undefined) {
  if (status === "recording") {
    return "继续自然描述，系统会逐步整理当前理解。";
  }

  if (requirementStatus === "clarifying") {
    return "系统有问题需要补充，继续用语音回答即可。";
  }

  if (requirementStatus === "requirement_ready") {
    return "检查中间的需求文档，确认后进入下一阶段。";
  }

  return "点击麦克风继续说，或点“我说完了”整理需求。";
}
