//! 文件监听：把「磁盘上发生了什么」折叠成一批批脏路径。
//!
//! 这一层的定位必须说清楚，否则很容易被误用成「真相来源」：
//!
//! **监听是线索，不是真相。** 平台事件天生不可靠 —— 缓冲区溢出会丢事件、
//! 网络盘与可移动介质的语义各不相同、重命名在不同平台报成不同的组合。
//! 因此本模块只做三件事：折掉抖动、滤掉噪声、吐出路径。
//! 拿到路径后一律重新 stat 磁盘（见 `index::reconcile_dirty`），
//! 这样无论事件本身准不准，最终状态都收敛到磁盘现状。
//!
//! 为什么需要去抖：一次「保存」在 Windows 上通常产生 3~5 个事件
//! （写临时文件 → 改名 → 改属性），逐个处理会让同一棵子树被反复对账。
//! 静默期过后一次性处理，既省事又避免中间态写进索引。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher};

use crate::error::{AppError, AppResult};
use crate::paths::to_slash;
use crate::scan::WalkOptions;

/// 静默多久算「这波改完了」。
const QUIET: Duration = Duration::from_millis(400);

/// 连续写入时也不能无限推迟 —— 拷一个大目录进去可能持续几分钟。
const MAX_HOLD: Duration = Duration::from_secs(2);

/// 轮询间隔。只影响「最迟多久发现静默期已过」，不影响事件捕获。
const TICK: Duration = Duration::from_millis(120);

/// 监听句柄。**必须持有**：drop 掉就等于停止监听（notify 的 watcher 析构即停）。
pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // 不 join：线程最多在一个 TICK 内退出，阻塞在这里只会拖慢应用退出。
        // 但要把句柄丢掉，让 JoinHandle 不泄漏。
        self.thread.take();
    }
}

/// 启动递归监听。
///
/// `sink` 在**独立线程**上被调用，收到一批已归一化为相对路径的脏路径
/// （已去重、已过滤噪声目录）。它可能因为数据库锁而阻塞较久，
/// 这是允许的 —— 阻塞期间事件继续在通道里排队，不会丢。
pub fn start<F>(content: PathBuf, opts: WalkOptions, sink: F) -> AppResult<WatchHandle>
where
    F: Fn(Vec<String>) + Send + 'static,
{
    let (tx, rx) = channel::<PathBuf>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else {
            // 事件本身出错（典型是缓冲区溢出）不做补救：
            // 丢失的变更由「下一次全量扫描」兜底，这里硬猜反而更糟。
            return;
        };

        // 读取文件会触发 Access 事件。不过滤掉的话，光是浏览目录就会
        // 引发无休止的重新对账 —— 这是文件监听最经典的「自己吵自己」。
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }

        for path in event.paths {
            // 通道断开说明线程已退出，忽略即可
            let _ = tx.send(path);
        }
    })
    .map_err(|e| AppError::Watch(format!("无法创建文件监听器：{e}")))?;

    watcher
        .watch(&content, RecursiveMode::Recursive)
        .map_err(|e| AppError::Watch(format!("无法监听内容区：{e}")))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_in_thread = Arc::clone(&stop);
    let base = content.clone();

    let thread = std::thread::Builder::new()
        .name("pl-watch".into())
        .spawn(move || {
            let mut pending: HashSet<String> = HashSet::new();
            let mut deadline: Option<Instant> = None;
            let mut first_seen: Option<Instant> = None;

            loop {
                if stop_in_thread.load(Ordering::SeqCst) {
                    break;
                }

                match rx.recv_timeout(TICK) {
                    Ok(path) => {
                        if let Some(rel) = normalize(&base, &path, &opts) {
                            pending.insert(rel);
                            let now = Instant::now();
                            let first = *first_seen.get_or_insert(now);
                            let quiet_at = now + QUIET;
                            let cap_at = first + MAX_HOLD;
                            deadline = Some(quiet_at.min(cap_at));
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }

                let due = deadline.is_some_and(|d| Instant::now() >= d);
                if due && !pending.is_empty() {
                    let batch: Vec<String> = pending.drain().collect();
                    deadline = None;
                    first_seen = None;
                    sink(batch);
                }
                if pending.is_empty() {
                    deadline = None;
                    first_seen = None;
                }
            }
        })
        .map_err(|e| AppError::Watch(format!("无法启动监听线程：{e}")))?;

    Ok(WatchHandle {
        _watcher: watcher,
        stop,
        thread: Some(thread),
    })
}

/// 绝对路径 → 相对 content 根的 `/` 分隔路径；不在盘内或属于噪声目录则丢弃。
fn normalize(base: &Path, path: &Path, opts: &WalkOptions) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    if rel.as_os_str().is_empty() {
        // content 根本身的事件没有对应节点，丢掉
        return None;
    }
    let rel = to_slash(rel);
    if opts.is_ignored_rel(&rel) {
        return None;
    }
    // 全盘范围之外的东西（`..` 之类）不该出现，出现了也不能进库
    if crate::paths::sanitize_rel(&rel).is_err() {
        return None;
    }
    Some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_base() {
        let base = Path::new("F:/pl/content");
        let opts = WalkOptions::default();
        assert_eq!(
            normalize(base, Path::new("F:/pl/content/a/b.md"), &opts),
            Some("a/b.md".to_string())
        );
    }

    #[test]
    fn normalize_drops_root_and_outside() {
        let base = Path::new("F:/pl/content");
        let opts = WalkOptions::default();
        assert_eq!(normalize(base, Path::new("F:/pl/content"), &opts), None);
        assert_eq!(normalize(base, Path::new("F:/pl/other/x.md"), &opts), None);
    }

    /// 噪声目录必须在**任意一段**命中时丢弃，
    /// 否则 node_modules 里每个文件都会引发一次对账。
    #[test]
    fn normalize_drops_noise() {
        let base = Path::new("F:/pl/content");
        let opts = WalkOptions::default();
        assert_eq!(
            normalize(
                base,
                Path::new("F:/pl/content/p/node_modules/a/b.js"),
                &opts
            ),
            None
        );
        assert_eq!(
            normalize(
                base,
                Path::new("F:/pl/content/p/notes/node_modules.md"),
                &opts
            ),
            Some("p/notes/node_modules.md".to_string())
        );
    }
}
