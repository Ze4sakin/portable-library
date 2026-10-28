//! 便携资料库 · P3 骨架
//!
//! 模块依赖方向（严格单向，不允许反向引用）：
//!
//! ```text
//! commands ──► index ──► db ──► schema
//!    │           │
//!    │           └──► scan ──► paths
//!    ├──► project ───────────► db
//!    ├──► open ──────────────► runtime ──► paths
//!    ├──► runtime ───────────► paths
//!    ├──► watcher ───────────► scan
//!    └──► state ─────────────► paths / open / runtime / watcher
//!
//! webview2 ──► （无依赖）  宿主运行时检测，只在启动最前面用一次
//! ```
//!
//! 五条不可违反的规矩：
//! 1. 只有 `paths` 模块能把相对路径变成绝对路径
//! 2. 只有 `db` 模块能出现 SQL
//! 3. `scan` 不写库，`db` 不读文件系统
//! 4. `runtime` 不写库、不改宿主环境（只作用于子进程）
//! 5. `watcher` **不认识 Tauri**：它只吐脏路径，怎么处理由上层决定

mod commands;
mod db;
mod error;
mod index;
mod open;
mod paths;
mod project;
mod runtime;
mod scan;
mod state;
mod watcher;
mod webview2;

/// 端到端链路测试（扫描 → 落库 → 对账 → 查询）。仅测试构建存在。
#[cfg(test)]
mod e2e;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::db::repo;
use crate::open::OpenerRegistry;
use crate::paths::RootCtx;
use crate::runtime::manifest;
use crate::scan::WalkOptions;
use crate::state::{AppState, WatchState};

pub const EVENT_LIBRARY_CHANGED: &str = "library:changed";
pub const EVENT_LIBRARY_ERROR: &str = "library:error";

pub fn run() {
    // 界面就是 WebView2，缺了它会得到一个「双击没反应」的进程。
    // 所以在建任何东西之前先问一句，缺了就弹窗说清楚再退（见 webview2 模块）。
    if let Err(msg) = webview2::ensure() {
        eprintln!("[webview2] {msg}");
        std::process::exit(1);
    }

    tauri::Builder::default()
        .setup(|app| {
            // 启动期只做四件轻量的事：定位盘根、开库、读清单、装打开器。
            // 全盘扫描与组件探测都不在这里做 —— 前者会把启动卡死，
            // 后者会 spawn 若干进程拖慢首屏；两者都交给前端显式触发。
            let root = RootCtx::detect()?;
            let conn = db::schema::open_and_migrate(&root.db_path())?;

            // 清单读坏了不是致命错误：当作「没有清单」继续启动，
            // 但把原因留在界面上 —— 静默吞掉才是真坑。
            let (manifest, manifest_error) = match manifest::load(&root.runtime_dir()) {
                Ok(Some(m)) => (Some(m), None),
                Ok(None) => (None, None),
                Err(e) => (None, Some(e.to_string())),
            };

            let registry = OpenerRegistry::for_root(&root, manifest.as_ref());

            // 内置 HTML 渲染要读同目录的相对资源（css、图片、字体）。把 HTML 文本
            // 塞进 `srcdoc` 是不够的 —— 相对路径会解析到 `tauri.localhost/...`，
            // 一律 404，用户看到的是「没有样式的裸 HTML」。所以放行 asset 协议。
            //
            // 作用域**必须在这里给，不能写进 tauri.conf.json**：盘符会漂，配置是
            // 编译期常量。只放行 `content/`（用户内容区），不放到盘根 ——
            // `.library/` 里有数据库，没有任何理由让 iframe 读得到。
            app.asset_protocol_scope()
                .allow_directory(root.content(), true)?;

            app.manage(AppState {
                root,
                db: Mutex::new(conn),
                registry,
                scanning: AtomicBool::new(false),
                manifest,
                manifest_error,
                runtime_probe: Mutex::new(None),
                watch: Mutex::new(WatchState {
                    handle: None,
                    error: None,
                }),
            });

            // 必须放在 manage 之后：监听回调里要取 AppState
            start_watcher(app.handle().clone());

            // 窗口也自己建，见 create_main_window 的说明（WebView2 数据目录外移）
            create_main_window(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::library::lib_init,
            commands::library::scan_full,
            commands::library::list_children,
            commands::library::dir_overview,
            commands::library::library_stats,
            commands::library::watch_info,
            commands::open::open_target,
            commands::open::open_cached,
            commands::open::read_text,
            commands::open::preview_base,
            commands::runtime::runtime_status,
            commands::runtime::runtime_probe,
            commands::runtime::doc_convert,
            commands::project::project_list,
            commands::project::project_terminal,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

/// 主窗口。
///
/// 窗口没有写在 `tauri.conf.json` 里，而是在这里建，只为一件事：
/// **把 WebView2 的用户数据目录钉在盘内**。
///
/// 默认位置是宿主 `%LOCALAPPDATA%\<identifier>\EBWebView`，缓存、GPU 着色器、
/// IndexedDB 都在里面，随手几十 MB。对普通桌面应用这没问题；对「插上就用、
/// 拔了不留痕」的便携盘，它就是往宿主 C 盘倒垃圾 —— 换一块盘、换一台机器，
/// 这些数据既不跟走，也没人去删。
///
/// 代价是 `.library/state/webview2` 会长到几十 MB。它落在系统区：
/// 不进备份、随时可删、删了自动重建，正是这类临时数据该待的地方。
fn create_main_window(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = app.state::<AppState>().root.state_component_dir("webview2");

    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("便携资料库")
        .inner_size(1280.0, 820.0)
        .min_inner_size(900.0, 600.0);

    // data_directory 只有 Windows 后端提供；其他平台保持默认。
    #[cfg(windows)]
    let builder = builder.data_directory(data_dir);
    #[cfg(not(windows))]
    let _ = data_dir;

    builder.build()?;
    Ok(())
}

/// 启动内容区监听，把「脏路径」接到增量对账上。
///
/// 三处刻意的取舍：
/// - **失败不致命**：监听起不来只是失去实时性，索引与打开功能都照常，
///   所以这里吞掉错误、留在状态里给界面看。
/// - **不判断 scanning 标志**：全量扫描期间事件继续排队，回调会阻塞在数据库锁上。
///   排队不丢事件，比「扫描期间丢弃事件」安全得多。
/// - **无变化不发事件**：一次保存动辄引出十几条事件，若每次都通知前端，
///   界面会自己刷新个不停。只有真改动了索引才广播。
fn start_watcher(app: tauri::AppHandle) {
    let content = app.state::<AppState>().root.content().to_path_buf();
    let handle_for_sink = app.clone();

    let result = watcher::start(
        content,
        WalkOptions::default(),
        move |dirty: Vec<String>| {
            let st = handle_for_sink.state::<AppState>();

            let Ok(mut conn) = st.db.lock() else {
                return;
            };
            let Ok(volume_id) = repo::find_volume_id(&conn, st.root.volume_serial()) else {
                // 索引还没建立。这批变更会被首次全量扫描覆盖，不必补救。
                return;
            };

            match index::reconcile_dirty(
                &mut conn,
                volume_id,
                st.root.content(),
                &dirty,
                &WalkOptions::default(),
            ) {
                Ok(report) => {
                    if report.added + report.updated + report.removed > 0 {
                        let _ = handle_for_sink.emit(EVENT_LIBRARY_CHANGED, &report);
                    }
                }
                Err(e) => {
                    let _ = handle_for_sink.emit(EVENT_LIBRARY_ERROR, e.to_string());
                }
            }
        },
    );

    // 先把 State 绑定到变量：`app.state::<AppState>()` 是临时值，
    // 直接在后面 `.watch.lock()` 会让它提前析构（E0716）。
    let st = app.state::<AppState>();
    let mut watch = match st.watch.lock() {
        Ok(w) => w,
        Err(_) => return,
    };

    match result {
        Ok(h) => watch.handle = Some(h),
        Err(e) => {
            // 打印一份到 stderr：界面没起来时也能在终端看到原因
            eprintln!("[watcher] 启动失败：{e}");
            watch.error = Some(e.to_string());
        }
    }
}
