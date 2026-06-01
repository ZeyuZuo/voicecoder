import {
  ArrowUp,
  ChevronDown,
  Folder,
  GitBranch,
  Mic,
  Plus,
  Search,
  ShieldCheck
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useGitBranch } from "../hooks/useGitBranch";
import { useVoiceSession } from "../hooks/useVoiceSession";
import { useAppState } from "../providers/AppStateProvider";
import { shortPath } from "../utils/project";

export function Composer() {
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
  const appendedVoiceSegments = useRef<Map<string, string>>(new Map());

  const visibleProjects = useMemo(() => projects.slice(0, 6), [projects]);
  const gitBranch = useGitBranch(currentProject);
  const voice = useVoiceSession();

  const submitDisabled = prompt.trim().length === 0;
  const voiceButtonLabel = voice.recording || voice.busy ? "停止语音输入" : "语音输入";
  const voiceStatusLabel = getVoiceStatusLabel(voice.status);
  const voiceProvider = voice.sessionSnapshot?.provider ?? voice.providerStatus?.autoProvider ?? voice.provider;
  const missingTencentEnv = voice.tencentConfigCheck?.missingEnv ?? voice.providerStatus?.missingTencentEnv ?? [];

  useEffect(() => {
    const changedFinalSegments = voice.segments.filter((segment) => appendedVoiceSegments.current.get(segment.id) !== segment.text);

    if (!changedFinalSegments.length) {
      return;
    }

    let nextPrompt = prompt;

    for (const segment of changedFinalSegments) {
      const nextText = segment.text.trim();
      if (!nextText) {
        continue;
      }

      const previousText = appendedVoiceSegments.current.get(segment.id);
      if (previousText && nextPrompt.includes(previousText)) {
        nextPrompt = replaceLast(nextPrompt, previousText, nextText);
      } else {
        nextPrompt = nextPrompt.trim() ? `${nextPrompt.trimEnd()}\n${nextText}` : nextText;
      }
      appendedVoiceSegments.current.set(segment.id, nextText);
    }

    if (nextPrompt === prompt) {
      return;
    }

    setPrompt(nextPrompt);
  }, [prompt, setPrompt, voice.segments]);

  return (
    <div className="composer-shell">
      <div className="composer-card">
        <textarea
          className="composer-input"
          placeholder="尽管问"
          value={prompt}
          onChange={(event) => setPrompt(event.target.value)}
        />
        <div className="composer-toolbar">
          <div className="composer-actions-left">
            <button className="icon-button quiet" aria-label="添加上下文">
              <Plus size={20} />
            </button>
            <button className="tool-button accent">
              <ShieldCheck size={17} />
              <span>自动审查</span>
              <ChevronDown size={14} />
            </button>
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
            <button className="send-button" disabled={submitDisabled} aria-label="发送需求">
              <ArrowUp size={22} />
            </button>
          </div>
        </div>
      </div>

      {voice.status !== "idle" || voice.segments.length ? (
        <div className={`voice-session-panel ${voice.recording ? "is-recording" : ""}`}>
          <div className="voice-session-header">
            <span className="voice-pulse" />
            <span>{voiceStatusLabel}</span>
            {voiceProvider ? <small>{voiceProvider}</small> : null}
          </div>
          {voice.providerStatus ? (
            <p className="voice-provider-note">
              {getVoiceProviderNote(voice.providerStatus.autoProvider, voice.providerStatus.providerOverride, voice.tencentConfigCheck?.ok)}
            </p>
          ) : null}
          {voiceProvider === "tencent" && missingTencentEnv.length ? (
            <p className="voice-provider-note">缺少腾讯云配置：{missingTencentEnv.join("、")}</p>
          ) : null}
          {voice.error ? <p className="voice-error">{voice.error}</p> : null}
          {voice.status === "error" && voice.sessionSnapshot?.active ? (
            <p className="voice-provider-note">后端仍有语音会话，点击麦克风会先自动清理后重试。</p>
          ) : null}
          {voice.partialText ? (
            <p className="voice-partial">
              <span>实时</span>
              {voice.partialText}
            </p>
          ) : null}
          {voice.segments.length ? (
            <div className="voice-transcripts">
              {voice.segments.slice(-4).map((segment) => (
                <p key={segment.id}>
                  <span>{segment.speakerId ?? "speaker"}</span>
                  {segment.text}
                </p>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}

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

function replaceLast(value: string, search: string, replacement: string) {
  const index = value.lastIndexOf(search);
  if (index < 0) {
    return value;
  }

  return `${value.slice(0, index)}${replacement}${value.slice(index + search.length)}`;
}

function getVoiceStatusLabel(status: ReturnType<typeof useVoiceSession>["status"]) {
  const labels = {
    idle: "语音已停止",
    starting: "正在启动语音输入",
    "requesting-permission": "正在请求麦克风权限",
    recording: "正在录音转写",
    transcribing: "正在等待转写结果",
    error: "语音输入出错"
  };

  return labels[status];
}

function getVoiceProviderNote(provider: string, override: string | undefined, tencentReady: boolean | undefined) {
  const source = override ? `已指定 ${override}` : "自动选择";

  if (provider === "tencent") {
    return `${source} · 腾讯云${tencentReady ? "配置就绪" : "配置待检查"}`;
  }

  return `${source} · Mock 转写`;
}
