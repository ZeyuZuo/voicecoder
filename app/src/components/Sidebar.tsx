import {
  Folder,
  MessageCircle,
  PanelLeftClose,
  Settings
} from "lucide-react";
import { useAppState } from "../providers/AppStateProvider";

const navItems = [
  { label: "快速对话", icon: MessageCircle, active: true }
];

export function Sidebar() {
  const { projects, conversations, currentProject, selectProject, toggleSidebar } = useAppState();

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <button className="icon-button" aria-label="折叠左侧边栏" onClick={toggleSidebar}>
          <PanelLeftClose size={18} />
        </button>
      </div>

      <nav className="sidebar-section nav-section" aria-label="全局导航">
        {navItems.map((item) => {
          const Icon = item.icon;
          return (
            <button className={`nav-item ${item.active ? "is-active" : ""}`} key={item.label}>
              <Icon size={18} />
              <span>{item.label}</span>
            </button>
          );
        })}
      </nav>

      <section className="sidebar-section">
        <h2 className="sidebar-title">项目</h2>
        <div className="project-stack">
          {projects.map((project) => {
            const projectConversations = conversations.filter((conversation) => conversation.projectId === project.id);
            const selected = currentProject?.id === project.id;

            return (
              <div className="project-group" key={project.id}>
                <button className={`project-row ${selected ? "is-selected" : ""}`} onClick={() => selectProject(project.id)}>
                  <Folder size={18} />
                  <span>{project.name}</span>
                </button>
                <div className="conversation-stack">
                  {projectConversations.length > 0 ? (
                    projectConversations.map((conversation) => (
                      <button className="conversation-row" key={conversation.id}>
                        <span className="conversation-title">{conversation.title}</span>
                        <span className="conversation-time">{conversation.lastActivity}</span>
                      </button>
                    ))
                  ) : (
                    <span className="sidebar-empty">暂无对话</span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </section>

      <section className="sidebar-section">
        <h2 className="sidebar-title">对话</h2>
        <div className="conversation-stack">
          {conversations.filter((conversation) => !conversation.projectId).length ? (
            conversations
              .filter((conversation) => !conversation.projectId)
              .map((conversation) => (
                <button className="conversation-row" key={conversation.id}>
                  <span className="conversation-title">{conversation.title}</span>
                  <span className="conversation-time">{conversation.lastActivity}</span>
                </button>
              ))
          ) : (
            <span className="sidebar-empty">暂无聊天</span>
          )}
        </div>
      </section>

      <div className="sidebar-footer">
        <button className="nav-item">
          <Settings size={18} />
          <span>设置</span>
        </button>
      </div>
    </aside>
  );
}
