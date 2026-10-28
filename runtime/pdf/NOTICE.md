# 盘内 PDF 阅读器

| 项 | 值 |
| --- | --- |
| 程序 | SumatraPDF（64 位便携版） |
| 版本 | 3.6.1 |
| 上游 | https://www.sumatrapdfreader.org/ |
| 下载 | https://www.sumatrapdfreader.org/dl/rel/3.6.1/SumatraPDF-3.6.1-64.zip |
| zip 的 SHA-256 | `98b33a518d42986856d225064b0cd2d3643ecf78cbf84ab873d26cc51877a544` |
| 本目录 `SumatraPDF.exe` 的 SHA-256 | `719f689b34f47be8ca105ce8484948474dafde0e106bab599e4a89326070c3d0` |
| 许可 | GNU AGPL v3（全文见同目录 `AGPL-3.0.txt`） |
| 对应源码 | https://github.com/sumatrapdfreader/sumatrapdf/tree/3.6.1rel |

## 为什么是它

选型只看三条硬指标，其余（界面、主题、批注）都不参与决策：

1. **单文件、零安装、零运行时依赖**。zip 里就一个 exe，解压即用，
   不需要 VC++ 运行库、不需要注册表、不需要管理员权限 —— 这是「整盘拷走还能用」的前提。
2. **配置目录可重定向**。`-appdata <目录>` 能把 `SumatraPDF-settings.txt` 和缩略图缓存
   钉在盘内 `.library/state/pdf/`，不落宿主 `%APPDATA%`。这一条把「便携」落到实处：
   换台电脑打开，阅读位置、缩放、最近文件都还在，而宿主没留下任何痕迹。
3. **体积与启动时间**。约 20 MB（内置 MuPDF 渲染引擎），冷启动亚秒级。
   对比之下，用 LibreOffice Draw 开 PDF 要几秒且会抢走文本选择能力，不适合当阅读器。

## 为什么把 exe 改名成 `SumatraPDF.exe`

上游文件名带版本号（`SumatraPDF-3.6.1-64.exe`）。这里去掉版本号，是为了让
`runtime/manifest.json` 里的 `exe` 字段在升级时保持不变 —— 否则每次升级都要
同步改清单，而清单改错的表现是「启动时报告组件缺失」，与「没装」无法区分。

真实版本记在清单的 `version` 字段和本文件里，不记在文件名里。

## 升级步骤

1. 从上游下载新的 `SumatraPDF-<版本>-64.zip`，记下 SHA-256；
2. 解压覆盖 `runtime/pdf/SumatraPDF.exe`；
3. 同步更新本文件的版本号、两个哈希，以及 `runtime/manifest.json` 的 `version`；
4. 打开一个 PDF 验证：阅读位置应沿用上一次的（同一个 `-appdata` 目录）。

不要在这一步之外改动 `manifest.json` 的结构 —— 清单是运行时层的唯一真相源，
它一旦自相矛盾，启动期就会直接拒绝加载（见 `src-tauri/src/runtime/manifest.rs`）。
