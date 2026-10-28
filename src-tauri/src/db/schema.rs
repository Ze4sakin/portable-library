//! 索引库结构定义与迁移。
//!
//! 迁移机制说明（这是「可扩展性」的地基）：
//! - 用 SQLite 内置的 `PRAGMA user_version` 记录结构版本
//! - 每次启动比对版本，逐级向上迁移，绝不「删库重扫」
//! - P1 只建 `volume` + `node` 两张表；后续版本按需追加
//!
//! 后续待接入的表（已在讲解文档中定稿，此处先不建，避免死表）：
//!   v2: meta(node_id, title, tags, summary, thumb_rel, stars, ...)
//!   v3: env_profile / opener(ext_glob, mode, exe_rel, args_tpl, env_profile)
//!   v4: project(root_node_id, name, readme_node, stack, status)
//!   v5: fts_content USING fts5(rel_path, body, tokenize='trigram')

use rusqlite::Connection;

use crate::error::AppResult;

pub const SCHEMA_VERSION: i32 = 1;

pub fn open_and_migrate(db_path: &std::path::Path) -> AppResult<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(db_path)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> AppResult<()> {
    // WAL：兼顾并发读与掉电恢复；NORMAL 在可移动介质上是性能与安全的折中
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )?;

    let current: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if current < 1 {
        conn.execute_batch(V1)?;
    }

    // P2 起在这里追加：
    // if current < 2 { conn.execute_batch(V2)?; }

    if current < SCHEMA_VERSION {
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    }

    Ok(())
}

/// v1：卷身份 + 文件节点。
///
/// `rel_path` 是相对 content 根、以 `/` 分隔的字符串，全表唯一的路径表达形式。
/// 库内不存在任何绝对路径 —— 这是换盘符后仍能工作的前提。
const V1: &str = r#"
CREATE TABLE IF NOT EXISTS volume (
  id            INTEGER PRIMARY KEY,
  volume_serial TEXT    NOT NULL UNIQUE,
  label         TEXT,
  fs_type       TEXT,
  schema_ver    INTEGER NOT NULL,
  first_seen    TEXT    NOT NULL,
  last_mount    TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS node (
  id         INTEGER PRIMARY KEY,
  volume_id  INTEGER NOT NULL REFERENCES volume(id) ON DELETE CASCADE,
  parent_id  INTEGER REFERENCES node(id) ON DELETE CASCADE,
  rel_path   TEXT    NOT NULL,
  name       TEXT    NOT NULL,
  ext        TEXT,
  kind       TEXT    NOT NULL CHECK (kind IN ('dir','file','link')),
  size       INTEGER NOT NULL DEFAULT 0,
  mtime      INTEGER NOT NULL DEFAULT 0,
  quick_hash TEXT    NOT NULL DEFAULT '',
  UNIQUE(volume_id, rel_path)
);

CREATE INDEX IF NOT EXISTS idx_node_parent ON node(volume_id, parent_id);
CREATE INDEX IF NOT EXISTS idx_node_ext    ON node(ext);
"#;
