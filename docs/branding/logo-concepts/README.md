# Shadow Player Logo 概念

当前采用 B 双声波方案，已替换应用、标题栏和托盘使用的图标资源。概念图使用内置 image_gen 生成，原始提示词见 [prompts.json](prompts.json)。

| 方案 | 方向 | 预览 |
| --- | --- | --- |
| A | 播放符号与影子叠合 | [查看](concept-a-play-shadow.png) |
| B（当前采用） | 双声波，表达原声与跟读 | [查看](concept-b-voice-echo.png) |
| C | S 字母与播放符号结合 | [查看](concept-c-shadow-monogram.png) |

图标源文件为项目根目录的 [app-icon.png](../../../app-icon.png)。运行 `pnpm icons`，通过 [Tauri 官方图标命令](https://v2.tauri.app/develop/icons/)生成桌面 PNG、ICO、ICNS，并同步到 `public`。桌面原生图标需要重新启动应用查看。
