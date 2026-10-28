//! 索引相关命令：初始化、全量扫描、目录列举、结构概览、统计、监听状态。
//!
//! 命令层只做三件事：取状态、调下层、转 DTO。
//! 不写 SQL、不碰文件系统 —— 这样每个命令都薄到可以一眼看完。

use std::sync::atomic::Ordering;

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::db::repo;
use crate::db::schema::SCHEMA_VERSION;
use crate::error::{AppError, AppResult};
use crate::index::{self, ReconcileReport};
use crate::paths::{self, to_slash};
use crate::scan::WalkOptions;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootInfo {
    pub root: String,
    pub content_root: String,
    pub runtime_dir: String,
    pub state_dir: String,
    pub volume_serial: String,
    pub db_path: String,
    pub schema_version: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDto {
    pub id: i64,
    pub rel_path: String,
    pub name: String,
    pub ext: Option<String>,
    pub kind: String,
    pub size: i64,
    pub mtime: i64,
    /// 是否有子项。现在是**真实值**（`EXISTS` 现查），不再是「是不是目录」近似。
    pub has_children: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsDto {
    pub total: i64,
    pub dirs: i64,
    pub files: i64,
    pub bytes: i64,
}

/// 一棵子树的规模。`dirs` / `files` / `bytes` **不含自身**，
/// 也就是说它回答的是「这个目录里装了多少东西」。
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeStatsDto {
    pub dirs: i64,
    pub files: i64,
    pub bytes: i64,
    pub latest_mtime: i64,
}

/// 面包屑的一节。`rel` 为空串表示内容区根。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrumbDto {
    pub name: String,
    pub rel: String,
}

/// 一个子项 + 它自己的规模。
///
/// 这是「结构视图」的基本单元：界面因此能表达「这一层是怎么分类的、
/// 每个分支各有多大」，而不是把文件按扩展名堆在一起。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildDto {
    pub node: NodeDto,
    pub subtree: TreeStatsDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirOverviewDto {
    /// `None` 表示内容区根，此时 `breadcrumb` 只有一节。
    pub rel: Option<String>,
    pub name: String,
    pub breadcrumb: Vec<CrumbDto>,
    /// 整个分支的总量（根节点则是全盘总量）
    pub totals: TreeStatsDto,
    pub children: Vec<ChildDto>,
    /// 直属子目录数 / 直属文件数。界面用它显示「有几个分支」。
    pub dirs_direct: i64,
    pub files_direct: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchInfo {
    /// 监听是否真的在跑。为 false 时索引只在手动扫描 / 启动时更新。
    pub active: bool,
    pub error: Option<String>,
}

/// 前端启动后的第一个调用：拿到盘根、内容区、卷身份。
#[tauri::command]
pub fn lib_init(state: State<'_, AppState>) -> AppResult<RootInfo> {
    Ok(RootInfo {
        root: to_slash(state.root.root()),
        content_root: to_slash(state.root.content()),
        runtime_dir: to_slash(&state.root.runtime_dir()),
        state_dir: to_slash(&state.root.state_dir()),
        volume_serial: state.root.volume_serial().to_string(),
        db_path: to_slash(&state.root.db_path()),
        schema_version: SCHEMA_VERSION,
    })
}

/// 全量扫描 + 对账。手动按钮与首次建索引用它。
#[tauri::command]
pub fn scan_full(state: State<'_, AppState>) -> AppResult<ReconcileReport> {
    if state.scanning.swap(true, Ordering::SeqCst) {
        return Err(AppError::ScanBusy);
    }

    let result = (|| -> AppResult<ReconcileReport> {
        let mut conn = state.db.lock().map_err(|_| AppError::Poisoned)?;
        index::rescan(
            &mut conn,
            state.root.content(),
            state.root.volume_serial(),
            &WalkOptions::default(),
        )
    })();

    // 无论成功失败都要复位，否则一次异常会让扫描功能永久锁死
    state.scanning.store(false, Ordering::SeqCst);
    result
}

#[tauri::command]
pub fn list_children(
    state: State<'_, AppState>,
    parent: Option<String>,
) -> AppResult<Vec<NodeDto>> {
    let conn = state.db.lock().map_err(|_| AppError::Poisoned)?;
    let volume_id = current_volume_id(&conn, state.root.volume_serial())?;
    let parent = normalize_rel(parent);
    let rows = repo::list_children(&conn, volume_id, parent.as_deref())?;
    Ok(rows.into_iter().map(to_node_dto).collect())
}

/// 结构概览：一次调用返回「当前这一层是什么样」。
///
/// `rel` 为 `None` 时是内容区根 —— 前端右侧的「分类全景」与中间栏的
/// 目录页共用这一条命令，避免两套逻辑给出不一致的数字。
#[tauri::command]
pub fn dir_overview(state: State<'_, AppState>, rel: Option<String>) -> AppResult<DirOverviewDto> {
    let conn = state.db.lock().map_err(|_| AppError::Poisoned)?;
    let volume_id = current_volume_id(&conn, state.root.volume_serial())?;
    let rel = normalize_rel(rel);

    // 只查库，但仍要校验：rel 会出现在面包屑与后续 open 调用里，
    // 放一个带 `..` 的字符串过去，错误会推迟到更远的地方才暴露。
    if let Some(r) = rel.as_deref() {
        paths::sanitize_rel(r)?;
    }

    let mut breadcrumb = vec![CrumbDto {
        name: "全部内容".to_string(),
        rel: String::new(),
    }];
    if let Some(r) = rel.as_deref() {
        for (p, n) in paths::ancestors(r) {
            breadcrumb.push(CrumbDto { name: n, rel: p });
        }
    }

    let rows = repo::list_children(&conn, volume_id, rel.as_deref())?;
    let mut children = Vec::with_capacity(rows.len());
    let mut dirs_direct = 0i64;
    let mut files_direct = 0i64;

    for row in rows {
        let is_dir = row.kind == "dir";
        if is_dir {
            dirs_direct += 1;
        } else {
            files_direct += 1;
        }

        // 每个子目录单独算一次子树规模。
        // N+1 查询换来的是一句话能说清的口径；一层几十到几百个子项，
        // 每次都是索引区间扫描，实测代价可以忽略。
        let subtree = if is_dir {
            to_tree_stats(&repo::subtree_stats(&conn, volume_id, &row.rel_path)?)
        } else {
            TreeStatsDto::default()
        };

        children.push(ChildDto {
            node: to_node_dto(row),
            subtree,
        });
    }

    let totals = match rel.as_deref() {
        None => {
            // 根不是一条节点记录，全盘统计就是它的子树
            let s = repo::stats(&conn, volume_id)?;
            TreeStatsDto {
                dirs: s.dirs,
                files: s.files,
                bytes: s.bytes,
                latest_mtime: s.latest_mtime,
            }
        }
        Some(r) => to_tree_stats(&repo::subtree_stats(&conn, volume_id, r)?),
    };

    let name = match rel.as_deref() {
        None => "全部内容".to_string(),
        Some(r) => r.rsplit('/').next().unwrap_or(r).to_string(),
    };

    Ok(DirOverviewDto {
        rel,
        name,
        breadcrumb,
        totals,
        children,
        dirs_direct,
        files_direct,
    })
}

#[tauri::command]
pub fn library_stats(state: State<'_, AppState>) -> AppResult<StatsDto> {
    let conn = state.db.lock().map_err(|_| AppError::Poisoned)?;
    let volume_id = current_volume_id(&conn, state.root.volume_serial())?;
    let s = repo::stats(&conn, volume_id)?;
    Ok(StatsDto {
        total: s.total,
        dirs: s.dirs,
        files: s.files,
        bytes: s.bytes,
    })
}

/// 监听是否可用。界面据此决定要不要提示「手动刷新」。
#[tauri::command]
pub fn watch_info(state: State<'_, AppState>) -> AppResult<WatchInfo> {
    let w = state.watch.lock().map_err(|_| AppError::Poisoned)?;
    Ok(WatchInfo {
        active: w.handle.is_some(),
        error: w.error.clone(),
    })
}

fn to_tree_stats(s: &repo::TreeStats) -> TreeStatsDto {
    TreeStatsDto {
        dirs: s.dirs,
        files: s.files,
        bytes: s.bytes,
        latest_mtime: s.latest_mtime,
    }
}

fn to_node_dto(r: repo::NodeRow) -> NodeDto {
    NodeDto {
        id: r.id,
        rel_path: r.rel_path,
        name: r.name,
        ext: r.ext,
        kind: r.kind,
        size: r.size,
        mtime: r.mtime,
        has_children: r.has_children,
    }
}

/// 空串与 `None` 等价，都表示内容区根。前端传 `""` 比传 `null` 更自然，
/// 两边都接受，省掉一类「明明在根上却查不到子项」的怪现象。
fn normalize_rel(rel: Option<String>) -> Option<String> {
    rel.map(|s| s.trim_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

/// 卷 id 每次现查而不是缓存在状态里 —— 缓存一旦与真实库不一致，
/// 排查成本远高于一次索引查询。具体实现在 `repo`，这里只是转调。
fn current_volume_id(conn: &Connection, serial: &str) -> AppResult<i64> {
    repo::find_volume_id(conn, serial)
}
