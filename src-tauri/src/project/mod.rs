//! 项目识别。
//!
//! **刻意不建表。** 这是本模块最重要的一个决定：
//!
//! 「一个目录是不是项目」完全由它**当时**包含哪些文件决定。如果把它固化成
//! `project` 表，就多了一份需要与文件系统对账的真相 —— 用户删了
//! `package.json`、或者把项目挪了位置，表里就会留下一条幽灵记录。
//! 而索引里本来就有全部文件名，于是「项目」天然可以表达成一条查询：
//!
//! ```sql
//! SELECT 目录, 命中文件 FROM node WHERE 子节点名 IN (marker 列表) AND 子节点是目录的子项
//! ```
//!
//! 代价是每次都要查一遍索引（几十万行的 join，SQLite 上是个位数毫秒），
//! 收益是永远不会出现「项目列表与磁盘不一致」。
//!
//! 唯一真正需要持久化的项目信息（启动命令、环境变量、备注）才是表该管的事，
//! 那属于 P3 的 `project_profile`，而不是「是不是项目」这个判断。

/// 项目标记文件。**顺序即优先级**：一个目录同时有 `pyproject.toml` 和
/// `package.json`（前端 + 后端混装），取更靠前的那个作为主语言。
pub struct Marker {
    pub name: &'static str,
    pub lang: &'static str,
}

pub const MARKERS: &[Marker] = &[
    Marker {
        name: "package.json",
        lang: "node",
    },
    Marker {
        name: "pnpm-workspace.yaml",
        lang: "node",
    },
    Marker {
        name: "pyproject.toml",
        lang: "python",
    },
    Marker {
        name: "requirements.txt",
        lang: "python",
    },
    Marker {
        name: "Pipfile",
        lang: "python",
    },
    Marker {
        name: "environment.yml",
        lang: "python",
    },
    Marker {
        name: "Cargo.toml",
        lang: "rust",
    },
    Marker {
        name: "go.mod",
        lang: "go",
    },
    Marker {
        name: "pom.xml",
        lang: "java",
    },
    Marker {
        name: "build.gradle",
        lang: "java",
    },
    Marker {
        name: "build.gradle.kts",
        lang: "java",
    },
    Marker {
        name: "composer.json",
        lang: "php",
    },
    Marker {
        name: "Gemfile",
        lang: "ruby",
    },
    Marker {
        name: "CMakeLists.txt",
        lang: "cpp",
    },
];

/// 命中这些目录名的一律不算项目。
///
/// 这一条不是洁癖，是必需：一个装了依赖的 Node 项目，`node_modules` 下
/// 每个包都带 `package.json`，不过滤的话「项目列表」会瞬间变成几千条噪声，
/// 真正的项目被埋掉。同理 `.venv` / `target` / `dist`。
pub const NOISE: &[&str] = &[
    "node_modules",
    ".venv",
    "venv",
    "site-packages",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".next",
    ".nuxt",
    ".cache",
    "vendor",
    ".git",
];

/// marker 名单，供 `repo::list_project_dirs` 拼 SQL 占位符用。
pub fn marker_names() -> Vec<String> {
    MARKERS.iter().map(|m| m.name.to_string()).collect()
}

/// 命中的文件名 → 优先级序号（越小越优先）。
pub fn marker_rank(name: &str) -> Option<usize> {
    MARKERS.iter().position(|m| m.name == name)
}

pub fn lang_of_rank(rank: usize) -> &'static str {
    MARKERS.get(rank).map(|m| m.lang).unwrap_or("unknown")
}

/// 语言 → 需要的运行时组件种类。返回 `None` 表示这门语言不需要盘内运行时
/// （Rust / Go 是静态编译，Java 有自带 JRE 的分发方式，不在此列）。
pub fn runtime_kind_for_lang(lang: &str) -> Option<&'static str> {
    match lang {
        "node" => Some("node"),
        "python" => Some("python"),
        _ => None,
    }
}

/// 路径里任意一段命中噪声名单即判定为噪声。
pub fn is_noise(rel_path: &str) -> bool {
    rel_path
        .split('/')
        .any(|seg| NOISE.iter().any(|n| n.eq_ignore_ascii_case(seg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_matched_per_segment() {
        assert!(is_noise("app/node_modules/foo"));
        assert!(is_noise("py/.venv/lib"));
        assert!(is_noise("rust/target"));
        assert!(!is_noise("notes/node-modules.md"));
        assert!(!is_noise("projects/build-tools"));
    }

    #[test]
    fn marker_rank_follows_declared_priority() {
        assert_eq!(marker_rank("package.json"), Some(0));
        assert!(marker_rank("pyproject.toml") > marker_rank("package.json"));
        assert_eq!(marker_rank("unknown.lock"), None);
    }

    #[test]
    fn lang_maps_to_runtime_kind() {
        assert_eq!(
            lang_of_rank(marker_rank("pyproject.toml").unwrap()),
            "python"
        );
        assert_eq!(runtime_kind_for_lang("python"), Some("python"));
        assert_eq!(runtime_kind_for_lang("rust"), None);
    }
}
