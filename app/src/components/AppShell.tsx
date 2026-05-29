import { ConversationPane } from "./ConversationPane";
import { Sidebar } from "./Sidebar";
import { WorkspacePane } from "./WorkspacePane";

export function AppShell() {
  return (
    <main className="app-shell">
      <Sidebar />
      <ConversationPane />
      <WorkspacePane />
    </main>
  );
}

