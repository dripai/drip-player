# SQLite 本地存储

当前已接入 SQLite：下载任务、播放列表、媒体资料、资源与字幕关联、通用及学习设置、字幕时间轴、翻译和收藏、录音索引及评测摘要、异步字幕识别任务。Key 保存在系统凭据库，不写入 SQLite。领域职责见 [领域模型与播放流程](domain-model.md)，操作步骤见 [影子跟读](learning-settings.md)。

## 文件与初始化

- 数据库位于可执行文件旁的 `config/shadow-player.sqlite3`。
- [Tauri 应用标识](https://v2.tauri.app/reference/config/#identifier)为 `com.shadow.player`，学习 Key 使用 `com.shadow.player.learning`。更名后需重新配置，旧数据库和旧 Key 不迁移、不删除。
- 首次启动创建数据库与默认设置，下载目录默认为可执行文件旁的 `cache`；主窗口启动后扫描当前目录生成列表。
- 不导入旧 JSON 或浏览器存储，不提供旧格式读取路径；旧文件不会被删除。
- 视频、音频、原始字幕和缓存仍存放在文件夹中；规范化后的字幕时间轴、翻译和学习元数据另存于数据库。
- 配置目录不可写、数据库损坏或版本不匹配时明确报错，不另建备用数据库。
- 当前核心结构版本为 **2**，不提供版本 1 的迁移。继续使用旧开发库会提示版本不匹配。需要重新初始化时，先完全退出应用，将可执行文件旁的整个 `config` 目录改名留存，再启动创建新库。应用只扫描当前设置中的下载目录，不汇总其他目录。

## 已实现的表

| 表 | 内容 |
| --- | --- |
| `media` | 稳定的媒体 ID、唯一媒体标识、本地路径或远程来源、标题、媒体类型 |
| `media_assets` | 关联媒体的播放文件、字幕文件路径、语言、资源来源；每份媒体最多一个下载播放文件 |
| `playlist_entries` | 播放列表条目 ID、关联媒体 ID、排列位置、添加时间 |
| `playlist_state` | 已提交的播放列表版本号 |
| `app_settings` | 设置版本号、主题、语言、播放模式、关闭到托盘 |
| `download_directory` | 当前下载目录及切换版本；由 `directory_schema.sql` 在初始化事务中创建 |
| `download_jobs` | 下载任务 JSON，包括 URL、阶段、尝试次数、目录记录、完成路径、错误；由 `download_schema.sql` 创建 |

学习表由 `learning_schema.sql` 在初始化事务中创建，核心 schema 版本保持 2，不重建已有播放列表，也不读取旧 JSON：

| 表 | 数据 |
| --- | --- |
| `learning_settings` | 非秘密参数 JSON、并发写入版本 |
| `transcripts` | 媒体 ID、字幕内容与时间决定的版本 ID、字幕句子 |
| `learning_cue_notes` | 字幕版本 + 句子 ID、译文及语言、收藏 |
| `learning_recordings` | 字幕版本 + 句子 ID、WAV 路径、时间、评测摘要 |
| `learning_transcription_jobs` | 云端任务 ID、媒体 ID、无密钥配置快照、状态及字幕版本 |

移除或清空播放列表只删除成员关系，保留媒体资料、学习记录、资源关联和文件。重新添加同一媒体时复用 `media_id`，生成新的播放列表条目 ID。直接播放本地文件也登记媒体身份，但不新增列表条目。

播放列表以当前下载目录为来源。保存目录时，先完整扫描并检查可写性，再在同一 SQLite 事务中提交目录、设置版本和整份列表；失败时保留此前提交的目录与列表。切换成功后停止旧播放；空目录会清空列表。手动刷新会移除已不在目录中的条目，并保留仍存在文件的条目 ID；当前播放项仍在列表时继续播放。移除或清空列表不会删除文件，下次刷新会重新显示目录中的文件。

完整刷新只保留实际存在的音视频；URL 保存在独立下载任务中，完成后才加入列表，刷新列表不取消正在下载的任务。

录音先转换并发布 WAV，再登记数据库；登记失败会清理新文件，清理失败会明确报告。删除录音先在事务中删除记录，并临时重命名文件；数据库提交失败会恢复文件，提交后文件清理失败会报告残留路径。录音文件和 SQLite 并非同一个事务介质，强制结束进程可能留下待清理文件；不执行后台静默删除。备份时同时备份录音目录、数据库及 SQLite 的 WAL 文件。

本地媒体目前按规范化路径识别，远程媒体按平台与资源 ID 识别；本地文件移动到其他路径后会被视为另一份媒体。尚未实现内容哈希识别或重新关联文件功能。

## 读写规则

- Vue 经 Tauri 命令调用 Rust；数据库连接由 Rust 统一持有。
- 添加列表通过事务登记媒体、资源和成员关系；移除条目只更新成员关系。下载完成直接登记媒体资源，不再全量替换列表。
- 列表以数据库为准；返回的界面快照有独立的递增序号。下载中的临时状态来自下载任务，不改写列表数据库版本号。
- 通用设置按字段提交，在后端合并；下载目录通过独立命令与列表一起保存；学习设置按完整快照和预期版本保存，过期写入会报错。提交后通知所有窗口，前端按版本号忽略过期通知。
- 托盘勾选在主线程读取最新已提交设置后同步，不持设置锁等待原生菜单调用；同步失败明确提示“设置已保存，但托盘同步失败”。
- 外键约束开启，使用 WAL 和 `synchronous=FULL`。数据库操作设置 5 秒锁等待时间。
- 同一进程内的窗口共享状态。`config/shadow-player.lock` 通过操作系统文件锁限定同一配置目录只有一个应用实例，避免启动恢复干扰另一个实例的下载；进程退出自动释放锁，不删除锁文件。
- 下载先写入 `<当前目录>/downloading/<job_id>`。文件处理完成后，先登记发布日志，再移动到 `<当前目录>/media/<media_id>/<job_id>`；资源、列表成员和任务完成状态在同一事务中提交。失败时在释放目录锁前恢复临时文件；回滚失败保留恢复状态并报错。
- 每次重试读取当前目录设置，历史目录仅用于展示和恢复未提交的发布操作。目录切换先停止下载进程；同目录继续可复用临时文件，换目录后重新下载。旧尝试不能覆盖新尝试结果。
- 正常退出时停止活动任务；下次启动把未完成任务标记为“已中断”，不自动下载。发布中崩溃时根据日志将尚未提交的文件移回临时目录，再等待手动继续。
- 扫描递归读取支持的音视频，排除根目录下保留的 `downloading`、`remux` 子目录；不跟随符号链接或进入目录外的联接。下载文件按已有资源关联复用媒体 ID，避免再作为本地文件重复入列。
- 文件系统和 SQLite 不构成跨系统事务。下载的发布日志支持恢复未提交的移动，但不自动清理旧目录临时文件；文件被外部删除、移动或断电损坏时明确报错。录音文件的孤立数据清理仍未实现。

运行中的 WAL 数据库不能仅复制主文件作为完整备份；应在应用完全退出后备份配置目录，或后续实现 SQLite 在线备份。媒体和录音文件需要一并备份。

## 依赖与验证边界

使用 `rusqlite 0.40.2` 的 `bundled`、`fallible_uint` 功能；锁文件中的 `libsqlite3-sys 0.38.2` 随包提供 SQLite 3.53.2，用户无需单独安装 SQLite。

学习功能测试使用临时真实数据库验证初始化、重复打开不重置媒体、设置版本冲突、字幕换版后的收藏和录音归属，以及录音文件删除失败时的事务回滚。学习功能的真实使用数据与外部服务仍需相应设备和 Key 验证。依赖升级的检查见[依赖说明](dependency-upgrade.md)。

目录功能已在 Windows 使用临时真实文件和 SQLite 验证整体替换、空目录、事务失败回滚、失效文件、下载去重、旧任务拒绝、旧播放停止与目录联接边界；浏览器使用模拟 Tauri IPC 检查设置窗口和主窗口的事件同步、刷新按钮及深浅主题布局。其他系统的桌面联调尚未验证。下载功能的新增验证见下方。

官方依据：[rusqlite](https://github.com/rusqlite/rusqlite)、[SQLite 外键](https://www.sqlite.org/foreignkeys.html)、[WAL](https://www.sqlite.org/wal.html)。

Windows 路径处理复用 [dunce 1.0.5](https://docs.rs/dunce/1.0.5/dunce/fn.canonicalize.html)；事务沿用 [rusqlite 0.40.2](https://docs.rs/rusqlite/0.40.2/rusqlite/struct.Connection.html#method.transaction_with_behavior)。目录联接测试使用 PowerShell 的 [New-Item -ItemType Junction](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/new-item?view=powershell-5.1)。

下载已在 Windows 验证：取消后停止子进程写入、强制结束应用宿主后的进程清理、重启任务恢复、改目录后继续、过期结果拒绝、文件和 SQLite 发布失败回滚。使用本机 yt-dlp 2026.01.29 和 FFmpeg 从本地 HTTP 下载生成的 WAV、转换 MP3、读取结构化进度与包含中文及 `%` 的输出路径。三窗口界面通过模拟 Tauri IPC 验证；YouTube、B站等平台的联网下载和登录尚未实测。

进程管理使用 [process-wrap 10.0.0](https://docs.rs/process-wrap/10.0.0/process_wrap/)，Windows 使用 Job Object 和 KillOnDrop，Unix 使用进程组；当前仅在 Windows 验证停止和强制退出行为。目录锁使用 [fslock 0.2.1](https://docs.rs/fslock/0.2.1/fslock/struct.LockFile.html)。下载进度与完成路径使用 [yt-dlp 的 progress-template 和 after_move](https://github.com/yt-dlp/yt-dlp#output-template)，不根据普通日志猜测下载完成。
