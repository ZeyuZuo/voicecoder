# Phase 2 Voice Acceptance

目标：验证语音输入 MVP 在 Tauri 客户端内可用，并能在 Mock、腾讯云、讯飞大模型和火山引擎 ASR 链路下稳定开始、转写、停止和释放资源。

## 1. 准备本地配置

复制 `app/.env.example` 为 `app/.env`，按需填写：

```bash
TENCENTCLOUD_APP_ID=
TENCENTCLOUD_SECRET_ID=
TENCENTCLOUD_SECRET_KEY=
TENCENT_ASR_ENGINE_MODEL_TYPE=16k_zh_en_speaker
TENCENT_ASR_SENTENCE_STRATEGY=0
TENCENT_ASR_VOICE_FORMAT=1
TENCENT_ASR_NEED_VAD=1

IFLYTEK_LLM_APP_ID=
IFLYTEK_LLM_ACCESS_KEY_ID=
IFLYTEK_LLM_ACCESS_KEY_SECRET=
IFLYTEK_LLM_ENDPOINT=wss://office-api-ast-dx.iflyaisol.com/ast/communicate/v1
IFLYTEK_LLM_LANG=autodialect
IFLYTEK_LLM_ROLE_TYPE=2
IFLYTEK_LLM_FEATURE_IDS=

VOLCENGINE_ASR_APP_KEY=
VOLCENGINE_ASR_ACCESS_KEY=
VOLCENGINE_ASR_ENDPOINT=wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async
VOLCENGINE_ASR_RESOURCE_ID=volc.bigasr.sauc.duration
VOLCENGINE_ASR_LANGUAGE=zh-CN
VOLCENGINE_ASR_ENABLE_NONSTREAM=true
VOLCENGINE_ASR_ENABLE_SPEAKER_INFO=true
VOLCENGINE_ASR_ENABLE_ACCELERATE_TEXT=true
VOLCENGINE_ASR_SSD_VERSION=200
VOLCENGINE_ASR_END_WINDOW_SIZE=400

VOICECODER_ASR_PROVIDER=auto
```

`VOICECODER_ASR_PROVIDER` 可选值：

- `auto`：优先使用讯飞大模型，其次腾讯云，最后回退 Mock。
- `mock`：强制使用 Mock。
- `tencent`：强制使用腾讯云。
- `iflytek_llm`：强制使用讯飞实时语音转写大模型。
- `volcengine`：强制使用火山引擎豆包语音大模型流式 ASR。

`IFLYTEK_LLM_FEATURE_IDS` 仅在已通过讯飞声纹注册拿到声纹 ID 时填写，多个声纹 ID 用英文逗号分隔。没有注册声纹时留空，配合 `IFLYTEK_LLM_ROLE_TYPE=2` 先测试实时角色盲分。

`VOLCENGINE_ASR_APP_KEY` 对应火山控制台里的 API 名称，`VOLCENGINE_ASR_ACCESS_KEY` 对应 API Key。

`.env` 已被忽略，不要提交真实凭证。

## 2. 启动客户端

先跑本地检查：

```bash
cd app
npm run check
```

再启动客户端：

```bash
cd app
npm run tauri:dev
```

浏览器预览不能作为语音最终验收环境。语音、麦克风权限、本地文件和后端命令都以 Tauri 客户端为准。

## 3. Mock 验收

在 `app/.env` 设置：

```bash
VOICECODER_ASR_PROVIDER=mock
```

验收步骤：

1. 启动 Tauri 客户端。
2. 点击中间输入框右侧麦克风按钮。
3. 授权麦克风。
4. 观察语音面板显示 `mock`。
5. 等待 Mock partial / final 文本出现。
6. 再次点击麦克风或等待 Mock 完成。

通过标准：

- 麦克风按钮进入录音状态。
- 能看到实时文本和最终句子。
- final 句子会自动追加到中间输入框。
- 最终句子可以保留 provider 返回的 speaker 标记。
- 停止后状态回到空闲，麦克风占用释放。

## 4. 腾讯云配置诊断

后端命令 `check_tencent_asr_config` 会检查腾讯云配置并生成脱敏签名 URL 预览。

后端命令 `get_voice_session_snapshot` 可用于诊断当前是否仍有活动语音会话、当前 provider 和已收到的音频分片数。

后端区分两种停止语义：

- `stop_voice_session`：用户主动停止，腾讯云链路会发送结束包并等待 final。
- `cancel_voice_session`：错误恢复或启动前清理旧会话，直接释放资源。

它不会：

- 连接腾讯云。
- 打开麦克风。
- 返回 `SecretKey`。
- 返回原始 `signature`。

通过标准：

- `ok=true`。
- `missingEnv=[]`。
- `signedUrlPreview` 中 `secretid=<redacted>`。
- `signedUrlPreview` 中 `signature=<redacted>`。

如果 `ok=false`，先根据 `missingEnv` 补齐本地 `.env`。

## 5. 腾讯云真实转写验收

在 `app/.env` 设置：

```bash
VOICECODER_ASR_PROVIDER=tencent
```

验收步骤：

1. 确认腾讯云 ASR 服务已开通并可用。
2. 启动 Tauri 客户端。
3. 点击麦克风按钮并授权。
4. 说一段中文或中英混合需求。
5. 观察语音面板显示 `tencent`。
6. 等待 partial / final 文本出现。
7. 点击停止。

通过标准：

- 能连接腾讯云 WebSocket。
- 能看到实时转写文本。
- final 句子进入转写列表。
- final 句子自动追加到中间输入框。
- 同一 final 句子更新时会替换旧文本，不会重复追加。
- speaker id 可以显示为 `speaker-*`，但腾讯云实时 speaker diarization 当前只作为实验性诊断，不作为通过标准。
- 如果腾讯返回普通实时识别 `result.voice_text_str`，也能进入同一条转写链路。
- 点击停止后会发送结束包并等待最终转写，随后麦克风和 WebSocket 都释放。
- 等待 final 期间麦克风按钮会暂时不可用，避免重复停止打乱会话状态。

腾讯云 speaker diarization 实测记录：

- 已对齐腾讯官方 speaker SDK 参数：`result_mod=1`、`speaker_diarization=1`、`sentence_strategy=0`、`enable_speaker_context=0`。
- 音频分片已调整为 16kHz / 16bit / mono PCM 下约 200ms / 6400 bytes。
- 真实测试中腾讯会返回大量 `speaker_id=-1`，并可能把同一人来回归到 `speaker_id=0` 和 `speaker_id=1`。
- 当前产品不应把腾讯实时 speaker 标签作为强可信信息；只保留解析和后端诊断日志。
- 后续如果需要稳定区分固定人员，优先评估讯飞大模型实时转写的声纹分离能力。

## 6. 讯飞大模型真实转写验收

在 `app/.env` 设置：

```bash
VOICECODER_ASR_PROVIDER=iflytek_llm
IFLYTEK_LLM_APP_ID=<讯飞 AppID>
IFLYTEK_LLM_ACCESS_KEY_ID=<讯飞 APIKey>
IFLYTEK_LLM_ACCESS_KEY_SECRET=<讯飞 APISecret>
IFLYTEK_LLM_ENDPOINT=wss://office-api-ast-dx.iflyaisol.com/ast/communicate/v1
IFLYTEK_LLM_LANG=autodialect
IFLYTEK_LLM_ROLE_TYPE=2
IFLYTEK_LLM_FEATURE_IDS=
```

如果要测试声纹分离，把 `IFLYTEK_LLM_FEATURE_IDS` 改成已注册声纹 ID 列表：

```bash
IFLYTEK_LLM_FEATURE_IDS=feature_id_1,feature_id_2
```

验收步骤：

1. 确认讯飞实时语音转写大模型服务已开通，并且 AppID、APIKey、APISecret 来自同一个服务页。
2. 启动 Tauri 客户端。
3. 点击麦克风按钮并授权。
4. 观察语音面板显示 `iflytek_llm`，并且 provider note 显示讯飞大模型配置就绪。
5. 说一段中文或中英混合需求。
6. 多人测试时，尽量让两个人轮流说短句，便于观察 `speaker-*` 是否切换。
7. 点击停止，等待最终转写结果进入输入框。

通过标准：

- 能连接讯飞 WebSocket，不出现鉴权或连接错误。
- 前端按讯飞 provider 推荐值发送 40ms / 1280 bytes 音频分片，后端 adapter 仍保留 1280 bytes 重分片作为兜底。
- 停止时会发送讯飞结束包，并等待最终帧或超时收尾。
- 能看到实时转写文本。
- final 句子进入转写列表，并自动追加到中间输入框。
- 讯飞返回的角色编号会映射为 `speaker-*`，例如 `rl=1 -> speaker-1`、`rl=2 -> speaker-2`。
- `rl=0` 会沿用上一位 speaker，不会显示成 `speaker-0`。
- 同一条讯飞结果里如果发生 speaker 切换，会拆成多个统一 transcript event。
- 如果 `IFLYTEK_LLM_FEATURE_IDS` 为空，本次只验收实时角色盲分。
- 如果 `IFLYTEK_LLM_FEATURE_IDS` 不为空，需要记录声纹分离是否比盲分更稳定。

测试记录建议：

- 记录测试模式：盲分 / 声纹分离。
- 记录说话人数、距离麦克风位置和大致环境噪声。
- 记录 speaker 是否稳定、是否混人、是否把同一人拆成多个 speaker。
- 记录错误码、连接失败信息或后端 `[voice][iflytek_llm]` 日志。
- 保留一段成功转写和一段失败样例，供 Step 10 决定 `auto` 默认优先级。

## 7. 火山引擎真实转写验收

在 `app/.env` 设置：

```bash
VOICECODER_ASR_PROVIDER=volcengine
VOLCENGINE_ASR_APP_KEY=<火山控制台 API 名称>
VOLCENGINE_ASR_ACCESS_KEY=<火山控制台 API Key>
VOLCENGINE_ASR_ENDPOINT=wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async
VOLCENGINE_ASR_RESOURCE_ID=volc.bigasr.sauc.duration
VOLCENGINE_ASR_LANGUAGE=zh-CN
VOLCENGINE_ASR_ENABLE_NONSTREAM=true
VOLCENGINE_ASR_ENABLE_SPEAKER_INFO=true
VOLCENGINE_ASR_ENABLE_ACCELERATE_TEXT=true
VOLCENGINE_ASR_SSD_VERSION=200
VOLCENGINE_ASR_END_WINDOW_SIZE=400
```

验收步骤：

1. 确认火山引擎 ASR 服务已开通，API 名称和 API Key 来自同一个服务页。
2. 启动 Tauri 客户端。
3. 点击麦克风按钮并授权。
4. 观察语音面板显示 `volcengine`，并且 provider note 显示火山配置就绪。
5. 说一段中文或中英混合需求。
6. 多人测试时，让两个人轮流说短句，观察 `speaker-*` 是否稳定切换。
7. 点击停止，等待最终转写结果进入输入框。

通过标准：

- 能连接火山 WebSocket，不出现鉴权或连接错误。
- 音频按火山 provider 默认 200ms / 6400 bytes 分片发送，不影响腾讯或讯飞分片策略。
- 停止时发送火山 V1 负 sequence 结束帧，并等待最终帧或超时收尾。
- 能看到实时转写文本。
- final 句子进入转写列表，并自动追加到中间输入框。
- 火山 speaker 标签会映射为 `speaker-*`，例如 `speaker=0 -> speaker-1`、`speaker=1 -> speaker-2`。
- `speaker_id` 只作为实验性诊断能力；实测可能不是每条 utterance 都返回。

测试记录建议：

- 记录 `VOLCENGINE_ASR_ENABLE_NONSTREAM` 开关对延迟和最终文本的影响。
- 记录内容识别是否比腾讯/讯飞稳定。
- 记录 speaker 是否稳定、是否混人、是否把同一人拆成多个 speaker。
- 记录后端 `[voice][volcengine]` 诊断日志，尤其是 `speaker_candidate` 和 `normalized` 字段。

## 8. 常见失败判断

- 语音面板显示 `mock`：当前没有完整腾讯云凭证，或设置了 `VOICECODER_ASR_PROVIDER=mock`。
- 麦克风权限失败：检查系统麦克风权限，重新启动 Tauri 客户端后再试。
- 腾讯云鉴权失败：检查 `TENCENTCLOUD_APP_ID`、`TENCENTCLOUD_SECRET_ID`、`TENCENTCLOUD_SECRET_KEY` 是否来自同一个账号和服务。
- WebSocket 连接失败：检查网络、腾讯云服务开通状态、账户余额和接口地域/域名。
- 讯飞鉴权失败：检查 `IFLYTEK_LLM_APP_ID`、`IFLYTEK_LLM_ACCESS_KEY_ID`、`IFLYTEK_LLM_ACCESS_KEY_SECRET` 是否来自讯飞实时语音转写大模型服务页。
- 讯飞返回 `35013`：检查 `utc` 时区格式，当前后端生成 `YYYY-MM-DDTHH:MM:SS+0000`。
- 讯飞返回 `35014` / `100012`：检查本机系统时间是否准确。
- 讯飞返回 `35030`：签名重复或过期，重启客户端后重试。
- 讯飞返回 `37005`：服务端长时间未收到音频，确认麦克风权限和音频分片发送是否正常。
- 讯飞返回 `100001`：音频发送过快，检查讯飞 provider 的前端分片和 adapter pacing 是否仍为 40ms / 1280 bytes。
- 火山鉴权失败：检查 `VOLCENGINE_ASR_APP_KEY` 是否填 API 名称，`VOLCENGINE_ASR_ACCESS_KEY` 是否填 API Key。
- 火山反应慢：优先确认 `VOLCENGINE_ASR_END_WINDOW_SIZE=400` 和 `VOLCENGINE_ASR_ENABLE_ACCELERATE_TEXT=true`；如果仍慢，再临时测试 `VOLCENGINE_ASR_ENABLE_NONSTREAM=false`，但这可能影响 speaker 聚类和最终文本稳定性。
- 没有转写文本：确认使用 16kHz / 16bit / mono PCM 分片；腾讯/Mock/火山默认约 200ms / 6400 bytes，讯飞大模型使用 40ms / 1280 bytes，并在停止时发送尾包。
- speaker 标签来回跳：这是腾讯实时 speaker diarization 的实测不稳定表现，不影响 Phase 2 的语音输入主链路。
- 讯飞 `speaker-*` 标签来回跳：先区分盲分和声纹分离模式；盲分不稳定时，再测试配置 `IFLYTEK_LLM_FEATURE_IDS` 的声纹分离。
- 火山 `speaker-*` 标签来回跳：先确认 `VOLCENGINE_ASR_ENABLE_SPEAKER_INFO=true` 和 `VOLCENGINE_ASR_SSD_VERSION=200`，再记录失败样例供 Step 16 横向比较。
- 客户端出现 `CryptoProvider` panic：检查 Rust TLS 依赖特性，当前项目显式使用 rustls `ring` provider。

## 9. 当前 Phase 2 边界

Phase 2 只负责语音输入、实时转写和转写展示。

不进入 Phase 2：

- 把语音整理成结构化需求。
- AI 主动追问。
- 触发 Coding Agent 修改代码。
- Diff / Review / Terminal / Browser 的完整闭环。
