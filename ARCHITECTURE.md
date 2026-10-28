# 便携资料库 · P3 骨架架构说明

> 适用范围：Windows 10/11 x64 · Tauri 2 + WebView2
> 本文回答四件事：**分了几块、各自负责什么、谁依赖谁、将来往哪儿长。**

---

## 一、范围

产品定义一句话：**这块盘是一个由前端统一查看 / 管理 / 编辑的库，库里既有文档也有代码项目，而运行它们所需的环境本身也装在盘里。**

对应的三条闭环：

```
① 文件闭环：插上盘 → 定位盘根 → 扫描 content/ → 建索引 → 浏览 → 预览 / 外部编辑
② 运行闭环：读 runtime/manifest.json → 现场探测组件 → 子进程环境沙箱 → 转换 / 开终端
③ 同步闭环：监听 content/ → 折成脏路径 → 局部对账 → 落库 → 推事件 → 界面自更新
```

三条闭环的关系不是并列，而是**②③ 都建立在①之上**：没有可靠的索引，运行与同步都无处落脚。

**P1 已完成**：盘根推导、卷身份、全量扫描与对账、SQLite 索引、懒加载文件树、文本 / Markdown 预览、外部打开与定位。

**P2 已完成**：运行时清单与校验、组件探测、子进程环境沙箱、带超时的进程启动、LibreOffice 无头转换与接管编辑、基于索引的项目识别、项目内开终端。

**P3 已完成**：文件监听与去抖、基于脏路径的增量对账（含「增量结果 ≡ 全量结果」的收敛测试）、目录结构视图（面包屑 + 分支规模 + 子树统计）、右栏分类全景与页签化。

**P3.1 已完成**：便携 PDF 阅读器（SumatraPDF，用 `-appdata` 把设置钉在盘内）、
WebView2 发行策略定案为「依赖宿主 Evergreen，不捆绑」（见 8.7）、WebView2 用户数据目录外移、
缺 WebView2 时的启动期提示。

**仍不做**（明确留给下一阶段，避免骨架虚胖）：全文检索、标签与元数据、docx/xlsx 前端渲染、
pdf.js 内置预览（PDF 走盘内阅读器外开）、项目级启动配置（`project_profile`）。

---

## 二、分层与依赖方向

依赖是**严格单向**的，箭头只能从上往下：

```
┌──────────────────────────────────────────────────────────┐
│  前端（WebView）                                          │
│  main.ts ──► views/*  ──►  api/client.ts  ★唯一 IPC 边界  │
└───────────────────────────┬──────────────────────────────┘
                            │ invoke
┌───────────────────────────▼──────────────────────────────┐
│  commands/        命令层：取状态 → 调下层 → 转 DTO         │
├──────────────────────────────────────────────────────────┤
│  index/           对账：扫描结果 × 旧索引 → 差集（全量 / 局部同一算法） │
│      │                                                   │
│      ├──► scan/   纯生产者：走文件系统，产出 FsEntry        │
│      └──► db/     纯持久化：所有 SQL 只在这里              │
├──────────────────────────────────────────────────────────┤
│  watcher/         ★监听：只吐脏路径，不认识 Tauri、不写库   │
│                   （事件是线索，磁盘才是真相）              │
├──────────────────────────────────────────────────────────┤
│  project/         项目识别：marker 名单 + 噪声目录（无表）  │
│  runtime/         ★盘内环境：清单 / 探测 / 环境沙箱 / 启动  │
│      ├── manifest 清单是真相（文件，不是库）                │
│      ├── env      子进程环境（不碰宿主）                    │
│      ├── launch   spawn + 超时 + 输出上限                  │
│      ├── probe    组件可用性快照（可丢弃）                  │
│      └── office   LibreOffice 专用参数与转换               │
├──────────────────────────────────────────────────────────┤
│  open/            打开器路由（trait + 注册表）             │
│  state/           全局状态（装配用，不含逻辑）              │
├──────────────────────────────────────────────────────────┤
│  paths/           ★盘根唯一真相源：相对 ⇄ 绝对 + 越界校验  │
│  error/           统一错误类型                            │
└──────────────────────────────────────────────────────────┘
```

### 依赖矩阵

行依赖列，`●` 表示有依赖：

| ↓依赖→ | paths | error | db | scan | index | open | project | runtime | watcher | state | commands |
|---|---|---|---|---|---|---|---|---|---|---|
| paths | — | ● | | | | | | | | | |
| error | | — | | | | | | | | | |
| db | | ● | — | | | | | | | | |
| scan | ● | ● | | — | | | | | | | |
| index | | ● | ● | ● | — | | | | | | |
| watcher | ● | ● | | ● | | | | | — | | |
| project | | | | | | | — | | | | |
| runtime | ● | ● | | | | | | — | | | |
| open | ● | ● | | | | | | ● | | | |
| state | ● | | | | | ● | | ● | ● | — | |
| commands | ● | ● | ● | ● | ● | ● | ● | ● | ● | ● | — |

**没有任何反向边**，也没有环。注意四条：
- `project` 只依赖业务常量，**不依赖 db** —— 它提供 marker 名单，SQL 由 `db::repo` 通用地执行
- `runtime` **不依赖 db**，因为它不写库；持久化不是运行时的职责
- `watcher` **不依赖 db，也不依赖 index** —— 它连「脏路径会被怎么处理」都不知道。
  这一条是它能被单独测试的原因，也是「事件不能当真相」的结构化表达：
  一个不持有数据库的模块，物理上不可能把事件写进库。
- `state` 依赖 `watcher`，是为了**持有监听句柄**（drop 即停止监听）

---

## 三、模块职责

### 后端（`src-tauri/src/`）

| 模块 | 职责 | 明确不做的事 |
|---|---|---|
| `paths` | 推导盘根、路径相对/绝对互转、越界校验、读卷序列号 | 不读业务数据，不认识数据库 |
| `error` | 统一 `AppError`，实现 `Serialize` 供 IPC 使用 | 不含业务分支 |
| `db::schema` | 建表、`user_version` 迁移 | 不含查询逻辑 |
| `db::repo` | 全部 SQL 收敛于此，向上返回 `NodeRow` 等纯结构 | 不碰文件系统，不做业务判断 |
| `scan` | 迭代式 DFS 遍历，产出 `FsEntry` 流；`probe` 单路径三态探测（在 / 不在 / 读不到） | **不写库**；不认识 SQLite |
| `index` | 扫描结果与旧索引求差集，事务内落库，产出 `ReconcileReport`；全量与增量共用同一套判据 | 不做渲染，不启进程 |
| `watcher` | 递归监听 content/，静默期去抖，把事件折成相对路径批次 | **不认识 Tauri、不写库、不判断事件类型** |
| `project` | marker 名单、优先级、噪声目录判定、语言→运行时映射 | **不建表**、不查库、不启进程 |
| `runtime::manifest` | 读/校验 `runtime/manifest.json`，给组件可执行路径 | 不探测、不缓存、不写库 |
| `runtime::env` | 构造子进程环境（HOME/TEMP/PATH 重定向） | **不改宿主环境**，不 spawn |
| `runtime::launch` | `spawn` + 超时 + 输出上限 + 隐藏窗口 | 不认识任何具体组件 |
| `runtime::probe` | 把清单变成「能不能跑」的事实 | 结果可丢弃，不落库 |
| `runtime::office` | LibreOffice 参数（`UserInstallation`）与 PDF 转换 | 不做通用渲染 |
| `runtime::pdf` | SumatraPDF 参数（`-appdata` 隔离、扩展名集合） | 不做渲染、不解析 PDF |
| `webview2` | 启动前查宿主 WebView2 是否在位，缺了弹窗说人话 | 不下载、不安装、不提权 |
| `open` | `open(target, mode)` 动作模型；`Opener` trait + 注册表 | 不做预览渲染（前端负责） |
| `state` | 持有 `RootCtx` / `Connection` / `OpenerRegistry` / 清单 / 探测快照 / 监听句柄 | 不含业务逻辑 |
| `commands` | IPC 表面：参数校验、取状态、调下层、转 DTO | **不写 SQL、不碰文件系统** |

### 前端（`src/`）

| 模块 | 职责 |
|---|---|
| `api/client.ts` | **唯一 IPC 边界**。视图层禁止直接 import `@tauri-apps/api` |
| `api/types.ts` | 与 Rust DTO 一一对应的类型契约 |
| `core/format.ts` | 纯格式化函数（含面包屑），无 IO 无状态 |
| `views/tree.ts` | 懒加载文件树；`refresh()` 原地重建并保留展开态，`reveal()` 逐级定位 |
| `views/dirview.ts` | **目录结构页**：面包屑 + 规模统计 + 子目录分支（带规模条）+ 本级文件 |
| `views/structure.ts` | 右栏**分类全景**：根层各分支的规模与占比，并高亮当前所在支 |
| `views/preview.ts` | 文本预览：md / html **渲染**（沙箱 iframe）与**源码**可切换；PDF 走盘内阅读器；Office 给「转 PDF」入口；项目卡片 |
| `views/runtime.ts` | 运行时面板：组件状态、重新探测；探完把「office/pdf 是否可用」回灌给预览视图 |
| `views/projects.ts` | 项目列表：语言徽标、运行时就绪指示 |
| `views/statusbar.ts` | 只读展示；含实时同步状态 |
| `main.ts` | 装配与编排：导航（`navigate` / `openFile`）、刷新（`refreshAll`）、事件订阅 |

---

## 四、关键接口

### 4.1 对前端暴露的命令（P3 全量 15 个）

| 命令 | 入参 | 出参 | 说明 |
|---|---|---|---|
| `lib_init` | — | `RootInfo` | 启动首个调用，拿盘根 / 内容区 / 运行时区 / 卷身份 |
| `scan_full` | — | `ScanReport` | 全量扫描 + 对账，带互斥保护 |
| `list_children` | `parent: string \| null` | `NodeDto[]` | 取一层子项，`null` 表示根 |
| `dir_overview` | `rel: string \| null` | `DirOverview` | **结构概览**：面包屑 + 整枝规模 + 每个子目录的子树规模 |
| `library_stats` | — | `StatsDto` | 文件数 / 目录数 / 总字节 / 最近改动 |
| `watch_info` | — | `WatchInfo` | 监听是否在跑；不在跑时带上原因 |
| `open_target` | `rel: string, mode: OpenMode` | `OpenResult` | 内容区动作模型入口 |
| `open_cached` | `rel: string` | `OpenResult` | 打开缓存区派生物（转换出的 PDF） |
| `read_text` | `rel: string, maxBytes?: number` | `TextPreview` | 文本预览，硬上限 2 MB |
| `preview_base` | `rel: string` | `string` | 内置渲染的基准目录（绝对路径），供 iframe 的 `<base>` 使用 |
| `runtime_status` | — | `RuntimeStatus` | 组件状态（读缓存，不重新探测） |
| `runtime_probe` | — | `RuntimeStatus` | 强制重新探测 |
| `doc_convert` | `rel: string` | `ConvertResult` | Office → PDF（无头 LibreOffice） |
| `project_list` | `limit?: number` | `ProjectDto[]` | 索引上的项目查询 |
| `project_terminal` | `rel: string` | `()` | 项目目录内开终端，环境指向盘内运行时 |

字段全部 `camelCase`（Rust 侧 `#[serde(rename_all = "camelCase")]`），Tauri 自动把 `maxBytes` 映射到 `max_bytes`。

**为什么 `open_cached` 要单独存在**：`open_target` 只能碰 `content/`。若为了打开一个生成的 PDF 而放宽它的作用域，`content/` 就不再是硬边界。两个命令 = 两个作用域 = 边界仍然可审计。

**为什么 `dir_overview` 是一个命令而不是三个**：面包屑、统计、子项列表若分开取，界面就有三个时刻，
期间任何一次刷新都可能让「标题说 42 个目录、列表只有 41 条」同时出现在屏幕上。
一次调用返回一份快照，界面内部就永远自洽。

### 4.1.1 推送事件（前端订阅，不是命令）

| 事件 | 载荷 | 触发条件 |
|---|---|---|
| `library:changed` | `ReconcileReport` | 增量对账**真的改动了索引**（新增/更新/移除 > 0） |
| `library:error` | `string` | 增量对账失败（库被占用等），不致命但要让用户看见 |

前端**不轮询**。没有事件 = 索引没变 = 界面不必动。这一条让「静置时的开销」降到零。

### 4.2 内部关键类型

```rust
// 盘根上下文 —— 唯一能产出绝对路径的地方
impl RootCtx {
    fn detect() -> AppResult<Self>;                          // 读 PL_ROOT，否则用 exe 所在目录
    fn resolve(&self, rel: &str) -> AppResult<PathBuf>;   // 含越界校验，作用域 = content/
    fn cache_path(&self, rel: &str) -> AppResult<PathBuf>; // 作用域 = .library/cache/
    fn relativize(&self, abs: &Path) -> AppResult<String>;
    fn detect_in(explicit: Option<PathBuf>) -> AppResult<Self>; // 盘根来源是参数，不是全局状态
    fn content(&self) -> &Path;                            // 索引作用域
    fn runtime_dir(&self) -> PathBuf;                      // runtime/（只读资产）
    fn home_dir(&self) -> PathBuf;                         // .library/home（子进程假 HOME）
    fn state_component_dir(&self, id: &str) -> PathBuf;    // .library/state/<id>
}

// 扫描产物 —— 不含任何绝对路径
pub struct FsEntry {
    pub rel_path: String,
    pub parent_rel: Option<String>,
    pub kind: EntryKind,
    pub size: i64,
    pub mtime: i64,
    pub quick_hash: String,   // "{size}-{mtime}"
}

// 打开器扩展点
pub trait Opener: Send + Sync {
    fn id(&self) -> &'static str;
    fn matches(&self, ext: Option<&str>) -> bool;
    fn launch(&self, abs: &Path) -> AppResult<()>;
}

// 运行时清单 —— 盘内环境的唯一真相（跟着文件走，不会因为整盘复制而失真）
pub struct Manifest { pub schema: u32, pub components: Vec<Component> }
pub struct Component {
    pub id: String,          // 同时是 .library/state/ 下的目录名
    pub kind: Kind,          // python | node | office | tool
    pub version: String,
    pub exe: String,         // 相对 runtime/，走同一套越界清洗
    pub version_args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

// 进程启动：超时与隐藏窗口是两个必填项，不是可选项
pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub plan: EnvPlan,
    pub hide: bool,          // 开终端时必须 false
}
```

### 4.3 五条隐性契约（比签名更重要）

1. **`scan` 的产出顺序**：迭代式 DFS，**父节点一定先于子节点**。
   `index` 依赖这条契约，才能用「已见过的父 id」直接建边，省掉一轮全表回填。
   想改成并行扫描？必须先补一轮 `parent_id` 回填，否则父子关系会断。

2. **全量与增量必须产出逐字段一致的 `FsEntry`**。
   两条路径共用 `walker::build_entry`，不是「各写一遍、祈祷一样」。
   一旦分叉，库里的节点就会在两种扫描之间反复抖动（表现为「每次扫描都在更新同一批文件」），
   而 `e2e::incremental_matches_full_scan` 就是守这条的。

3. **`probe` 是三态，不是 `Option`**：`Present` / `Absent` / `Inaccessible`。
   把「读不到」当成「不存在」，会在权限抖动时静默删掉索引里的整棵子树 ——
   这种错误没有任何日志可查。

4. **`repo` 的出参不含绝对路径**。任何一层都不该缓存绝对路径——包括状态对象。

5. **`commands` 是唯一能碰到 `AppState` 的地方**。下层一律靠参数传入 `RootCtx` / `&Connection`，
   这样下层可以脱离 Tauri 单独测试。

还有一条关于顺序的：**`project_list` 依赖 `runtime_probe` 的结果**。前端必须先探测运行时、
再拉项目列表，否则第一批项目会被标成「运行时缺失」，而事实只是「还没探测」。
后端为此做了降级：探测缓存为空时 `runtime_ready` 一律返回 `true`（未知 ≠ 缺失）。

---

## 五、数据流

### 5.1 启动流

```
前端 boot()
  └─► lib_init ──────────────► RootCtx::detect()
                                 ├─ current_exe().parent()    ← 或 PL_ROOT 覆盖
                                 ├─ create_dir_all(content/, .library/)
                                 └─ GetVolumeInformationW → volume_serial
  └─► library_stats ─────────► 空库？
        ├─ 是 ──► scan_full ──► refreshAll()
        └─ 否 ──► navigate(null)    ← 直接展示分类全景
  └─► watch_info ────────────► 状态栏显示「实时同步 开/关」
  └─► onLibraryChanged(…) ───► 订阅推送（必须早于首次扫描，避免漏掉扫描期的变更）
```

扫盘**不放在启动钩子里**：大目录会把窗口卡成白屏，改成前端显式触发，并显示进度文案。
监听则相反，**必须在后端 setup 里就起来** —— 早于任何界面动作，才不会漏掉第一波变更。

### 5.2 扫描流

```
scan_full
  └─► index::rescan
        ├─ repo::upsert_volume(serial)         → volume_id
        └─ index::full_scan
              ├─ scan::walk_full(content)      → Vec<FsEntry>（纯内存）
              ├─ BEGIN TRANSACTION
              ├─ repo::load_index(volume_id)   → HashMap<rel_path, (id, old_hash)>
              ├─ for entry in entries:
              │    命中 & hash 相同  → 跳过（零写入）
              │    命中 & hash 不同  → repo::update_node
              │    未命中            → repo::insert_node
              ├─ 剩余 known           → repo::delete_node
              └─ COMMIT
```

**未变化的节点零写入**——这是让重复扫描快起来的关键，也是「真相是对账，不是事件」的落地。

### 5.3 打开流

```
open_target(rel, mode)
  └─► RootCtx::resolve(rel)          ← 越界校验在这里，只有这一处
        └─► OpenerRegistry::dispatch(abs, ext, mode)
              ├─ Preview → Internal        （前端渲染）
              ├─ Reveal  → explorer /select,    → External
              ├─ Edit    → 首个 matches 的 Opener → External
              │             ├─ portable-office（盘内 soffice + 独立 profile）
              │             └─ system-default（宿主文件关联，兜底）
              └─ Convert → 走独立的 doc_convert 命令，不在此分支
```

**注册顺序就是优先级。** 便携 Office 排在系统默认之前，但只在「exe 真的存在」时才注册；
不存在就自动回落到系统关联，这样「组件没装」表现为「用宿主的程序打开」，
而不是一个没人看得懂的启动失败。

### 5.4 预览流（内置渲染）

```
点击文件 → preview.show(node)
  ├─ 扩展名不在 TEXT_EXT ──► 操作卡片（PDF / Office / 系统程序）
  └─ 文本类
       ├─ read_text(rel)     → 正文（上限 256 KB，超了截断并提示）
       └─ preview_base(rel)  → 所在目录的绝对路径（只有 md / html 才取）
       ▼
     前端组装
       ├─ md   → markdown-it 渲染 → 套上内联排版样式
       ├─ html → 原样，只在 <head> 后插一行 <base>
       └─ 其余 → 原样源码
       ▼
     <iframe sandbox="" srcdoc=…>
       └─ 相对资源（css / 图片）→ http://asset.localhost/<盘内目录>/
```

三条必须一起成立的约束，缺一条就只剩「没有样式的裸 HTML」：

1. **`asset` 协议要放行盘根下的 `content/`** —— 在 `setup` 里运行时给
   （`allow_directory`），因为盘符会漂，写进 `tauri.conf.json` 就会在换盘后失效。
2. **`<base href>` 必须指向「文件所在目录」的 asset URL，且末尾有 `/`** ——
   `convertFileSrc` 把整条路径编码成**一个** path segment（`\` → `%5C`），
   末尾不带斜杠时，URL 解析会把最后一段当文件名替换掉，目录就丢了。
3. **`<base>` 必须插在 `head` 之后** —— 它只影响其后出现的相对 URL。

沙箱用**空 sandbox**：不执行脚本、不提交表单、不做顶层跳转。盘里的 HTML 是用户内容，
不是本程序的代码——「看」不需要「跑」。要交互效果的，走外部程序。

为什么不自己起一个本地 HTTP 服务：多一个监听端口、多一套 MIME / Range / 路径穿越防护
要自己写，而 `asset` 协议这三件事 Tauri 已经做完了（含 Range）。代价是 webview 因此
能读 `content/` 下的任何文件——对「预览自己的文档」这件事，正是想要的边界。

### 5.5 运行流（P2 新增）

```
启动期（setup）
  └─► runtime::manifest::load(runtime/)
        ├─ 文件不存在 ──► manifest = None（合法：纯文件库）
        ├─ 解析/校验失败 ──► manifest = None + manifest_error 留在界面
        └─ 成功 ──► OpenerRegistry::for_root()

前端首屏
  └─► runtime_status ──► AppState.runtime_probe（None 才探测）
        └─► 对每个组件：exe 存在？→ 跑 version_args（6 s 超时）→ 解析首行
  └─► project_list
        └─► repo::list_project_dirs(volume_id, markers)   ← 一条 JOIN
              └─► 按 marker 优先级归并 + 噪声目录过滤 → ProjectDto[]

点「转 PDF」
  └─► doc_convert(rel)
        └─► RootCtx::resolve(rel)          内容区边界
        └─► office::convert_to_pdf
              ├─ EnvPlan::build            子进程 HOME/TEMP/PATH 重定向
              ├─ args: -env:UserInstallation=file:///…/state/libreoffice
              ├─ run_capture(--headless --convert-to pdf, 90 s)
              └─ **只认产物**：文件存在且非空才算成功
                    （退出码 0 不代表成功，这是 LibreOffice 的已知行为）
```

为什么探测不放在启动钩子里：三个组件就是三次进程启动，慢的能到几秒。
首屏白屏是最贵的失败 —— 所以探测延到前端首帧之后。

为什么转换结果放在 `.library/cache/pdf/<源路径哈希>/`：平铺会撞名，
`a/report.docx` 和 `b/report.docx` 转出来的 PDF 会互相覆盖，而覆盖是**静默**的错误。

### 5.6 同步流（P3 新增）

```
后端线程（pl-watch）
  ├─ notify 递归监听 content/  → ReadDirectoryChangesW
  ├─ 过滤 Access 事件           ← 否则「看文件」也会触发对账，自己吵自己
  ├─ 绝对路径 → 相对路径，丢弃噪声目录（任意一段命中即丢）
  ├─ 静默期去抖：安静 400 ms 才处理；连续写入最多等 2 s
  └─ sink(脏路径批次)
        └─ AppState.db.lock()   ← 全量扫描期间会阻塞在这里，事件在通道里排队不丢
              └─ index::reconcile_dirty
                    ├─ 排序去重 + 剪掉「祖先已整棵处理」的后代
                    ├─ 逐条 walker::probe：
                    │     Absent       → 删节点（后代靠外键级联），removed += 子树规模+1
                    │     Inaccessible → 记一次 skipped，**绝不删**
                    │     Present(dir) → upsert 自身 + 整棵子树对账
                    │     Present(file)→ upsert 自身
                    ├─ 父目录一并 upsert（子项增删会改父目录 mtime）
                    └─ COMMIT
              └─ 只有 added+updated+removed > 0 才 emit("library:changed")

前端
  └─ onLibraryChanged → refreshAll()
        ├─ tree.refresh()        原地重建已展开层级，保留展开态
        ├─ 当前在看文件？重取内容 : 重取当前目录页
        ├─ structure / projects 重载
        └─ 状态栏「已同步 +a ~u -r」
```

**为什么监听线程要阻塞在数据库锁上**：另一条路是「扫描期间丢弃事件」，
但那样会漏掉扫描启动之后才发生的变更 —— 扫描读的是某个时刻的磁盘快照，
它结束后的半小时里发生的事就永久丢了（直到下次全量）。
排队等待虽然让这次对账晚一点，但脏路径集合是「扫完之后再 stat」，
结果必然收敛到最新状态。**宁可慢，不可漏。**

**为什么父目录也要更新**：目录节点的指纹含 mtime，而子项增删会改父目录 mtime。
少这一步，增量之后紧接着跑一次全量就会报出一堆 `updated` ——
那不是真实变化，是两条路径口径不一致。`e2e::incremental_matches_full_scan` 就是守这一条的。

---

## 六、不变量

违反任何一条都会在生产环境炸，而且通常只在**换盘符之后**才炸：

| # | 不变量 | 守卫位置 |
|---|---|---|
| 1 | 磁盘上不存在写死的绝对路径 | `paths::RootCtx` 是唯一推导点 |
| 2 | 库内 `rel_path` 一律是相对 content 根、`/` 分隔 | `scan::to_slash` |
| 3 | 前端拿到的任何路径都过越界校验 | `RootCtx::resolve` / `cache_path` |
| 4 | 前端不持有 fs 权限 | `capabilities/default.json` 只给 `core:default` |
| 5 | 单次只允许一个扫描在跑 | `AppState.scanning` + `ScanBusy` |
| 6 | 父节点先于子节点入库 | `scan` 的 DFS 契约 |
| 7 | 任何子进程都有超时 | `launch::run_capture` 的 `timeout` 参数（必填） |
| 8 | 环境改动只作用于子进程 | `EnvPlan::apply(&mut Command)`，全仓无 `env::set_var` |
| 9 | 打开终端时不得隐藏窗口 | `Spec.hide = false` |
| 10 | 运行时状态不落库 | `AppState.runtime_probe` 是 `Mutex<Option<_>>` |
| 11 | 增量与全量算出的索引必须逐行相同 | `e2e::incremental_matches_full_scan` |
| 12 | 「读不到」不得被当成「不存在」 | `scan::Probe` 三态；`Inaccessible` 只计数不删 |
| 13 | 监听不写库、不认识 Tauri | `watcher` 的依赖只有 `paths` / `error` / `scan` |
| 14 | 索引变化才推送，未变化不打扰 | `lib::start_watcher` 里 `added+updated+removed > 0` 才 emit |

第 4 条值得多说一句：**不把 `fs:*` 权限交给前端**，是为了让「路径校验」只有一条通道。一旦前端能直接读文件，`resolve()` 的防线就被绕过了。

第 8 条同样值得多说：`std::env::set_var` 改的是当前进程的环境，会被所有后续子进程继承。
插一次盘、点一次「转 PDF」就污染全局，正是便携系统最不该犯的错。
全仓搜索 `set_var` 应该**零命中** —— 这条可以直接当 CI 检查用。

第 11 条是这一版最要紧的不变量。监听会丢事件、会把一次操作报成几条、不同平台语义还不同 ——
**只要「局部重算」与「整体重算」结果一致，这些不确定性就都无害**，差异会被下一次对账自然抹平。
反过来，两条路径一旦口径分叉，索引会在两种扫描之间反复抖动，而这种抖动极难从日志里看出来。

---

## 七、增量对账与实时同步（P3 核心）

### 7.1 监听不能当真相，只能当线索

文件监听有四个天生缺陷，任何一个都足以让「收到事件就改库」这种做法出错：

| 缺陷 | 后果 |
|---|---|
| 缓冲区溢出会丢事件 | 有些变更**永远收不到通知** |
| 一次保存产生 3~5 个事件（写临时文件 → 改名 → 改属性） | 同一棵树被反复对账，中间态还会被写进库 |
| 重命名在不同平台报成不同组合（old+new / 只有 new / 只有 old） | 只报新路径时，旧节点会永远留在索引里 |
| 整目录移动往往只有一条目录级事件 | 子项变化不会逐个上报 |

所以监听的定位被压到最低：**它只负责告诉你「哪些路径可能变了」。**
真正的判断一律重新 stat 磁盘。这样无论事件准不准，最终状态都收敛到磁盘现状。

这条定位在代码里是结构化的，不是靠自觉：`watcher` 模块**不依赖 `db`、也不依赖 `index`**。
一个不持有数据库的模块，物理上不可能把事件写进库。

### 7.2 去抖：两个时间常数

```
安静 400 ms 就处理      ← 一次保存的事件簇通常在 300 ms 内结束
最长等 2 s              ← 拷贝大目录可能连续写几分钟，不能无限推迟
```

400 ms 是「一次保存的事件簇」与「两次独立操作」的分界。再短会把同一次保存拆成两批，
再长会让粘贴文件的手感变钝。2 s 上限则是为了「持续写入」这种情况：
没有上限时，一个人往盘里拷 10 GB 目录，索引会一直不动，看起来像卡死了。

### 7.3 局部对账的三个判断

| 情况 | 处理 | 为什么 |
|---|---|---|
| 磁盘上没有 | 删节点（后代靠 `ON DELETE CASCADE`），`removed` 数子树规模 + 1 | 计数要如实，否则状态栏的「移除 N」会说谎 |
| 读不到（权限/占用） | 只记 `skipped`，**绝不删** | 把「读不到」当「不存在」会静默删掉整棵子树 |
| 存在且是目录 | upsert 自身 + **整棵子树**对账 | 整目录移动只报一条事件，只处理自身会漏掉全部子项 |

外加两条收尾动作：

- **父目录一并 upsert**：子项增删会改父目录 mtime，不补这一步就与全量口径不一致。
- **剪掉「祖先已被整棵处理」的后代**：一次目录重命名会带出成百上千条事件，
  不剪枝就会重复走同一棵树。

### 7.4 界面：为什么按结构展示，而不是按类型

需求原话是「反映文件架构，比如文件如何分类、从属关系如何」，而不是「按文件类型分类展示」。
这两者的差别不是审美，是**信息模型**的差别：

| 维度 | 按类型分组 | 按结构展示（当前做法） |
|---|---|---|
| 回答的问题 | 「这是什么东西」 | 「它属于哪里、和谁在一起」 |
| 一个项目的散件 | 被拆到 doc/、png/、md/ 三个格子里 | 留在同一个分支下，看得出是一件事 |
| 新增一层分类 | 要改分组规则 | 建一个目录即可，零代码 |
| 与磁盘的一致性 | 需要维护「类型映射表」 | 目录结构就是分类，永远一致 |

三处界面共同表达结构：

1. **面包屑**：从属链。中间栏与文件预览各有一份，点任一级就跳回那个目录。
2. **目录页**：这一层怎么分的 —— 每个子目录一行，带**子树规模条**（按同级最大值归一）
   与「占本层 X%」。规模条不是装饰：它让「哪个分支是主体」一眼可见，而这正是分类的结果。
3. **右栏分类全景**：根层各分支的规模与占比，始终展示全景、只高亮当前分支 ——
   全景图一跟着移动就失去了参照价值。

统计口径统一为「**子树**」而非「本级」：`dirs` / `files` / `bytes` 都不含自身。
因为人问「这个文件夹有多大」时，问的是它装了多少东西，不是它自己的 inode 元数据。

---

## 八、盘内运行时层（P2 核心）

### 8.1 三个分区，三种生命周期

盘根下只有三类东西，混在一起就必然出问题：

| 分区 | 内容 | 可写 | 备份 | 丢失后果 |
|---|---|---|---|---|
| `content/` | 用户的文档与项目 | 是（用户改） | **必须**（唯一备份对象） | 数据损失 |
| `runtime/` | 便携 Python / Node / LibreOffice / PDF 阅读器 | 否（只读资产） | 不必（可重建） | 重下组件，几十分钟 |
| `.library/` | 索引库、缓存、假 HOME、日志 | 是（系统写） | 不必（可重建） | 重扫，几分钟 |

这条分区是整个设计的地基。它带来三个直接结论：

1. **备份只搬 `content/`** —— 其余都是可再生的，搬了只是浪费空间
2. **换盘只换 `runtime/`** —— 用户内容不动，所以「升级运行时」是安全操作
3. **`runtime/` 不写库** —— 因为它是可整体替换的，任何对它的登记都会随替换失效

### 8.2 「隔离」不是一个动作，是三层手段

上一轮技术核实推翻了一个流行说法：**环境变量挡不住污染**。
大量 Windows 程序绕开 `%APPDATA%` 直接调 `SHGetKnownFolderPath` 问注册表，注册表写入更是拦不住。
所以这一层必须按「组件愿意配合到什么程度」分档：

| 手段 | 拦住什么 | 对谁有效 | 代码位置 |
|---|---|---|---|
| 环境变量重定向（HOME / TEMP / APPDATA / PATH） | 愿意读环境变量的程序 | 大多数 CLI 工具、Python、Node | `runtime::env` |
| 组件私有状态参数 | 自己有 profile 概念、但不看环境变量的程序 | **LibreOffice**（`-env:UserInstallation`）、**SumatraPDF**（`-appdata`） | `runtime::office` / `runtime::pdf` |
| 进程边界 | 以上都不吃、只认注册表的程序 | MS Office / WPS 这类 | **做不到，只能不进盘** |

第三档是这套系统的**硬边界**，应该写进产品定义而不是当实现细节：
盘里能装的是「认路径的软件」，不认路径的（MS Office 全家桶）装进来也一定会污染宿主。

LibreOffice 正好落在这张表的第二档，值得单独记一笔：
它的配置路径由程序内部拼出，`%APPDATA%` 改了也没用，唯一手段是命令行参数
`-env:UserInstallation=file:///…`。顺带解决了另一个坑 ——
**用户机器上开着 LibreOffice 时，无头转换会因 profile 被锁而静默退出、退出码还是 0。**
独立 profile 让这两件事一起消失。

### 8.3 为什么清单是文件，不是表

`runtime/manifest.json` 是运行时的唯一真相。如果把它做成数据库里的表，会遇到一个致命场景：

> 这块盘的核心用法是**整盘复制到另一块硬盘**。复制之后，库里的「运行时已安装」状态描述的
> 是**旧盘**。而 manifest 跟着文件走，复制到哪都自洽。

由此得出一条纪律，并落到代码里：**`runtime` 模块不依赖 `db`**（见依赖矩阵）。
探测结果只存在内存（`Mutex<Option<Vec<Status>>>`），可丢弃、可重算、不影响正确性。

### 8.4 清单长什么样

```json
{
  "schema": 1,
  "components": [
    { "id": "python", "kind": "python", "version": "3.15.0",
      "exe": "python/python.exe", "version_args": ["--version"],
      "env": { "PYTHONNOUSERSITE": "1", "PYTHONUTF8": "1" } },
    { "id": "node", "kind": "node", "version": "22",
      "exe": "node/node.exe", "version_args": ["--version"] },
    { "id": "libreoffice", "kind": "office", "version": "24.8",
      "exe": "office/program/soffice.exe", "version_args": ["--version"] },
    { "id": "pdf", "kind": "pdf", "version": "3.6.1",
      "exe": "pdf/SumatraPDF.exe", "version_args": [] }
  ]
}
```

校验规则（全部在 `manifest::validate`，失败即拒绝启动该清单）：

- `schema` 必须等于本版本认识的值 —— 不认识就报错，**不猜**
- `id` 只允许 `[a-z0-9._-]`，因为它同时是 `.library/state/<id>` 的目录名
- `exe` 走与前端路径同一套 `sanitize_rel`：拒绝 `..`、绝对路径、盘符、ADS
- `id` 不许重复
- **单例种类（python / node / office / pdf）各只允许一个**，见 8.5

`pdf` 的 `version_args` 故意留空：SumatraPDF 没有 `--version`，能拿到输出的
`-list-printers` 有副作用，不值得为了界面上多一行版本号去跑它。
空数组的语义是「文件在位即视为可用」（见 `probe` 的分支），界面显示清单里声明的版本。

一条刻意的取舍：`kind` 用 enum 而不是字符串。新增种类会让旧版本解析失败 ——
这是故意的，宁可启动报「清单有误」，也不要静默忽略一个自己看不懂的组件。

### 8.5 单一运行时版本：一条被写进代码的产品决定

本盘的策略是**每种运行时只装一个版本**（当前 Python 3.15、Node 22）。这不是懒，是
因为「多个版本」会立刻产生一个架构问题：

> 盘里同时存在 Python 3.13 和 3.15，某个项目该用哪个？

两个答案都不好：让用户逐个配置（多一套配置要维护、要备份、会漂移），
或者「取第一个」（等于随机）。后者的表现是**「昨天能跑，今天不能跑」** ——
这类故障的排查成本远高于省下的那点磁盘空间。

所以这条决定落成了代码，而不是注释：

- `Kind::singleton()` 声明哪几类只允许一个（`tool` 不在此列，工具可以并存）
- `Manifest::validate()` 在启动时校验；写两个 Python → **拒绝启动并报错**
- `Manifest::get(kind)` 返回 `Option<&Component>` 而不是 `Vec`，
  调用方在类型上就没有「挑一个」的余地

**由此还带出一条：不建 venv。** `venv` 会在 `pyvenv.cfg` 和 `Scripts/*.exe` 里
写死解释器的绝对路径，换盘符即失效 —— 与「便携」正面冲突。本盘的做法是
第三方库直接装进 `runtime/python/Lib/site-packages`，随盘整体移动。
代价是「所有项目共享一套依赖」，收益是**零配置、零漂移**。
P1 的 Python 选型也因此排除了官方 `python-3.15-embed-amd64.zip`：
embed 版禁用 site-packages、不带 pip、也没有 venv，装不了任何第三方库。

### 8.6 PDF 阅读器：为什么值得单独一个 `kind`

第一反应是把它当 `tool`（工具可以有多个），或者干脆塞进 `office`。两个都不对：

- 当 `tool` 就没有单例约束，界面上「用盘内阅读器打开」这句话立刻变得含糊；
- 塞进 `office` 更糟：`office` 的语义是「需要排版引擎重算版面的流式文档」，
  而 PDF 是**已经排好的页面**。用 LibreOffice 开 PDF 能开，但那等于拿排版引擎
  当图片查看器使唤 —— 慢，而且会抢掉文本选择与链接。

所以 `Kind::Pdf` 是一个独立的、单例的种类。选型只看三条硬指标：

1. **单文件、零安装、零运行时依赖** —— 解压即用，不需要 VC++ 运行库、不写注册表、
   不需要管理员权限。这是「整盘拷走还能用」的前提。
2. **配置目录可重定向** —— 见下面的参数对照表。
3. **体积与启动** —— 约 20 MB（内置 MuPDF 渲染引擎），冷启动亚秒级。

最终选 SumatraPDF（AGPLv3，来源、版本、两个 SHA-256 都记在
`runtime/pdf/NOTICE.md`，许可证全文在同目录）。协议上要注意：分发二进制需附许可证
并提供对应源码位置，NOTICE 里两者都写了；我们没有修改它。

**参数对照表**（两个组件都用命令行开关，但长得完全不一样）：

| | LibreOffice | SumatraPDF |
|---|---|---|
| 开关 | `-env:UserInstallation=file:///…` | `-appdata <目录>` |
| 取值形态 | **必须是 `file://` URL** | **必须是本地路径**，写成 URL 就失效 |
| 管到的东西 | profile 目录（配置 + 锁文件） | `SumatraPDF-settings.txt` + 缩略图缓存 |

第二行是真正会咬人的地方：把 `-appdata` 按 LibreOffice 的习惯填成 `file:///F:/…`，
SumatraPDF 会**静默**当作不存在的目录、回落到 `%APPDATA%\SumatraPDF`，
界面上没有任何报错 —— 只有某天在宿主 C 盘看到那个 settings 文件才会发现。
`runtime::pdf` 的单测因此专门断言「`-appdata` 的值不以 `file:` 开头」。

**另一个决定：不传 `-reuse-instance`。** 复用已有实例（DDE）会把文件交给
「先到的那一个」进程，而那个进程未必是本盘启动的（用户可能自己装过 SumatraPDF），
于是文件被开到宿主实例里、用宿主的配置，本盘的 `-appdata` 形同虚设。
隔离比「少开一个窗口」重要，所以每次都是新进程。

链路很短，没有中间产物：`open_target(mode="edit")` → `dispatch` 命中 `PdfOpener`
→ `spawn_detached`。Office 转出来的 PDF 也走同一条路（`open_cached` → 同一个 registry），
所以「转 PDF 之后用什么看」不需要第二套逻辑。

### 8.7 宿主 WebView2：为什么依赖，而不是捆绑

界面本身就是 WebView2 实例。这个运行时有三条路：

| 方案 | 代价 | 结论 |
|---|---|---|
| **A 依赖宿主 Evergreen** | 0 MB；缺了就打不开 | **采用** |
| B 盘内 Fixed Version | +120~180 MB；需自己跟安全更新 | 不采用 |
| C 启动前检测 + 离线安装包 | 需要提权、需要维护一份安装器 | 不采用 |

决定性理由：这块盘是**便携资产**，不是发行渠道。B 会让盘多背一个浏览器内核，
而且内核版本从此与系统脱钩 —— 用户半年后插上这块盘，拿到的是一个不再修补的旧内核。
C 在最典型的使用场景（离线）根本不成立。

A 的唯一代价是「宿主没装怎么办」，而这个代价必须付在**失败路径**上，不能装作不存在：
什么都不做的结果是双击 exe、窗口不出现、没有任何提示 —— 这是最难自查的一种失败。
所以启动最前面查一次注册表（`webview2` 模块），缺了就弹一句人话再退出。
这不是 C：不下载、不安装、不提权。检测要查**四个位置**
（HKLM/HKCU × 原生/WOW6432 视图），只查 HKLM 原生视图正是「明明装了却说没装」的经典错法；
`pv` 为 `0.0.0.0` 也要当成没装。

同时把 **WebView2 的用户数据目录钉在盘内**。默认位置是宿主
`%LOCALAPPDATA%\<identifier>\EBWebView`，缓存、GPU 着色器、IndexedDB 都在里面，
随手几十 MB —— 对「插上就用、拔了不留痕」的盘，这就是往 C 盘倒垃圾。
做法是把窗口从 `tauri.conf.json` 挪进代码（`create_main_window`），
用 `WebviewWindowBuilder::data_directory()` 指向 `.library/state/webview2`。
代价是这块目录会长到几十 MB，但它落在系统区：不进备份、随时可删、删了自动重建。

补一句边界：`tauri.conf.json` 的 `bundle` 段保留 Tauri 默认的 WebView2 引导策略。
它只在「打成 NSIS 安装包」这条路上起作用（安装时按需装系统运行时），
与「盘内不带内核」并不矛盾 —— 盘内资产永远只有 `runtime/` 下那几样。
本项目发行走的是 `--no-bundle`，那条路径根本不会触发。

### 8.8 编辑器：为什么它是 `editor` 而不是 `tool`

`.md` 与 `.html` 有个共同点：**源文本可读，但排版效果要渲染**。它们既不该只给源码
（Markdown 的表格、HTML 的样式全丢了），也不该只给渲染结果（用户可能就是想改字）。
所以这类格式需要一个「源码 + 渲染」两件套，盘内选的是 Notepad++ + PreviewHTML 插件。

`Kind::Editor` 与 `Kind::Pdf` 单例化的理由一模一样：界面上那句话叫「用盘内编辑器打开」，
它默认了唯一性。归到 `tool` 就逃掉了 `validate()` 的单例检查，盘里放两个编辑器
时「用哪个」就变成了「取清单里第一个」—— 等于随机。

**参数与隔离**（和 SumatraPDF 的机制完全不同，这是本节的要点）：

| | SumatraPDF | Notepad++ |
|---|---|---|
| 配置隔离 | `-appdata <本地路径>` | **没有命令行开关**，靠 `doLocalConf.xml` 标记文件 |
| 实例复用 | 默认复用，故**不传** `-reuse-instance` | 默认复用，故**要传** `-multiInst` |
| 会话 | 无此概念 | 传 `-nosession` |

第一行是硬约束：Notepad++ 8.x **没有** `-settingsDir` 这类开关（在二进制里搜过，
只有 `-multiInst`）。想让它不写 `%APPDATA%\Notepad++`，唯一途径是 exe 同目录放
`doLocalConf.xml`。于是 `runtime/` 的「只读资产」在这里出现一个**由厂商设计决定的例外**：
`config.xml`、`session.xml`、`langs.xml`、`stylers.xml` 会被写在这里。这些生成物
因此进 `.gitignore`，而标记文件本身必须随盘 —— **删了它，程序会静默退回宿主配置目录**，
界面上没有任何提示，这块盘也就不再便携了。

第二行和 PDF 是同一条推理的反面：Notepad++ 的默认行为是「再开一个文件就交给已在跑的
实例」，而且**不区分那个实例是从哪个 exe 启动的**。宿主机自己装过 Notepad++ 且正在跑时，
从盘里启动的进程会把文件转交给宿主那个窗口 —— 用宿主的配置、宿主的插件集，
本盘的 `doLocalConf.xml` 与 PreviewHTML 全部形同虚设，而用户完全看不出发生了什么。
`-multiInst` 强制起新进程，把「一定是本盘这一份」变成结构保证。代价是每点一次多一个
窗口而不是一个标签页；这里选隔离。

**编辑时的预览，为什么必须由用户点一下**：这是 Notepad++ 那条路的能力，不是本程序预览的前提。
本程序自己就能渲染（见 5.4），PreviewHTML 的价值在于「边改边看」。而它是**进程内插件**，
Notepad++ 没有「启动时执行某个插件命令」的机制；硬凑（自己发窗口消息模拟点击）会把
一个稳定的启动路径绑死在某个 UI 布局上，插件一改版就断。所以系统在这一侧的职责止于
「把文件正确地在盘内那份编辑器里打开」，渲染那一步走 `插件 → Preview HTML`
（可在 `设置 → 快捷键 → 插件命令` 里绑定键位）。

一个可验证的副作用：插件首次加载会在 `plugins/Config/PreviewHTML/` 下生成
`Settings.ini` 与 `WebView2Cache/`。看到这两个东西，就说明插件确实被加载了 ——
比截图更硬的证据。


---

## 九、扩展点

每个后续需求对应一个明确的落点，不需要重构：

| 需求 | 落点 | 改动范围 |
|---|---|---|
| 再加一个便携组件 | `runtime/manifest.json` 加一条 | **零代码** |
| 新的组件类型 | `runtime::manifest::Kind` 加变体 + 对应用法 | 1 个 enum + 1 个分支 |
| 换成别的 PDF 阅读器 | `runtime::pdf::open_args` + 清单 `exe` | 一个函数（参数形态必须重写，别照抄） |
| LibreOffice 精确预览 | `dispatch` 的 `Preview` 分支先查外部打开器 | 改一个 match 分支 |
| 项目级启动命令（`npm run dev` / `pytest`） | 加 v2 迁移建 `project_profile` | 加迁移 + 新命令 |
| 标签 / 元数据 | 加 v3 迁移建 `meta` | 加迁移 + `repo` 查询 |
| 全文检索 | 加 v4 迁移建 `fts_content`（trigram） | 加迁移 + 检索命令 |
| docx/xlsx 前端渲染 | `views/preview.ts` 加格式分支 | 纯前端 |
| 多内容根（不只看 `content/`） | `RootCtx` 持 `Vec<PathBuf>` + `volume_id` 区分 | 波及 `resolve` / 索引 / 打开 |

迁移机制已经在 `db::schema::migrate` 里跑通：递增 `SCHEMA_VERSION`，加一个 `if current < N` 分支，
**不删库、不重扫**。

注意「再加一个便携组件」那一行是**零代码**的 —— 这是分层做对的直接证据：
组件是数据（清单），不是代码。如果加个组件要改 Rust，说明抽象漏了。

---

## 十、裁剪与已知欠账

诚实列出来，免得后面误以为已经做了：

| 项 | 状态 | 说明 |
|---|---|---|
| pdf.js 内置预览 | **未做** | `asset` 协议已启用（HTML / Markdown 渲染在用），接 pdf.js 只差一个前端分支；当前 PDF 仍走**盘内阅读器**（SumatraPDF）外开，不依赖宿主装没装阅读器 |
| 图片预览 | **未做** | 同上。图片现在由 md / html 渲染间接显示（相对路径已能加载），单独点开一张图仍交给外部程序 |
| PreviewHTML 缓存 | 已知 | `runtime/Notepad++/plugins/Config/PreviewHTML/WebView2Cache/` 只增不减。它落在「只读资产区」，清理需要显式动作；量级远小于 WebView2 主数据目录 |
| 索引内存峰值 | 已知上限 | **全量**对账仍把 `rel_path → (id, hash)` 全载入内存，约 300k 节点 30 MB 量级。日常改文件只走增量，不再付这份代价；但全量仍是首次建索引与兜底手段 |
| WebView2 用户数据目录 | **已外移** | `data_directory()` → `.library/state/webview2`。窗口因此改由代码创建，见 8.7 |
| PDF 批注 / 表单填写 | 未做 | 阅读器是「只读展示」定位。批注要写回用户文件，属编辑语义，当前不给入口 —— 需要时应当走「显式编辑模式」，而不是让双击顺手改文件 |
| 长路径 (>260) | 未处理 | 需要 manifest 声明 `longPathAware` |
| 组件安装 | **不做界面** | 下载解压是装配期动作；做成按钮会诱导用户在一块盘上调三个版本 |
| 项目运行配置 | 未做 | 当前只给「开终端」，不解析 `npm run dev` 这类脚本 |
| 转换结果清理 | 未做 | `.library/cache/pdf/` 只增不减，需一套 LRU 或按时间清理 |
| exFAT 上的 ACL | **待实测** | WebView2 数据目录与 LibreOffice profile 在非 NTFS 上的行为未验证 |
| 多版本共存 | **已明确不支持** | 单例校验，写两个 Python 直接拒绝启动。理由见 8.5 |
| 监听资源占用 | 已知 | `ReadDirectoryChangesW` 占一个句柄 + 一个线程；插盘期间常驻，拔出即随进程退出 |
| 通知风暴 | 已缓解 | 静默期去抖 + 前端 refresh 合并（同一时刻只跑一次，多来的合并成一次补刷） |

---

## 十一、验证状态

| 侧 | 状态 | 说明 |
|---|---|---|
| Rust 编译 | ✅ 通过 | `cargo check` / `cargo clippy` 零错误零警告；`npm run tauri build --no-bundle` 产出带内嵌前端的 exe |
| Rust 测试 | ✅ 通过 | **46 个**测试全绿（单测 + 监听 + 结构 + 2 个端到端链路 + 2 个打开器路由 + 渲染基准目录） |
| Clippy | ✅ 通过 | 零警告 |
| 格式 | ✅ 通过 | `cargo fmt --check` 无差异 |
| TypeScript | ✅ 通过 | `tsc --noEmit` 零错误（`strict` + `noUncheckedIndexedAccess`） |
| 前端构建 | ✅ 通过 | `vite build` 产出 130.6 kB JS + 10.7 kB CSS（35 个模块）。JS 变大的那 100 kB 是 markdown-it —— 手写渲染器撑不起表格 / 脚注这些真文档里常见的东西 |
| 配置 JSON | ✅ 通过 | `tauri.conf.json` / `capabilities/default.json` / `runtime/manifest.json` |
| 图标资源 | ✅ 通过 | `icons/icon.ico`，16/32/48/256 四尺寸，头部已校验 |
| 启动冒烟 | ✅ 通过 | 带 `PL_ROOT` 启动，正确创建 `content/`、`runtime/`、`.library/catalog.db`，常驻不崩 |
| 真实窗口 | ✅ 通过 | 发行版 exe 启动后 `MainWindowHandle` 非零、标题「便携资料库」，`.library/state/webview2` 自动生成 —— 证明窗口确实建起来了，且 WebView2 数据目录已外移（8.7） |
| PDF 链路 | ✅ 通过 | 用盘内 SumatraPDF 打开测试 PDF：日志显示 `LoadSettings('…\.library\state\pdf\SumatraPDF-settings.txt')`、1 页渲染完成；进程存活到被主动结束；宿主 `%APPDATA%\SumatraPDF` 自始至终不存在 —— `-appdata` 隔离成立 |
| 布局核验 | ✅ 通过 | 用真实构建产物的 CSS + 等结构 DOM，经系统 Edge 无头渲染截图，逐项核对三栏宽度、面包屑/统计/分支/文件列表的排列、规模条比例、右栏页签、状态栏无裁切 |
| 监听实时性 | ✅ 通过 | 见下方「监听冒烟」：应用运行中新建/改写/删除/重命名，索引在 0.5~0.8 s 内自动跟上 |
| 内置渲染 | ✅ 通过 | CDP 截图：md 的表格 / 代码块 / 引用 / 图片、html 的外部 CSS 与图片都按预期呈现；iframe `securityOrigin` 为不透明源，沙箱里的脚本确实没执行（探针文本停在初始值） |

**真实窗口的截图证据**（本轮补上了）：自动截窗被本机安全策略挡住（`Add-Type` 被拒），
headless 浏览器又不执行外部 module 脚本。最后走的是 **WebView2 的远程调试口**：

```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9333" \
PL_ROOT=F:/Work_space/portable-library ./portable-library.exe
node scripts/cdp-smoke.mjs /tmp/smoke      # 点目录 → 点文件 → Page.captureScreenshot
```

这条路的额外收益是能顺手把 iframe 的隔离**量出来**而不是相信它：`Page.getFrameTree`
里子 frame 的 `securityOrigin` 是 `://`（不透明源），父页面 `contentDocument === null`。
而 iframe 内部的 DOM 只能通过 `Page.createIsolatedWorld(frameId)` 才查得到。

它当场抓到过一个真 bug：`preview_base` 一开始用 `resolve()`，Windows 上返回
`\\?\F:\…`，拼进 `<base href>` 后样式与图片**静默 404** —— 截图里就是「没有样式的裸 HTML」。
现在有用例守着（`paths::tests::dir_of_has_no_verbatim_prefix`）。

结论：**布局与内置渲染都已按真实窗口截图核对；双击打开、切换模式这类会起进程/改状态的
交互仍靠人工过一眼**。这类「工具测一半」的项，如实标出来比假装通过更有用。


编译验证过程本身值得记一笔：第一次 `cargo check` 只报了 **2 个错误，都在同一处** ——
`probe.rs` 里先把 `out.stdout` 移动进 `text`，后面失败分支又要用它拼错误信息
（`E0382`）。其余 26 个文件一次通过，说明前面那轮「能用函数就不用带生命周期的闭包」
的收紧是有效的：`query_map` 的 HRTB 确实是最容易翻车的地方。

单元测试的打击面：

| 文件 | 覆盖的静默错误 |
|---|---|
| `paths.rs` | `..` 穿越、盘符注入、ADS 路径、中文路径、面包屑逐级展开 |
| `scan/walker.rs` | 扩展名边界 |
| `watcher.rs` | 相对路径归一、根自身事件被丢弃、盘外路径被丢弃、噪声目录任意一段命中即丢（`node_modules.md` 不能误杀） |
| `index/reconcile.rs` | 日期换算（跨年 / 闰年 / 世纪边界） |
| `runtime/manifest.rs` | 越界 exe、重复 id、未知 schema、bin 目录去重、**双 Python 被拒**、**双 PDF 阅读器被拒** |
| `runtime/env.rs` | PATH 前置顺序、空宿主 PATH |
| `runtime/office.rs` | 盘符路径转 file URL、空格编码、反斜杠归一、哈希稳定 |
| `runtime/pdf.rs` | `-appdata` 的值**不是** `file:` URL（写成 URL 会静默失效）、文件路径必须排在参数最后、扩展名集合不漏 |
| `open/registry.rs` | PDF 在位时路由到 `portable-pdf`、文件缺失时回落到系统默认（不注册「指向空气」的打开器） |
| `runtime/probe.rs` | 版本行截断按字符而非字节（中文不会截出半个字） |
| `project/mod.rs` | 噪声目录按段匹配（`node-modules.md` 不能误杀） |

这些测试有个共同点：**打的全是「错了也不会报错，只是结果不对」的地方**。
`to_file_url` 少一个斜杠、`is_noise` 误判一次，表现出来的都只是「转换失败」或
「项目列表多几百条」，没有异常堆栈可看。

### 监听冒烟（`scripts/watcher-smoke.py`）

起真进程、改真磁盘、查真索引 —— 这一段编译与单测都覆盖不到：

```
启动应用（PL_ROOT 指向临时盘根）→ 等首扫完成
  ① 新建文件            → 0.51 s 进索引
  ② 改写已索引文件       → 0.76 s 后 size 由 7 变 24
  ③ 一次建出两层新目录   → 0.76 s 后最深层文件进索引
  ④ 删除文件            → 0.51 s 后节点消失
  ⑤ 删除整棵子树        → 0.51 s 后 3 个节点（含 2 个后代）全部消失
  ⑥ 目录重命名          → 0.51 s 后旧路径清空、新路径与子项就位
```

六个场景全部在 1 s 内收敛，说明「400 ms 静默期 + 一次对账」的节奏是对的。

**这次冒烟顺带抓到一个比监听更严重的问题**：一开始它连「首扫」都等不到，索引里一条记录都没有。
查下来是页面 URL 停在 `http://localhost:1420` —— 也就是说那个「release 二进制」
根本没有内嵌前端，还在等开发服务器。

根因是 `cargo build --release` **不带 `custom-protocol` feature**，编译期就不会把 `dist/`
打进去，而构建过程一声不响。表现是「双击 exe 得到一个空白窗口，标题栏还有字」。
已在 `Cargo.toml` 里显式声明该 feature 并注明：**发行只能走 `npm run tauri build`**。

值得记一笔：这个问题从 P1 就存在，前面几轮「release 启动冒烟通过」其实只验证了进程不崩，
界面从来没有真正加载过 —— 这也解释了当时为什么只能靠静态 HTML 去核布局。

### 端到端链路测试（`src/e2e.rs`）

单独说，因为它验证的是**模块拼起来还对不对**，而不是某个函数算得对不对：

```
建临时盘根 → 造 4 文件 5 目录 → 首扫全新增 → 懒加载逐层取 → 统计
          → 复扫（应全 unchanged，即零写入）→ 改造/删/增三件事同时发生
          → 对账收敛 → 越界路径被拒 → 正常路径可还原
```

它抓到了两个真问题，都不是「代码错」，而是「认知错」：

1. **`updated` 会比直觉多。** 目录节点的指纹同样含 `mtime`，而目录的 mtime 会在
   **子项增删**时改变。于是「新增一个文件」会让父目录也算一次更新。
   这是真实变化，不是误判 —— 反过来，改写文件**内容**不影响父目录 mtime，
   所以父目录仍留在 `unchanged` 里。这一正一反同时成立，才证明判据是准的。

2. **我自己写的断言是错的。** 原先断言
   `added + updated + removed + unchanged == scanned`，
   但 `removed` 是「本次没扫到」的旧节点，**本来就不在 `scanned` 里**。
   正确的划分是 `added + updated + unchanged == scanned`。

这两条都写回测试注释了。**测试最大的价值不是证明代码对，而是暴露自己哪儿想错了。**

---

## 十二、开发期怎么跑

开发时 `current_exe()` 落在 `target/debug/`，与发行版「exe 就在盘根」的布局不同。
**不要在代码里写「向上找两级」的猜测逻辑**——猜测一出现，发行版的确定性就没了。
正确做法是显式指定一个仿真盘根：

```bash
mkdir -p /d/pl-dev-root/content/sample
$EDITOR /d/pl-dev-root/content/sample/readme.md

# 把示例清单放进去（没有真组件也能启动，界面会显示「未就位」）
mkdir -p /d/pl-dev-root/runtime
cp runtime/manifest.json /d/pl-dev-root/runtime/

# PowerShell
$env:PL_ROOT = "D:\pl-dev-root"
npm run tauri dev
```

真组件按清单里的相对路径解压即可，例如：

```
D:\pl-dev-root\runtime\
├── manifest.json
├── python\        ← python-build-standalone 的 install_only_stripped 解压内容
├── node\          ← node-v22-win-x64.zip 解压内容
└── office\        ← LibreOffice 便携版解压内容
```

**不需要改任何代码，也不需要跑步进。** 点界面上的「重新探测」即可看到状态变化。

发行版则不需要 `PL_ROOT` 变量：`Launch.exe` 放在盘根，`content/` 与它同级，`detect()` 自然落在正确位置。

### 构建：两种模式，别弄混

```
npm run tauri dev                     # 开发：连 http://localhost:1420，改前端即热更新
npm run tauri build -- --no-bundle    # 发行：dist/ 在编译期嵌入 exe
```

判断手里的 exe 是哪一种：**不开 vite 直接双击它，能出界面就是发行版**。
`cargo build --release` 属于前者（会被编译成连 devUrl 的模式），
所以它产出的 exe 换一台机器就是白窗口 —— 这个坑吃过一次，已在 `Cargo.toml` 注明。
