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
import { useMemo, useState } from "react";
import { useAppState } from "../providers/AppStateProvider";
import { shortPath } from "../utils/project";

export function Composer() {
  const {
    projects,
    currentProject,
    prompt,
    addProjectFromPicker,
    selectProject,
    setPrompt
  } = useAppState();
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);

  const visibleProjects = useMemo(() => projects.slice(0, 6), [projects]);

  const submitDisabled = prompt.trim().length === 0;

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
            <button className="icon-button quiet" aria-label="语音输入">
              <Mic size={18} />
            </button>
            <button className="send-button" disabled={submitDisabled} aria-label="发送需求">
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
            </div>
          ) : null}
        </div>
        <button className="context-chip">
          <GitBranch size={16} />
          <span>main</span>
          <ChevronDown size={14} />
        </button>
      </div>
    </div>
  );
}
