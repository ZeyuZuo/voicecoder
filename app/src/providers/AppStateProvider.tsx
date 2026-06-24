import { createContext, ReactNode, useContext, useMemo, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { BrowserPreviewState, Conversation, Project, WorkspaceMode, WorkspaceTab, WorkspaceTabKind } from "../types/app";
import { registerBrowserDirectory } from "../utils/browserFileSystem";
import { createId, getProjectName } from "../utils/project";

type PersistedState = {
  projects: Project[];
  conversations: Conversation[];
  currentProjectId?: string;
};

type AppStateContextValue = {
  projects: Project[];
  conversations: Conversation[];
  currentProject?: Project;
  workspaceMode: WorkspaceMode;
  workspaceTabs: WorkspaceTab[];
  activeWorkspaceTabId?: string;
  browserPreview: BrowserPreviewState;
  sidebarCollapsed: boolean;
  workspaceCollapsed: boolean;
  maximizedPane: "conversation" | "workspace" | null;
  projectPickerMessage?: string;
  prompt: string;
  addProjectFromPicker: () => Promise<void>;
  addProjectFromPath: (path: string) => void;
  addBrowserProject: (name: string, path: string, handle: FileSystemDirectoryHandle) => void;
  selectProject: (projectId?: string) => void;
  setWorkspaceMode: (mode: WorkspaceMode) => void;
  openWorkspaceTab: (kind: WorkspaceTabKind) => void;
  openBrowserPreview: (url: string) => void;
  selectWorkspaceTab: (tabId: string) => void;
  closeWorkspaceTab: (tabId: string) => void;
  toggleSidebar: () => void;
  toggleWorkspace: () => void;
  toggleMaximizedPane: (pane: "conversation" | "workspace") => void;
  setPrompt: (value: string) => void;
};

const STORAGE_KEY = "voicecoder.phase1.state";

const initialConversations: Conversation[] = [];

const initialProjects: Project[] = [];

function loadState(): PersistedState {
  if (typeof window === "undefined") {
    return {
      projects: initialProjects,
      conversations: initialConversations,
      currentProjectId: undefined
    };
  }

  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return {
        projects: initialProjects,
        conversations: initialConversations,
        currentProjectId: undefined
      };
    }

    const parsed = JSON.parse(raw) as PersistedState;
    const projects = parsed.projects ?? [];
    const currentProjectExists = projects.some((project) => project.id === parsed.currentProjectId);

    return {
      projects,
      conversations: parsed.conversations ?? [],
      currentProjectId: currentProjectExists ? parsed.currentProjectId : undefined
    };
  } catch {
    return {
      projects: initialProjects,
      conversations: initialConversations,
      currentProjectId: undefined
    };
  }
}

function persist(state: PersistedState) {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
}

const AppStateContext = createContext<AppStateContextValue | null>(null);

export function AppStateProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<PersistedState>(() => loadState());
  const [workspaceMode, setWorkspaceMode] = useState<WorkspaceMode>("launcher");
  const [workspaceTabs, setWorkspaceTabs] = useState<WorkspaceTab[]>([]);
  const [activeWorkspaceTabId, setActiveWorkspaceTabId] = useState<string | undefined>();
  const [browserPreview, setBrowserPreview] = useState<BrowserPreviewState>({});
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [workspaceCollapsed, setWorkspaceCollapsed] = useState(false);
  const [maximizedPane, setMaximizedPane] = useState<"conversation" | "workspace" | null>(null);
  const [projectPickerMessage, setProjectPickerMessage] = useState<string | undefined>();
  const [prompt, setPrompt] = useState("");

  const currentProject = useMemo(
    () => state.projects.find((project) => project.id === state.currentProjectId),
    [state.currentProjectId, state.projects]
  );

  const updateState = (nextState: PersistedState) => {
    setState(nextState);
    persist(nextState);
  };

  const addProject = ({ name, path }: { name: string; path: string }) => {
    const existingProject = state.projects.find((project) => project.path === path);

    if (existingProject) {
      updateState({
        ...state,
        projects: [
          { ...existingProject, lastActivity: "刚刚" },
          ...state.projects.filter((project) => project.id !== existingProject.id)
        ],
        currentProjectId: existingProject.id
      });
      return;
    }

    const project: Project = {
      id: createId("project"),
      name,
      path,
      lastActivity: "刚刚"
    };

    updateState({
      ...state,
      projects: [project, ...state.projects],
      conversations: [
        {
          id: createId("conversation"),
          title: "新对话",
          projectId: project.id,
          lastActivity: "刚刚"
        },
        ...state.conversations
      ],
      currentProjectId: project.id
    });
  };

  const addProjectFromPath = (path: string) => {
    addProject({
      name: getProjectName(path),
      path
    });
  };

  const addBrowserProject = (name: string, path: string, handle: FileSystemDirectoryHandle) => {
    registerBrowserDirectory({ name, path, handle });
    addProject({
      name,
      path
    });
  };

  const addProjectFromPicker = async () => {
    setProjectPickerMessage(undefined);

    try {
      if (isTauri()) {
        const selected = await open({
          directory: true,
          multiple: false,
          title: "选择前端项目"
        });

        if (typeof selected === "string") {
          addProjectFromPath(selected);
        }
        return;
      }

      const browserProject = await pickDirectoryInBrowser();
      if (browserProject) {
        addBrowserProject(browserProject.name, browserProject.path, browserProject.handle);
        return;
      }

      setProjectPickerMessage("当前浏览器不支持选择文件夹，请在 Tauri 窗口中使用。");
    } catch (error) {
      console.warn("Project picker failed.", error);
      setProjectPickerMessage("没有打开文件夹选择器，请在 Tauri 窗口中重试。");
    }
  };

  const selectProject = (projectId?: string) => {
    if (!projectId && !state.conversations.some((conversation) => !conversation.projectId)) {
      updateState({
        ...state,
        conversations: [
          {
            id: createId("conversation"),
            title: "快速对话",
            lastActivity: "刚刚"
          },
          ...state.conversations
        ],
        currentProjectId: undefined
      });
      return;
    }

    updateState({
      ...state,
      currentProjectId: projectId
    });
  };

  const toggleSidebar = () => {
    setSidebarCollapsed((collapsed) => !collapsed);
  };

  const toggleWorkspace = () => {
    setWorkspaceCollapsed((collapsed) => !collapsed);
    setMaximizedPane((pane) => (pane === "workspace" ? null : pane));
  };

  const toggleMaximizedPane = (pane: "conversation" | "workspace") => {
    setMaximizedPane((currentPane) => (currentPane === pane ? null : pane));
    if (pane === "workspace") {
      setWorkspaceCollapsed(false);
    }
  };

  const openWorkspaceTab = (kind: WorkspaceTabKind) => {
    const existingTab = workspaceTabs.find((tab) => tab.kind === kind);

    if (existingTab) {
      setActiveWorkspaceTabId(existingTab.id);
      setWorkspaceMode(kind);
      return;
    }

    const tab: WorkspaceTab = {
      id: createId("workspace"),
      kind,
      title: getWorkspaceTabTitle(kind)
    };

    setWorkspaceTabs((tabs) => [...tabs, tab]);
    setActiveWorkspaceTabId(tab.id);
    setWorkspaceMode(kind);
  };

  const openBrowserPreview = (url: string) => {
    const normalizedUrl = normalizePreviewUrl(url);
    setBrowserPreview({
      url: normalizedUrl,
      updatedAt: Date.now().toString()
    });
    openWorkspaceTab("browser");
    setWorkspaceCollapsed(false);
  };

  const selectWorkspaceTab = (tabId: string) => {
    const tab = workspaceTabs.find((candidate) => candidate.id === tabId);

    if (!tab) {
      return;
    }

    setActiveWorkspaceTabId(tab.id);
    setWorkspaceMode(tab.kind);
  };

  const closeWorkspaceTab = (tabId: string) => {
    setWorkspaceTabs((tabs) => {
      const nextTabs = tabs.filter((tab) => tab.id !== tabId);

      if (activeWorkspaceTabId === tabId) {
        const nextActiveTab = nextTabs[nextTabs.length - 1];
        setActiveWorkspaceTabId(nextActiveTab?.id);
        setWorkspaceMode(nextActiveTab?.kind ?? "launcher");
      }

      return nextTabs;
    });
  };

  const value = useMemo<AppStateContextValue>(
    () => ({
      projects: state.projects,
      conversations: state.conversations,
      currentProject,
      workspaceMode,
      workspaceTabs,
      activeWorkspaceTabId,
      browserPreview,
      sidebarCollapsed,
      workspaceCollapsed,
      maximizedPane,
      projectPickerMessage,
      prompt,
      addProjectFromPicker,
      addProjectFromPath,
      addBrowserProject,
      selectProject,
      setWorkspaceMode,
      openWorkspaceTab,
      openBrowserPreview,
      selectWorkspaceTab,
      closeWorkspaceTab,
      toggleSidebar,
      toggleWorkspace,
      toggleMaximizedPane,
      setPrompt
    }),
    [activeWorkspaceTabId, browserPreview, currentProject, maximizedPane, projectPickerMessage, prompt, sidebarCollapsed, state.conversations, state.projects, workspaceCollapsed, workspaceMode, workspaceTabs]
  );

  return <AppStateContext.Provider value={value}>{children}</AppStateContext.Provider>;
}

function normalizePreviewUrl(url: string) {
  const trimmedUrl = url.trim();
  if (/^https?:\/\//i.test(trimmedUrl)) {
    return trimmedUrl;
  }

  return `http://${trimmedUrl}`;
}

function getWorkspaceTabTitle(kind: WorkspaceTabKind): string {
  const titles: Record<WorkspaceTabKind, string> = {
    files: "文件",
    browser: "浏览器",
    review: "审查",
    terminal: "终端"
  };

  return titles[kind];
}

async function pickDirectoryInBrowser(): Promise<{ name: string; path: string; handle: FileSystemDirectoryHandle } | undefined> {
  const picker = window.showDirectoryPicker;

  if (picker) {
    const directory = await picker.call(window);
    return {
      name: directory.name,
      path: `browser://${directory.name}`,
      handle: directory
    };
  }

  return undefined;
}

declare global {
  interface Window {
    showDirectoryPicker?: () => Promise<FileSystemDirectoryHandle>;
  }
}

export function useAppState() {
  const context = useContext(AppStateContext);

  if (!context) {
    throw new Error("useAppState must be used within AppStateProvider");
  }

  return context;
}
