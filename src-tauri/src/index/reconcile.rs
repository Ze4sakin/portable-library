//! 索引对账：把「扫描结果」与「库里的旧索引」求差集，只写变化的部分。
//!
//! 设计要点：
//! - **真相是对账，不是事件。** 全量扫描得到完整集合，与旧集合比对，
//!   消失的删掉。不依赖任何文件监听信号。
//! - 未变化的节点**零写入**。所以第二次扫描的耗时主要是遍历文件系统，
//!   而不是写数据库。
//! - 整个对账在单个事务里完成，中途断电不会留下半张表。

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use rusqlite::Connection;
use serde::Serialize;

use crate::db::repo;
use crate::error::AppResult;
use crate::scan::{walk_full, walker, EntryKind, FsEntry, WalkOptions};

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileReport {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub skipped_dirs: usize,
    pub duration_ms: u64,
}

/// 全量重扫并落库。调用方需保证同一时刻只有一次扫描在跑。
pub fn full_scan(
    conn: &mut Connection,
    volume_id: i64,
    content: &Path,
    opts: &WalkOptions,
) -> AppResult<ReconcileReport> {
    let started = Instant::now();

    let walk = walk_full(content, opts)?;

    let tx = conn.transaction()?;
    let mut known: HashMap<String, (i64, String)> = repo::load_index(&tx, volume_id)?;

    let mut ids: HashMap<String, i64> = HashMap::with_capacity(walk.entries.len());
    let mut added = 0usize;
    let mut updated = 0usize;
    let mut unchanged = 0usize;

    for entry in &walk.entries {
        // walker 契约：父节点先于子节点产出，因此这里父 id 一定已在表中
        let parent_id = entry
            .parent_rel
            .as_deref()
            .and_then(|p| ids.get(p).copied());

        match known.remove(&entry.rel_path) {
            Some((id, old_hash)) => {
                ids.insert(entry.rel_path.clone(), id);
                if old_hash != entry.quick_hash {
                    repo::update_node(&tx, id, parent_id, entry)?;
                    updated += 1;
                } else {
                    unchanged += 1;
                }
            }
            None => {
                let id = repo::insert_node(&tx, volume_id, parent_id, entry)?;
                ids.insert(entry.rel_path.clone(), id);
                added += 1;
            }
        }
    }

    // 旧索引里剩下的，就是磁盘上已经消失的
    let removed = known.len();
    for (_, (id, _)) in known {
        repo::delete_node(&tx, id)?;
    }

    tx.commit()?;

    Ok(ReconcileReport {
        scanned: walk.entries.len(),
        added,
        updated,
        removed,
        unchanged,
        skipped_dirs: walk.skipped,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}

/// 基于「脏路径集合」的增量对账。
///
/// 与 `full_scan` 的关系是**同一个算法的局部化**，不是另一套逻辑：
/// 全量 = 把整盘当成一个脏路径；增量 = 对每个脏路径，只把它自己
/// （目录则连同整棵子树）重新对账一遍。
///
/// 三条必须守住的规则：
/// 1. **事件只是线索，磁盘才是真相。** 拿到脏路径后一律重新 stat，
///    不看事件类型。这样「重命名」「先删后建」「同名覆盖」都收敛到同一处理。
/// 2. **目录要整棵对账，不只信事件。** 某些操作（如资源管理器里的整目录移动）
///    只报一个目录级事件，子项变化不会逐个上报。
/// 3. **父目录一起更新。** 目录的指纹含 mtime，而子项增删会改父目录 mtime；
///    漏了这一步，增量之后紧接着跑一次全量，会看到一堆「更新」——
///    那不是真实变化，是两条路径口径不一致。
pub fn reconcile_dirty(
    conn: &mut Connection,
    volume_id: i64,
    content: &Path,
    dirty: &[String],
    opts: &WalkOptions,
) -> AppResult<ReconcileReport> {
    let started = Instant::now();
    let mut report = ReconcileReport::default();

    let mut paths: Vec<String> = dirty
        .iter()
        .map(|p| p.trim_matches('/').to_string())
        .filter(|p| !p.is_empty() && !opts.is_ignored_rel(p))
        .collect();
    paths.sort();
    paths.dedup();

    let tx = conn.transaction()?;

    // 已整棵处理过的目录。它们的后代不必再单独处理 ——
    // 一次目录重命名会带出成百上千条事件，不做这个剪枝就会重复走同一棵树。
    let mut covered: Vec<String> = Vec::new();
    let mut parents: HashSet<String> = HashSet::new();

    for rel in paths {
        if covered
            .iter()
            .any(|c| rel == *c || rel.starts_with(&format!("{c}/")))
        {
            continue;
        }

        match walker::probe(content, &rel) {
            walker::Probe::Absent => remove_rel(&tx, volume_id, &rel, &mut report)?,
            walker::Probe::Inaccessible => report.skipped_dirs += 1,
            walker::Probe::Present(entry) => {
                report.scanned += 1;
                let is_dir = entry.kind == EntryKind::Dir;
                upsert_entry(&tx, volume_id, &entry, &mut report)?;
                if is_dir {
                    reconcile_subtree(&tx, volume_id, content, &rel, opts, &mut report)?;
                    covered.push(rel.clone());
                }
            }
        }

        if let Some(p) = parent_of(&rel) {
            parents.insert(p);
        }
    }

    for p in parents {
        // 父目录可能也已被上面删掉，因此仍要探测一次
        if let walker::Probe::Present(entry) = walker::probe(content, &p) {
            upsert_entry(&tx, volume_id, &entry, &mut report)?;
        }
    }

    tx.commit()?;
    report.duration_ms = started.elapsed().as_millis() as u64;
    Ok(report)
}

/// 对账一棵子树：以磁盘现状为准，与索引求差集。
fn reconcile_subtree(
    conn: &Connection,
    volume_id: i64,
    content: &Path,
    rel: &str,
    opts: &WalkOptions,
    report: &mut ReconcileReport,
) -> AppResult<()> {
    let Ok(safe) = crate::paths::sanitize_rel(rel) else {
        return Ok(());
    };
    let abs = content.join(safe);

    let walk = walker::walk_from(content, &abs, opts)?;
    report.skipped_dirs += walk.skipped;

    let mut known = repo::load_subtree_index(conn, volume_id, rel)?;
    let mut ids: HashMap<String, i64> = HashMap::with_capacity(walk.entries.len() + 1);

    // 子树根（即 rel 自身）刚刚已 upsert，把它的 id 作为第一层父节点
    if let Some(id) = repo::node_id_of(conn, volume_id, rel)? {
        ids.insert(rel.to_string(), id);
    }

    for entry in &walk.entries {
        report.scanned += 1;
        let parent_id = entry
            .parent_rel
            .as_deref()
            .and_then(|p| ids.get(p).copied());

        match known.remove(&entry.rel_path) {
            Some((id, old_hash)) => {
                ids.insert(entry.rel_path.clone(), id);
                if old_hash != entry.quick_hash {
                    repo::update_node(conn, id, parent_id, entry)?;
                    report.updated += 1;
                } else {
                    report.unchanged += 1;
                }
            }
            None => {
                let id = repo::insert_node(conn, volume_id, parent_id, entry)?;
                ids.insert(entry.rel_path.clone(), id);
                report.added += 1;
            }
        }
    }

    // 索引里有、磁盘上没有的，就是这棵子树里被删掉的
    for (_, (id, _)) in known {
        repo::delete_node(conn, id)?;
        report.removed += 1;
    }

    Ok(())
}

/// 单条路径的 upsert，返回节点 id。
fn upsert_entry(
    conn: &Connection,
    volume_id: i64,
    entry: &FsEntry,
    report: &mut ReconcileReport,
) -> AppResult<i64> {
    let parent_id = match parent_of(&entry.rel_path) {
        None => None,
        Some(p) => repo::node_id_of(conn, volume_id, &p)?,
    };

    match repo::node_fingerprint(conn, volume_id, &entry.rel_path)? {
        Some((id, old_hash)) => {
            if old_hash != entry.quick_hash {
                repo::update_node(conn, id, parent_id, entry)?;
                report.updated += 1;
            } else {
                report.unchanged += 1;
            }
            Ok(id)
        }
        None => {
            let id = repo::insert_node(conn, volume_id, parent_id, entry)?;
            report.added += 1;
            Ok(id)
        }
    }
}

/// 删掉一条路径及其后代。
///
/// 后代靠 `ON DELETE CASCADE` 连同删除，这里只需删根节点；
/// 但 `removed` 要如实计数，所以先数一遍子树规模。
fn remove_rel(
    conn: &Connection,
    volume_id: i64,
    rel: &str,
    report: &mut ReconcileReport,
) -> AppResult<()> {
    let Some(id) = repo::node_id_of(conn, volume_id, rel)? else {
        return Ok(());
    };
    let descendants = repo::load_subtree_index(conn, volume_id, rel)?.len();
    repo::delete_node(conn, id)?;
    report.removed += descendants + 1;
    Ok(())
}

fn parent_of(rel: &str) -> Option<String> {
    rel.rsplit_once('/').map(|(p, _)| p.to_string())
}

/// 扫一遍并登记卷身份，对外只暴露这一个入口。
pub fn rescan(
    conn: &mut Connection,
    content: &Path,
    volume_serial: &str,
    opts: &WalkOptions,
) -> AppResult<ReconcileReport> {
    let now = now_rfc3339();
    let volume_id = repo::upsert_volume(conn, volume_serial, &now)?;
    full_scan(conn, volume_id, content, opts)
}

/// 无第三方依赖的 ISO8601 UTC 时间戳（Howard Hinnant civil_from_days）。
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_day_zero() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    /// 基准值全部来自 `date -u -d <日期> +%s` 除以 86400，
    /// 不是手算的 —— 手算过一次，算错了，被这条测试抓出来。
    #[test]
    fn known_dates() {
        assert_eq!(civil_from_days(20_754), (2026, 10, 28));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29)); // 2000 是闰年
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(47_541), (2100, 3, 1)); // 2100 不是闰年
    }

    /// 跨年、跨月的最后一天最容易被 off-by-one 打中。
    #[test]
    fn month_and_year_boundaries() {
        assert_eq!(civil_from_days(59), (1970, 3, 1));
        assert_eq!(civil_from_days(60), (1970, 3, 2));
        assert_eq!(civil_from_days(10_956), (1999, 12, 31));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
    }
}
