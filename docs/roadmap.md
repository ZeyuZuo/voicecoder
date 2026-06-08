# Roadmap

目标：构建一个跨平台桌面编码工作台。用户可以选择本地项目，通过文本或语音描述需求，系统持续整理和澄清需求，在确认后调用 Coding Agent 修改代码，并在同一个界面里查看文件、变更、预览和终端反馈。

当前产品方向已经收敛：

- 前端 App Shell 已经具备可继续开发的基础形态，暂时不再投入大量 UI 打磨。
- 下一阶段优先实现语音输入链路，让“说需求”成为产品的核心差异点。
- 前端后续只围绕真实功能补必要控件和状态，不再先做空入口。
- 所有本地能力优先通过 Tauri 后端封装，保持跨平台桌面软件结构。

## 当前状态：Phase 1 已完成

已完成：

- Tauri + React + TypeScript 应用骨架。
- 三栏主布局：
  - 左侧项目和对话列表。
  - 中间对话输入区。
  - 右侧工作区。
- 左右侧栏折叠、展开、放大和手动拖拽调整宽度。
- 项目选择：
  - 使用现有文件夹。
  - 不使用项目。
  - 已选择项目复用，不重复创建。
- 右侧工作区标签页架构：
  - 文件。
  - 浏览器。
  - 审查。
  - 终端。
- 文件工作区第一版：
  - 读取项目文件树。
  - 支持浏览器预览模式下的目录句柄读取。
  - Tauri 客户端下读取真实本地路径。
- Git 分支显示：
  - Git 项目显示真实当前分支。
  - 非 Git 项目不显示分支组件。
- 基础响应式和窄侧栏适配。

暂时不继续做：

- 更复杂的前端视觉打磨。
- 未接真实能力的设置页、插件页、自动化页。
- 文件内容编辑器、diff viewer、终端和浏览器的完整实现。

判断：前端壳子已经够用了。接下来应该让产品获得真实能力，而不是继续装修空面板。

## 当前状态：Phase 2 语音输入 MVP 已跑通

目标：先把麦克风输入、实时转写、录音状态和转写结果跑通，为后续需求整理和多人讨论打基础。

已完成：

- 已建立前后端语音事件协议。
- 中间输入框麦克风按钮已接入语音状态。
- 前端会通过 Web Audio 请求麦克风权限，采集音频并转换为 16kHz / 16bit / mono PCM。
- 前端按 provider 推荐值分片发送给 Tauri 后端：腾讯/Mock/火山使用约 200ms / 6400 bytes，讯飞大模型使用 40ms / 1280 bytes。
- 当前采集关闭浏览器 echo cancellation、noise suppression 和 auto gain control，尽量保留原始音色，便于后续 provider 做 speaker diarization。
- 停止录音时会 flush 不足当前 provider 分片大小的尾包，避免最后一小段音频丢失。
- final 转写句子会自动追加进中间输入框，语音结果进入真实 prompt 输入链路。
- 同一 final 句子更新时会替换已有文本，避免重复灌入 prompt。
- 后端已提供语音 session 生命周期管理：
  - 开始。
  - 停止。
  - 分片接收。
  - 资源释放。
- 腾讯云停止流程会发送结束包并等待最终转写，避免用户点击停止时丢失 final 句子。
- 等待 final 期间前端会锁住麦克风按钮，避免重复停止打乱会话。
- 错误恢复和启动前旧会话清理使用强制取消，不会卡在等待 final 的路径上。
- 后端已提供语音 session snapshot 诊断命令，用于确认是否存在活动会话、当前 provider 和已收到分片数。
- Mock ASR Provider 已可推送 partial / final 转写事件，用于稳定联调 UI 和状态机。
- 腾讯云 ASR Provider 已具备后端直连骨架：
  - 本地环境变量读取凭证。
  - WebSocket 签名 URL 生成。
  - 音频分片发送。
  - `sentences.sentence` / `sentences.sentence_type` / `sentences.speaker_id` 返回解析。
  - 普通实时 ASR `result.voice_text_str` / `result.slice_type` 返回解析。
  - speaker id、时间戳和 final 状态映射。
  - 对齐腾讯官方 speaker SDK 参数：`result_mod=1`、`speaker_diarization=1`、`sentence_strategy=0`、`enable_speaker_context=0`。
  - `speaker_id=-1` 作为未稳定识别处理，不展示为用户 speaker。
  - 腾讯返回的 speaker index 会映射为从 1 开始的 UI 标签，例如 `0 -> speaker-1`。
  - 后端保留 speaker 诊断日志，用于观察腾讯真实返回的 speaker id 集合。
- 自动 Provider 策略：
  - 本地存在讯飞大模型凭证时使用讯飞。
  - 未配置讯飞但存在腾讯云凭证时使用腾讯云。
  - 未配置讯飞和腾讯但存在火山引擎凭证时使用火山。
  - 未配置凭证时自动回退 Mock。

实测结论：

- Tauri 客户端内真实麦克风采集和腾讯云实时转写可以跑通。
- 腾讯云实时 speaker diarization 在当前单麦多人场景下不稳定：
  - 云端会返回大量 `speaker_id=-1`。
  - 同一个人可能在 `speaker_id=0` 和 `speaker_id=1` 之间摇摆。
  - 因此 speaker 标签不能作为产品强可信能力。
- Phase 2 保留 speaker 解析和诊断日志，但默认产品能力应聚焦“可靠语音输入”，不要依赖腾讯实时 speaker 结果做关键逻辑。

下一步 provider 方向：

- 优先评估讯飞实时语音转写：
  - 标准版支持 `roleType=2` 实时角色分离。
  - 大模型版支持 `role_type=2`，并可通过 `feature_ids` 做声纹分离。
- 火山引擎豆包语音大模型已作为实验 provider 接入，默认使用 `bigmodel_async`、`enable_nonstream`、`enable_speaker_info` 和 `ssd_version=200`，用于横向比较内容识别和 speaker label 稳定性。
- 如果需要稳定区分固定人员，优先考虑“声纹注册 + 实时转写”，而不是纯盲分。
- 阿里、百度更适合作为“录音后处理 / 文件转写 + 说话人分离”的备选，不建议直接押注实时盲分。

验收步骤见 `docs/phase2-voice-acceptance.md`。

交付物：

- 输入框语音按钮接入真实状态：
  - 空闲。
  - 请求麦克风权限。
  - 录音中。
  - 转写中。
  - 出错。
  - 已停止。
- Tauri 后端语音模块：
  - 麦克风权限检查。
  - 音频采集入口。
  - 音频流生命周期管理。
  - 停止录音时释放资源。
- ASR Provider 抽象：
  - `start()`
  - `sendAudioChunk()`
  - `onPartial()`
  - `onFinal()`
  - `onError()`
  - `stop()`
- Mock ASR Provider：
  - 不依赖云服务。
  - 用于前端联调和状态机测试。
- 腾讯云实时说话人分离实验：
  - WebSocket 签名连接。
  - 16kHz / 16bit / mono PCM 音频分片。
  - 约 200ms 一包发送。
  - 接收 partial / final 文本。
  - 解析 speaker id、时间戳和普通实时识别句子。
- 语音转写面板或对话流展示：
  - 实时临时文本。
  - 最终句子。
  - final 句子追加到输入框。
  - 说话人诊断标记。
  - 错误提示。

验收标准：

- 点击语音按钮后可以开始录音。
- 停止后麦克风释放，并等待 ASR final 结果后关闭 WebSocket。
- Mock Provider 可以稳定模拟一段转写流程。
- 腾讯云 Provider 可以在真实麦克风下得到实时文本。
- final 转写可以进入输入框，成为后续发送或需求整理的文本来源。
- 前端不接触 SecretKey。
- 出现麦克风权限失败、鉴权失败、网络断开时有明确 UI 反馈。
- 腾讯云 speaker diarization 只作为实验性诊断，不作为 Phase 2 验收必需项。

## Phase 3：语音转需求状态机

目标：把语音转写结果从“文字流”升级成“可确认的需求”。

交付物：

- 文本输入和语音输入进入同一个需求流。
- 转写句子按时间顺序进入对话上下文。
- 多人语音结果可以保留 provider 返回的 speaker 信息，但默认不强依赖 speaker 准确性。
- 需求整理状态：
  - 原始输入。
  - 当前理解。
  - 待澄清问题。
  - 已确认约束。
  - 验收标准。
- AI 总结当前需求。
- AI 主动追问。
- 用户确认需求后生成 Coding Prompt。

验收标准：

- 语音输入不会只停留在转写文本，而是能被整理成结构化需求。
- 不明确需求不会直接触发编码。
- 用户可以编辑或确认整理后的 Coding Prompt。
- 文本和语音混合输入时上下文连续。

## Phase 4：Coding Agent MVP

目标：跑通“确认需求后自动修改代码”的第一条闭环。

交付物：

- Coding Agent Provider 抽象。
- 第一版使用 `codex exec --json --sandbox workspace-write --cd <project>`。
- Tauri 后端管理子进程。
- JSONL 事件解析：
  - agent message。
  - plan / reasoning。
  - command execution。
  - file change。
  - final result。
  - error。
- 对话区展示执行进度。
- 执行完成后展示变更摘要。

验收标准：

- 对一个测试前端项目可以完成一次真实代码修改。
- 用户能看到 Agent 正在做什么。
- 失败时保留可排查日志。
- 执行前必须确认目标项目和 Coding Prompt。

## Phase 5：审查与 Diff

目标：让用户能看懂 Agent 改了什么，并决定继续、接受或回滚。

交付物：

- Git 状态读取。
- 本轮变更文件列表。
- Diff Viewer。
- 变更摘要卡片。
- 操作按钮：
  - 继续修改。
  - 打开文件。
  - 回滚本轮变更。

验收标准：

- 每轮 Agent 完成后能看到清晰变更摘要。
- 可以逐文件查看 diff。
- 回滚只作用于本轮 Agent 变更，不误删用户已有改动。
- 项目存在未提交改动时明确提示风险。

## Phase 6：预览与终端

目标：让用户改完代码后能在同一个应用里看结果和日志。

交付物：

- 识别常见前端项目启动命令。
- Dev server 启动、停止、重启。
- 右侧浏览器标签页接入真实预览。
- 终端标签页展示命令日志。
- 端口占用、依赖缺失、构建失败等错误提示。

验收标准：

- 至少支持 Vite 项目启动和预览。
- 代码修改后可以刷新预览。
- 终端日志可复制、清空、停止。

## Phase 7：长期能力

目标：在核心闭环稳定后，再扩展成完整工作台。

候选方向：

- Codex app-server 深度集成。
- 多轮会话恢复。
- 审批流。
- 本地设置和凭证管理。
- 多 ASR Provider：
  - 腾讯云。
  - 讯飞。
  - 阿里云。
  - 本地 FunASR。
- 多 Coding Agent Provider。
- 自动化任务。
- 技能和插件系统。
- 跨平台打包、签名和发布。

这些能力不进入当前优先级，除非语音和 Coding Agent 主链路已经稳定。

## 近期开发顺序

建议下一步按这个顺序推进：

1. 语音按钮状态和 Mock ASR Provider。
2. Tauri 后端语音模块骨架。
3. 腾讯云 ASR WebSocket 签名和连接实验。
4. 麦克风音频采集、重采样和分片发送。
5. 实时转写结果进入对话流。
6. 语音转需求总结。
7. 确认需求后接 Coding Agent MVP。

## 风险

- 浏览器预览无法代表完整客户端能力，语音、文件、Git、子进程应以 Tauri 客户端验证为准。
- 实时语音识别受麦克风、噪声、网络和云服务稳定性影响，需要尽早真实测试。
- 多人说话人分离可能在短句、重叠说话、噪声场景下不稳定，第一版要允许用户手动修正文本。
- SecretKey 不能进入前端代码、日志或仓库。
- Coding Agent 修改本地文件有风险，必须先做好确认、日志和回滚边界。
