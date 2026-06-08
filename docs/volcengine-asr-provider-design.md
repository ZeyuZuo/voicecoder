# Volcengine ASR Provider Design

日期：2026-06-04

## 目标

火山引擎 provider 已进入当前 `auto` 默认链路，优先级为：

```text
iflytek_llm > tencent > volcengine > mock
```

## Provider 边界

- `VOLCENGINE_ASR_*` 配置只在 `voice/volcengine.rs` 中读取。
- 腾讯、讯飞、火山的鉴权、音频分片节奏、结束帧和 speaker 归一化互不复用。
- 前端默认 200ms / 6400 bytes 音频分片适用于火山、腾讯和 Mock；讯飞仍使用 40ms / 1280 bytes。

## 默认配置

```env
VOICECODER_ASR_PROVIDER=volcengine
VOLCENGINE_ASR_APP_ID=<火山 APP ID>
VOLCENGINE_ASR_ACCESS_TOKEN=<火山 Access Token>
VOLCENGINE_ASR_ENDPOINT=wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async
VOLCENGINE_ASR_RESOURCE_ID=volc.bigasr.sauc.duration
VOLCENGINE_ASR_ENABLE_NONSTREAM=true
VOLCENGINE_ASR_ENABLE_SPEAKER_INFO=true
VOLCENGINE_ASR_ENABLE_ACCELERATE_TEXT=true
VOLCENGINE_ASR_SSD_VERSION=200
VOLCENGINE_ASR_END_WINDOW_SIZE=400
```

`VOLCENGINE_ASR_APP_ID` 对应火山官方 Header 说明中的 APP ID；`VOLCENGINE_ASR_ACCESS_TOKEN` 对应 Access Token。底层协议仍会把它们分别发送到 `X-Api-App-Key` 和 `X-Api-Access-Key`。

## 协议

- WebSocket Header：`X-Api-App-Key`、`X-Api-Access-Key`、`X-Api-Resource-Id`、`X-Api-Connect-Id`。
- 消息协议：火山 V1 二进制帧。
- 初始帧：JSON full request。
- 音频帧：PCM binary payload。
- 结束帧：audio-only request，使用负 sequence，并同时设置 sequence flag 和 last-sequence flag。
- 服务端 error frame：按 `Header + Error code + Error message size + Error message` 解析，不能按普通 response payload size 处理。

## Speaker 解析

adapter 会从以下位置查找 speaker 标签：

- `speaker`
- `speaker_id`
- `speakerId`
- `speakerID`
- `additions` 中的同名字段
- `speaker_info` / `speakerInfo`
- `speaker_result` / `speakerResult`

数字 speaker 从 0 开始映射到 UI 标签：

```text
0 -> speaker-1
1 -> speaker-2
```

实测 `bigmodel_async` 会间歇返回 `speaker_id`，但不是每条 utterance 都带 speaker 标签。因此 speaker 仍然作为实验性诊断能力，不作为产品强可信逻辑。
