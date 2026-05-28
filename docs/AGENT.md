# AGENT.md

本项目是一个跨平台桌面编码工作台：Tauri + React + TypeScript 前端，Rust 后端负责本地文件、子进程、语音和 Codex 集成。

## 工作原则

- 先做桌面外壳、项目管理、语音链路和需求状态，再接 Codex。
- 第一开发平台是 Fedora，但代码必须按 Linux/Windows/macOS 跨平台设计。
- 平台差异集中在 Tauri/Rust 后端或明确的 `platform` 层，不要散落在 React UI 中。
- 腾讯 ASR 使用 WebSocket API 直连，SDK 只作为参考。
- Codex 先作为外部二进制调用，不要一开始 fork `openai/codex`。

## Commit 管理

- 每个 commit 只做一件清晰的事。
- 能跑测试的改动，commit 前必须跑相关测试。
- 不要把真实 API key、token、录音文件、构建产物提交进仓库。
- 大功能按阶段拆小 commit：骨架、UI、状态、后端命令、测试分别提交。
- 如果工作树里有用户已有改动，不要回滚或覆盖；先理解，再在自己的改动范围内继续。

## 测试要求

- 业务逻辑优先写单元测试：需求状态机、ASR 事件解析、Codex JSONL 事件解析。
- 跨平台风险点要有测试或明确的手动验证记录：路径、子进程、环境变量、文件读写。
- ASR 和 Codex 这类外部服务必须有 mock provider，不能让普通测试依赖真实云服务。
- UI 改动至少保证启动无报错；关键流程后续补 Playwright 或等价端到端测试。

## 实现边界

- 前端不能直接持有 `TENCENTCLOUD_SECRET_KEY`。
- 不要在 UI 层手写系统路径拼接。
- 不要把 Fedora 专用命令写进通用逻辑。
- 不要默认自动修改用户真实项目；第一轮 Codex 集成只对测试项目开放。
- 不要过早引入复杂插件系统，先用清晰 provider 接口隔离 ASR、LLM、Codex。
