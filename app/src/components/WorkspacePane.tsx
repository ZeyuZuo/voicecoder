import {
  Bot,
  Code2,
  ExternalLink,
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
import { useEffect, useRef, useState } from "react";
import { useAppState } from "../providers/AppStateProvider";
import type { BrowserPreviewState, DemoSession, Project, WorkspaceTabKind } from "../types/app";
import { DEMO_SESSION_UPDATED_EVENT } from "../utils/demoSession";
import { DemoProgressPanel } from "./DemoProgressPanel";
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
    browserPreview,
    currentProject,
    maximizedPane,
    openBrowserPreview,
    openWorkspaceTab,
    selectWorkspaceTab,
    closeWorkspaceTab,
    toggleMaximizedPane,
    toggleWorkspace
  } = useAppState();
  const maximized = maximizedPane === "workspace";
  const activeTab = workspaceMode === "launcher" ? undefined : workspaceTabs.find((tab) => tab.id === activeWorkspaceTabId);
  const demoSession = useWorkspaceDemoSession(currentProject?.path);
  const openedDemoSessionIdsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!demoSession?.runs.length || openedDemoSessionIdsRef.current.has(demoSession.id)) {
      return;
    }

    openedDemoSessionIdsRef.current.add(demoSession.id);
    openWorkspaceTab("demo");
  }, [demoSession?.id, demoSession?.runs.length, openWorkspaceTab]);

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
        <WorkspaceLauncher onOpenWorkspaceTab={openWorkspaceTab} />
      ) : (
        <WorkspaceTabContent
          browserPreview={browserPreview}
          currentProject={currentProject}
          demoSession={demoSession}
          kind={activeTab.kind}
          onOpenBrowserPreview={openBrowserPreview}
          onOpenLauncher={() => setWorkspaceMode("launcher")}
        />
      )}
    </section>
  );
}

function WorkspaceLauncher({
  onOpenWorkspaceTab
}: {
  onOpenWorkspaceTab: (kind: WorkspaceTabKind) => void;
}) {
  return (
    <div className="workspace-launcher">
      <div className="launcher-grid">
        {launcherItems.map((item) => {
          const Icon = item.icon;
          return (
            <button className="launcher-card" key={item.mode} onClick={() => onOpenWorkspaceTab(item.mode)}>
              <Icon size={32} />
              <span>{item.title}</span>
              <small>{item.description}</small>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function DemoWorkspacePanel({ session }: { session: DemoSession }) {
  return (
    <div className="workspace-demo-panel">
      <div className="workspace-demo-panel-heading">
        <span>Demo 生成</span>
        <p>右侧会在 dev server URL 就绪后自动切换到浏览器预览。</p>
      </div>
      {session.error ? <p className="workspace-demo-error">{session.error}</p> : null}
      <DemoProgressPanel session={session} compact />
    </div>
  );
}

function useWorkspaceDemoSession(projectPath: string | undefined) {
  const [session, setSession] = useState<DemoSession | undefined>();

  useEffect(() => {
    const handleDemoSessionUpdate = (event: Event) => {
      const detail = (event as CustomEvent<{ session?: DemoSession }>).detail;
      const nextSession = detail?.session;
      if (!nextSession || (projectPath && nextSession.projectPath !== projectPath)) {
        return;
      }

      setSession(nextSession);
    };

    window.addEventListener(DEMO_SESSION_UPDATED_EVENT, handleDemoSessionUpdate);
    return () => {
      window.removeEventListener(DEMO_SESSION_UPDATED_EVENT, handleDemoSessionUpdate);
    };
  }, [projectPath]);

  useEffect(() => {
    setSession((currentSession) => {
      if (!currentSession || !projectPath || currentSession.projectPath === projectPath) {
        return currentSession;
      }

      return undefined;
    });
  }, [projectPath]);

  return session;
}

function WorkspaceTabContent({
  browserPreview,
  kind,
  currentProject,
  demoSession,
  onOpenBrowserPreview,
  onOpenLauncher
}: {
  browserPreview: BrowserPreviewState;
  kind: WorkspaceTabKind;
  currentProject?: Project;
  demoSession?: DemoSession;
  onOpenBrowserPreview: (url: string) => void;
  onOpenLauncher: () => void;
}) {
  if (kind === "demo") {
    return demoSession ? <DemoWorkspacePanel session={demoSession} /> : (
      <div className="workspace-placeholder">
        <div className="workspace-placeholder-icon">
          <Bot size={30} />
        </div>
        <h2>Demo 生成</h2>
        <p>点击生成 demo 后，这里会显示 Codex 交互过程。</p>
        <button className="tool-button" onClick={onOpenLauncher}>
          返回工具入口
        </button>
      </div>
    );
  }

  if (kind === "files") {
    return <FileExplorer project={currentProject} />;
  }

  if (kind === "browser") {
    return <BrowserPreview preview={browserPreview} onOpenPreview={onOpenBrowserPreview} />;
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

function BrowserPreview({
  preview,
  onOpenPreview
}: {
  preview: BrowserPreviewState;
  onOpenPreview: (url: string) => void;
}) {
  const [draftUrl, setDraftUrl] = useState(preview.url ?? "http://localhost:5173");

  useEffect(() => {
    if (preview.url) {
      setDraftUrl(preview.url);
      return;
    }

    setDraftUrl("http://localhost:5173");
  }, [preview.updatedAt, preview.url]);

  const openDraftUrl = () => {
    if (!draftUrl.trim()) {
      return;
    }

    onOpenPreview(draftUrl);
  };

  return (
    <div className="browser-preview">
      <div className="browser-preview-toolbar">
        <Globe2 size={16} />
        <input
          aria-label="预览地址"
          value={draftUrl}
          onChange={(event) => setDraftUrl(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              openDraftUrl();
            }
          }}
        />
        <button className="tool-button" onClick={openDraftUrl}>
          <ExternalLink size={14} />
          <span>打开</span>
        </button>
      </div>

      {preview.url ? (
        <iframe className="browser-preview-frame" src={preview.url} title="Demo preview" />
      ) : (
        <div className="browser-preview-empty">
          <div className="workspace-placeholder-icon">
            <Globe2 size={30} />
          </div>
          <h2>浏览器预览</h2>
          <p>等待 dev server URL，或手动输入本地预览地址。</p>
        </div>
      )}
    </div>
  );
}

function getWorkspaceTitle(mode: WorkspaceTabKind) {
  const titles: Record<WorkspaceTabKind, string> = {
    demo: "Demo 生成",
    files: "文件",
    browser: "浏览器",
    review: "审查",
    terminal: "终端"
  };

  return titles[mode];
}

function getWorkspaceIcon(kind: WorkspaceTabKind) {
  const icons: Record<WorkspaceTabKind, typeof FolderOpen> = {
    demo: Bot,
    files: FolderOpen,
    browser: Globe2,
    review: FileText,
    terminal: SquareTerminal
  };

  return icons[kind];
}
