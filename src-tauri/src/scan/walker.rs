//! 文件系统扫描：只负责把磁盘走一遍，产出 `FsEntry` 流。
//!
//! 契约（重要，下游依赖它）：
//! **迭代式 DFS，父节点一定先于其子节点被产出。**
//! 有了这条契约，写索引时可以直接用「已知父 id」建边，不必再做一轮
//! 全表回填 parent_id。
//!
//! 本模块不碰数据库、不认识 SQLite —— 它是纯生产者。

use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::paths::to_slash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Link,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryKind::Dir => "dir",
            EntryKind::File => "file",
            EntryKind::Link => "link",
        }
    }
}

/// 一条扫描结果。注意没有任何绝对路径字段。
#[derive(Debug, Clone)]
pub struct FsEntry {
    pub rel_path: String,
    pub parent_rel: Option<String>,
    pub name: String,
    pub ext: Option<String>,
    pub kind: EntryKind,
    pub size: i64,
    pub mtime: i64,
    /// 变化指纹：size + mtime。够快，且对「编辑保存」这类操作足够敏感。
    /// 需要强一致校验时才升级到 content_hash（P3）。
    pub quick_hash: String,
}

#[derive(Debug, Clone)]
pub struct WalkOptions {
    pub max_depth: usize,
    pub skip_dir_names: Vec<String>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            // 256 字符路径上限对应的实际深度远小于此，纯粹是防爆栈的保险丝
            max_depth: 64,
            skip_dir_names: vec![
                ".git".into(),
                "node_modules".into(),
                "target".into(),
                "__pycache__".into(),
                "$RECYCLE.BIN".into(),
                "System Volume Information".into(),
            ],
        }
    }
}

impl WalkOptions {
    pub fn should_skip(&self, name: &str) -> bool {
        self.skip_dir_names
            .iter()
            .any(|s| s.eq_ignore_ascii_case(name))
    }

    /// 判断一条**相对路径**的任意一段是否属于噪声目录。
    ///
    /// 监听回调拿到的是绝对路径，需要先转成相对路径再来问这里：
    /// 只判断「最后一段」是不够的 —— `node_modules/x/y.js` 的末段是 `y.js`，
    /// 但它整个都在噪声目录里。
    pub fn is_ignored_rel(&self, rel: &str) -> bool {
        rel.split('/').any(|seg| self.should_skip(seg))
    }
}

#[derive(Debug, Default)]
pub struct WalkResult {
    pub entries: Vec<FsEntry>,
    /// 因权限不足 / 句柄异常被跳过的目录数。不算错误，但要让用户知道。
    pub skipped: usize,
}

pub fn walk_full(root: &Path, opts: &WalkOptions) -> AppResult<WalkResult> {
    walk_from(root, root, opts)
}

/// 从 `start`（必须是 `root` 之内的目录）开始走。
///
/// 增量对账用它：某个目录变脏时，只走那一棵子树，而不是重扫整盘。
/// `rel_path` 仍然相对 `root` 计算 —— 这一点不能变，否则增量写进库里的
/// 路径与全量扫描产出的路径对不上，会出现「同一个文件两条记录」。
pub fn walk_from(root: &Path, start: &Path, opts: &WalkOptions) -> AppResult<WalkResult> {
    let mut entries: Vec<FsEntry> = Vec::new();
    let mut skipped = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(start.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                // 一个目录读不了不该中断整盘扫描（常见于系统保护目录）
                skipped += 1;
                continue;
            }
        };

        let mut batch: Vec<(PathBuf, std::fs::FileType)> = Vec::new();
        for e in rd.flatten() {
            match e.file_type() {
                Ok(ft) => batch.push((e.path(), ft)),
                Err(_) => skipped += 1,
            }
        }
        batch.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));

        let mut child_dirs: Vec<PathBuf> = Vec::new();

        for (path, ft) in batch {
            let name = match path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };
            if name.is_empty() {
                continue;
            }

            let kind = if ft.is_symlink() {
                EntryKind::Link
            } else if ft.is_dir() {
                EntryKind::Dir
            } else {
                EntryKind::File
            };

            // 用 symlink_metadata 避免跟随链接把盘外文件也算进来
            let meta = std::fs::symlink_metadata(&path).ok();
            if let Some(entry) = build_entry(root, &path, &name, kind, meta.as_ref()) {
                entries.push(entry);
            }

            // 不跟随符号链接：只登记，不展开，避免盘外环状引用
            if kind == EntryKind::Dir && depth < opts.max_depth && !opts.should_skip(&name) {
                child_dirs.push(path);
            }
        }

        for d in child_dirs.into_iter().rev() {
            stack.push((d, depth + 1));
        }
    }

    Ok(WalkResult { entries, skipped })
}

/// 单路径探测结果。
///
/// 三态而不是 `Option`：增量对账必须能区分「确实不见了」和「读不到」。
/// 把「读不到」当成「不见了」，会在权限抖动时**静默删掉索引里的节点**，
/// 而这种错误没有任何日志可查。
pub enum Probe {
    Present(FsEntry),
    Absent,
    Inaccessible,
}

/// 探测一条相对路径的当前状态。增量对账的唯一入参形态。
pub fn probe(root: &Path, rel: &str) -> Probe {
    let Ok(rel_parts) = crate::paths::sanitize_rel(rel) else {
        return Probe::Inaccessible;
    };
    let abs = root.join(rel_parts);

    let meta = match std::fs::symlink_metadata(&abs) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Probe::Absent,
        Err(_) => return Probe::Inaccessible,
    };

    let ft = meta.file_type();
    let kind = if ft.is_symlink() {
        EntryKind::Link
    } else if ft.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File
    };

    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    match build_entry(root, &abs, &name, kind, Some(&meta)) {
        Some(e) => Probe::Present(e),
        None => Probe::Inaccessible,
    }
}

/// 由「已知的名字 + 类型 + 元数据」拼出一条扫描结果。
///
/// 全量走查与增量探测共用它 —— 两条路径必须产出**逐字段一致**的记录，
/// 否则库里的节点会在两种扫描之间来回抖动（表现为「每次扫描都在更新」）。
fn build_entry(
    root: &Path,
    path: &Path,
    name: &str,
    kind: EntryKind,
    meta: Option<&std::fs::Metadata>,
) -> Option<FsEntry> {
    let rel_path = to_slash(path.strip_prefix(root).ok()?);
    if rel_path.is_empty() {
        return None;
    }
    let parent_rel = rel_path.rsplit_once('/').map(|(p, _)| p.to_string());

    let (size, mtime) = match meta {
        Some(m) => (if m.is_file() { m.len() as i64 } else { 0 }, mtime_nanos(m)),
        None => (0, 0),
    };

    let ext = if kind == EntryKind::File {
        extension_of(name)
    } else {
        None
    };

    Some(FsEntry {
        rel_path,
        parent_rel,
        name: name.to_string(),
        ext,
        kind,
        size,
        mtime,
        // 变化指纹：size + mtime。够快，且对「编辑保存」足够敏感。
        quick_hash: format!("{size}-{mtime}"),
    })
}

fn mtime_nanos(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn extension_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() || ext.len() > 16 {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_parsing() {
        assert_eq!(extension_of("a.MD"), Some("md".to_string()));
        assert_eq!(extension_of(".gitignore"), None);
        assert_eq!(extension_of("noext"), None);
        assert_eq!(extension_of("a.tar.gz"), Some("gz".to_string()));
    }
}
