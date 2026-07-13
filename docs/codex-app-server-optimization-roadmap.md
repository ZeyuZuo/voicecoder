# Codex App Server 优化 Roadmap

日期：2026-07-13

## 背景

VoiceCoder 已经通过 `codex app-server --stdio` 跑通第一版 Demo Agent Loop，但当前集成仍是“最小事件解析”：Codex 在分析、执行命令、创建或修改文件时，页面可能长时间没有可见更新，让用户误以为程序卡死。

本 Roadmap 的目标不是简单增加几条日志，而是把 app-server 集成升级成一个可靠的富客户端链路：能够静默执行、持续显示进度、正确处理完整 JSON-RPC 流，并在协议变化或异常时保留可排查证据。

官方协议参考：<https://developers.openai.com/codex/app-server/>

当前开发和验收基线：`codex-cli 0.144.1`。每次升级 Codex CLI 后，必须重新生成并核对版本匹配的 schema：

```bash
codex app-server generate-json-schema --out ./schemas
codex app-server generate-ts --out ./schemas
```

## 核心判断

### 静默执行不是 `approvalPolicy: "never"`

VoiceCoder 当前向 `thread/start` 和 `turn/start` 发送：

```json
{
  "approvalPolicy": "on-failure"
}
```

但本机 `codex-cli 0.144.1` 生成的 schema 中，`approvalPolicy` 支持：

- `untrusted`
- `on-request`
- `never`
- granular policy object

审批由谁处理是另一个独立字段：

```json
{
  "approvalsReviewer": "auto_review"
}
```

因此 VoiceCoder 的推荐静默策略是：

```json
{
  "approvalPolicy": "on-request",
  "approvalsReviewer": "auto_review"
}
```

语义是：Agent 确实需要提升权限时允许发起请求，但请求交给 Codex 自动风险审查，而不是阻塞等待用户点击。自动审查可以批准，也可以拒绝；页面应展示简短结果，但不弹出阻塞式确认框。

`approvalPolicy: "never"` 只适合明确要求“任何情况下都不提升权限”的运行。它并不表示自动批准，而是禁止发起审批，越出当前 sandbox 的操作会失败。

### 页面展示应以 Item 状态为中心

app-server 的主要工作单元是 Item：

```text
item/started
  ↓
多个 delta / patch / progress 通知
  ↓
item/completed
```

同一个 `itemId` 的通知必须更新同一张页面卡片。不能继续把所有 delta 追加成互不关联的扁平事件。

### 所有协议消息都要被消费，但不需要全部挤进主界面

- 主界面：用户能理解的计划、说明、文件、命令、工具、警告和结果。
- 折叠详情：reasoning summary、命令完整输出、diff、工具参数和结果。
- 调试日志：原始 JSONL、未知事件、协议响应、stderr。

## 目标体验

```text
Demo 生成中 · 01:24
3 个文件  +126 -18

✓ 分析项目结构
✓ 制定实现计划
● 修改文件
  ├─ 新建 src/components/Hero.tsx       +84
  ├─ 修改 src/App.tsx                  +26 -12
  └─ 修改 src/styles.css               +16 -6
● 运行 npm run build
  └─ 正在执行… 12s
○ 启动预览
```

用户应始终能回答三个问题：

1. Codex 现在在做什么？
2. 最近一次有效进展是什么？
3. 如果停住了，是正在等待、正在重试、被自动审批拒绝，还是已经失败？

## Milestone 0：固定协议基线和静默审批策略

目标：在扩展 UI 前，先确保当前 Codex 版本、参数和审批语义明确。

完成日期：2026-07-13

- [x] 0.1 记录启动时的 Codex CLI 版本和 app-server transport。
- [x] 0.2 增加开发脚本，生成当前 CLI 对应的 JSON Schema / TypeScript schema。
- [x] 0.3 为实际使用的 `thread/start`、`turn/start`、通知和 server request 建立协议 fixture。
- [x] 0.4 把 `approvalPolicy: "on-failure"` 改为当前 schema 支持的策略。
- [x] 0.5 在 `thread/start` 和 `turn/start` 显式发送 `approvalsReviewer: "auto_review"`。
- [x] 0.6 在 provider diagnostic 中显示生效的 approval policy、reviewer 和 sandbox。
- [x] 0.7 保留环境变量覆盖入口，便于诊断时切换为人工审批或 `never`。
- [x] 0.8 增加单元测试，确保 thread 和 turn 使用同一套权限策略。

退出标准：

- 启动日志能明确看到 Codex 版本、sandbox、approval policy 和 reviewer。
- 普通 workspace-write Demo 生成不弹出人工审批框。
- 自动审查拒绝操作时能得到明确事件，而不是静默挂起。

## Milestone 1：重构 JSON-RPC Transport

目标：正确区分 response、notification 和 app-server 主动 request，消除协议层假死。

完成日期：2026-07-13

- [x] 1.1 将 stdout 读取改为单一持续 reader，所有 JSONL 消息只在一个地方解析。
- [x] 1.2 按消息形态路由：
  - `id + result/error`：VoiceCoder request 的 response。
  - `method + params`：notification。
  - `id + method + params`：app-server 主动 server request。
- [x] 1.3 用 pending request map 匹配 VoiceCoder 发出的 JSON-RPC request。
- [x] 1.4 为 notification 建立独立事件 channel，不因未知事件阻塞 stdout reader。
- [x] 1.5 为 server request 建立处理 channel 和超时策略。
- [x] 1.6 持续消费 app-server stderr，并写入诊断日志。
- [x] 1.7 子进程退出时携带 exit code、最后 stderr 和最后协议消息。
- [x] 1.8 未识别消息不得丢弃：写入原始日志并发出低优先级 diagnostic event。
- [x] 1.9 增加 heartbeat 信息：最后收到消息时间、最后有效进展时间。

退出标准：

- 任意未知 notification 不会卡住后续事件。
- 任意 server request 不会被误当成普通 response。
- stderr 持续输出不会阻塞 app-server。
- 传输异常能提供足够的排查上下文。

## Milestone 2：建立可更新的 Agent 领域模型

目标：从扁平 `AgentEvent[]` 升级成以 `itemId` 为主键的运行视图状态。

完成日期：2026-07-13

建议模型：

```text
AgentRun
├─ messagesByItemId
├─ currentPlan
├─ itemsById
├─ itemOrder
├─ filesByPath
├─ aggregateDiff
├─ warnings
├─ pendingRequests
├─ tokenUsage
└─ rawEventLogPath
```

- [x] 2.1 所有 item 事件保留 `threadId`、`turnId`、`itemId`、status 和时间戳。
- [x] 2.2 `item/started` 创建 Item，delta 更新 Item，`item/completed` 覆盖最终状态。
- [x] 2.3 `item/completed` 作为 Item 最终权威数据，避免 delta 与最终消息重复。
- [x] 2.4 `turn/plan/updated` 保存结构化 plan step，不再只拼成纯文本。
- [x] 2.5 区分 agent message 的 `commentary`、`final_answer` 和 unknown phase。
- [x] 2.6 区分 warning、retryable error、terminal error。
- [x] 2.7 `turn/completed` 根据 `completed`、`interrupted`、`failed` 正确结束 AgentRun。
- [x] 2.8 高频 delta 在进入 React 前按 50–100ms 合并，避免逐 token 渲染。
- [x] 2.9 为 Item upsert、乱序通知和重复通知补 reducer 测试。

退出标准：

- 同一个命令、文件修改或消息只显示一张持续更新的卡片。
- 最终 Item 可以覆盖中间状态，但不会丢失已经收到的输出。
- 可重试错误不会提前把本轮标记为失败。

## Milestone 3：优先补齐文件与命令实时反馈

目标：首先解决用户感知最强的“改文件时没有反馈”。

完成日期：2026-07-13

### 文件修改

- [x] 3.1 处理 `item/started` 中的 `fileChange`，立即显示“准备修改文件”。
- [x] 3.2 处理 `item/fileChange/patchUpdated`，实时更新 `changes[]`。
- [x] 3.3 处理 `turn/diff/updated`，将其作为本轮最新聚合 diff 快照。
- [x] 3.4 解析 unified diff，按文件计算 additions / deletions。
- [x] 3.5 支持 `add`、`update`、`delete` 和 move path 显示。
- [x] 3.6 处理 `item/completed fileChange`，用最终 changes 覆盖中间结果。
- [x] 3.7 `item/fileChange/outputDelta` 仅作为旧 Codex 版本兼容路径。
- [x] 3.8 文件树在文件 Item 完成后增量刷新；本轮结束后再做一次完整刷新。

### 命令执行

- [x] 3.9 `item/started commandExecution` 显示 command、cwd 和进行中状态。
- [x] 3.10 处理 `item/commandExecution/outputDelta`，持续追加 stdout/stderr。
- [x] 3.11 完成时显示 exit code、duration、completed/failed/declined。
- [x] 3.12 页面只保留命令输出尾部和摘要，完整输出写入日志。
- [x] 3.13 对长驻命令显示持续时间和最新输出，不把“没有新行”误判成崩溃。

退出标准：

- Codex 开始创建或修改文件后 1 秒内出现文件卡片。
- 页面能持续显示每个文件的 `+N/-N`。
- 长命令期间持续显示状态、耗时和最新输出。
- 文件和命令失败都能定位到具体 Item。

## Milestone 4：重做 Demo 生成时间线

目标：让页面呈现接近 Codex App 的连续工作过程。

- [ ] 4.1 移除“只显示最后 5 条事件”的限制。
- [ ] 4.2 使用滚动时间线；默认跟随最新事件，用户上滚后暂停自动滚动。
- [ ] 4.3 顶部显示运行状态、耗时、文件数、总 additions / deletions。
- [ ] 4.4 计划使用结构化步骤列表显示 pending / inProgress / completed。
- [ ] 4.5 agent commentary 作为过程说明流式显示。
- [ ] 4.6 文件修改按文件聚合，不为每个 patch 重复生成卡片。
- [ ] 4.7 命令卡片支持展开完整输出、复制命令和复制错误。
- [ ] 4.8 diff 默认只显示统计和摘要，展开后显示当前 diff。
- [ ] 4.9 warning 使用黄色非阻塞提示，terminal error 使用红色失败卡片。
- [ ] 4.10 增加“最后进展于 N 秒前”，超过阈值时显示“仍在等待 Codex”。
- [ ] 4.11 运行结束后保留完整时间线，不被 preview 状态覆盖。

退出标准：

- 用户始终能看到最近一次真实进展。
- 文件、命令和计划不会被 assistant 文本淹没。
- 1000 条以上 delta 不造成明显卡顿。

## Milestone 5：覆盖其余 Turn / Item 返回

目标：所有与当前 AgentRun 有关的主要返回都有可理解的呈现或明确的调试归宿。

- [ ] 5.1 reasoning summary：显示可折叠“正在分析”卡片。
- [ ] 5.2 raw reasoning text：默认不展示，只进入受限调试详情。
- [ ] 5.3 `mcpToolCall`：显示 server、tool、status、duration、result/error。
- [ ] 5.4 `dynamicToolCall`：显示工具名、参数摘要、状态和结果。
- [ ] 5.5 `collabAgentToolCall` / `subAgentActivity`：显示子 Agent 生命周期。
- [ ] 5.6 `webSearch`：显示搜索、打开页面、页内查找动作。
- [ ] 5.7 `imageView` / `imageGeneration`：显示路径、状态和保存位置。
- [ ] 5.8 `hook/started` / `hook/completed`：显示 hook 名称和结果，默认折叠。
- [ ] 5.9 `contextCompaction`：显示“正在整理长对话上下文”。
- [ ] 5.10 `thread/tokenUsage/updated`：在顶部显示低优先级 token 使用信息。
- [ ] 5.11 `model/rerouted`、safety buffering、verification：显示对应状态提示。
- [ ] 5.12 `warning`、`configWarning`、guardian warning：按严重程度展示。

退出标准：

- 已知的 thread/turn/item 事件不会静默消失。
- 低价值高频数据不会挤占主时间线。
- 所有未知 method 都能在协议调试日志中找到。

## Milestone 6：自动审批和异常请求兜底

目标：默认保持“Approve for me”式静默体验，同时任何意外请求都不会造成假死。

- [x] 6.1 处理 `item/autoApprovalReview/started`，显示低优先级“正在自动审查权限”。（Milestone 0 提前完成）
- [x] 6.2 处理 `item/autoApprovalReview/completed`，显示批准或拒绝结果。（Milestone 0 提前完成）
- [ ] 6.3 处理 guardian warning，说明自动审查拒绝的原因。
- [ ] 6.4 对 `item/commandExecution/requestApproval` 建立协议响应能力。
- [ ] 6.5 对 `item/fileChange/requestApproval` 建立协议响应能力。
- [ ] 6.6 对 `item/permissions/requestApproval` 建立协议响应能力。
- [ ] 6.7 对 `item/tool/requestUserInput` 和 MCP elicitation 建立非静默兜底 UI。
- [ ] 6.8 自动 reviewer 已接管的请求不弹人工确认框，只显示状态。
- [ ] 6.9 必须由用户决定的请求显示内联卡片，并有明确超时和取消行为。
- [ ] 6.10 处理 `serverRequest/resolved`，清理页面上的 pending request。
- [ ] 6.11 AgentRun 结束或取消时清理所有未决请求。

退出标准：

- 普通 Demo 生成保持全程静默。
- 自动审查期间页面仍有进度反馈。
- 自动审查拒绝不会表现为卡死。
- 任何不能自动处理的 server request 都有明确 UI 和超时结果。

## Milestone 7：日志、恢复和协议兼容

目标：出现版本变化、崩溃或页面刷新时仍可排查和恢复。

- [ ] 7.1 每个 AgentRun 保存独立原始 inbound/outbound JSONL。
- [ ] 7.2 保存 app-server stderr、Codex 版本、启动参数和退出信息。
- [ ] 7.3 原始日志进行敏感字段过滤，不写入 token 或凭证。
- [ ] 7.4 DemoSession 日志保存结构化 Item 状态，而不只保存扁平事件。
- [ ] 7.5 应用刷新后可以加载最近一次 DemoSession 时间线。
- [ ] 7.6 保存 codexThreadId，并为后续 `thread/resume` 做准备。
- [ ] 7.7 启动时比较当前 CLI 版本和上次验收版本，版本变化时给出提示。
- [ ] 7.8 在 CI 或开发检查中验证使用到的字段仍存在于生成 schema。
- [ ] 7.9 未知事件只降级展示，不导致 AgentRun 失败。

退出标准：

- 任意失败都能从 `.voicecoder` 日志还原最后收到的协议消息。
- 刷新 VoiceCoder 后仍能查看上一轮时间线和失败原因。
- Codex CLI 升级导致协议变化时能尽早发现，而不是在运行中静默丢事件。

## Milestone 8：测试与最终验收

### 自动测试

- [ ] 8.1 JSON-RPC response / notification / server request 路由测试。
- [ ] 8.2 app-server schema fixture 解析测试。
- [ ] 8.3 Item started / delta / completed upsert 测试。
- [ ] 8.4 文件 add / update / delete / move 和 diff 行数测试。
- [ ] 8.5 命令 output delta、exit code、duration 测试。
- [ ] 8.6 retryable error 不终止 AgentRun 测试。
- [ ] 8.7 failed / interrupted turn 状态测试。
- [ ] 8.8 auto review started / completed 测试。
- [ ] 8.9 未知事件和 stderr 日志测试。
- [ ] 8.10 高频 delta 合并和内存上限测试。

### 手工验收场景

- [ ] 8.11 新建一个包含多个文件的前端 Demo。
- [ ] 8.12 连续修改同一个文件多次，确认页面只更新一张文件卡片。
- [ ] 8.13 同一轮新增、修改、删除和移动文件。
- [ ] 8.14 运行成功、失败和长时间无输出的命令。
- [ ] 8.15 触发一次自动审批批准和一次自动审批拒绝。
- [ ] 8.16 模拟可重试网络错误和最终失败。
- [ ] 8.17 模拟未知 notification，确认主流程继续运行。
- [ ] 8.18 运行期间刷新页面，确认可以恢复时间线。
- [ ] 8.19 Agent 完成后自动启动 dev server 并打开 preview。

最终验收标准：

- Codex 开始工作后，页面不会出现超过合理阈值的无解释空白等待。
- 文件创建或修改期间持续显示文件名和 `+N/-N`。
- 命令执行、工具调用、自动审批和重试都有明确状态。
- 普通 Demo 生成无需人工审批即可完成。
- 自动审查拒绝、sandbox 拒绝和协议异常不会表现成假死。
- 完成后的文件统计、文件树和 preview 与真实项目状态一致。

## 推荐开发顺序与提交边界

每个提交只完成一个可验证的协议或 UI 能力：

1. `fix: align app-server approval policy with current schema`
2. `refactor: route app-server json-rpc message kinds`
3. `feat: persist raw app-server logs and stderr`
4. `refactor: upsert agent items by item id`
5. `feat: render live file diff statistics`
6. `feat: stream command execution output`
7. `feat: replace compact progress list with timeline`
8. `feat: render remaining app-server item types`
9. `feat: handle automatic approval lifecycle`
10. `feat: restore persisted agent run timeline`
11. `test: cover app-server protocol fixtures and acceptance cases`

## 非目标

- 本 Roadmap 不实现 Phase 5 的 Git 回滚和逐文件接受/拒绝。
- 不在第一轮加入复杂多 Agent 编排。
- 不把 raw reasoning 默认暴露给普通用户。
- 不为了兼容未知未来协议而直接展示未经处理的完整 JSON。
- 不让 app-server 自行启动项目 dev server；仍由 VoiceCoder 统一管理 preview 进程。
