# 第三方组件声明

本项目自身代码采用 [MIT](./LICENSE)。下列第三方组件**不在** MIT 范围内，
各自的许可证条款单独适用。

| 组件 | 路径 | 许可证 | 来源 |
|---|---|---|---|
| SumatraPDF | `runtime/pdf/SumatraPDF.exe` | **AGPLv3** | https://www.sumatrapdfreader.org/ |
| Notepad++ | `runtime/Notepad++/notepad++.exe` | **GPLv3** | https://notepad-plus-plus.org/ |
| PreviewHTML（Notepad++ 插件） | `runtime/Notepad++/plugins/` | 见插件目录内 `NOTICE.md` | 随插件包分发 |
| markdown-it | 前端依赖，打包进 `dist/` 与 exe | MIT | https://github.com/markdown-it/markdown-it |

## 分发时要求

`runtime/pdf/` 下的三个文件必须一起分发，缺一不可：

- `SumatraPDF.exe` —— 程序本体，未做任何修改
- `AGPL-3.0.txt` —— 许可证全文
- `NOTICE.md` —— 版本号、来源地址、SHA-256 校验值、对应源码 tag

AGPLv3 第 6 条要求：分发二进制时须提供对应源码的获取方式。`NOTICE.md`
中已记录版本号与源码 tag，按该信息可在上游取得完全相同的源码。

`runtime/Notepad++/` 同理：GPLv3 要求分发时一并给出许可证与源码获取方式，
`NOTICE.md` 里记了版本号、上游下载地址与 `notepad++.exe` 的 SHA-256，
`doLocalConf.xml` 必须随盘（删了会静默退回宿主 `%APPDATA%`）。
本仓库不含该目录的二进制，只保留 `NOTICE.md`。

## 其他

`runtime/python/`、`runtime/node/`、`runtime/office/` 目录若存在，其中内容
同样为第三方软件，遵循各自上游许可证（Python PSF、Node.js MIT、
LibreOffice MPL-2.0）。本仓库不包含这些目录。

`markdown-it` 是唯一的运行时前端依赖（MIT），已随构建产物打包进 exe，
不需要在盘里额外放置任何文件。
