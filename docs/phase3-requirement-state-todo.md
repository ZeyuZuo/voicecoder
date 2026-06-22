# Phase 3 Requirement State Todo

日期：2026-06-08

## 背景

Phase 2 已经完成语音输入 MVP：

- 麦克风输入可以转为 16kHz / 16bit / mono PCM。
- Mock、腾讯云、讯飞大模型、火山引擎 ASR 已接入统一 provider adapter。
- final transcript 会自动进入中间输入框。

Phase 3 的入口是语音输入按钮。用户点击麦克风按钮后，系统进入语音专用需求采集模式，隐藏或禁用普通文本输入，只允许用户通过语音描述需求。系统持续记录本轮说话内容，按安静期、批量阈值或最长等待时间实时整理当前理解，并用小云朵提示“当前理解”和“还缺什么”。用户继续自然说话即可补充缺口；缺口被新语音覆盖后，小云朵自动消除对应提示。用户最后只需要点击一次“我说完了”，系统停止录音、等待最后 ASR final、生成完整需求文档并写入项目 `.voicecoder` 目录。Phase 3 不再要求用户一轮一轮点击回答澄清问题。

## 目标

- 用户点击语音输入按钮后才进入需求采集流程。
- 进入语音需求采集模式后，普通文本输入不再作为需求入口。
- ASR final transcript 进入本轮语音需求会话，成为需求整理的唯一主输入。
- 系统实时维护结构化需求状态，而不是只保存一段长文本。
- LLM 通过真实 OpenAI-compatible API 实时理解需求、识别缺口、生成需求文档和确认版 Coding Prompt。
- 用户确认之前，系统不得自动触发编码。
- LLM 调用必须通过 provider adapter 抽象，第一版实现真实 `openai_compatible` provider。
- API key、base URL 和模型配置只在 Tauri 后端读取，不能进入前端代码。

## 非目标

- Phase 3 不直接调用 Codex 修改代码。
- Phase 3 不实现完整 diff、review、terminal、browser 闭环。
- Phase 3 不依赖 speaker diarization 的准确性做关键逻辑；speaker 只作为上下文和诊断信息。
- Phase 3 不把普通文本框作为主需求输入入口；主入口是语音输入按钮。
- Phase 3 不实现 mock LLM provider；开发和验收直接使用真实 OpenAI-compatible API。
- Phase 3 不先做复杂设置页，开发期继续用 `.env` 配置。

## 核心流程

```text
用户点击语音输入按钮
  ↓
进入语音专用需求采集模式，隐藏或禁用普通文本输入
  ↓
创建 VoiceRequirementSession
  ↓
ASR final transcript
  ↓
RequirementUtterance 追加到需求流
  ↓
LLM 按时间间隔或批量阈值增量总结
  ↓
页面展示当前理解和原始语音记录
  ↓
用户点击“我说完了”
  ↓
停止录音并等待最后 ASR final
  ↓
LLM 生成完整需求文档和 Coding Prompt
  ↓
写入 .voicecoder/voice_requirements_{time}.md
  ↓
页面展示需求文档，可展开查看
```

## 需求状态机

```text
idle
  -> listening
  -> finalizing
  -> document_ready
  -> confirmed
```

状态含义：

- `idle`：没有正在整理的需求。
- `listening`：用户已经打开语音需求模式，系统持续接收 ASR final transcript 并实时维护当前理解。
- `finalizing`：用户点击“我说完了”后，系统停止录音、等待最后 ASR final，并生成完整需求文档。
- `document_ready`：需求文档已经生成并写入项目目录，页面展示文档卡片，可展开查看。
- `confirmed`：用户已确认需求文档，可作为 Phase 4 Coding Agent 的输入。

约束：

- `confirmed` 之前不得触发 Coding Agent。
- 小云朵缺口提示只是实时辅助，不是阻塞式提问；用户继续自然说话即可补充。
- `finalizing` 之后不允许继续录音；如需大幅修改，应另开一轮语音需求会话或后续做“重新补充需求”操作。
- 如果点击“我说完了”时仍存在缺口，系统仍生成需求文档，但必须在文档中写明“未明确项 / 默认假设”，不能静默编造确定事实。
- 普通文本输入模式不进入这套状态机；这套状态机只在用户点击语音按钮后接管本轮语音需求会话。
- `confirmed` 后生成的 Coding Prompt 只是 Phase 3 产物，不在 Phase 3 自动交给 Coding Agent 执行。

## 输入语义边界

每一条 ASR final transcript 在 `listening` 状态下都标记为 `source="voice"`，表示用户正在自由描述或补充本轮需求。

- `idle`：没有需求会话，收到 stray transcript 时忽略。
- `listening`：允许录音和追加 transcript。
- `finalizing`：禁止录音；只允许等待已经在途的最后 ASR final 落入当前需求会话。
- `document_ready`：禁止录音；只展示生成后的需求文档。
- `confirmed`：禁止录音；只展示已确认的需求文档和 Coding Prompt 草稿。

`start_voice_session` 只在没有需求会话时创建 `VoiceRequirementSession` 并进入 `listening`。如果需求会话已经存在，点击麦克风只能在 `listening` 状态下继续当前语音会话，不能把 `finalizing`、`document_ready` 或 `confirmed` 拉回 `listening`。

## 防误操作规则

```ts
type VoiceInputPermission = {
  canUseMic: boolean;
  canFinishTurn: boolean;
  transcriptSource?: "voice";
};
```

规则：

- `idle`：允许麦克风，创建语音需求会话，进入 `listening`。
- `listening`：允许麦克风，final transcript 作为 `voice`；有内容后允许“我说完了”。
- `finalizing`：禁止麦克风，禁止重复点击“我说完了”。
- `document_ready`：禁止麦克风，只展示需求文档，允许用户确认。
- `confirmed`：禁止麦克风，只展示已确认需求文档和 Coding Prompt 草稿。

这些规则必须集中在一个 helper 中，组件只能消费 helper 结果，不能在多个组件里散落判断。

## 数据模型草案

### RequirementUtterance

```ts
type RequirementUtterance = {
  id: string;
  source: "voice";
  speakerId?: string;
  text: string;
  createdAt: string;
  transcriptId?: string;
};
```

### VoiceRequirementSession

```ts
type VoiceRequirementSession = {
  id: string;
  voiceSessionIds: string[];
  requirementState: RequirementState;
  startedAt: string;
  endedAt?: string;
};
```

### RequirementState

```ts
type RequirementState = {
  id: string;
  status:
	    | "idle"
	    | "listening"
	    | "finalizing"
	    | "document_ready"
	    | "confirmed";
  utterances: RequirementUtterance[];
  summary: string;
  requirementDocument: string;
  confirmedFacts: string[];
  constraints: string[];
	  openGaps: RequirementGap[];
  acceptanceCriteria: string[];
  outOfScope: string[];
  risks: string[];
  codingPrompt?: string;
	  pendingAction?: "summarize" | "finalize" | "save";
  updatedAt: string;
};
```

### RequirementGap

```ts
type RequirementGap = {
  id: string;
  question: string;
  reason: string;
  severity: "blocking" | "helpful";
  status: "open" | "resolved";
};
```

## LLM Provider Adapter

LLM 必须和 ASR 一样使用 provider adapter 架构。

```text
LlmProvider
  ├─ kind()
  ├─ validate_start()
  ├─ diagnostic()
  └─ complete_json(request) -> LlmJsonResponse
```

第一版 provider：

```text
openai_compatible
```

后续可扩展：

- `openai`
- `qwen`
- `deepseek`
- `volcengine`
- `local_openai_compatible`

### 配置

开发期通过 `app/.env` 配置：

```env
VOICECODER_LLM_PROVIDER=openai_compatible
VOICECODER_LLM_BASE_URL=https://api.openai.com/v1
VOICECODER_LLM_API_KEY=
VOICECODER_LLM_MODEL=
VOICECODER_LLM_TEMPERATURE=0.2
VOICECODER_LLM_TIMEOUT_SECS=30
VOICECODER_LLM_STRICT_JSON_MODE=true
```

约定：

- `VOICECODER_LLM_BASE_URL` 指向 OpenAI-compatible API base URL，例如 `https://api.example.com/v1`。
- 后端请求 Chat Completions 时拼接 `/chat/completions`。
- `VOICECODER_LLM_API_KEY` 只在 Tauri 后端读取。
- 前端只展示脱敏 provider diagnostic。

### OpenAI-compatible 请求形态

第一版使用 Chat Completions 兼容接口：

```http
POST {base_url}/chat/completions
Authorization: Bearer {api_key}
Content-Type: application/json
```

请求体：

```json
{
  "model": "configured-model",
  "temperature": 0.2,
  "messages": [
    {
      "role": "system",
      "content": "You maintain a structured requirement state..."
    },
    {
      "role": "user",
      "content": "JSON payload with current state and new utterances"
    }
  ],
  "response_format": {
    "type": "json_object"
  }
}
```

兼容性要求：

- 如果某些 provider 不支持 `response_format`，允许配置关闭严格 JSON mode。
- 后端必须对 LLM 输出做 JSON parse 和 schema 校验。
- JSON parse 失败时不能污染当前需求状态，应返回错误给 UI。
- `complete_json` 统一处理 timeout、HTTP 错误、OpenAI-compatible error body、`choices[0].message.content` 提取、markdown JSON fence 清理和 JSON object 校验。
- `test_llm_provider_connection` 使用同一条 `complete_json` 路径发起极小健康检查，要求模型返回 `{ "ok": true }`，不读取或修改前端需求状态。

## LLM 任务类型

### 1. 实时需求理解

输入：

- 当前完整 `RequirementState`。
- 全量 `RequirementUtterance[]`，由 LLM 自行综合当前需求画像。

输出：

```json
{
  "summary": "",
  "confirmedFacts": [],
  "constraints": [],
  "acceptanceCriteria": [],
  "outOfScope": [],
  "risks": [],
  "openGaps": [
    {
      "id": "",
      "question": "",
      "reason": "",
      "severity": "blocking",
      "status": "open"
    }
  ]
}
```

触发条件：

- 有新增 final transcript 且没有其它 LLM 请求在运行时，等待约 6 秒安静期后更新一次“小云朵”。
- 累计 2-3 条新增 final transcript 时，可以提前更新。
- 距离上次实时理解超过约 30 秒时，即使用户持续说话也应更新一次。

约束：

- 小云朵实时理解只能更新 `summary`、结构化事实和 `openGaps`，不能生成需求文档。
- `openGaps` 最多展示 3 条，优先展示会影响目标、范围、验收或关键交互的缺口。
- 当用户后续语音已经回答某个缺口，LLM 应把该缺口移除或标记为 `resolved`；前端不再展示 resolved 缺口。
- 小云朵不能推进到最终文档状态。

### 2. 最终需求文档生成

输入：

- 当前完整 `RequirementState`。
- 原始语音记录。
- 实时理解阶段维护的 `openGaps`。

输出：

```json
{
  "summary": "",
  "requirementDocumentDraft": "",
  "confirmedFacts": [],
  "constraints": [],
  "acceptanceCriteria": [],
  "outOfScope": [],
  "risks": [],
  "questions": [],
  "readyToConfirm": true
}
```

约束：

- `requirementDocumentDraft` 应面向用户阅读，结构清晰，不应只是 Coding Agent 指令。
- 如果仍有未明确项，写入文档的“未明确项 / 默认假设”部分，不再阻塞生成。
- `questions` 仅作为兼容字段保留，最终整理阶段应返回空数组。
- `readyToConfirm` 在最终整理成功时应为 `true`，前端进入 `document_ready`。

## 前端 UI 草案

点击麦克风后，Composer 从普通文本输入切换为语音需求采集工作台。语音模式下不展示可编辑的普通 prompt 文本框。

工作台展示：

- 录音状态、ASR provider、LLM provider、当前需求阶段。
- 实时转写和最近几条 final transcript。
- 小云朵形式的当前理解和最多 3 条关键缺口。
- 点击“说完了”后的需求文档卡片。
- 需求文档保存路径。

核心操作：

- 麦克风按钮：开始或继续本轮语音需求采集。
- `listening` 下的“我说完了”：停止录音、等待最后 ASR final 并进入 `finalizing`。
- `finalizing` 下禁用麦克风和重复提交。
- `document_ready` 下展示完整需求文档，用户可点击确认进入 `confirmed`。

第一版不需要复杂视觉打磨，但必须像一个语音访谈工作台，而不是把状态面板附着在普通 prompt 文本框下面。普通 prompt 文本框不作为 Phase 3 的输入入口；Coding Prompt 只作为确认后的只读产物展示给 Phase 4。

## 后端命令草案

```text
get_llm_provider_status() -> LlmProviderStatus
test_llm_provider_connection() -> LlmConnectionTestResult
summarize_requirement_state(request) -> RequirementSummaryResult
process_requirement_turn(request) -> RequirementProcessingResult
finalize_requirement_document(request) -> RequirementFinalizationResult
```

诊断模型：

```text
LlmProviderDiagnostic
  ├─ provider
  ├─ configured
  ├─ missing_env
  ├─ endpoint
  ├─ model
  ├─ details
  └─ error
```

`get_llm_provider_status` 额外返回当前 resolved provider 的可用性：

```text
LlmProviderStatus
  ├─ auto_provider
  ├─ provider_override
  ├─ active_provider_configured
  ├─ active_provider_error
  └─ diagnostics
```

## 开发 Todo

- [ ] Step 1：更新 Phase 3 TypeScript 类型：`VoiceRequirementSession`、`RequirementUtterance`、`RequirementState`、`RequirementGap`、`requirementDocument`、`pendingAction`。
- [ ] Step 2：实现前端 requirement reducer，让语音 final transcript 只进入当前 active `VoiceRequirementSession`。
- [ ] Step 3：点击麦克风后切换到语音需求采集工作台，隐藏或禁用普通文本输入。
- [ ] Step 4：实现语音工作台 UI，展示实时转写、小云朵当前理解和缺口提示、需求文档卡片和 Coding Prompt 只读草稿。
- [x] Step 5：新增 Rust `llm` 模块，抽出 `LlmProvider`、`LlmProviderDiagnostic` 和 provider registry。
- [x] Step 6：实现真实 `openai_compatible` LLM provider，支持 base URL、API key、model、temperature、timeout。
- [x] Step 7：新增 `get_llm_provider_status` 命令，前端可显示 LLM 配置状态和缺失环境变量。
- [x] Step 8：实现 `summarize_requirement_state` 命令，输入当前状态，输出实时理解和缺口提示。
- [x] Step 9：实现 `process_requirement_turn` 命令，用户点击“说完了”后生成最终需求文档。
- [ ] Step 10：按需实现独立 `finalize_requirement_document` 命令；当前可复用 `process_requirement_turn` 的最终整理语义。
- [x] Step 11：增加 LLM JSON 输出 schema 校验和错误恢复，避免坏响应污染当前状态。
- [x] Step 12：增加 debounce / batching 策略，避免每条 transcript 都调用 LLM。
- [x] Step 13：把“我说完了”接到 `finalizing`，取消显式澄清循环，但不自动编码。
- [x] Step 14：补充单元测试：状态机 reducer、防误操作规则、LLM JSON parser、provider diagnostics、OpenAI-compatible 响应 parser。
- [ ] Step 15：补充 Phase 3 验收文档，记录真实 LLM provider 的配置和验收流程。

## 验收标准

- 点击语音按钮后进入语音专用需求采集模式，普通文本输入不再作为需求入口。
- 语音 final transcript 会进入需求流，而不是追加到 prompt 文本框。
- 未点击语音输入按钮时，不会创建 Phase 3 需求会话。
- 用户边说话，系统能按安静期、批量阈值或最长等待时间周期性更新“当前理解”和最多 3 个缺口提示。
- 用户继续说话补充缺口后，小云朵缺口能自动减少或消失。
- 用户点击“我说完了”后，系统能停止录音、生成需求文档并写入 `.voicecoder`。
- `finalizing`、`document_ready` 和 `confirmed` 下点击麦克风不会回到 `listening`，也不会追加新的 transcript。
- 系统能形成一份面向用户确认的完整需求文档。
- 用户确认前不会触发 Coding Agent。
- 用户确认后能生成可用于 Phase 4 的 Coding Prompt。
- LLM provider 配置缺失时，UI 有明确诊断，不能静默降级到假结果。
- API key 不进入前端代码、前端状态、日志或文档。
- Phase 3 验收使用真实 OpenAI-compatible API。

## 风险

- ASR 文本可能有错字，LLM 总结时必须保留不确定性，不要擅自补全关键需求。
- speaker label 不稳定，不能基于 speaker 自动判断“谁有最终决策权”。
- 增量总结可能漂移，需要保留 utterance 原文用于回溯。
- OpenAI-compatible provider 的兼容程度不同，`response_format`、streaming、错误格式可能不一致。
- 如果 LLM 输出非法 JSON，必须失败得清楚，不能静默改坏需求状态。
- 不做 mock LLM 会让本地验收依赖真实 API key 和网络；普通测试应集中覆盖纯状态机和 JSON parser，不测试真实模型质量。
