//! 端到端链路测试：扫描 → 落库 → 对账 → 查询。
//!
//! 为什么单独放在这里，而不写进各模块的单测：
//! 单测验证的是「一个函数算得对不对」，而这条链要验证的是**模块之间拼起来
//! 还对不对** —— 相对路径的写法是否统一、父 id 是否真的建上了、差集算法
//! 在「改一个、删一个、加一个」同时发生时是否收敛。这类错误单测抓不到。
//!
//! 这一层刻意不碰 Tauri：`commands` 需要 `State<AppState>`，属于 IPC 表面，
//! 真正的数据通路在它之下，正好可以脱离框架测。

use std::fs;
use std::path::{Path, PathBuf};

use crate::db::{repo, schema};
use crate::index::reconcile;
use crate::paths::RootCtx;
use crate::scan::WalkOptions;

fn temp_root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!("pl-e2e-{tag}-{}-{nanos}", std::process::id()));
    p
}

fn write_file(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).expect("建目录失败");
    }
    fs::write(&p, body).expect("写文件失败");
}

#[test]
fn scan_reconcile_query_roundtrip() {
    let root_dir = temp_root("roundtrip");
    let _ = fs::remove_dir_all(&root_dir);
    fs::create_dir_all(root_dir.join("content")).expect("建 content 失败");

    // 4 个文件 + 5 个目录（notes / notes/sub / code / code/proj / empty）
    write_file(&root_dir, "content/notes/a.md", "# 甲\n");
    write_file(&root_dir, "content/notes/sub/b.txt", "乙\n");
    write_file(&root_dir, "content/code/proj/main.py", "print('hi')\n");
    write_file(&root_dir, "content/code/proj/README.md", "# proj\n");
    fs::create_dir_all(root_dir.join("content/empty")).expect("建空目录失败");

    let root = RootCtx::detect_in(Some(root_dir.clone())).expect("构造盘根失败");
    let mut conn = schema::open_and_migrate(&root.db_path()).expect("开库失败");
    let opts = WalkOptions::default();

    // —— 首次全量：全部新增 ——
    let r1 = reconcile::rescan(&mut conn, root.content(), root.volume_serial(), &opts)
        .expect("首次扫描失败");
    assert_eq!(r1.scanned, 9, "应扫到 9 个节点");
    assert_eq!((r1.added, r1.updated, r1.removed), (9, 0, 0));
    assert_eq!(r1.skipped_dirs, 0);

    let vid = repo::find_volume_id(&conn, root.volume_serial()).expect("取 volume_id 失败");

    // —— 懒加载：一次一层，目录排在文件之前 ——
    let top = repo::list_children(&conn, vid, None).expect("取根层失败");
    assert_eq!(
        top.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        vec!["code", "empty", "notes"]
    );

    let notes = repo::list_children(&conn, vid, Some("notes")).expect("取 notes 失败");
    assert_eq!(
        notes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
        vec!["sub", "a.md"],
        "目录必须排在文件前面"
    );

    // 子目录里的文件能取到，说明 parent_id 这条边真的建上了
    let sub = repo::list_children(&conn, vid, Some("notes/sub")).expect("取 sub 失败");
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].rel_path, "notes/sub/b.txt");
    assert_eq!(sub[0].ext.as_deref(), Some("txt"));

    let stats = repo::stats(&conn, vid).expect("统计失败");
    assert_eq!((stats.total, stats.dirs, stats.files), (9, 5, 4));

    // —— 重复扫描：未变化的节点零写入 ——
    let r2 = reconcile::rescan(&mut conn, root.content(), root.volume_serial(), &opts)
        .expect("二次扫描失败");
    assert_eq!(
        (r2.added, r2.updated, r2.removed, r2.unchanged),
        (0, 0, 0, 9),
        "第二次扫描应当全部命中 unchanged"
    );

    // —— 改一个（长度也变）、删一个、加一个，同时发生 ——
    write_file(&root_dir, "content/notes/a.md", "# 甲\n内容变长了\n");
    fs::remove_file(root_dir.join("content/notes/sub/b.txt")).expect("删文件失败");
    write_file(&root_dir, "content/code/new.rs", "fn main() {}\n");

    let r3 = reconcile::rescan(&mut conn, root.content(), root.volume_serial(), &opts)
        .expect("三次扫描失败");
    assert_eq!(r3.scanned, 9);
    // updated 是 3 而不是 1，这是**预期行为**：节点指纹取的是 size+mtime，
    // 而目录的 mtime 会在「子项增删」时改变 —— 于是 `code`（新增 new.rs）与
    // `notes/sub`（删除 b.txt）也各算一次更新。这不是误判。
    // 反过来，改写 a.md 的**内容**不影响父目录 notes 的 mtime，
    // 所以 `notes` 仍在 unchanged 里 —— 这两条一起证明了判据是准的。
    assert_eq!((r3.added, r3.updated, r3.removed), (1, 3, 1));
    assert_eq!(r3.unchanged, 5);

    // 当前磁盘状态被完整划分成三份，一个不能多、一个不能少。
    // 注意 `removed` 不参与这个等式：它是「本次没扫到」的旧节点，
    // 本来就不在 scanned 里。
    assert_eq!(r3.added + r3.updated + r3.unchanged, r3.scanned);

    // 删掉的那条不能留在库里
    let sub_after = repo::list_children(&conn, vid, Some("notes/sub")).expect("取 sub 失败");
    assert!(sub_after.is_empty(), "b.txt 已删除，索引里不该还有");

    // —— 路径防线：越界一律拒绝，正常路径能取到实体，且能被还原 ——
    assert!(root.resolve("../etc/passwd").is_err());
    assert!(root.resolve("C:/Windows/win.ini").is_err());
    assert!(root.resolve("notes/a.md:hidden").is_err());

    let abs = root.resolve("notes/a.md").expect("正常路径不该失败");
    assert!(abs.is_file());
    assert_eq!(root.relativize(&abs).unwrap(), "notes/a.md");

    drop(conn);
    let _ = fs::remove_dir_all(&root_dir);
}

/// 把索引按「界面看到的形状」摊平成文本行：缩进即层级，字段即节点属性。
///
/// 不直接查 SQL，而是走 `list_children` 递归 —— 这样比对的是**用户能看到的
/// 那棵树**，而不是存储细节。若两棵树的展开结果逐字相同，界面就不可能不一致。
fn dump_tree(conn: &rusqlite::Connection, vid: i64, parent: Option<&str>, out: &mut Vec<String>) {
    let rows = repo::list_children(conn, vid, parent).expect("取子项失败");
    for r in rows {
        let indent = "  ".repeat(r.rel_path.matches('/').count());
        out.push(format!(
            "{indent}{} | {} | size={} | mtime={} | children={}",
            r.name, r.kind, r.size, r.mtime, r.has_children
        ));
        if r.kind == "dir" {
            dump_tree(conn, vid, Some(&r.rel_path), out);
        }
    }
}

/// 增量对账必须与全量扫描**收敛到同一个索引状态**。
///
/// 这是整套实时同步的地基：监听会丢事件、会把一次操作报成几条、
/// 会在不同平台给出不同的组合。只要「局部重算」与「整体重算」结果一致，
/// 这些不确定性就都无害 —— 差异会被下一次对账自然抹平。
/// 反过来，一旦两条路径口径不同，索引就会在两种扫描之间反复抖动，
/// 表现为「每次扫描都在更新同一批文件」，而这种抖动极难从日志里看出来。
#[test]
fn incremental_matches_full_scan() {
    let root_dir = temp_root("incr");
    let _ = fs::remove_dir_all(&root_dir);
    fs::create_dir_all(root_dir.join("content")).expect("建 content 失败");

    write_file(&root_dir, "content/notes/a.md", "# 甲\n");
    write_file(&root_dir, "content/notes/sub/b.txt", "乙\n");
    write_file(&root_dir, "content/code/proj/main.py", "print('hi')\n");
    write_file(&root_dir, "content/code/proj/README.md", "# proj\n");
    write_file(&root_dir, "content/code/top.rs", "fn main() {}\n");
    fs::create_dir_all(root_dir.join("content/empty")).expect("建空目录失败");

    let root = RootCtx::detect_in(Some(root_dir.clone())).expect("构造盘根失败");
    let mut conn = schema::open_and_migrate(&root.db_path()).expect("开库失败");
    let opts = WalkOptions::default();

    reconcile::rescan(&mut conn, root.content(), root.volume_serial(), &opts)
        .expect("首次扫描失败");
    let vid = repo::find_volume_id(&conn, root.volume_serial()).expect("取 volume_id 失败");

    // —— 制造四类变更，覆盖监听事件的主要形态 ——
    // 1) 改写内容      2) 删文件        3) 一次建出两层新目录
    // 4) 目录重命名（会同时报出旧路径与新路径）  5) 整棵子树消失
    write_file(
        &root_dir,
        "content/notes/a.md",
        "# 甲\n内容变长了，长度也变了\n",
    );
    fs::remove_file(root_dir.join("content/notes/sub/b.txt")).expect("删文件失败");
    write_file(&root_dir, "content/code/new/deep/x.rs", "fn x() {}\n");
    fs::rename(
        root_dir.join("content/empty"),
        root_dir.join("content/void"),
    )
    .expect("重命名目录失败");
    fs::remove_dir_all(root_dir.join("content/code/proj")).expect("删子树失败");

    // 监听会给出的脏路径：包含被删除/被改名的**旧**路径。
    // 只报新路径是不够的 —— 旧节点不会自己消失，这正是「对账」要补的洞。
    let dirty: Vec<String> = vec![
        "notes/a.md".into(),
        "notes/sub/b.txt".into(),
        "code/new".into(),
        "empty".into(),
        "void".into(),
        "code/proj".into(),
    ];

    let inc = reconcile::reconcile_dirty(&mut conn, vid, root.content(), &dirty, &opts)
        .expect("增量对账失败");

    let mut after_incremental = Vec::new();
    dump_tree(&conn, vid, None, &mut after_incremental);

    // 删掉整棵 proj（proj + main.py + README.md），删掉 b.txt，删掉旧名 empty
    assert_eq!(inc.removed, 5, "移除数应含整棵子树的节点");
    assert!(inc.added >= 3, "code/new、code/new/deep、x.rs 至少三个新增");
    assert_eq!(inc.skipped_dirs, 0);

    // —— 同一块磁盘再走一次全量，索引状态必须与增量结果完全相同 ——
    reconcile::rescan(&mut conn, root.content(), root.volume_serial(), &opts)
        .expect("全量复扫失败");
    let mut after_full = Vec::new();
    dump_tree(&conn, vid, None, &mut after_full);

    assert_eq!(
        after_incremental, after_full,
        "增量与全量必须收敛到同一棵树；不一致说明两条路径的判据口径不同"
    );

    // 全量复扫若还报出大量「更新」，说明增量留下的指纹是错的
    let again = reconcile::rescan(&mut conn, root.content(), root.volume_serial(), &opts)
        .expect("二次全量失败");
    assert_eq!(
        (again.added, again.updated, again.removed),
        (0, 0, 0),
        "增量之后紧接着全量，不该再有任何变化"
    );

    drop(conn);
    let _ = fs::remove_dir_all(&root_dir);
}
