use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::open::OpenerRegistry;
use crate::paths::RootCtx;
use crate::runtime::manifest::Manifest;
use crate::runtime::probe::Status;
use crate::watcher::WatchHandle;

/// 监听状态。
///
/// 用 `Mutex` 包住而不是直接放字段，是因为 `WatchHandle` 只保证 `Send`、
/// 不保证 `Sync`，而 `AppState` 必须 `Sync` 才能交给 Tauri 托管。
pub struct WatchState {
    /// `None` = 监听没起来。这**不是错误**：索引依然可用，只是要手动刷新。
    pub handle: Option<WatchHandle>,
    /// 起不来的原因。留在界面上，而不是静默降级。
    pub error: Option<String>,
}

/// 全局状态。由 Tauri 托管，命令通过 `State<'_, AppState>` 取得。
///
/// 依赖方向：`AppState` 只依赖 `paths` / `open` / `runtime`，不依赖 `commands`，
/// 避免出现「状态反过来认识命令」的循环依赖。
///
/// 两类状态的区别值得留意：
/// - `db` 是**真相**，掉了要重建
/// - `runtime_probe` 是**快照**，掉了下次探测就有，因此允许 `None`
pub struct AppState {
    pub root: RootCtx,
    /// SQLite 连接不是 Sync 的，用 Mutex 串行化。P1 的并发压力可以忽略。
    pub db: Mutex<Connection>,
    pub registry: OpenerRegistry,
    /// 扫描互斥标志：防止用户连点导致两次全盘扫描叠加
    pub scanning: AtomicBool,

    /// 运行时清单。`None` = 盘里没有清单（合法状态），不是错误。
    pub manifest: Option<Manifest>,
    /// 清单存在但读坏了时，把原因留在界面上，而不是静默当作没有清单
    pub manifest_error: Option<String>,
    /// 探测快照，可丢弃
    pub runtime_probe: Mutex<Option<Vec<Status>>>,

    /// 文件监听。索引的「实时性」全靠它，但它缺席时系统仍然完整可用。
    pub watch: Mutex<WatchState>,
}
