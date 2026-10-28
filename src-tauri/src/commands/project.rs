//! 项目相关命令。
//!
//! 「项目」在这里不是一张表，而是索引上的一次查询（见 `project` 模块）。
//! 命令层负责把原始命中整理成前端要的形状，并顺带回答一个用户真正关心的问题：
//! **这门语言需要的运行时，盘里到底有没有？**

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::db::repo;
use crate::error::{AppError, AppResult};
use crate::project;
use crate::runtime::env::EnvPlan;
use crate::runtime::launch::{self, Spec};
use crate::state::AppState;

/// 默认最多返回多少个项目。索引可能有几十万节点，但没人会看一千个项目卡片；
/// 需要更多时前端传 `limit`，后端不做硬限制。
const DEFAULT_LIMIT: usize = 200;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub node_id: i64,
    pub rel_path: String,
    pub name: String,
    pub lang: String,
    pub markers: Vec<String>,
    /// 这门语言对应的运行时种类（`python` / `node`），静态语言为 null
    pub runtime_kind: Option<String>,
    /// 盘内是否有可用的对应运行时。静态语言恒为 true
    pub runtime_ready: bool,
}

struct Acc {
    node_id: i64,
    rel_path: String,
    name: String,
    rank: usize,
    markers: Vec<String>,
}

#[tauri::command]
pub fn project_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<Vec<ProjectDto>> {
    // 作用域刻意收紧：数据库锁只覆盖查询，不覆盖后面的分组与组装。
    // 一旦这里把锁拿去等探测锁，就会埋下「A 等 B、B 等 A」的种子。
    let hits = {
        let conn = state.db.lock().map_err(|_| AppError::Poisoned)?;
        let volume_id = repo::find_volume_id(&conn, state.root.volume_serial())?;
        repo::list_project_dirs(&conn, volume_id, &project::marker_names())?
    };

    let mut grouped: BTreeMap<i64, Acc> = BTreeMap::new();
    for hit in hits {
        if project::is_noise(&hit.rel_path) {
            continue;
        }
        let Some(rank) = project::marker_rank(&hit.marker) else {
            continue;
        };

        match grouped.get_mut(&hit.dir_id) {
            Some(acc) => {
                if rank < acc.rank {
                    acc.rank = rank;
                }
                if !acc.markers.iter().any(|m| m == &hit.marker) {
                    acc.markers.push(hit.marker.clone());
                }
            }
            None => {
                grouped.insert(
                    hit.dir_id,
                    Acc {
                        node_id: hit.dir_id,
                        rel_path: hit.rel_path.clone(),
                        name: hit.dir_name.clone(),
                        rank,
                        markers: vec![hit.marker.clone()],
                    },
                );
            }
        }
    }

    // 已就绪的运行时种类。探测缓存为空时视为「未知」，此时不宣称未就绪，
    // 避免首次启动就把所有项目标成红色。
    let (ready_kinds, probe_known): (Vec<String>, bool) = {
        let cache = state.runtime_probe.lock().map_err(|_| AppError::Poisoned)?;
        match cache.as_ref() {
            Some(list) => (
                list.iter()
                    .filter(|s| s.runnable)
                    .map(|s| s.kind.clone())
                    .collect(),
                true,
            ),
            None => (Vec::new(), false),
        }
    };

    let mut list: Vec<ProjectDto> = grouped
        .into_values()
        .map(|acc| {
            let lang = project::lang_of_rank(acc.rank).to_string();
            let runtime_kind = project::runtime_kind_for_lang(&lang).map(|s| s.to_string());
            let runtime_ready = match (&runtime_kind, probe_known) {
                (None, _) => true,
                (Some(_), false) => true,
                (Some(kind), true) => ready_kinds.iter().any(|k| k == kind),
            };

            ProjectDto {
                node_id: acc.node_id,
                rel_path: acc.rel_path,
                name: acc.name,
                lang,
                markers: acc.markers,
                runtime_kind,
                runtime_ready,
            }
        })
        .collect();

    list.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    list.truncate(limit.unwrap_or(DEFAULT_LIMIT).max(1));
    Ok(list)
}

/// 在项目目录里开一个终端，且环境已指向盘内运行时。
///
/// 这是「代码项目能跑起来」的最小可用形态：比做一个运行按钮更通用
/// —— 用户拿到的是完整 shell，`npm run dev`、`pytest`、`python main.py`
/// 都能直接敲，不用为每门语言实现一套启动配置。
#[tauri::command]
pub fn project_terminal(state: State<'_, AppState>, rel: String) -> AppResult<()> {
    let dir = state.root.resolve(&rel)?;
    if !dir.is_dir() {
        return Err(AppError::NotFound(rel));
    }

    let prepend: Vec<PathBuf> = match state.manifest.as_ref() {
        Some(m) => m.bin_dirs(&state.root.runtime_dir()),
        None => Vec::new(),
    };
    let plan = EnvPlan::build(&state.root, &prepend, &BTreeMap::new());

    let (program, args) = terminal_command();
    let spec = Spec {
        program,
        args,
        cwd: dir,
        plan,
        // 关键：终端**不能**隐藏窗口，否则用户拿到一个看不见的进程
        hide: false,
    };
    launch::spawn_detached(&spec)
}

#[cfg(windows)]
fn terminal_command() -> (PathBuf, Vec<String>) {
    // 用 cmd 而不是 PowerShell：启动快、不用过执行策略，
    // 且 `cmd /K` 在 current_dir 下天然就是项目目录。
    (PathBuf::from("cmd.exe"), vec!["/K".to_string()])
}

#[cfg(not(windows))]
fn terminal_command() -> (PathBuf, Vec<String>) {
    (
        PathBuf::from("x-terminal-emulator"),
        vec!["-e".to_string(), "bash".to_string()],
    )
}
