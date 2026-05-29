import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { Project } from "../types/app";
import { readBrowserGitBranch } from "../utils/browserFileSystem";

export function useGitBranch(project?: Project) {
  const [branch, setBranch] = useState<string | undefined>();

  useEffect(() => {
    let cancelled = false;

    async function loadBranch() {
      if (!project) {
        setBranch(undefined);
        return;
      }

      setBranch(undefined);

      try {
        if (!isTauri() || project.path.startsWith("browser://")) {
          const browserBranch = await readBrowserGitBranch(project.path);
          if (!cancelled) {
            setBranch(browserBranch);
          }
          return;
        }

        const nextBranch = await invoke<string | null>("read_git_branch", { path: project.path });
        if (!cancelled) {
          setBranch(nextBranch ?? undefined);
        }
      } catch (error) {
        console.warn("Failed to read Git branch.", error);
        if (!cancelled) {
          setBranch(undefined);
        }
      }
    }

    void loadBranch();

    return () => {
      cancelled = true;
    };
  }, [project]);

  return branch;
}
