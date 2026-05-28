# Roadmap

目标：先把跨平台桌面外壳、语音输入、需求整理闭环做扎实，再接入 Codex 自动编码能力。

## Phase 0：项目准备

交付物：

- Tauri + React + TypeScript 项目骨架。
- 基础目录结构：`app/`、`docs/`、`experiments/`。
- `.env.example`，只放变量名，不放真实密钥。
- 基础 lint、format、test 命令。

验收标准：

- Fedora 本机能启动开发窗口。
- 项目能跑通 `npm run lint`、`npm test` 或等价命令。
- README 或文档中说明开发启动方式。

## Phase 1：桌面外壳与项目管理

交付物：

- 三栏主界面：项目/文件树、对话/需求、预览/日志。
- 本地项目选择。
- 文件树浏览。
- 本地状态持久化：最近项目、窗口布局、基础设置。

验收标准：

- 可选择一个前端项目并展示文件树，添加项目，最右侧自动显示文件夹内文件
- 路径处理不写死 Linux 路径，保留 Windows/macOS 兼容性。

## Phase 2：语音识别实验

交付物：

- `experiments/asr-tencent` 实验。
- 腾讯云实时说话人分离 WebSocket API 直连。
- 音频采集、16kHz/16bit/mono PCM 转换、200ms 分片发送。
- 实时展示 partial/final 结果和 `speaker_id`。

验收标准：

- 能用麦克风实时获得识别文本。
- 能解析并展示说话人 ID。
- 断线、鉴权失败、麦克风权限失败有明确错误信息。

## Phase 3：语音接入主应用

交付物：

- 主应用录音开关。
- 语音转写流进入对话区。
- 转写结果按说话人和时间分组。
- ASR provider 抽象，腾讯云只是第一个实现。

验收标准：

- 前端不接触腾讯云 SecretKey。
- 停止录音后 WebSocket 和音频资源正确释放。
- 可以用 mock ASR provider 做测试。

## Phase 4：需求整理与澄清

交付物：

- 结构化需求状态：目标、约束、页面、组件、交互、待澄清问题。
- 周期性总结。
- 主动追问。
- 用户确认后生成 coding prompt。

验收标准：

- 文本输入和语音输入都进入同一需求状态机。
- 不明确需求不会直接触发编码。
- coding prompt 可预览、可编辑、可确认。

## Phase 5：Codex MVP 集成

交付物：

- `experiments/codex-exec` 实验。
- 使用 `codex exec --json --sandbox workspace-write --cd <project>`。
- 解析 JSONL 事件流。
- 展示 agent message、命令执行、文件变更摘要。

验收标准：

- 可对测试前端项目发起一次代码修改。
- 可看到 Codex 过程日志和最终结果。
- 失败时保留原始事件日志用于排查。

## Phase 6：预览与反馈闭环

交付物：

- 自动识别并启动前端 dev server。
- 内嵌预览页面。
- 修改代码后自动刷新。
- 展示文件 diff。

验收标准：

- Vite/Next 等常见项目至少支持一个。
- dev server 可启动、停止、重启。
- 预览失败时能展示端口、命令和 stderr。

## Phase 7：Codex App Server 集成

交付物：

- `codex app-server` 子进程管理。
- 生成并使用协议类型。
- 支持线程、turn、流式事件、审批请求。
- 文件变更、命令审批、用户追问都回到主 UI。

验收标准：

- 可持续对同一项目多轮编码。
- 用户能在 UI 中批准或拒绝敏感操作。
- app-server 协议版本变化有兼容处理。

## Phase 8：跨平台打包

交付物：

- Linux 包。
- Windows 包。
- macOS 包。
- 跨平台凭证存储。
- 平台差异测试清单。

验收标准：

- Linux/Windows/macOS 至少完成一次本地打包验证。
- 麦克风权限、文件选择、子进程、路径处理在各平台可用。
- 不把任何真实密钥打进安装包。
