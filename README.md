# 便携资料库 · P3 骨架

装在一块固态移动硬盘里的资料与项目管理系统：**一个前端统一查看 / 管理 / 编辑的库，
库里既有文档也有代码项目，而运行它们所需的环境（Python / Node / LibreOffice / PDF 阅读器）也装在盘里。**

启动器放在盘根，索引与渲染全在本机，所有路径在运行时从「exe 所在目录」推导 —— 换盘符不会坏。

技术栈：**Tauri 2 + WebView2 + SQLite（rusqlite bundled）**，仅 Windows。
架构细节见 [ARCHITECTURE.md](./ARCHITECTURE.md)。

---

## 能做什么

**文件侧（P1）**

- 启动即定位盘根，读取卷序列号（认盘不认盘符）
- 扫描 `content/` 并建立索引，重复扫描只写变化部分
- 懒加载文件树：一次只取一层，几十万文件也不卡
- 内置预览：纯文本 / 代码 / Markdown / HTML
  - Markdown 与 HTML **默认渲染**（沙箱 iframe），可一键切「源码」
  - 渲染会加载同目录的 css 与图片；脚本一律不执行
- 单击预览、双击交给外部程序；其他格式一键打开或在资源管理器中定位

**运行时侧（P2）**

- 读 `runtime/manifest.json`，现场探测每个便携组件的存在性与版本
- 子进程环境沙箱：`HOME` / `TEMP` / `APPDATA` / `PATH` 全部指向盘内，**不碰宿主**
- 便携 LibreOffice：接管 Office 文档编辑、无头转 PDF，配置用 `-env:UserInstallation` 钉在盘内
- 便携 PDF 阅读器（SumatraPDF）：PDF / XPS / EPUB 等固定版式文档直接看，
  设置与阅读位置用 `-appdata` 钉在盘内；转出来的 PDF 也走同一个阅读器
- 便携编辑器（Notepad++ + PreviewHTML 插件）：`.md` / `.html` 交给它改，
  靠 `doLocalConf.xml` 便携、`-multiInst` 防止被宿主那份实例接走；
  预览不必进编辑器（本程序自己渲染），插件用于「边改边看」
- 代码项目自动识别（marker 文件），在项目目录开终端，环境已就绪

---

## 环境要求

| 依赖 | 版本 | 说明 |
|---|---|---|
| Node.js | ≥ 18 | 前端构建 |
| Rust | ≥ 1.77.2 | 后端；需装 VS「使用 C++ 的桌面开发」 |
| WebView2 Runtime | 任意 | **必需**。Win11 自带；Win10 一般随 Edge 装过。盘里刻意不捆绑（见 ARCHITECTURE.md 8.7），缺了启动时会弹窗说明并给出下载链接 |

```powershell
winget install Rustlang.Rustup
```

---

## 开发

```bash
npm install

# 开发期需要一个仿真盘根（不要用 C 盘）
mkdir -p /d/pl-dev-root/content/sample
mkdir -p /d/pl-dev-root/runtime
cp runtime/manifest.json /d/pl-dev-root/runtime/

# PowerShell
$env:PL_ROOT = "D:\pl-dev-root"
npm run tauri dev
```

`PL_ROOT` 只用于开发。发行版里 `Launch.exe` 与 `content/` 同级，无需该变量。

---

## 构建

```bash
npm run build          # 前端类型检查 + 打包
npm run tauri build    # 产出 exe / 安装包
```

发行时把产物 exe 放到盘根，与 `content/`、`runtime/`、`.library/` 同级。

---

## 盘上布局

```
X:\
├─ Launch.exe              # 启动器
├─ content\                # ★用户数据：文档与代码项目（唯一备份对象）
├─ runtime\                # 便携运行时资产（只读，可重建，不必备份）
│  ├─ manifest.json        #   ★运行时的唯一真相源
│  ├─ python\              #   python-build-standalone 3.15（可重定位）
│  ├─ node\                #   解压 node-v22-win-x64.zip
│  ├─ office\              #   LibreOffice 便携版
│  ├─ pdf\                 #   SumatraPDF（单文件）+ NOTICE.md + AGPL 许可证
│  └─ Notepad++\           #   Notepad++ + PreviewHTML 插件（例外：会被写配置，见下）
└─ .library\               # 系统区（可重建）
   ├─ catalog.db           #   索引库
   ├─ cache\pdf\           #   转换产物
   ├─ state\<组件>\        #   组件私有状态（LibreOffice profile、SumatraPDF 设置、webview2 数据）
   └─ home\                #   子进程的「假 HOME」
```

> `runtime\Notepad++\` 是「只读资产」这条规矩的唯一例外：Notepad++ 的便携模式
> 靠 exe 同目录的 `doLocalConf.xml` 标记，配置只能写在它旁边，
> 没有命令行开关可以外移。所以 `config.xml`、`session.xml`、`langs.xml`、
> `stylers.xml`、`plugins\Config\PreviewHTML\` 都是它**生成**的，
> 已在 `.gitignore` 里排除；但 `doLocalConf.xml` 必须随盘，
> 删了它程序会静默退回宿主 `%APPDATA%`。

三个分区的边界是硬约束：

| 分区 | 可写 | 备份 | 丢失后果 |
|---|---|---|---|
| `content/` | 用户改 | **必须** | 数据损失 |
| `runtime/` | 否 | 不必 | 重下组件 |
| `.library/` | 系统写 | 不必 | 重扫几分钟 |

**备份只搬 `content/`；升级只换 `runtime/` 与 `Launch.exe`。**

---

## 装运行时（装配期动作，不是界面动作）

| 组件 | 来源 | 放到 |
|---|---|---|
| Python | **[python-build-standalone](https://github.com/astral-sh/python-build-standalone)** 的 `cpython-3.15.0+…-x86_64-pc-windows-msvc-install_only_stripped.tar.gz` | `runtime\python\`（解压后应为 `runtime\python\python.exe`） |
| Node.js | `node-v22-win-x64.zip`（官方 zip 版） | `runtime\node\` |
| LibreOffice | LibreOffice Portable 或官方解压版 | `runtime\office\` |
| PDF 阅读器 | SumatraPDF 3.6.1 的 `SumatraPDF-3.6.1-64.zip`（见下方校验值） | `runtime\pdf\`，解压出的 exe **改名为 `SumatraPDF.exe`** |
| 编辑器 | Notepad++ 8.9.8.1 便携包（`npp.*.portable.zip`）+ PreviewHTML 插件 1.4.6.0 | `runtime\Notepad++\`（解压后应为 `runtime\Notepad++\notepad++.exe`） |

放好之后启动（或点右侧「重新探测」），组件状态即变绿。**不需要改代码，也不需要跑步进。**

SumatraPDF 的三个校验值都记在 `runtime\pdf\NOTICE.md`：

```
zip      98b33a518d42986856d225064b0cd2d3643ecf78cbf84ab873d26cc51877a544
exe      719f689b34f47be8ca105ce8484948474dafde0e106bab599e4a89326070c3d0
来源     https://www.sumatrapdfreader.org/dl/rel/3.6.1/SumatraPDF-3.6.1-64.zip
```

> **为什么 exe 要改名：** 上游文件名带版本号，而清单里的 `exe` 是固定路径。
> 带版本号意味着每次升级都要改清单，改错的表现是「组件缺失」—— 与「没装」无法区分。
> 真实版本记在清单的 `version` 字段里。

> **许可证：** SumatraPDF 是 AGPLv3。分发这块盘时，`runtime\pdf\` 下的
> `AGPL-3.0.txt` 与 `NOTICE.md`（含源码地址、版本、哈希）要一起带走。我们没有修改它。

> **为什么不用官方 `python-3.15-embed-amd64.zip`：** embed 版禁用了 `site-packages`、
> 不带 `pip`、也没有 `venv`，装不了任何第三方库 —— 它面向的是「把解释器嵌进别的程序」，
> 不是给人开发用。python-build-standalone 的 `install_only` 包自带 pip、
> 且**可重定位**（装到任意路径都能跑），这才是便携开发环境该用的东西。

> **为什么不建 venv：** `venv` 会把解释器的绝对路径写进 `pyvenv.cfg` 和
> `Scripts\*.exe`，换盘符即失效 —— 与「便携」正面冲突。本盘的做法是第三方库
> 直接装进 `runtime\python\Lib\site-packages`，随盘整体移动：
> `runtime\python\python.exe -m pip install …`，**不要加 `--target`，也不要激活任何 venv**。

> **本盘的 Python 只有一个版本**（策略性决定，写在 `manifest::validate` 里）：
> 清单里出现两个 Python 会直接拒绝启动。理由见 ARCHITECTURE.md 7.5 ——
> 「多个版本」必然要回答「某项目用哪个」，而任何答案要么增加配置负担，
> 要么等于随机，最终表现为「昨天能跑今天不能跑」。

不认路径的软件（MS Office、WPS）**不要放进盘** —— 它们只认注册表，任何环境变量都拦不住，
装进来必然污染宿主。这条是产品定义，不是实现细节。

---

## 目录结构

```
portable-library/
├─ index.html                # Vite 入口
├─ runtime/manifest.json     # 运行时清单示例（发到盘根的 runtime/ 下）
├─ src/                      # 前端
│  ├─ api/
│  │  ├─ client.ts           # ★唯一 IPC 边界
│  │  └─ types.ts            # 与 Rust DTO 对齐的类型契约
│  ├─ core/format.ts
│  ├─ views/
│  │  ├─ tree.ts             # 懒加载文件树
│  │  ├─ preview.ts          # 文本 / Markdown / PDF 卡片 / 项目卡片 / 转 PDF
│  │  ├─ runtime.ts          # 运行时状态面板
│  │  ├─ projects.ts         # 项目列表
│  │  └─ statusbar.ts
│  ├─ main.ts                # 装配与编排
│  └─ styles.css
└─ src-tauri/
   ├─ tauri.conf.json
   ├─ capabilities/default.json   # ★前端零 fs 权限
   └─ src/
      ├─ lib.rs              # 装配 + 命令注册
      ├─ main.rs
      ├─ paths.rs            # ★盘根唯一真相源
      ├─ error.rs
      ├─ state.rs
      ├─ db/{schema,repo}.rs # DDL / 迁移 + 全部 SQL
      ├─ scan/walker.rs      # 纯扫描器
      ├─ index/reconcile.rs  # 对账
      ├─ project/mod.rs      # 项目识别（无表）
      ├─ runtime/
      │  ├─ manifest.rs      # 清单解析与校验
      │  ├─ env.rs           # 子进程环境沙箱
      │  ├─ launch.rs        # spawn + 超时 + 输出上限
      │  ├─ probe.rs         # 组件探测（可丢弃快照）
      │  ├─ office.rs        # LibreOffice 参数与转换
      │  ├─ pdf.rs           # SumatraPDF 参数（`-appdata` 隔离）
      │  └─ notepadpp.rs     # Notepad++ 参数（`-multiInst -nosession`）
      ├─ webview2.rs         # 宿主运行时检测 + 缺失提示（启动最前面）
      ├─ open/registry.rs    # 打开器 trait + 注册表
      ├─ commands/{library,open,runtime,project}.rs
      └─ e2e.rs              # 端到端链路测试（仅测试构建）
```

---

## 命令一览

| 命令 | 说明 |
|---|---|
| `lib_init` | 盘根 / 内容区 / 运行时区 / 卷序列号 |
| `scan_full` | 全量扫描 + 对账（带互斥） |
| `list_children` | 取一层子项，懒加载树用 |
| `library_stats` | 文件数 / 目录数 / 总字节 |
| `open_target` | 内容区打开：`preview` / `edit` / `reveal` |
| `open_cached` | 打开缓存区派生物（转换出的 PDF） |
| `read_text` | 文本预览，硬上限 2 MB |
| `preview_base` | 内置渲染的基准目录（绝对路径），iframe 的 `<base>` 用 |
| `runtime_status` | 组件状态（读缓存） |
| `runtime_probe` | 强制重新探测 |
| `doc_convert` | Office → PDF（无头 LibreOffice） |
| `project_list` | 索引上的项目查询 |
| `project_terminal` | 项目目录开终端，环境已就绪 |
| `dir_overview` | 结构概览：面包屑 + 整枝规模 + 每个子目录的子树规模 |
| `watch_info` | 监听是否在跑（不在跑时带原因） |

---

## 验证状态

| 项 | 结果 |
|---|---|
| `cargo check` / `cargo clippy` / `cargo fmt --check` | ✅ 零错误、零警告、无格式差异 |
| `cargo test` | ✅ 45 个全绿（含「增量 ≡ 全量」收敛测试、打开器路由测试） |
| `tsc --noEmit` / `vite build` | ✅ 零错误；27.55 kB JS + 10.69 kB CSS（16 个模块） |
| `npm run tauri build` | ✅ 产出带内嵌前端的 exe |
| 真实窗口 | ✅ 发行版 exe 启动后窗口句柄非零、标题正确，`.library/state/webview2` 自动生成 |
| PDF 链路 | ✅ 盘内 SumatraPDF 打开测试 PDF 成功；设置写在 `.library/state/pdf`，宿主 `%APPDATA%` 无痕迹 |
| 编辑器链路 | ✅ 盘内 Notepad++ 以 `-multiInst -nosession` 打开 md/html；两次启动得到两个独立进程，宿主 `%APPDATA%\Notepad++` 全程不存在 |
| 布局核验 | ✅ 用构建产物的真实 CSS 渲染截图，逐项核对三栏布局 |
| 监听实时性 | ✅ 进程运行中增删改文件，索引 2 s 内自动跟上 |

复验命令：

```bash
cd src-tauri && cargo fmt --check && cargo clippy && cargo test
```

## 发行：只能用 `npm run tauri build`

**`cargo build --release` 出来的不是发行版。** 它不带 `custom-protocol` feature，
二进制仍然去连 `http://localhost:1420` —— 双击 exe 只会得到一个空白窗口，
而构建过程**没有任何报错**。发行一律走：

```bash
npm run tauri build                    # 前端产物 + 嵌入 + 打包
npm run tauri build -- --no-bundle     # 只要 exe，不做安装包
```

判断一个 exe 是不是真发行版：不起 vite、直接双击它，能出界面就是对的。

## 已知欠账

内置 PDF / 图片预览、项目启动命令、缓存清理、exFAT 上的 ACL 实测、长路径支持 ——
详见 ARCHITECTURE.md 第十节。WebView2 数据目录外移已完成（`.library/state/webview2`）。

## 许可证

本项目代码采用 **MIT**，全文见 [LICENSE](./LICENSE)。

**例外**：`runtime/pdf/` 下的 SumatraPDF 是第三方组件，采用 **AGPLv3**，
不在 MIT 范围内。分发本项目的副本时，该组件的许可证条款单独适用——
`runtime/pdf/AGPL-3.0.txt` 与 `runtime/pdf/NOTICE.md` 必须随行。
完整清单见 [THIRD-PARTY.md](./THIRD-PARTY.md)。
