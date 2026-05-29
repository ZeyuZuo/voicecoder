import { CSSProperties, KeyboardEvent, PointerEvent, useCallback, useEffect, useRef, useState } from "react";
import { ConversationPane } from "./ConversationPane";
import { Sidebar } from "./Sidebar";
import { WorkspacePane } from "./WorkspacePane";
import { useAppState } from "../providers/AppStateProvider";

const SIDEBAR_WIDTH_KEY = "voicecoder.layout.sidebarWidth";
const WORKSPACE_WIDTH_KEY = "voicecoder.layout.workspaceWidth";
const DEFAULT_SIDEBAR_WIDTH = 292;
const DEFAULT_WORKSPACE_WIDTH = 460;
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 420;
const MIN_WORKSPACE_WIDTH = MIN_SIDEBAR_WIDTH;
const MAX_WORKSPACE_WIDTH = 760;

export function AppShell() {
  const { sidebarCollapsed, workspaceCollapsed, maximizedPane } = useAppState();
  const [sidebarWidth, setSidebarWidth] = useStoredWidth(SIDEBAR_WIDTH_KEY, DEFAULT_SIDEBAR_WIDTH);
  const [workspaceWidth, setWorkspaceWidth] = useStoredWidth(WORKSPACE_WIDTH_KEY, DEFAULT_WORKSPACE_WIDTH);
  const [resizingEdge, setResizingEdge] = useState<"left" | "right" | null>(null);
  const resizeStateRef = useRef({
    edge: null as "left" | "right" | null,
    startX: 0,
    startSidebarWidth: DEFAULT_SIDEBAR_WIDTH,
    startWorkspaceWidth: DEFAULT_WORKSPACE_WIDTH
  });
  const shellClasses = [
    "app-shell",
    sidebarCollapsed ? "is-sidebar-collapsed" : "",
    workspaceCollapsed ? "is-workspace-collapsed" : "",
    maximizedPane ? `is-${maximizedPane}-maximized` : "",
    resizingEdge ? "is-resizing" : ""
  ]
    .filter(Boolean)
    .join(" ");

  const resizeLayout = useCallback(
    (edge: "left" | "right", delta: number, commit = true) => {
      if (edge === "left") {
        const nextWidth = clamp(resizeStateRef.current.startSidebarWidth + delta, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        if (commit) {
          setSidebarWidth(nextWidth);
        }
        return nextWidth;
      }

      const nextWidth = clamp(resizeStateRef.current.startWorkspaceWidth - delta, MIN_WORKSPACE_WIDTH, MAX_WORKSPACE_WIDTH);
      if (commit) {
        setWorkspaceWidth(nextWidth);
      }
      return nextWidth;
    },
    [setSidebarWidth, setWorkspaceWidth]
  );

  useEffect(() => {
    if (!resizingEdge) {
      return;
    }

    const handlePointerMove = (event: globalThis.PointerEvent) => {
      const state = resizeStateRef.current;
      if (!state.edge) {
        return;
      }

      resizeLayout(state.edge, event.clientX - state.startX);
    };

    const finishResize = () => {
      resizeStateRef.current.edge = null;
      setResizingEdge(null);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize, { once: true });
    window.addEventListener("pointercancel", finishResize, { once: true });
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
  }, [resizeLayout, resizingEdge]);

  const startResize = (edge: "left" | "right", event: PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    resizeStateRef.current = {
      edge,
      startX: event.clientX,
      startSidebarWidth: sidebarWidth,
      startWorkspaceWidth: workspaceWidth
    };
    setResizingEdge(edge);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const nudgeResize = (edge: "left" | "right", event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 12;
    const direction = event.key === "ArrowRight" ? step : event.key === "ArrowLeft" ? -step : 0;

    if (!direction) {
      return;
    }

    event.preventDefault();
    resizeStateRef.current = {
      edge,
      startX: 0,
      startSidebarWidth: sidebarWidth,
      startWorkspaceWidth: workspaceWidth
    };
    resizeLayout(edge, direction);
  };

  const layoutStyle = {
    "--sidebar-width": `${sidebarWidth}px`,
    "--workspace-width": `${workspaceWidth}px`
  } as CSSProperties;

  return (
    <main className={shellClasses} style={layoutStyle}>
      <Sidebar />
      <ResizeHandle
        edge="left"
        hidden={sidebarCollapsed || maximizedPane !== null}
        label="调整左侧边栏宽度"
        value={sidebarWidth}
        min={MIN_SIDEBAR_WIDTH}
        max={MAX_SIDEBAR_WIDTH}
        active={resizingEdge === "left"}
        onPointerDown={startResize}
        onKeyDown={nudgeResize}
      />
      <ConversationPane />
      <ResizeHandle
        edge="right"
        hidden={workspaceCollapsed || maximizedPane !== null}
        label="调整右侧边栏宽度"
        value={workspaceWidth}
        min={MIN_WORKSPACE_WIDTH}
        max={MAX_WORKSPACE_WIDTH}
        active={resizingEdge === "right"}
        onPointerDown={startResize}
        onKeyDown={nudgeResize}
      />
      <WorkspacePane />
    </main>
  );
}

function ResizeHandle({
  edge,
  hidden,
  label,
  value,
  min,
  max,
  active,
  onPointerDown,
  onKeyDown
}: {
  edge: "left" | "right";
  hidden: boolean;
  label: string;
  value: number;
  min: number;
  max: number;
  active: boolean;
  onPointerDown: (edge: "left" | "right", event: PointerEvent<HTMLDivElement>) => void;
  onKeyDown: (edge: "left" | "right", event: KeyboardEvent<HTMLDivElement>) => void;
}) {
  return (
    <div
      className={`pane-resizer pane-resizer-${edge} ${active ? "is-active" : ""}`}
      role="separator"
      aria-hidden={hidden}
      aria-label={label}
      aria-orientation="vertical"
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      tabIndex={hidden ? -1 : 0}
      onPointerDown={(event) => {
        if (!hidden) {
          onPointerDown(edge, event);
        }
      }}
      onKeyDown={(event) => {
        if (!hidden) {
          onKeyDown(edge, event);
        }
      }}
    />
  );
}

function useStoredWidth(key: string, fallback: number) {
  const [width, setWidth] = useState(() => {
    if (typeof window === "undefined") {
      return fallback;
    }

    const stored = Number(window.localStorage.getItem(key));
    return Number.isFinite(stored) && stored > 0 ? stored : fallback;
  });

  const updateWidth = useCallback(
    (nextWidth: number) => {
      setWidth(nextWidth);
      window.localStorage.setItem(key, String(nextWidth));
    },
    [key]
  );

  return [width, updateWidth] as const;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}
