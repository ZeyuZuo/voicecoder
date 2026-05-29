import {
  Code2,
  FileText,
  FolderOpen,
  Globe2,
  Maximize2,
  Minimize2,
  PanelRightClose,
  Plus,
  SquareTerminal,
  X
} from "lucide-react";
import { useAppState } from "../providers/AppStateProvider";
import type { Project, WorkspaceTabKind } from "../types/app";
import { FileExplorer } from "./workspace/FileExplorer";

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
  const {
    workspaceMode,
    setWorkspaceMode,
    workspaceTabs,
    activeWorkspaceTabId,
    currentProject,
    maximizedPane,
    openWorkspaceTab,
    selectWorkspaceTab,
    closeWorkspaceTab,
    toggleMaximizedPane,
    toggleWorkspace
  } = useAppState();
  const maximized = maximizedPane === "workspace";
  const activeTab = workspaceMode === "launcher" ? undefined : workspaceTabs.find((tab) => tab.id === activeWorkspaceTabId);

  return (
    <section className="workspace-pane">
      <header className="pane-header workspace-header">
        <div className="workspace-tabs">
          {workspaceTabs.map((tab) => {
            const Icon = getWorkspaceIcon(tab.kind);
            const selected = activeTab?.id === tab.id;

            return (
              <div className={`workspace-tab ${selected ? "is-active" : ""}`} key={tab.id}>
                <button className="workspace-tab-main" onClick={() => selectWorkspaceTab(tab.id)}>
                  <Icon size={15} />
                  <span>{tab.title}</span>
                </button>
                <button
                  className="tab-close-button"
                  aria-label={`关闭${tab.title}标签页`}
                  onClick={() => closeWorkspaceTab(tab.id)}
                >
                  <X size={13} />
                </button>
              </div>
            );
          })}
          <button className="add-tab-button" aria-label="打开工作区入口" onClick={() => setWorkspaceMode("launcher")}>
            <Plus size={17} />
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

      {!activeTab ? (
        <div className="workspace-launcher">
          <div className="launcher-grid">
            {launcherItems.map((item) => {
              const Icon = item.icon;
              return (
                <button className="launcher-card" key={item.mode} onClick={() => openWorkspaceTab(item.mode)}>
                  <Icon size={32} />
                  <span>{item.title}</span>
                  <small>{item.description}</small>
                </button>
              );
            })}
          </div>
        </div>
      ) : (
        <WorkspaceTabContent kind={activeTab.kind} currentProject={currentProject} onOpenLauncher={() => setWorkspaceMode("launcher")} />
      )}
    </section>
  );
}

function WorkspaceTabContent({
  kind,
  currentProject,
  onOpenLauncher
}: {
  kind: WorkspaceTabKind;
  currentProject?: Project;
  onOpenLauncher: () => void;
}) {
  if (kind === "files") {
    return <FileExplorer project={currentProject} />;
  }

  return (
    <div className="workspace-placeholder">
      <div className="workspace-placeholder-icon">
        <Code2 size={30} />
      </div>
      <h2>{getWorkspaceTitle(kind)}</h2>
      <p>{currentProject ? `${currentProject.name} 的${getWorkspaceTitle(kind)}工作区将在下一阶段接入真实数据。` : "选择项目后，这里会显示对应工作区内容。"}</p>
      <button className="tool-button" onClick={onOpenLauncher}>
        返回工具入口
      </button>
    </div>
  );
}

function getWorkspaceTitle(mode: WorkspaceTabKind) {
  const titles: Record<WorkspaceTabKind, string> = {
    files: "文件",
    browser: "浏览器",
    review: "审查",
    terminal: "终端"
  };

  return titles[mode];
}

function getWorkspaceIcon(kind: WorkspaceTabKind) {
  const icons: Record<WorkspaceTabKind, typeof FolderOpen> = {
    files: FolderOpen,
    browser: Globe2,
    review: FileText,
    terminal: SquareTerminal
  };

  return icons[kind];
}
