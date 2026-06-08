# ASR Provider Adapter Completion Record

日期：2026-06-01
完成日期：2026-06-08

## 当前状态

ASR Provider Adapter 分支已合并到 `main`。Phase 2 语音输入 MVP 已完成，Mock、腾讯云、讯飞大模型和火山引擎都已接入统一 provider/session/diagnostic 边界。

当前 `auto` 默认优先级：

```text
iflytek_llm > tencent > volcengine > mock
```

火山引擎已进入 `auto` fallback；如果三家云服务都未配置完整凭证，则自动回退 Mock。

## 背景

腾讯云实时 speaker diarization 在当前多人单麦场景下不稳定：

- 同一个人可能被拆成多个 `speaker_id`。
- 不同人可能被合并成同一个 `speaker_id`。
- 云端会返回大量未稳定的 `speaker_id=-1`。

已完成讯飞实时语音转写大模型接入，支持 `role_type=2` 和 `feature_ids` 配置。为了避免每换一家云服务都改主语音链路，当前 ASR 代码已经收敛到 provider adapter 架构。

讯飞大模型和火山引擎豆包语音大模型流式 ASR 均已接入。speaker label 仍作为实验性诊断信息保留，不作为产品强可信逻辑。

## 目标

- 前端和 Tauri command 只依赖统一的语音事件、音频分片和 provider 诊断模型。
- 每家 ASR 的鉴权、WebSocket URL、发送节奏、结束包、返回 JSON 解析和 speaker 归一化都封装在自己的 adapter 里。
- Mock、腾讯、讯飞、火山可以通过 `VOICECODER_ASR_PROVIDER` 切换。
- `auto` 选择策略集中在 registry，不散落在 UI 或 provider 实现里。
- 普通测试不依赖真实云服务。

## Provider 接口

```text
AsrProvider
  ├─ kind()
  ├─ is_available()
  ├─ missing_env()
  ├─ diagnostic()
  └─ start(ctx) -> AsrSession

AsrSession
  ├─ send_audio_chunk(chunk)
  ├─ finish()
  └─ cancel()
```

统一事件：

```text
VoiceTranscriptEvent
  ├─ id
  ├─ session_id
  ├─ speaker_id
  ├─ text
  ├─ is_final
  ├─ started_at_ms
  ├─ ended_at_ms
  └─ created_at
```

统一诊断：

```text
VoiceProviderDiagnostic
  ├─ provider
  ├─ configured
  ├─ missing_env
  ├─ endpoint
  ├─ details
  └─ error
```

## 讯飞 API 记录

### 实时语音转写标准版

官方文档：https://www.xfyun.cn/doc/asr/rtasr/API.html

- WebSocket 地址：`wss://rtasr.xfyun.cn/v1/ws`
- 音频：16kHz / 16bit / mono PCM
- 推荐发送节奏：40ms / 1280 bytes
- 鉴权参数：`appid`、`ts`、`signa`
- 签名逻辑：先 `MD5(appid + ts)`，再用 `apiKey` 做 HMAC-SHA1 并 Base64
- 角色分离：`roleType=2`
- 结束包：发送 `{"end": true}`

### 实时语音转写大模型

官方文档：https://www.xfyun.cn/doc/spark/asr_llm/rtasr_llm.html

- WebSocket 地址：`wss://office-api-ast-dx.iflyaisol.com/ast/communicate/v1`
- 音频：16kHz / 16bit / mono PCM
- 推荐发送节奏：40ms / 1280 bytes
- 角色分离：`role_type=2`
- 声纹分离：可通过 `feature_ids` 指定已注册声纹
- 适合作为当前多人识别不稳定问题的优先评估目标

## 火山引擎 API 记录

官方文档：火山引擎豆包语音大模型流式 ASR / Seed ASR 相关文档。

- WebSocket 地址：`wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async`
- 鉴权 Header：`X-Api-App-Key`、`X-Api-Access-Key`、`X-Api-Resource-Id`、`X-Api-Connect-Id`
- 默认 Resource ID：`volc.bigasr.sauc.duration`
- 音频：16kHz / 16bit / mono PCM
- 前端默认 200ms / 6400 bytes 分片，不影响腾讯或讯飞分片策略
- 默认开启：`enable_nonstream=true`、`enable_speaker_info=true`、`enable_accelerate_text=true`
- 说话人聚类：`ssd_version=200`
- 当前实测：`speaker_id` 会间歇返回，adapter 能解析已有字段，但云端不保证每条 utterance 都带 speaker 标签

## 开发 Todo

- [x] Step 1：抽出通用 provider/session/diagnostic 边界，保持 Mock 和腾讯行为不变。
- [x] Step 2：把腾讯实现从 `voice.rs` 移到独立 `voice/tencent.rs`。
- [x] Step 3：把 Mock 实现移到独立 `voice/mock.rs`。
- [x] Step 4：增加 provider registry，集中处理 `auto` 和显式 provider override。
- [x] Step 5：把前端 `TencentAsrConfigCheck` 升级为通用 provider diagnostics。
- [x] Step 6：新增讯飞大模型 provider 骨架和配置诊断。
- [x] Step 7：实现讯飞大模型 WebSocket 鉴权、40ms pacing、结束包和错误处理。
- [x] Step 8：实现讯飞返回 JSON parser，把角色编号归一化为 `speaker-*`。
- [x] Step 9：增加 `VOICECODER_ASR_PROVIDER=iflytek_llm` 验收文档。
- [x] Step 10：真实多人测试后决定 `auto` 默认优先级：讯飞大模型 > 腾讯 > 火山 > Mock。
- [x] Step 11：补充火山引擎 ASR 配置、环境变量和验收文档。
- [x] Step 12：实现火山引擎 WebSocket V1 二进制协议、鉴权 Header、音频发送和结束帧。
- [x] Step 13：实现火山引擎返回 parser，处理实时结果、二遍最终结果和 transcript event。
- [x] Step 14：实现火山 speaker label 归一化，解析 `speaker`、`speaker_id`、嵌套 `speaker_info` 和字符串化 `additions`。
- [x] Step 15：增加火山 speaker 诊断日志、`bigmodel_async` 默认 endpoint 和 final frame 修复。
- [x] Step 16：完成腾讯、讯飞、火山横向接入后的默认优先级收敛：讯飞大模型 > 腾讯 > 火山 > Mock。

## 最终验收

- Mock 语音输入行为不变。
- 腾讯、讯飞、火山配置诊断进入统一 provider diagnostics。
- 腾讯真实转写路径的 public command 和前端调用名保持兼容。
- 讯飞大模型支持 40ms / 1280 bytes pacing、结束包、speaker 归一化和同结果 speaker 切分。
- 火山引擎支持 WebSocket V1 二进制协议、server error frame 解析、结束帧和 speaker 归一化。
- `VOICECODER_ASR_PROVIDER=auto` 使用 `iflytek_llm > tencent > volcengine > mock`。
- `cargo test` 通过。
- `npm run typecheck` 通过。
