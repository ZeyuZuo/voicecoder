import { createContext, ReactNode, useContext, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { Conversation, Project, WorkspaceMode } from "../types/app";
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
  prompt: string;
  addProjectFromPicker: () => Promise<void>;
  addProjectFromPath: (path: string) => void;
  selectProject: (projectId?: string) => void;
  setWorkspaceMode: (mode: WorkspaceMode) => void;
  setPrompt: (value: string) => void;
};

const STORAGE_KEY = "voicecoder.phase1.state";

const initialConversations: Conversation[] = [
  {
    id: "conversation_seed_1",
    title: "阅读 docs 后规划页面结构",
    projectId: "project_voicecoder",
    lastActivity: "刚刚"
  }
];

const initialProjects: Project[] = [
  {
    id: "project_voicecoder",
    name: "voicecoder",
    path: "/Users/zzy/Desktop/code/voicecoder",
    lastActivity: "当前项目"
  }
];

function loadState(): PersistedState {
  if (typeof window === "undefined") {
    return {
      projects: initialProjects,
      conversations: initialConversations,
      currentProjectId: "project_voicecoder"
    };
  }

  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return {
        projects: initialProjects,
        conversations: initialConversations,
        currentProjectId: "project_voicecoder"
      };
    }

    const parsed = JSON.parse(raw) as PersistedState;
    return {
      projects: parsed.projects?.length ? parsed.projects : initialProjects,
      conversations: parsed.conversations ?? [],
      currentProjectId: parsed.currentProjectId ?? parsed.projects?.[0]?.id ?? "project_voicecoder"
    };
  } catch {
    return {
      projects: initialProjects,
      conversations: initialConversations,
      currentProjectId: "project_voicecoder"
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
  const [prompt, setPrompt] = useState("");

  const currentProject = useMemo(
    () => state.projects.find((project) => project.id === state.currentProjectId),
    [state.currentProjectId, state.projects]
  );

  const updateState = (nextState: PersistedState) => {
    setState(nextState);
    persist(nextState);
  };

  const addProjectFromPath = (path: string) => {
    const existingProject = state.projects.find((project) => project.path === path);

    if (existingProject) {
      updateState({
        ...state,
        currentProjectId: existingProject.id
      });
      return;
    }

    const project: Project = {
      id: createId("project"),
      name: getProjectName(path),
      path,
      lastActivity: "刚刚"
    };

    updateState({
      ...state,
      projects: [project, ...state.projects],
      currentProjectId: project.id
    });
  };

  const addProjectFromPicker = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "选择前端项目"
      });

      if (typeof selected === "string") {
        addProjectFromPath(selected);
      }
    } catch {
      console.warn("System project picker is unavailable in this runtime.");
    }
  };

  const selectProject = (projectId?: string) => {
    updateState({
      ...state,
      currentProjectId: projectId
    });
  };

  const value = useMemo<AppStateContextValue>(
    () => ({
      projects: state.projects,
      conversations: state.conversations,
      currentProject,
      workspaceMode,
      prompt,
      addProjectFromPicker,
      addProjectFromPath,
      selectProject,
      setWorkspaceMode,
      setPrompt
    }),
    [currentProject, prompt, state.conversations, state.projects, workspaceMode]
  );

  return <AppStateContext.Provider value={value}>{children}</AppStateContext.Provider>;
}

export function useAppState() {
  const context = useContext(AppStateContext);

  if (!context) {
    throw new Error("useAppState must be used within AppStateProvider");
  }

  return context;
}
