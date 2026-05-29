import { Maximize2, Minimize2, MoreHorizontal, PanelLeft, PanelLeftClose, PanelRight, PanelRightClose } from "lucide-react";
import { useAppState } from "../providers/AppStateProvider";
import { Composer } from "./Composer";

export function ConversationPane() {
  const { currentProject, maximizedPane, sidebarCollapsed, workspaceCollapsed, toggleMaximizedPane, toggleSidebar, toggleWorkspace } = useAppState();
  const maximized = maximizedPane === "conversation";

  return (
    <section className="conversation-pane">
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

      <div className="empty-conversation">
        <div className="prompt-stage">
          <h2>{currentProject ? `我们应该在 ${currentProject.name} 中构建什么？` : "我们应该聊些什么？"}</h2>
          <Composer />
        </div>
      </div>
    </section>
  );
}
