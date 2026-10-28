# 第三方组件：Notepad++

本目录是 Notepad++ 便携版**原样解压**的结果，未做任何二进制修改。
本系统只通过命令行参数启动它，并依赖它自带的便携模式。

| 项 | 值 |
|---|---|
| 程序 | Notepad++ |
| 版本 | 8.9.8.1（见 `change.log`） |
| 可执行文件 | `notepad++.exe` |
| SHA-256 | `c52c9ed2ec6d0b5b3f243b29f2124f98db56b6ba92bb42a9af2665709464b3cf` |
| 官方站点 | https://notepad-plus-plus.org/ |
| 源码 | https://github.com/notepad-plus-plus/notepad-plus-plus （tag `v8.9.8.1`） |
| 许可证 | **GNU GPL v3**，全文见本目录 `license.txt` |

## 附带插件：PreviewHTML

| 项 | 值 |
|---|---|
| 插件 | Preview HTML（`plugins/PreviewHTML/PreviewHTML.dll`） |
| 版本 | 1.4.6.0（见 `plugins/PreviewHTML/ReleaseNotes.txt`） |
| 作者 | rdipardo |
| 源码与文档 | https://github.com/rdipardo/npp_preview |
| 许可证 | **GNU GPL v3**，全文见 `plugins/PreviewHTML/LICENSE.txt` |

其余插件（NppConverter / NppExport / mimeTools）为 Notepad++ 官方发行包自带，
同样遵循 GPLv3。

## 分发要求

GPLv3 不要求你附带一份源码，但要求你**提供获取源码的途径**。上面的源码地址
与版本号即为该途径；如果你修改过本目录内的任何二进制（本系统没有），
则必须以 GPLv3 公开修改后的源码。

分发时 `license.txt`、`change.log`、`plugins/PreviewHTML/LICENSE.txt`
与 `plugins/PreviewHTML/ReleaseNotes.txt` 必须随行，其余文件请勿单独挑出。

## 便携模式说明（不是许可证问题，但会踩）

本目录内的 `doLocalConf.xml` 是 Notepad++ 便携模式的**开关标记**：
只要它与 `notepad++.exe` 同目录，配置就写在 exe 旁边，而不是
`%APPDATA%\Notepad++`。**删掉它，程序会静默退回宿主配置目录**，
界面上没有任何提示 —— 这块盘也就不再「便携」了。

因此本目录会被写入：`config.xml`、`session.xml`、`langs.xml`、
`stylers.xml`、`shortcuts.xml`、`contextMenu.xml`、
`plugins/Config/PreviewHTML/Settings.ini` 与 `plugins/Config/PreviewHTML/WebView2Cache/`。
这些是**生成物**，已在 `.gitignore` 中排除，不属于交付内容。
