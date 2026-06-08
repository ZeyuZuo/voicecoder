# Phase 3 Requirement State Todo

日期：2026-06-08

## 背景

Phase 2 已经完成语音输入 MVP：

- 麦克风输入可以转为 16kHz / 16bit / mono PCM。
- Mock、腾讯云、讯飞大模型、火山引擎 ASR 已接入统一 provider adapter。
- final transcript 会自动进入中间输入框。

Phase 3 的入口是语音输入按钮。用户点击麦克风按钮后，系统创建一段语音需求采集会话，持续记录本轮说话内容、实时整理当前理解，并在用户停止语音输入后主动追问不明确的需求。只有用户确认后，系统才生成 Coding Prompt，交给后续 Coding Agent 阶段。

## 目标

- 用户点击语音输入按钮后才进入需求采集流程。
- ASR final transcript 进入本轮语音需求会话，成为需求整理的主输入。
- 系统实时维护结构化需求状态，而不是只保存一段长文本。
- LLM 可以增量总结需求、识别不明确点、生成澄清问题和确认版 Coding Prompt。
- 用户确认之前，系统不得自动触发编码。
- LLM 调用必须通过 provider adapter 抽象，第一版实现 `openai_compatible`，后续可扩展其他 LLM provider。
- API key、base URL 和模型配置只在 Tauri 后端读取，不能进入前端代码。

## 非目标

- Phase 3 不直接调用 Codex 修改代码。
- Phase 3 不实现完整 diff、review、terminal、browser 闭环。
- Phase 3 不依赖 speaker diarization 的准确性做关键逻辑；speaker 只作为上下文和诊断信息。
- Phase 3 不把普通文本框作为主需求输入入口；主入口是语音输入按钮。
- Phase 3 不先做复杂设置页，开发期继续用 `.env` 配置。

## 核心流程

```text
用户点击语音输入按钮
  ↓
创建 VoiceRequirementSession
  ↓
ASR final transcript
  ↓
RequirementUtterance 追加到需求流
  ↓
LLM 增量总结
  ↓
维护 RequirementState
  ↓
用户说完 / 点击整理
  ↓
LLM 生成澄清问题
  ↓
用户回答澄清问题
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
  -> summarizing
  -> need_clarification
  -> awaiting_confirm
  -> confirmed
  -> ready_to_code
```

状态含义：

- `idle`：没有正在整理的需求。
- `collecting`：正在收集本轮语音输入。
- `summarizing`：正在调用 LLM 更新当前理解。
- `need_clarification`：LLM 判断存在阻塞编码的问题，需要追问用户。
- `awaiting_confirm`：需求已经足够明确，等待用户确认。
- `confirmed`：用户已确认需求。
- `ready_to_code`：已生成 Coding Prompt，可交给 Phase 4。

约束：

- `confirmed` 之前不得触发 Coding Agent。
- `need_clarification` 状态只问影响实现的问题，不问可由 Coding Agent 自行判断的细节。
- 用户可以重新点击语音输入继续补充需求，状态应回到 `collecting` 或 `summarizing`。
- 用户回答澄清问题属于本轮需求会话的补充信息，不是独立的文本需求入口。

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
    | "summarizing"
    | "need_clarification"
    | "awaiting_confirm"
    | "confirmed"
    | "ready_to_code";
  utterances: RequirementUtterance[];
  summary: string;
  confirmedFacts: string[];
  constraints: string[];
  openQuestions: RequirementQuestion[];
  answeredQuestions: RequirementQuestion[];
  acceptanceCriteria: string[];
  outOfScope: string[];
  risks: string[];
  codingPrompt?: string;
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
- `mock`

第一版仍建议加 `mock`，用于不依赖真实 LLM 的 UI 和状态机测试。

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
- 用户停止语音输入。
- 用户点击“整理需求”。

### 2. 澄清问题生成

输入：

- 当前 `RequirementState`。
- 最近上下文。

输出：

```json
{
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

### 3. 确认版需求生成

输入：

- 当前 `RequirementState`。
- 用户对澄清问题的回答。

输出：

```json
{
  "summary": "",
  "confirmedFacts": [],
  "constraints": [],
  "acceptanceCriteria": [],
  "outOfScope": [],
  "codingPrompt": "",
  "readyToCode": true
}
```

约束：

- `codingPrompt` 应明确目标、范围、验收标准和不做什么。
- `codingPrompt` 不应包含 ASR 原始噪声文本。
- 如果仍有阻塞问题，不能返回 `readyToCode=true`。

## 前端 UI 草案

在中间对话区或右侧工作区新增需求状态面板：

- 当前理解。
- 已确认约束。
- 待澄清问题。
- 验收标准。
- 不做范围。
- 风险提示。
- Coding Prompt 草稿。

核心操作：

- 麦克风按钮：开始或继续本轮语音需求采集。
- “整理需求”：手动触发增量总结。
- “我说完了”：停止收集并触发澄清判断。
- “回答问题”：把用户对澄清问题的回答写回本轮需求会话。
- “确认需求”：进入 `confirmed`。
- “生成 Coding Prompt”：进入 `ready_to_code`。

第一版可以把面板做成朴素工具面板，不需要复杂视觉打磨。普通 prompt 文本框不作为 Phase 3 的主输入入口；它可以保留给后续确认稿编辑或 Phase 4 Coding Prompt 展示。

## 后端命令草案

```text
get_llm_provider_status() -> LlmProviderStatus
summarize_requirement_state(request) -> RequirementStatePatch
clarify_requirement_state(request) -> RequirementClarificationResult
finalize_requirement_state(request) -> RequirementFinalizationResult
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

- [ ] Step 1：新增 Phase 3 TypeScript 类型：`VoiceRequirementSession`、`RequirementUtterance`、`RequirementState`、`RequirementQuestion`。
- [ ] Step 2：实现前端 requirement reducer，让语音 final transcript 进入当前 `VoiceRequirementSession`。
- [ ] Step 3：增加需求状态面板，展示 summary、open questions、acceptance criteria 和 coding prompt 草稿。
- [ ] Step 4：新增 Rust `llm` 模块，抽出 `LlmProvider`、`LlmProviderDiagnostic` 和 provider registry。
- [ ] Step 5：实现 `mock` LLM provider，用固定 JSON 响应联调 UI 和状态机。
- [ ] Step 6：实现 `openai_compatible` LLM provider，支持 base URL、API key、model、temperature、timeout。
- [ ] Step 7：新增 `get_llm_provider_status` 命令，前端可显示 LLM 配置状态和缺失环境变量。
- [ ] Step 8：实现 `summarize_requirement_state` 命令，输入当前状态和新增 utterances，输出结构化 patch。
- [ ] Step 9：实现 `clarify_requirement_state` 命令，最多生成 3 个 blocking questions。
- [ ] Step 10：实现 `finalize_requirement_state` 命令，生成确认版需求和 Coding Prompt。
- [ ] Step 11：增加 LLM JSON 输出 schema 校验和错误恢复，避免坏响应污染当前状态。
- [ ] Step 12：增加 debounce / batching 策略，避免每条 transcript 都调用 LLM。
- [ ] Step 13：把“停止语音输入”后的行为接到澄清判断，但不自动确认、不自动编码。
- [ ] Step 14：补充单元测试：状态机 reducer、LLM JSON parser、provider diagnostics、mock provider。
- [ ] Step 15：补充 Phase 3 验收文档，记录真实 LLM provider 和 mock provider 的验收流程。

## 验收标准

- 语音 final transcript 会进入需求流，而不是只追加到 prompt 文本框。
- 未点击语音输入按钮时，不会创建 Phase 3 需求会话。
- 用户边说话，系统能周期性更新“当前理解”。
- 用户停止语音输入后，系统能提出不超过 3 个关键澄清问题。
- 用户回答澄清问题后，需求状态会被更新。
- 用户确认前不会触发 Coding Agent。
- 用户确认后能生成可用于 Phase 4 的 Coding Prompt。
- LLM provider 配置缺失时，UI 有明确诊断，不影响 Mock 流程。
- API key 不进入前端代码、前端状态、日志或文档。
- `mock` LLM provider 可在无网络、无真实 API key 时完成状态机联调。
- 普通测试不依赖真实 LLM 服务。

## 风险

- ASR 文本可能有错字，LLM 总结时必须保留不确定性，不要擅自补全关键需求。
- speaker label 不稳定，不能基于 speaker 自动判断“谁有最终决策权”。
- 增量总结可能漂移，需要保留 utterance 原文用于回溯。
- OpenAI-compatible provider 的兼容程度不同，`response_format`、streaming、错误格式可能不一致。
- 如果 LLM 输出非法 JSON，必须失败得清楚，不能静默改坏需求状态。
