//! 运行时清单 —— 盘内便携环境的**唯一真相源**。
//!
//! 为什么真相是文件而不是数据库：
//! 这块盘的核心用法是「整盘复制到另一块硬盘」。如果运行时的安装状态存在
//! `.library/catalog.db` 里，复制之后那份状态就是**错的**（它描述的是旧盘）。
//! 而 `runtime/manifest.json` 跟着文件走，复制到哪都自洽。
//!
//! 因此本模块只做三件事：读文件、校验、给路径。
//! 它不探测、不缓存、不写库 —— 探测是 `probe` 的事，缓存是可丢弃的。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::paths::sanitize_rel;

pub const SCHEMA_VERSION: u32 = 1;

/// 组件种类。用于「项目 → 运行时」的绑定，以及决定私有状态的隔离方式。
///
/// 新增种类会导致旧版本解析失败（serde 对 enum 严格匹配）。这是故意的：
/// 宁可启动时报「清单有误」，也不要静默忽略一个自己看不懂的组件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Python,
    Node,
    Office,
    /// 固定版式文档阅读器（PDF / XPS / EPUB 等）。
    ///
    /// 与 `Office` 的区别不是「能不能看」，而是「看不看得懂」：Office 文档
    /// 是**可编辑的流式排版**，需要排版引擎重算版面；PDF 是**已排好的页面**，
    /// 只需要渲染。用 LibreOffice 开 PDF 也能开，但那等于把排版引擎当图片
    /// 查看器使唤 —— 慢、且会抢走文本选择与链接。
    Pdf,
    Tool,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Python => "python",
            Kind::Node => "node",
            Kind::Office => "office",
            Kind::Pdf => "pdf",
            Kind::Tool => "tool",
        }
    }

    /// 单例种类：整块盘只允许存在一个。
    ///
    /// 这条规则是**产品决定**，不是技术限制：盘里同时放两个 Python，
    /// 就必须回答「某个项目该用哪个」——而这个答案要么来自用户的显式配置
    /// （多一套配置要维护），要么来自「取第一个」的隐式约定（等于随机）。
    /// 后者表现为「昨天能跑、今天不能跑」，是最难排查的一类故障。
    ///
    /// 所以在这里把它变成**启动即失败**：清单写错了，宁可打不开，
    /// 也不要带着歧义跑起来。`tool` 不在此列（工具可以有多个，互不冲突）。
    ///
    /// `pdf` 的单例性比 Python 弱一些（选错阅读器不会毁数据，只是偏好不同），
    /// 但仍然单例：界面上的动作叫「用盘内阅读器打开」，这个措辞默认了唯一性。
    /// 允许两个就得在点击处加一个下拉选择 —— 为一个一次性选型问题，
    /// 在每次打开时都收一次税，不划算。
    pub fn singleton(self) -> bool {
        matches!(self, Kind::Python | Kind::Node | Kind::Office | Kind::Pdf)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Component {
    /// 组件标识，同时用作私有状态目录名。只允许 `[a-z0-9._-]`。
    pub id: String,
    pub kind: Kind,
    #[serde(default)]
    pub version: String,
    /// 相对 `runtime/` 的可执行文件路径
    pub exe: String,
    /// 版本探测参数。为空表示「不探测版本」，只检查文件存在。
    #[serde(default)]
    pub version_args: Vec<String>,
    /// 需要额外注入的环境变量（会覆盖默认沙箱值）
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub schema: u32,
    #[serde(default)]
    pub components: Vec<Component>,
}

impl Manifest {
    /// 取某个种类的组件。
    ///
    /// 因为 `validate` 已保证单例种类唯一，这里返回 `Option` 而非 `Vec`——
    /// 调用方不需要、也无法在「用哪个」上做选择。
    pub fn get(&self, kind: Kind) -> Option<&Component> {
        self.components.iter().find(|c| c.kind == kind)
    }

    /// 同类组件的可执行目录，用于 `PATH` 前置。
    pub fn bin_dirs(&self, runtime_dir: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for c in &self.components {
            if let Ok(exe) = c.exe_path(runtime_dir) {
                if let Some(parent) = exe.parent() {
                    let p = parent.to_path_buf();
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        out
    }

    fn validate(&self) -> AppResult<()> {
        if self.schema != SCHEMA_VERSION {
            return Err(AppError::Manifest(format!(
                "清单 schema 为 {}，本版本只认识 {}",
                self.schema, SCHEMA_VERSION
            )));
        }

        let mut seen: Vec<&str> = Vec::new();
        let mut kinds: Vec<Kind> = Vec::new();
        for c in &self.components {
            if c.id.trim().is_empty() {
                return Err(AppError::Manifest("存在 id 为空的组件".into()));
            }
            if !c.id.chars().all(|ch| {
                ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.')
            }) {
                return Err(AppError::Manifest(format!(
                    "组件 id「{}」只允许小写字母、数字与 - _ .",
                    c.id
                )));
            }
            if seen.contains(&c.id.as_str()) {
                return Err(AppError::Manifest(format!("组件 id「{}」重复", c.id)));
            }
            seen.push(&c.id);

            // 单例约束：见 `Kind::singleton` 的说明。
            if c.kind.singleton() {
                if kinds.contains(&c.kind) {
                    return Err(AppError::Manifest(format!(
                        "盘内只允许一个 {} 运行时（本盘策略：单一版本），但清单里声明了多个",
                        c.kind.as_str()
                    )));
                }
                kinds.push(c.kind);
            }

            // exe 走与前端路径同一套清洗：拒绝 `..`、绝对路径、盘符、ADS
            c.exe_path(Path::new(""))?;

            for key in c.env.keys() {
                if key.trim().is_empty() || key.contains('=') {
                    return Err(AppError::Manifest(format!(
                        "组件「{}」的环境变量名非法：{key}",
                        c.id
                    )));
                }
            }
            for a in &c.version_args {
                if a.len() > 200 {
                    return Err(AppError::Manifest(format!(
                        "组件「{}」的探测参数过长",
                        c.id
                    )));
                }
            }
        }

        Ok(())
    }
}

impl Component {
    /// 可执行文件的绝对路径。绝对路径由 `runtime_dir` 推导，清单里只存相对量。
    pub fn exe_path(&self, runtime_dir: &Path) -> AppResult<PathBuf> {
        let rel = sanitize_rel(&self.exe)?;
        if rel.as_os_str().is_empty() {
            return Err(AppError::Manifest(format!(
                "组件「{}」的 exe 为空",
                self.id
            )));
        }
        Ok(runtime_dir.join(rel))
    }
}

/// 读取并校验清单。
///
/// 返回 `Ok(None)` 表示「盘里没有清单」—— 这是合法状态（纯文件库也能用），
/// 不是错误。真正的错误（JSON 坏了、schema 不懂）才走 `Err`。
pub fn load(runtime_dir: &Path) -> AppResult<Option<Manifest>> {
    let path = runtime_dir.join("manifest.json");
    if !path.is_file() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&path)?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| AppError::Manifest(format!("manifest.json 解析失败：{e}")))?;
    manifest.validate()?;
    Ok(Some(manifest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(id: &str, exe: &str) -> Component {
        Component {
            id: id.to_string(),
            kind: Kind::Python,
            version: "3.12".into(),
            exe: exe.to_string(),
            version_args: vec!["--version".into()],
            env: BTreeMap::new(),
            note: String::new(),
        }
    }

    fn manifest_of(components: Vec<Component>) -> Manifest {
        Manifest {
            schema: SCHEMA_VERSION,
            components,
        }
    }

    #[test]
    fn rejects_escaping_exe() {
        for bad in [
            "../python.exe",
            "C:/Windows/system32/cmd.exe",
            "/python.exe",
            "a.exe:ads",
        ] {
            let m = manifest_of(vec![comp("py", bad)]);
            assert!(m.validate().is_err(), "应拒绝：{bad}");
        }
    }

    #[test]
    fn rejects_duplicate_id() {
        let m = manifest_of(vec![
            comp("py", "python/python.exe"),
            comp("py", "python2/python.exe"),
        ]);
        assert!(m.validate().is_err());
    }

    #[test]
    fn rejects_unknown_schema() {
        let mut m = manifest_of(vec![]);
        m.schema = 99;
        assert!(m.validate().is_err());
    }

    #[test]
    fn rejects_two_pythons() {
        // 产品决定：盘内只放一个 Python 版本。写成两个必须在启动时就失败。
        let m = manifest_of(vec![
            comp("python", "python/python.exe"),
            comp("python2", "python2/python.exe"),
        ]);
        assert!(m.validate().is_err(), "两个 Python 应被拒绝");
    }

    #[test]
    fn rejects_two_pdf_readers() {
        // 界面上只有一个「用盘内阅读器打开」，两个候选等于让点击去猜。
        let mut a = comp("pdf", "pdf/SumatraPDF.exe");
        a.kind = Kind::Pdf;
        let mut b = comp("pdf-alt", "pdf/other.exe");
        b.kind = Kind::Pdf;
        assert!(
            manifest_of(vec![a, b]).validate().is_err(),
            "两个 PDF 阅读器应被拒绝"
        );
    }

    #[test]
    fn allows_multiple_tools() {
        // tool 不是单例：工具可以有多个，互不冲突。
        let mut a = comp("tool-a", "tools/a.exe");
        a.kind = Kind::Tool;
        let mut b = comp("tool-b", "tools/b.exe");
        b.kind = Kind::Tool;
        assert!(manifest_of(vec![a, b]).validate().is_ok());
    }

    #[test]
    fn repo_manifest_matches_this_build() {
        // 仓库里那份清单本身是交付物：`Kind` 改了名而清单没同步，表现是
        // 「发出去的盘启动即报清单有误」。这条测试把它挡在提交之前。
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../runtime/manifest.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return; // 裁剪过的环境里允许没有这份文件
        };
        let m: Manifest = serde_json::from_str(&text).expect("runtime/manifest.json 解析失败");
        m.validate().expect("runtime/manifest.json 校验失败");
        assert!(m.get(Kind::Pdf).is_some(), "清单里应声明 pdf 组件");
    }

    #[test]
    fn bin_dirs_are_deduped() {
        let m = manifest_of(vec![
            comp("py", "python/python.exe"),
            comp("py2", "python/pythonw.exe"),
            comp("node", "node/node.exe"),
        ]);
        let dirs = m.bin_dirs(Path::new("/d"));
        assert_eq!(dirs.len(), 2);
    }
}
