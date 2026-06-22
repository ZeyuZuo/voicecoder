# Phase 4 Demo Agent Loop Todo

日期：2026-06-22

## 背景

Phase 3 已经能把语音需求整理成需求文档和 Coding Prompt，并在用户确认后进入 `confirmed` 状态。原先 Phase 4 只计划用一次 `codex exec --json` 根据 Coding Prompt 修改代码，但真实产品目标已经变成一个持续交互流程：

```text
说需求
  ↓
生成第一版 demo
  ↓
展示 demo
  ↓
用户语音反馈不满意的地方
  ↓
系统整理成本轮修改指令
  ↓
Agent 基于当前项目继续修改
  ↓
刷新 demo
  ↓
继续循环
```

因此 Phase 4 的核心不只是“自动编码”，而是建立 DemoSession 和 Codex thread 的长期映射，让第一版生成和后续反馈修改都落在同一个上下文里。

## 目标

- 用户确认需求后，可以启动第一轮 demo 生成。
- 第一轮 demo 使用 Phase 3 产物中的需求文档和 Coding Prompt。
- Agent 运行过程以事件流展示在对话区。
- Agent 完成后展示本轮变更摘要，并刷新项目文件树。
- 第一版 demo 完成后，Composer 进入 demo 反馈模式。
- 用户可以继续通过语音描述“不满意 / 要调整”的地方。
- 系统把语音反馈整理为增量修改指令，而不是重新生成完整需求文档。
- 同一个 DemoSession 继续调用 Coding Agent 修改项目。
- 失败时保留事件日志和错误信息，方便排查。

## 非目标

- Phase 4 不实现完整 Diff Viewer；逐文件 diff、接受和回滚进入 Phase 5。
- Phase 4 不实现完整 dev server 管理和浏览器预览控制；完整预览闭环进入 Phase 6。
- Phase 4 不做复杂权限审批 UI；先保留最小确认弹层和清晰日志。
- Phase 4 不支持多个 Coding Agent 同时运行。
- Phase 4 不把每轮反馈都重新写成完整需求文档。
- Phase 4 不 fork Codex CLI；优先使用公开的 app-server / SDK / CLI 能力。

## 核心设计

### DemoSession

```ts
type DemoSessionStatus =
  | "idle"
  | "ready_to_start"
  | "agent_running"
  | "preview_ready"
  | "feedback_listening"
  | "feedback_processing"
  | "agent_modifying"
  | "error";

type DemoSession = {
  id: string;
  projectPath: string;
  requirementId: string;
  initialRequirementDocument: string;
  initialCodingPrompt: string;
  codexThreadId?: string;
  runs: AgentRun[];
  feedbackTurns: DemoFeedbackTurn[];
  currentPreviewUrl?: string;
  status: DemoSessionStatus;
  error?: string;
  createdAt: string;
  updatedAt: string;
};
```

### AgentRun

```ts
type AgentRunKind = "initial_build" | "feedback_change";

type AgentRunStatus =
  | "queued"
  | "starting"
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled";

type AgentRun = {
  id: string;
  kind: AgentRunKind;
  prompt: string;
  status: AgentRunStatus;
  codexThreadId?: string;
  codexTurnId?: string;
  events: AgentEvent[];
  changedFiles: string[];
  finalMessage?: string;
  error?: string;
  startedAt?: string;
  completedAt?: string;
};
```

### DemoFeedbackTurn

```ts
type DemoFeedbackTurn = {
  id: string;
  utterances: RequirementUtterance[];
  summary: string;
  modificationPrompt: string;
  linkedAgentRunId?: string;
  createdAt: string;
};
```

### AgentEvent

```ts
type AgentEvent =
  | { type: "thread_started"; threadId: string; createdAt: string }
  | { type: "turn_started"; turnId?: string; createdAt: string }
  | { type: "agent_message"; text: string; createdAt: string }
  | { type: "plan_update"; text: string; createdAt: string }
  | { type: "command"; command: string; status: string; createdAt: string }
  | { type: "file_change"; path: string; changeType?: string; createdAt: string }
  | { type: "turn_completed"; finalMessage?: string; createdAt: string }
  | { type: "error"; message: string; createdAt: string };
```

## Codex 集成策略

### 主路径：codex app-server

Phase 4 正式主线使用：

```bash
codex app-server --stdio
```

Tauri 后端负责：

- 启动和维护 app-server 子进程。
- 通过 stdin/stdout 发送和接收 JSON-RPC / JSONL 消息。
- 初始化连接：
  - `initialize`
  - `initialized`
- 创建或恢复 thread：
  - `thread/start`
  - `thread/resume`
- 发起 turn：
  - `turn/start`
- 读取并归一化通知事件：
  - `thread/started`
  - `turn/started`
  - `item/started`
  - `item/completed`
  - `item/agentMessage/delta`
  - `turn/completed`
  - `turn/failed`
  - `error`
- 将事件转发给前端。
- 记录原始事件日志，方便排查。

### 后备路径：codex exec --json

保留 `codex exec --json` provider：

```bash
codex exec --json --sandbox workspace-write --cd <project> "<prompt>"
```

用途：

- 快速诊断本机 Codex 可用性。
- app-server 接入初期对照事件解析。
- app-server 不可用时提供一次性生成能力。

限制：

- 不作为长期多轮交互主路径。
- 如果使用 `codex exec resume`，也只能作为过渡方案，不替代 DemoSession / app-server thread 设计。

## Prompt 策略

### 第一轮 initial_build

```text
你正在为 VoiceCoder 生成第一版可运行 demo。

目标项目路径：
{projectPath}

已确认需求文档：
{initialRequirementDocument}

Coding Prompt：
{initialCodingPrompt}

请基于当前项目实现第一版 demo。优先保证可运行、可展示、交互完整。
完成后给出简短变更摘要和后续可改进点。
```

### 后续 feedback_change

```text
你正在继续修改一个已经生成过的 demo。

原始需求基准：
{initialRequirementDocument}

用户刚刚看完 demo 后提出的新反馈：
{feedbackSummary}

本轮修改指令：
{modificationPrompt}

请只围绕本轮反馈修改当前项目，保留已有可用功能，不要从零重写。
完成后给出本轮变更摘要。
```

## 状态流

```text
RequirementState.confirmed
  ↓
DemoSession.ready_to_start
  ↓
AgentRun(initial_build).running
  ↓
DemoSession.preview_ready
  ↓
DemoSession.feedback_listening
  ↓
DemoFeedbackTurn
  ↓
DemoSession.feedback_processing
  ↓
AgentRun(feedback_change).running
  ↓
DemoSession.preview_ready
```

## 后端命令草案

```text
get_coding_agent_provider_status() -> CodingAgentProviderStatus
start_demo_session(request) -> DemoSession
start_agent_run(request) -> AgentRun
cancel_agent_run(request) -> AgentRunCancelResult
resume_demo_session(request) -> DemoSession
process_demo_feedback(request) -> DemoFeedbackProcessingResult
```

第一版也可以合并为更少命令：

```text
start_initial_demo_run(request)
start_feedback_demo_run(request)
cancel_active_agent_run()
```

但内部仍应保留 DemoSession / AgentRun 的领域模型，避免 UI 直接绑定 Codex 协议细节。

## 前端 UI 草案

- 需求文档 `confirmed` 后展示“生成 demo”按钮。
- 点击生成前展示确认信息：
  - 当前项目。
  - Git 分支。
  - 本轮 prompt 摘要。
  - 运行类型：生成第一版 demo。
- Agent 运行中在对话区展示：
  - 当前阶段。
  - agent message。
  - plan update。
  - command execution。
  - file change。
  - final summary。
- 运行完成后：
  - 展示“第一版 demo 已生成”卡片。
  - 文件树刷新。
  - Composer 切换为“说说哪里需要改”的反馈模式。
- 反馈模式下：
  - 麦克风录入反馈。
  - 小卡片展示整理后的本轮修改指令。
  - 用户确认后继续修改。

## 开发 Todo

- [x] Step 1：更新路线图和 Phase 4 文档，明确 Demo Agent Loop 是 Phase 4 主目标。
- [ ] Step 2：新增 TypeScript 类型：`DemoSession`、`AgentRun`、`DemoFeedbackTurn`、`AgentEvent`、`CodingAgentProviderStatus`。
- [ ] Step 3：新增前端 demo session reducer，管理 `ready_to_start`、`agent_running`、`preview_ready`、`feedback_listening` 等状态。
- [ ] Step 4：在需求 `confirmed` 后创建 DemoSession，并显示“生成 demo”操作。
- [ ] Step 5：新增 Rust `coding_agent` 模块，定义 `CodingAgentProvider`、`CodingAgentSession`、provider diagnostic。
- [ ] Step 6：实现 `codex_app_server` provider：启动 `codex app-server --stdio`、完成 initialize / initialized。
- [ ] Step 7：实现 app-server JSON-RPC 客户端：请求 id 管理、响应匹配、通知分发、子进程退出处理。
- [ ] Step 8：实现 `thread/start` 和 `turn/start`，支持传入 `cwd`、sandbox、prompt。
- [ ] Step 9：实现 app-server 事件归一化，把 Codex 通知转换成前端 `AgentEvent`。
- [ ] Step 10：新增 Tauri 事件：`agent://run-started`、`agent://event`、`agent://run-completed`、`agent://error`。
- [ ] Step 11：前端订阅 Agent 事件，并在对话区展示运行进度。
- [ ] Step 12：Agent 运行完成后触发 `voicecoder:project-files-changed`，刷新文件树。
- [ ] Step 13：新增 `codex_exec_json` 后备 provider，用于诊断和 app-server 不可用时的一次性运行。
- [ ] Step 14：实现“第一版 demo”启动确认：目标项目、当前分支、运行类型、prompt 摘要。
- [ ] Step 15：实现 demo 反馈模式：第一版成功后 Composer 文案和操作切换为反馈输入。
- [ ] Step 16：复用语音链路收集反馈 utterances，但不要进入 Phase 3 需求文档状态机。
- [ ] Step 17：新增 LLM 反馈整理命令：把反馈 utterances 转成 `summary` 和 `modificationPrompt`。
- [ ] Step 18：实现后续 `feedback_change` run：复用同一个 `codexThreadId` 发起新 turn。
- [ ] Step 19：保存 DemoSession / AgentRun 基础日志到项目 `.voicecoder` 目录，避免刷新后完全丢失。
- [ ] Step 20：补充单元测试：Agent 事件归一化、DemoSession reducer、feedback prompt 生成。
- [ ] Step 21：补充 Rust 测试：JSON-RPC message parser、Codex event parser、provider diagnostics。
- [ ] Step 22：用测试前端项目手动验收：生成第一版 demo、语音反馈修改、刷新文件树。

## 验收标准

- 用户确认需求后，必须先看到目标项目和 Coding Prompt 摘要，再启动 Agent。
- Agent 可以在测试前端项目里生成第一版 demo。
- 前端能看到 Agent 的阶段性进度，而不是只等待最终结果。
- Agent 事件日志在失败时可排查。
- 第一版成功后，用户可以进入语音反馈模式。
- 反馈语音会整理成增量修改指令。
- 后续修改复用同一个 DemoSession 和 Codex thread。
- 每轮完成后文件树刷新。
- 普通测试不依赖真实 Codex 服务，事件解析和状态机可用 fixture 覆盖。

## 风险

- app-server 协议仍可能变化，必须生成或固定当前 Codex 版本的协议类型。
- Codex 账号、网络和地区可用性会影响真实验收。
- Agent 可能大范围重写项目，后续 Phase 5 必须尽快补 diff 和回滚。
- demo 预览体验依赖 dev server，Phase 4 只能做最小刷新，完整体验要等 Phase 6。
- 多轮反馈容易漂移，必须保留初始需求文档作为每轮修改的基准。
