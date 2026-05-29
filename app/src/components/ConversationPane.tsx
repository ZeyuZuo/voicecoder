import { ChevronDown, Maximize2, MoreHorizontal } from "lucide-react";
import { useAppState } from "../providers/AppStateProvider";
import { Composer } from "./Composer";

export function ConversationPane() {
  const { currentProject } = useAppState();

  return (
    <section className="conversation-pane">
      <header className="pane-header conversation-header">
        <div className="conversation-title-block">
          <h1>{currentProject ? `我们应该在 ${currentProject.name} 中构建什么？` : "我们应该在 voicecoder 中构建什么？"}</h1>
        </div>
        <div className="header-actions">
          <button className="chip-button compact">
            <span className="provider-dot" />
            <ChevronDown size={15} />
          </button>
          <button className="icon-button" aria-label="更多">
            <MoreHorizontal size={19} />
          </button>
          <button className="icon-button" aria-label="最大化">
            <Maximize2 size={17} />
          </button>
        </div>
      </header>

      <div className="empty-conversation">
        <div className="prompt-stage">
          <h2>{currentProject ? `我们应该在 ${currentProject.name} 中构建什么？` : "我们应该在 voicecoder 中构建什么？"}</h2>
          <Composer />
        </div>
      </div>
    </section>
  );
}

