# ASR Provider Adapter Todo

日期：2026-06-01

## 背景

腾讯云实时 speaker diarization 在当前多人单麦场景下不稳定：

- 同一个人可能被拆成多个 `speaker_id`。
- 不同人可能被合并成同一个 `speaker_id`。
- 云端会返回大量未稳定的 `speaker_id=-1`。

下一步需要评估讯飞实时语音转写，尤其是支持 `role_type=2` 和 `feature_ids` 的大模型版。为了避免每换一家云服务都改主语音链路，需要把当前 ASR 代码正式收敛成 provider adapter 架构。

## 目标

- 前端和 Tauri command 只依赖统一的语音事件、音频分片和 provider 诊断模型。
- 每家 ASR 的鉴权、WebSocket URL、发送节奏、结束包、返回 JSON 解析和 speaker 归一化都封装在自己的 adapter 里。
- Mock、腾讯、讯飞可以通过 `VOICECODER_ASR_PROVIDER` 切换。
- `auto` 选择策略集中在 registry，不散落在 UI 或 provider 实现里。
- 普通测试不依赖真实云服务。

## Provider 接口草案

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

## 开发 Todo

- [x] Step 1：抽出通用 provider/session/diagnostic 边界，保持 Mock 和腾讯行为不变。
- [x] Step 2：把腾讯实现从 `voice.rs` 移到独立 `voice/tencent.rs`。
- [x] Step 3：把 Mock 实现移到独立 `voice/mock.rs`。
- [x] Step 4：增加 provider registry，集中处理 `auto` 和显式 provider override。
- [x] Step 5：把前端 `TencentAsrConfigCheck` 升级为通用 provider diagnostics。
- [x] Step 6：新增讯飞大模型 provider 骨架和配置诊断。
- [ ] Step 7：实现讯飞大模型 WebSocket 鉴权、40ms pacing、结束包和错误处理。
- [ ] Step 8：实现讯飞返回 JSON parser，把角色编号归一化为 `speaker-*`。
- [ ] Step 9：增加 `VOICECODER_ASR_PROVIDER=iflytek_llm` 验收文档。
- [ ] Step 10：真实多人测试后决定 `auto` 默认优先级。

## Step 1 验收

- Mock 语音输入行为不变。
- 腾讯配置诊断命令继续可用。
- 腾讯真实转写路径的 public command 和前端调用名不变。
- `cargo test` 通过。
- `npm run check` 通过。
