# Phase 3 Requirement State Todo

日期：2026-06-08

## 背景

Phase 2 已经完成语音输入 MVP：

- 麦克风输入可以转为 16kHz / 16bit / mono PCM。
- Mock、腾讯云、讯飞大模型、火山引擎 ASR 已接入统一 provider adapter。
- final transcript 会自动进入中间输入框。

Phase 3 的入口是语音输入按钮。用户点击麦克风按钮后，系统进入语音专用需求采集模式，隐藏或禁用普通文本输入，只允许用户通过语音描述需求、补充信息和回答澄清问题。系统持续记录本轮说话内容，按时间间隔实时整理当前理解，并在用户点击“我说完了”后生成较完整的需求文档。如果需求仍不明确，系统必须主动追问关键问题；用户继续用语音回答，直到需求文档足够完整。只有用户确认后，系统才生成可交给后续 Coding Agent 阶段的 Coding Prompt。

## 目标

- 用户点击语音输入按钮后才进入需求采集流程。
- 进入语音需求采集模式后，普通文本输入不再作为需求入口。
- ASR final transcript 进入本轮语音需求会话，成为需求整理的唯一主输入。
- 系统实时维护结构化需求状态，而不是只保存一段长文本。
- LLM 通过真实 OpenAI-compatible API 增量总结需求、识别不明确点、生成澄清问题、生成需求文档和确认版 Coding Prompt。
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
LLM 生成完整需求文档草稿和澄清问题
  ↓
如果有阻塞问题，用户继续用语音回答澄清问题
  ↓
LLM 更新需求状态
  ↓
用户确认
  ↓
生成 Coding Prompt
```

## 需求状态机

```text
idle
  -> collecting
  -> processing
  -> clarifying
  -> processing
  -> clarifying
  -> processing
  -> ready_to_confirm
  -> confirmed
```

状态含义：

- `idle`：没有正在整理的需求。
- `collecting`：用户正在通过语音自由描述本轮需求。
- `processing`：用户点击“我说完了”或“回答完了”后，系统正在整理当前回合并判断需求是否明确。
- `clarifying`：LLM 判断存在阻塞实现或验收的问题，需要用户继续用语音回答当前问题。
- `ready_to_confirm`：需求已经足够明确，等待用户确认生成需求文档。
- `confirmed`：用户已确认需求，系统可以生成 Coding Prompt 供 Phase 4 使用。

约束：

- `confirmed` 之前不得触发 Coding Agent。
- `clarifying` 状态只问影响目标、范围、验收、交互行为或关键约束的问题，不问可由 Coding Agent 自行判断的细节。
- `clarifying -> processing -> clarifying` 可以循环多次；只有需求明确后才进入 `ready_to_confirm`。
- `ready_to_confirm` 下不允许继续语音输入；用户只能确认生成文档。后续如需修改，应另做明确的“重新补充需求”操作，不属于当前 MVP 主路径。
- 用户回答澄清问题属于本轮需求会话的语音补充信息，不是独立的文本需求入口。
- 普通文本输入模式不进入这套状态机；这套状态机只在用户点击语音按钮后接管本轮语音需求会话。
- `confirmed` 后生成的 Coding Prompt 只是 Phase 3 产物，不在 Phase 3 自动交给 Coding Agent 执行。

## 输入语义边界

每一条 ASR final transcript 必须根据当前需求状态标记来源：

- `collecting`：`source="voice"`，表示用户自由描述需求。
- `clarifying`：`source="clarification_answer"`，表示用户正在回答当前澄清问题。
- `processing`：不允许录音和追加 transcript。
- `ready_to_confirm`：不允许录音和追加 transcript。
- `confirmed`：不允许录音和追加 transcript。

`start_voice_session` 只在没有需求会话时创建 `VoiceRequirementSession` 并进入 `collecting`。如果需求会话已经存在，点击麦克风只能开始当前状态允许的下一段语音 turn，不能重置状态，也不能把 `clarifying` 或 `ready_to_confirm` 拉回 `collecting`。

## 防误操作规则

```ts
type VoiceInputPermission = {
  canUseMic: boolean;
  canFinishTurn: boolean;
  transcriptSource?: "voice" | "clarification_answer";
};
```

规则：

- `idle`：允许麦克风，创建语音需求会话，进入 `collecting`。
- `collecting`：允许麦克风，final transcript 作为 `voice`；有内容后允许“我说完了”。
- `processing`：禁止麦克风，禁止“我说完了 / 回答完了”。
- `clarifying`：允许麦克风，final transcript 作为 `clarification_answer`；有回答后允许“回答完了”。
- `ready_to_confirm`：禁止麦克风，只允许确认生成需求文档。
- `confirmed`：禁止麦克风，只展示需求文档和 Coding Prompt 草稿。

这些规则必须集中在一个 helper 中，组件只能消费 helper 结果，不能在多个组件里散落判断。

## 数据模型草案

### RequirementUtterance

```ts
type RequirementUtterance = {
  id: string;
  source: "voice" | "clarification_answer";
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
    | "collecting"
    | "processing"
    | "clarifying"
    | "ready_to_confirm"
    | "confirmed";
  utterances: RequirementUtterance[];
  summary: string;
  requirementDocument: string;
  confirmedFacts: string[];
  constraints: string[];
  openQuestions: RequirementQuestion[];
  answeredQuestions: RequirementQuestion[];
  activeQuestionId?: string;
  acceptanceCriteria: string[];
  outOfScope: string[];
  risks: string[];
  codingPrompt?: string;
  pendingAction?: "summarize" | "process" | "finalize";
  updatedAt: string;
};
```

### RequirementQuestion

```ts
type RequirementQuestion = {
  id: string;
  question: string;
  reason: string;
  blocksCoding: boolean;
  answer?: string;
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

## LLM 任务类型

### 1. 增量总结

输入：

- 当前 `RequirementState`。
- 自上次总结以来新增的 `RequirementUtterance[]`。

输出：

```json
{
  "summary": "",
  "requirementDocument": "",
  "confirmedFacts": [],
  "constraints": [],
  "openQuestions": [],
  "acceptanceCriteria": [],
  "outOfScope": [],
  "risks": [],
  "readyToConfirm": false
}
```

触发条件：

- 新增 final transcript 达到 2-3 条。
- 距离上次总结超过 5-8 秒。
- 用户持续停顿超过 2-3 秒且存在新增 final transcript。
- 用户点击“我说完了”。
- 用户点击“整理需求”。

### 2. 需求处理与澄清判断

输入：

- 当前 `RequirementState`。
- 本轮新增的 `RequirementUtterance[]`。

输出：

```json
{
  "summary": "",
  "requirementDocumentDraft": "",
  "questions": [
    {
      "question": "",
      "reason": "",
      "blocksCoding": true
    }
  ],
  "readyToConfirm": false
}
```

约束：

- 最多生成 3 个问题。
- 只问会影响实现、验收、范围或交互行为的问题。
- 不问“你想要更好看一点吗”这种泛泛问题。
- 澄清问题的回答必须继续进入本轮语音需求会话。
- `readyToConfirm=true` 且没有 blocking questions 时，前端进入 `ready_to_confirm`。
- 否则前端进入 `clarifying`。

### 3. 完整需求文档和确认版 Prompt 生成

输入：

- 当前 `RequirementState`。
- 原始语音记录。
- 已回答的澄清问题。

输出：

```json
{
  "summary": "",
  "requirementDocument": "",
  "confirmedFacts": [],
  "constraints": [],
  "acceptanceCriteria": [],
  "outOfScope": [],
  "codingPrompt": "",
  "readyToCode": true
}
```

约束：

- `requirementDocument` 应面向用户确认，结构清晰，不应只是 Coding Agent 指令。
- `codingPrompt` 应明确目标、范围、验收标准和不做什么。
- `codingPrompt` 不应包含 ASR 原始噪声文本。
- 如果仍有阻塞问题，不能返回 `readyToCode=true`。

## 前端 UI 草案

点击麦克风后，Composer 从普通文本输入切换为语音需求采集工作台。语音模式下不展示可编辑的普通 prompt 文本框。

工作台展示：

- 录音状态、ASR provider、LLM provider、当前需求阶段。
- 实时转写和最近几条 final transcript。
- 当前理解。
- 小云朵形式的阶段性理解。
- 待澄清问题。
- 已确认约束。
- 验收标准。
- 不做范围。
- 风险提示。
- Coding Prompt 草稿。

核心操作：

- 麦克风按钮：开始或继续本轮语音需求采集。
- “整理需求”：手动触发增量总结。
- `collecting` 下的“我说完了”：停止本轮自由描述并进入 `processing`。
- `clarifying` 下的“回答完了”：停止本轮语音回答并进入 `processing`。
- 澄清问题：用户继续点击麦克风用语音回答当前问题，回答写回本轮需求会话。
- `ready_to_confirm` 下禁用麦克风，只显示“确认需求”：生成并展示完整需求文档和 Coding Prompt，进入 `confirmed`。

第一版不需要复杂视觉打磨，但必须像一个语音访谈工作台，而不是把状态面板附着在普通 prompt 文本框下面。普通 prompt 文本框不作为 Phase 3 的输入入口；Coding Prompt 只作为确认后的只读产物展示给 Phase 4。

## 后端命令草案

```text
get_llm_provider_status() -> LlmProviderStatus
summarize_requirement_state(request) -> RequirementStatePatch
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

## 开发 Todo

- [ ] Step 1：更新 Phase 3 TypeScript 类型：`VoiceRequirementSession`、`RequirementUtterance`、`RequirementState`、`RequirementQuestion`、`requirementDocument`、`activeQuestionId`、`pendingAction`。
- [ ] Step 2：实现前端 requirement reducer，让语音 final transcript 只进入当前 active `VoiceRequirementSession`。
- [ ] Step 3：点击麦克风后切换到语音需求采集工作台，隐藏或禁用普通文本输入。
- [ ] Step 4：实现语音工作台 UI，展示实时转写、当前理解、需求文档草稿、澄清问题、验收标准和 Coding Prompt 只读草稿。
- [ ] Step 5：新增 Rust `llm` 模块，抽出 `LlmProvider`、`LlmProviderDiagnostic` 和 provider registry。
- [ ] Step 6：实现真实 `openai_compatible` LLM provider，支持 base URL、API key、model、temperature、timeout。
- [ ] Step 7：新增 `get_llm_provider_status` 命令，前端可显示 LLM 配置状态和缺失环境变量。
- [ ] Step 8：实现 `summarize_requirement_state` 命令，输入当前状态和新增 utterances，输出结构化 patch。
- [ ] Step 9：实现 `process_requirement_turn` 命令，统一返回 summary、需求草稿、澄清问题和 readyToConfirm。
- [ ] Step 10：实现 `finalize_requirement_document` 命令，生成完整需求文档和确认版 Coding Prompt。
- [ ] Step 11：增加 LLM JSON 输出 schema 校验和错误恢复，避免坏响应污染当前状态。
- [ ] Step 12：增加 debounce / batching 策略，避免每条 transcript 都调用 LLM。
- [ ] Step 13：把“我说完了 / 回答完了”接到 `processing`，支持 `clarifying -> processing -> clarifying` 的确认循环，但不自动确认、不自动编码。
- [ ] Step 14：补充单元测试：状态机 reducer、防误操作规则、LLM JSON parser、provider diagnostics、OpenAI-compatible 响应 parser。
- [ ] Step 15：补充 Phase 3 验收文档，记录真实 LLM provider 的配置和验收流程。

## 验收标准

- 点击语音按钮后进入语音专用需求采集模式，普通文本输入不再作为需求入口。
- 语音 final transcript 会进入需求流，而不是追加到 prompt 文本框。
- 未点击语音输入按钮时，不会创建 Phase 3 需求会话。
- 用户边说话，系统能按时间间隔或批量阈值周期性更新“当前理解”。
- 用户停止语音输入后，系统能提出不超过 3 个关键澄清问题。
- 用户用语音回答澄清问题后，需求状态和需求文档会被更新。
- `ready_to_confirm` 下点击麦克风不会回到 `collecting`，也不会追加新的 transcript。
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
