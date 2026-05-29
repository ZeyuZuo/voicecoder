import { ConversationPane } from "./ConversationPane";
import { Sidebar } from "./Sidebar";
import { WorkspacePane } from "./WorkspacePane";
import { useAppState } from "../providers/AppStateProvider";

export function AppShell() {
  const { sidebarCollapsed, workspaceCollapsed, maximizedPane } = useAppState();
  const shellClasses = [
    "app-shell",
    sidebarCollapsed ? "is-sidebar-collapsed" : "",
    workspaceCollapsed ? "is-workspace-collapsed" : "",
    maximizedPane ? `is-${maximizedPane}-maximized` : ""
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <main className={shellClasses}>
      <Sidebar />
      <ConversationPane />
      <WorkspacePane />
    </main>
  );
}
