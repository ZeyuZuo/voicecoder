import { ChevronRight, FileCode2, FileText, Folder, FolderOpen } from "lucide-react";
import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { FileTreeEntry, Project } from "../../types/app";
import { getBrowserDirectoryHandle, readBrowserProjectTree } from "../../utils/browserFileSystem";

type BackendFileTreeEntry = {
  name: string;
  path: string;
  is_directory: boolean;
  children?: BackendFileTreeEntry[];
};

type FileExplorerProps = {
  project?: Project;
};

export function FileExplorer({ project }: FileExplorerProps) {
  const [entries, setEntries] = useState<FileTreeEntry[]>([]);
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => new Set());
  const [selectedPath, setSelectedPath] = useState<string | undefined>();
  const [status, setStatus] = useState<"idle" | "loading" | "error">("idle");
  const [error, setError] = useState<string | undefined>();

  useEffect(() => {
    let cancelled = false;

    async function loadTree() {
      if (!project) {
        setEntries([]);
        setStatus("idle");
        return;
      }

      setStatus("loading");
      setError(undefined);

      try {
        if (isTauri() && !project.path.startsWith("browser://")) {
          const tree = await invoke<BackendFileTreeEntry[]>("read_project_tree", { path: project.path });
          if (!cancelled) {
            setEntries(tree.map(normalizeEntry));
            setExpandedPaths(new Set([project.path]));
            setStatus("idle");
          }
          return;
        }

        const browserTree = await readBrowserProjectTree(project.path);
        if (!cancelled && browserTree) {
          setEntries(browserTree);
          setExpandedPaths(new Set([project.path]));
          setStatus("idle");
          return;
        }

        if (!cancelled) {
          setEntries([]);
          setExpandedPaths(new Set());
          setStatus("error");
          setError(getBrowserDirectoryHandle(project.path) ? "当前浏览器无法读取这个文件夹内容。" : "浏览器刷新后需要重新选择这个文件夹，才能继续读取文件。");
        }
      } catch (loadError) {
        if (!cancelled) {
          setStatus("error");
          setError(loadError instanceof Error ? loadError.message : String(loadError));
        }
      }
    }

    void loadTree();

    return () => {
      cancelled = true;
    };
  }, [project]);

  if (!project) {
    return (
      <div className="file-explorer-empty">
        <FolderOpen size={34} />
        <h2>未选择项目</h2>
        <p>先在输入框下方选择一个本地文件夹，然后这里会显示文件结构。</p>
      </div>
    );
  }

  return (
    <div className="file-explorer">
      <div className="file-explorer-title">
        <FolderOpen size={18} />
        <span>{project.name}</span>
      </div>
      {status === "loading" ? <div className="file-explorer-status">正在读取文件...</div> : null}
      {status === "error" ? <div className="file-explorer-status is-error">{error}</div> : null}
      <div className="file-tree" role="tree">
        {entries.map((entry) => (
          <FileTreeNode
            entry={entry}
            expandedPaths={expandedPaths}
            selectedPath={selectedPath}
            level={0}
            key={entry.path}
            onSelect={setSelectedPath}
            onToggle={(path) =>
              setExpandedPaths((paths) => {
                const nextPaths = new Set(paths);
                if (nextPaths.has(path)) {
                  nextPaths.delete(path);
                } else {
                  nextPaths.add(path);
                }
                return nextPaths;
              })
            }
          />
        ))}
      </div>
    </div>
  );
}

function FileTreeNode({
  entry,
  expandedPaths,
  selectedPath,
  level,
  onSelect,
  onToggle
}: {
  entry: FileTreeEntry;
  expandedPaths: Set<string>;
  selectedPath?: string;
  level: number;
  onSelect: (path: string) => void;
  onToggle: (path: string) => void;
}) {
  const expanded = expandedPaths.has(entry.path);
  const Icon = entry.isDirectory ? (expanded ? FolderOpen : Folder) : getFileIcon(entry.name);

  return (
    <div role="treeitem" aria-expanded={entry.isDirectory ? expanded : undefined}>
      <button
        className={`file-tree-row ${selectedPath === entry.path ? "is-selected" : ""}`}
        style={{ paddingLeft: 12 + level * 16 }}
        onClick={() => {
          if (entry.isDirectory) {
            onToggle(entry.path);
          } else {
            onSelect(entry.path);
          }
        }}
      >
        {entry.isDirectory ? <ChevronRight className={`file-tree-chevron ${expanded ? "is-expanded" : ""}`} size={14} /> : <span className="file-tree-spacer" />}
        <Icon size={16} />
        <span>{entry.name}</span>
      </button>
      {entry.isDirectory && expanded && entry.children?.length ? (
        <div role="group">
          {entry.children.map((child) => (
            <FileTreeNode
              entry={child}
              expandedPaths={expandedPaths}
              selectedPath={selectedPath}
              level={level + 1}
              key={child.path}
              onSelect={onSelect}
              onToggle={onToggle}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function normalizeEntry(entry: BackendFileTreeEntry): FileTreeEntry {
  return {
    name: entry.name,
    path: entry.path,
    isDirectory: entry.is_directory,
    children: entry.children?.map(normalizeEntry)
  };
}

function getFileIcon(name: string) {
  return /\.(tsx?|jsx?|json|css|html|md)$/i.test(name) ? FileCode2 : FileText;
}
