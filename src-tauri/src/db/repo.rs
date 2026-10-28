//! 仓储层：所有 SQL 都收敛在这里，上层不认识 rusqlite 类型。
//!
//! 这样做的好处是 P3 换索引后端（比如加一层内存索引做加速）时，
//! 只需替换本文件，`index` / `commands` 一行不动。

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::db::schema::SCHEMA_VERSION;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct NodeRow {
    pub id: i64,
    pub rel_path: String,
    pub name: String,
    pub ext: Option<String>,
    pub kind: String,
    pub size: i64,
    pub mtime: i64,
    /// 是否真的有子项。用 `EXISTS` 现查，而不是「是不是目录」近似 ——
    /// 否则空目录也会长出展开箭头，点开却是空的。
    pub has_children: bool,
}

/// 一棵子树的规模（**不含自身**）。
#[derive(Debug, Clone, Copy, Default)]
pub struct TreeStats {
    pub dirs: i64,
    pub files: i64,
    pub bytes: i64,
    pub latest_mtime: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LibraryStats {
    pub total: i64,
    pub dirs: i64,
    pub files: i64,
    pub bytes: i64,
    pub latest_mtime: i64,
}

/// 登记/更新卷身份，返回 volume_id。
pub fn upsert_volume(conn: &Connection, serial: &str, now: &str) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO volume (volume_serial, fs_type, schema_ver, first_seen, last_mount)
         VALUES (?1, NULL, ?2, ?3, ?3)
         ON CONFLICT(volume_serial) DO UPDATE SET
           last_mount = excluded.last_mount,
           schema_ver = excluded.schema_ver",
        params![serial, SCHEMA_VERSION, now],
    )?;

    let id = conn.query_row(
        "SELECT id FROM volume WHERE volume_serial = ?1",
        params![serial],
        |r| r.get(0),
    )?;
    Ok(id)
}

/// 一次性载入现有索引：`rel_path -> (id, quick_hash)`。
///
/// 全量对账用它算出「新增 / 变更 / 消失」三个集合，
/// 未变化的节点零写入 —— 这是让重复扫描变快的关键。
pub fn load_index(conn: &Connection, volume_id: i64) -> AppResult<HashMap<String, (i64, String)>> {
    let mut stmt =
        conn.prepare("SELECT rel_path, id, quick_hash FROM node WHERE volume_id = ?1")?;
    let rows = stmt.query_map(params![volume_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut map = HashMap::new();
    for row in rows {
        let (rel, id, hash) = row?;
        map.insert(rel, (id, hash));
    }
    Ok(map)
}

pub fn insert_node(
    conn: &Connection,
    volume_id: i64,
    parent_id: Option<i64>,
    entry: &crate::scan::FsEntry,
) -> AppResult<i64> {
    conn.execute(
        "INSERT INTO node (volume_id, parent_id, rel_path, name, ext, kind, size, mtime, quick_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            volume_id,
            parent_id,
            entry.rel_path,
            entry.name,
            entry.ext,
            entry.kind.as_str(),
            entry.size,
            entry.mtime,
            entry.quick_hash,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_node(
    conn: &Connection,
    id: i64,
    parent_id: Option<i64>,
    entry: &crate::scan::FsEntry,
) -> AppResult<()> {
    conn.execute(
        "UPDATE node SET parent_id = ?1, name = ?2, ext = ?3, kind = ?4,
                         size = ?5, mtime = ?6, quick_hash = ?7
         WHERE id = ?8",
        params![
            parent_id,
            entry.name,
            entry.ext,
            entry.kind.as_str(),
            entry.size,
            entry.mtime,
            entry.quick_hash,
            id,
        ],
    )?;
    Ok(())
}

pub fn delete_node(conn: &Connection, id: i64) -> AppResult<()> {
    conn.execute("DELETE FROM node WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_children(
    conn: &Connection,
    volume_id: i64,
    parent_rel: Option<&str>,
) -> AppResult<Vec<NodeRow>> {
    // has_children 用相关子查询现算。idx_node_parent 让它是索引探测，
    // 一层几十行时可忽略；换来的是「空目录不显示展开箭头」。
    let sql = "SELECT id, rel_path, name, ext, kind, size, mtime,
                      EXISTS(SELECT 1 FROM node c
                              WHERE c.volume_id = node.volume_id AND c.parent_id = node.id)
               FROM node
               WHERE volume_id = ?1 AND {cond}
               ORDER BY (kind = 'dir') DESC, name COLLATE NOCASE ASC";

    let out = match parent_rel {
        None => {
            let sql = sql.replace("{cond}", "parent_id IS NULL");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![volume_id], map_node_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        }
        Some(rel) => {
            let parent_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM node WHERE volume_id = ?1 AND rel_path = ?2",
                    params![volume_id, rel],
                    |r| r.get(0),
                )
                .ok();

            let Some(pid) = parent_id else {
                return Ok(Vec::new());
            };

            let sql = sql.replace("{cond}", "parent_id = ?2");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![volume_id, pid], map_node_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        }
    };

    Ok(out)
}

/// 独立函数而非闭包：`query_map` 需要 HRTB，写成函数可以避开
/// 闭包对 `Row<'_>` 生命周期标注的推导问题。
fn map_node_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<NodeRow> {
    Ok(NodeRow {
        id: r.get(0)?,
        rel_path: r.get(1)?,
        name: r.get(2)?,
        ext: r.get(3)?,
        kind: r.get(4)?,
        size: r.get(5)?,
        mtime: r.get(6)?,
        has_children: r.get::<_, i64>(7)? != 0,
    })
}

/// 「以 `rel` 为前缀」的区间上界。
///
/// 为什么不用 `LIKE 'rel/%'`：SQLite 的 LIKE 默认对 ASCII **不区分大小写**，
/// 因此用不上 `rel_path` 上的索引，几十万行会退化成全表扫描。
/// 改成字符串区间则一定走索引：
///   rel_path >= "rel/"  且  rel_path < "rel0"
/// 成立的原因是 `/` 的码位是 0x2F，紧跟其后的 `0` 是 0x30 ——
/// 所有以 `rel/` 开头的串都落在这个左闭右开区间里，且不会误伤 `rel2/x`。
const PREFIX_UPPER: char = '0';

fn prefix_bounds(rel: &str) -> (String, String) {
    (format!("{rel}/"), format!("{rel}{PREFIX_UPPER}"))
}

/// 载入某棵子树在索引里的全部节点：`rel_path -> (id, quick_hash)`。
///
/// 增量对账用它算「这棵子树里哪些节点已经不存在了」—— 不信任监听事件，
/// 一律以磁盘现状为准。
pub fn load_subtree_index(
    conn: &Connection,
    volume_id: i64,
    rel: &str,
) -> AppResult<HashMap<String, (i64, String)>> {
    let (lo, hi) = prefix_bounds(rel);
    let mut stmt = conn.prepare(
        "SELECT rel_path, id, quick_hash FROM node
          WHERE volume_id = ?1 AND rel_path >= ?2 AND rel_path < ?3",
    )?;
    let rows = stmt.query_map(params![volume_id, lo, hi], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;

    let mut map = HashMap::new();
    for row in rows {
        let (p, id, hash) = row?;
        map.insert(p, (id, hash));
    }
    Ok(map)
}

/// 一棵子树的规模统计（不含 `rel` 自身）。
pub fn subtree_stats(conn: &Connection, volume_id: i64, rel: &str) -> AppResult<TreeStats> {
    let (lo, hi) = prefix_bounds(rel);
    let mut stmt = conn.prepare(
        "SELECT COALESCE(SUM(kind = 'dir'), 0),
                COALESCE(SUM(kind = 'file'), 0),
                COALESCE(SUM(CASE WHEN kind = 'file' THEN size ELSE 0 END), 0),
                COALESCE(MAX(mtime), 0)
           FROM node
          WHERE volume_id = ?1 AND rel_path >= ?2 AND rel_path < ?3",
    )?;
    let s = stmt.query_row(params![volume_id, lo, hi], |r| {
        Ok(TreeStats {
            dirs: r.get(0)?,
            files: r.get(1)?,
            bytes: r.get(2)?,
            latest_mtime: r.get(3)?,
        })
    })?;
    Ok(s)
}

/// 按相对路径取节点 id。不存在返回 `None`（`NotFound` 在上层才是异常）。
pub fn node_id_of(conn: &Connection, volume_id: i64, rel: &str) -> AppResult<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM node WHERE volume_id = ?1 AND rel_path = ?2",
            params![volume_id, rel],
            |r| r.get::<_, i64>(0),
        )
        .ok();
    Ok(id)
}

/// 取某个相对路径节点的「存在指纹」。增量对账判断「要不要写」用。
pub fn node_fingerprint(
    conn: &Connection,
    volume_id: i64,
    rel: &str,
) -> AppResult<Option<(i64, String)>> {
    let row = conn
        .query_row(
            "SELECT id, quick_hash FROM node WHERE volume_id = ?1 AND rel_path = ?2",
            params![volume_id, rel],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .ok();
    Ok(row)
}

pub fn stats(conn: &Connection, volume_id: i64) -> AppResult<LibraryStats> {
    let mut stmt = conn.prepare(
        "SELECT COUNT(*),
                COALESCE(SUM(kind = 'dir'), 0),
                COALESCE(SUM(kind = 'file'), 0),
                COALESCE(SUM(CASE WHEN kind = 'file' THEN size ELSE 0 END), 0),
                COALESCE(MAX(mtime), 0)
         FROM node WHERE volume_id = ?1",
    )?;
    let s = stmt.query_row(params![volume_id], |r| {
        Ok(LibraryStats {
            total: r.get(0)?,
            dirs: r.get(1)?,
            files: r.get(2)?,
            bytes: r.get(3)?,
            latest_mtime: r.get(4)?,
        })
    })?;
    Ok(s)
}

/// 按卷序列号取 volume_id。
///
/// 不缓存到内存：一旦缓存与真实库不一致，排查成本远高于一次索引查询。
pub fn find_volume_id(conn: &Connection, serial: &str) -> AppResult<i64> {
    let id = conn
        .query_row(
            "SELECT id FROM volume WHERE volume_serial = ?1",
            params![serial],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| match e {
            // 空库不是「错误」，是「还没扫描过」，给一句能指导用户的话
            rusqlite::Error::QueryReturnedNoRows => {
                AppError::NotFound("索引尚未建立，请先执行一次扫描".into())
            }
            other => AppError::Db(other),
        })?;
    Ok(id)
}

/// 一条「目录里命中了某个项目标记文件」的原始命中。
#[derive(Debug, Clone)]
pub struct MarkerHit {
    pub dir_id: i64,
    pub rel_path: String,
    pub dir_name: String,
    pub marker: String,
}

/// 找出所有「直接包含 marker 文件」的目录。
///
/// 本函数**只认文件名**，不认识什么是「项目」—— 优先级、噪声目录过滤都由
/// `project` 模块决定。这样仓储层不必知道任何业务语义，换成别的识别规则
/// 时这里一行都不用改。
pub fn list_project_dirs(
    conn: &Connection,
    volume_id: i64,
    markers: &[String],
) -> AppResult<Vec<MarkerHit>> {
    if markers.is_empty() {
        return Ok(Vec::new());
    }

    // 占位符数量由 markers 长度决定，值全部走绑定参数 —— 不拼字符串进 SQL
    let placeholders = std::iter::repeat("?")
        .take(markers.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT p.id, p.rel_path, p.name, m.name
         FROM node p
         JOIN node m ON m.parent_id = p.id AND m.volume_id = p.volume_id
         WHERE p.volume_id = ? AND p.kind = 'dir' AND m.name IN ({placeholders})
         ORDER BY p.rel_path"
    );

    let mut values: Vec<rusqlite::types::Value> = Vec::with_capacity(markers.len() + 1);
    values.push(rusqlite::types::Value::Integer(volume_id));
    for m in markers {
        values.push(rusqlite::types::Value::Text(m.clone()));
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values), map_marker_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn map_marker_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MarkerHit> {
    Ok(MarkerHit {
        dir_id: r.get(0)?,
        rel_path: r.get(1)?,
        dir_name: r.get(2)?,
        marker: r.get(3)?,
    })
}
