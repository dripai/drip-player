# 影子跟读

入口在播放器顶部的“影子跟读”。右键 → 设置仍打开独立窗口，自定义标题栏随主题变化，窗口包含通用、模型、字幕、影子跟读、录音五组。

1. 打开音视频，再进入影子跟读。有已关联的字幕时自动载入；也可以导入 UTF-8 的 SRT、VTT、ASS、SSA 文件。统一通过 FFmpeg 的原生 text 字幕编码器去除样式与行内时间标记，保留句子时间轴。每条字幕作为一句，推荐导入英文原文轨道。转换依据：[FFmpeg text 编码器](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/srtenc.c)。
2. 点击字幕跳转；逐句复读从选中句开始，每句播放指定次数，停顿后继续下一句。只看收藏时，复读只包含后续收藏句。A、B 按钮标记当前播放位置，形成持续循环。手动暂停、跳转、调整参数、切换媒体或退出学习会停止循环。
3. “双语”显示已生成的译文；“遮挡”隐藏原文和译文。“跟随”控制自动滚动，也可以搜索和收藏句子。字幕偏移、倍速、重复次数和停顿是本次练习的临时值。
4. 在设置 → 录音中选目录和麦克风。选中句子，点击开始录音；停止后保存为 16 kHz、16 bit、单声道 WAV。每次新建一个录音版本，可选择历史录音试听、对比原声或删除。转换或保存失败时，音频保留在当前窗口，可重试保存；关闭前需先保存或主动丢弃。

视频跟读使用 Video.js 的实际播放时间。音频跟读使用 Rust 播放引擎的状态快照，精度受 250 ms 轮询间隔影响，当前只支持 1×。外部 MPV 没有学习控制。学习模式下，播放结束不会自动跳到另一首。

## 设置与播放器的分工

右键菜单保持“刷新、设置”。设置窗口按以下五组组织，录音参数放在这里。

| 分组 | 设置窗口中的内容 | 播放器中的操作 |
| --- | --- | --- |
| 通用 | 外观、界面语言、关闭到托盘 | 保留现有快捷操作 |
| 模型 | 百炼北京业务空间、文字模型、API Key；讯飞 APPID 和凭据 | 生成字幕、翻译、讲解、上传本次跟读评测 |
| 字幕 | 默认原文/双语显示、翻译语言、字号、自动滚动 | 临时切换字幕、定位当前句、调整本视频字幕偏移 |
| 影子跟读 | 默认语速、每句重复次数、句间停顿、跳过空白 | 开始练习、上一句/下一句、逐句复读、AB 区间、遮挡字幕 |
| 录音 | 输入设备、保存目录、录音后回放顺序 | 开始/停止录音、试听、重录、删除本次录音 |

默认值：双语、中文翻译、字号 18 px、自动滚动；视频语速 1.0、每句重复 2 次、句间停顿 1 秒、跳过空白关闭；录音使用系统默认输入设备、先原声后录音回放。保存目录首次使用时明确选择。指定设备失效或目录不可写时提示错误。跳过空白依据字幕区间，不是音频静音检测。

普通设置、字幕版本、翻译语言、收藏、录音索引和评测摘要写入 SQLite；音频文件留在选定目录。字幕内容或时间变化产生新版本，旧收藏和录音仍绑定原版本。Key 使用 `keyring 4.2` 的系统凭据库，界面只显示是否配置，不回读完整 Key；数据库和模型任务参数不包含 Key。设置保存使用版本检查，冲突时重新载入再修改。具体边界见 [SQLite 存储说明](sqlite-storage.md)和 [领域模型](domain-model.md)。

## 先申请哪些服务

目前接入下面两家服务。只有点击生成字幕、翻译、讲解或上传评测才发送对应内容；进入学习和录音本身不会调用模型。

| 用途 | 建议服务或模型 | 需要准备 |
| --- | --- | --- |
| 翻译、词汇和语法讲解 | 阿里云百炼 `qwen-plus` | 北京地域的百炼 API Key、业务空间 ID |
| 没有字幕时生成带时间轴的字幕 | 阿里云百炼 `paraformer-v2` | 可访问该模型的百炼 API Key，与接入域名对应的地域及业务空间 |
| 英语跟读的发音评测 | 讯飞“语音评测（流式版）” | 对应服务的 APPID、APIKey、APISecret |

百炼固定使用北京业务空间域名 `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com`，文字模型默认 `qwen-plus`，可改为该业务空间已授权的文字模型名。字幕模型固定 `paraformer-v2`。这两个服务共用百炼 Key，地域、空间和权限必须匹配。参见[获取 API Key](https://help.aliyun.com/zh/model-studio/get-api-key)和[文本生成接口](https://help.aliyun.com/zh/model-studio/text-generation)。

在学习面板点击“识别”，填写云端能直接下载的音视频 URL，再点击“生成字幕”。视频网站播放页和本地路径不能用于这个接口。本地文件上传尚未实现。异步任务 ID 保存在数据库，重新打开媒体后可继续查询；网络错误保留任务，避免重复提交。转写结果根据句子的毫秒时间轴建立字幕版本。参见[文件转写 REST API](https://help.aliyun.com/zh/model-studio/paraformer-recorded-speech-recognition-restful-api)。

讯飞需开通“语音评测（流式版）”英文能力。项目使用 `en_vip/read_sentence`，WebSocket HMAC 鉴权，并按 40 ms 发送 PCM 音频帧。项目录音上限为 3 分钟；单句限 100 个词、1024 字节，英文原文不得混入中文译文。点击“上传评测”后才发送选中录音和原文，展示服务实际返回的总分及分项；拒识、超时和服务错误明确显示，不使用文字模型推测发音分数。参见[讯飞语音评测文档](https://www.xfyun.cn/doc/Ise/IseAPI.html)。

当前没有接入其他模型服务或语音合成。

已有 SRT/VTT 字幕的逐句复读、本地录音、原声对比均不需要模型 Key。第一阶段使用视频原声跟读，不需要申请语音合成服务。

## 验证与平台边界

- `pnpm build`：类型检查和前端构建；`pnpm test:learning`：复读次数、停顿、AB、取消和字幕边界。
- `cargo test --locked --manifest-path src-tauri/Cargo.toml`：字幕解析、SQLite 版本冲突、字幕版本与录音归属、删除失败回滚、模型结果解析等。
- Windows Edge 浏览器验证：真实视频的点句跳转、复读和 AB、手动暂停取消、收藏、遮挡、五组设置；使用模拟麦克风运行真实 `MediaRecorder`，检查保存失败保留、重试、回放和对比。该验证替换了 Tauri IPC 和模型响应，不代表桌面麦克风或云端服务已验证。
- 麦克风通过 WebView 的 `getUserMedia` / `MediaRecorder` 使用；拒绝权限或不支持时明确报错。macOS 已添加用途说明和签名的音频输入 entitlement，仍需在 macOS 实机验证。依据：[MediaRecorder](https://developer.mozilla.org/en-US/docs/Web/API/MediaRecorder)、[Tauri macOS 配置](https://v2.tauri.app/reference/config/#macconfig)、[Apple 音频输入权限](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.security.device.audio-input)。
- 没有配置真实 Key，百炼转写、翻译及讯飞评分的线上连通性、账号权限和效果尚未验证。
- Windows 本机另外验证了 FFmpeg 录音转 WAV、系统凭据库的临时条目写入/读取/删除、音频控制完成后的状态确认。凭据测试不读取或修改正式服务的 Key。三个检查分别使用 `cargo test` 的 `converts_recording_to_mono_wav_and_cleans_up_failed_conversion`、`credential_store_round_trip`、`audio_controls_acknowledge_the_updated_snapshot` 过滤器并传入 `-- --ignored`；字幕样式转换用 `imports_styled_subtitles_as_spoken_text_with_original_timestamps`。
