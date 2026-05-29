import {
  Code2,
  FileText,
  FolderOpen,
  Globe2,
  Maximize2,
  Minimize2,
  PanelRightClose,
  SquareTerminal
} from "lucide-react";
import { useAppState } from "../providers/AppStateProvider";

const launcherItems = [
  {
    mode: "files" as const,
    title: "文件",
    description: "浏览项目文件",
    icon: FolderOpen
  },
  {
    mode: "browser" as const,
    title: "浏览器",
    description: "打开网站",
    icon: Globe2
  },
  {
    mode: "review" as const,
    title: "审查",
    description: "查看代码更改",
    icon: FileText
  },
  {
    mode: "terminal" as const,
    title: "终端",
    description: "启动交互式 shell",
    icon: SquareTerminal
  }
];

export function WorkspacePane() {
  const { workspaceMode, setWorkspaceMode, currentProject, maximizedPane, toggleMaximizedPane, toggleWorkspace } = useAppState();
  const maximized = maximizedPane === "workspace";

  return (
    <section className="workspace-pane">
      <header className="pane-header workspace-header">
        <div className="workspace-tabs">
          <button className="add-tab-button" aria-label="新建工作区">
            +
          </button>
        </div>
        <div className="header-actions">
          <button className="icon-button" aria-label={maximized ? "还原右侧区域" : "放大右侧区域"} onClick={() => toggleMaximizedPane("workspace")}>
            {maximized ? <Minimize2 size={17} /> : <Maximize2 size={17} />}
          </button>
          <button className="icon-button" aria-label="折叠右侧边栏" onClick={toggleWorkspace}>
            <PanelRightClose size={18} />
          </button>
        </div>
      </header>

      {workspaceMode === "launcher" ? (
        <div className="workspace-launcher">
          <div className="launcher-grid">
            {launcherItems.map((item) => {
              const Icon = item.icon;
              return (
                <button className="launcher-card" key={item.mode} onClick={() => setWorkspaceMode(item.mode)}>
                  <Icon size={32} />
                  <span>{item.title}</span>
                  <small>{item.description}</small>
                </button>
              );
            })}
          </div>
        </div>
      ) : (
        <div className="workspace-placeholder">
          <div className="workspace-placeholder-icon">
            <Code2 size={30} />
          </div>
          <h2>{getWorkspaceTitle(workspaceMode)}</h2>
          <p>{currentProject ? `${currentProject.name} 的${getWorkspaceTitle(workspaceMode)}工作区将在下一阶段接入真实数据。` : "选择项目后，这里会显示对应工作区内容。"}</p>
          <button className="tool-button" onClick={() => setWorkspaceMode("launcher")}>
            返回工具入口
          </button>
        </div>
      )}
    </section>
  );
}

function getWorkspaceTitle(mode: "files" | "browser" | "review" | "terminal") {
  const titles = {
    files: "文件",
    browser: "浏览器",
    review: "审查",
    terminal: "终端"
  };

  return titles[mode];
}
