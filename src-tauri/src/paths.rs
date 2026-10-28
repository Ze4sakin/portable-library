//! 根目录上下文 —— 全系统「盘符漂移」的唯一真相源。
//!
//! 不变量（Invariant）：
//! 1. 只有本模块允许推导盘根路径。其他模块一律接收 `RootCtx`，不得自行
//!    `current_exe()` 或拼接绝对路径。
//! 2. 所有对外暴露的路径都是**相对 content 根**的字符串，分隔符统一为 `/`。
//! 3. 所有来自前端的相对路径必须经 `resolve()` 校验后才能落到文件系统。
//!
//! 这样做的收益：把「换盘符」这件事压缩成一处逻辑，出问题只在一个地方查。

use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult};

/// 允许覆盖根目录的环境变量（仅开发期使用）。
///
/// 开发时 `current_exe()` 指向 `target/debug/`，与发行版布局不同，
/// 因此用环境变量显式指定一个仿真盘根，而不是在代码里写「向上找两级」的
/// 猜测逻辑——猜测逻辑一旦存在，发行版的稳定性就要打折。
pub const ENV_ROOT_OVERRIDE: &str = "PL_ROOT";

#[derive(Debug, Clone)]
pub struct RootCtx {
    root: PathBuf,
    content: PathBuf,
    volume_serial: String,
}

impl RootCtx {
    /// 从可执行文件位置推导盘根。这是全系统唯一一次「猜路径」。
    pub fn detect() -> AppResult<Self> {
        let from_env = match std::env::var(ENV_ROOT_OVERRIDE) {
            Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v)),
            _ => None,
        };
        Self::detect_in(from_env)
    }

    /// 显式指定盘根；`None` 时退回「可执行文件所在目录」。
    ///
    /// 拆出这一层不是为测试方便，而是为了让「盘根从哪来」变成一个参数而非
    /// 一个隐式的全局状态：测试、将来的「打开别的盘」、以及开发期仿真，
    /// 都走同一条构造路径，就不会出现两套行为。
    pub fn detect_in(explicit: Option<PathBuf>) -> AppResult<Self> {
        let root = match explicit {
            Some(p) => p,
            None => {
                let exe = std::env::current_exe()?;
                exe.parent()
                    .ok_or_else(|| AppError::PathEscape("无法定位可执行文件所在目录".into()))?
                    .to_path_buf()
            }
        };

        let content = root.join("content");
        std::fs::create_dir_all(&content)?;
        std::fs::create_dir_all(root.join(".library"))?;
        std::fs::create_dir_all(root.join("runtime"))?;
        std::fs::create_dir_all(root.join(".library").join("home").join("temp"))?;
        std::fs::create_dir_all(root.join(".library").join("cache"))?;

        let volume_serial = volume_serial_of(&root).unwrap_or_else(|| "UNKNOWN".to_string());

        Ok(Self {
            root,
            content,
            volume_serial,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 用户内容区。索引、扫描、打开器的作用域都以它为根。
    pub fn content(&self) -> &Path {
        &self.content
    }

    pub fn volume_serial(&self) -> &str {
        &self.volume_serial
    }

    pub fn db_path(&self) -> PathBuf {
        self.root.join(".library").join("catalog.db")
    }

    /// 盘内运行时区。**只读资产**：可重建（重下组件）、不进备份。
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    /// 系统区。索引、缓存、日志、子进程 HOME 都在这里。
    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".library")
    }

    /// 子进程的「假 HOME」。便携软件的家目录在此，而非宿主的 `C:\Users`。
    pub fn home_dir(&self) -> PathBuf {
        self.root.join(".library").join("home")
    }

    /// 派生产物（PDF 预览等）。随时可删，删了自动重建。
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(".library").join("cache")
    }

    /// 某个组件的私有状态目录（LibreOffice 的 `UserInstallation` 等）。
    ///
    /// `id` 已在清单校验阶段限制为 `[a-z0-9._-]`，因此这里不做二次清洗；
    /// 若将来放开该校验，此处必须补上 `sanitize_rel`。
    pub fn state_component_dir(&self, id: &str) -> PathBuf {
        self.root.join(".library").join("state").join(id)
    }

    /// 缓存区路径：相对 `.library/cache/`。
    pub fn cache_path(&self, rel: &str) -> AppResult<PathBuf> {
        Ok(self.cache_dir().join(sanitize_rel(rel)?))
    }

    /// 相对路径 → 绝对路径，并做越界校验。
    ///
    /// 三重防线：字符串层拒绝 `..` / 绝对路径 / 冒号；存在性校验时再做
    /// canonicalize 比对，防止符号链接与 8.3 短名绕过。
    pub fn resolve(&self, rel: &str) -> AppResult<PathBuf> {
        let joined = self.content.join(sanitize_rel(rel)?);

        if joined.exists() {
            let canon_root = self.content.canonicalize()?;
            let canon = joined.canonicalize()?;
            if !canon.starts_with(&canon_root) {
                return Err(AppError::PathEscape(rel.to_string()));
            }
            Ok(canon)
        } else {
            Ok(joined)
        }
    }

    /// 文件所在目录。
    ///
    /// **刻意不走 `resolve()`**：后者对已存在的路径做 `canonicalize`，Windows 上
    /// 会带上 verbatim 前缀（`\\?\F:\…`）。这个值一旦拼进 `http://asset.localhost/…`
    /// 就失效（asset 协议不认 verbatim），而失败方式是**静默 404** ——
    /// 界面表现只是「样式和图片都没了」，最难查的一类 bug。
    ///
    /// 越界防线没有因此削弱：`sanitize_rel` 仍然拒 `..`、绝对路径、盘符与数据流，
    /// 结果必然落在 `content/` 之下；而 asset 协议的作用域也只放行 `content/`。
    pub fn dir_of(&self, rel: &str) -> AppResult<PathBuf> {
        let joined = self.content.join(sanitize_rel(rel)?);
        Ok(match joined.parent() {
            Some(dir) => dir.to_path_buf(),
            None => self.content.clone(),
        })
    }

    /// 绝对路径 → 相对路径（统一 `/` 分隔）。
    ///
    /// 当前无调用方，但保留：这是 `resolve()` 的严格逆运算，也是「把盘外
    /// 绝对路径映射回库内相对路径」唯一正确的写法（canonicalize + strip_prefix，
    /// 少了任一步都会被符号链接绕过）。将来做导入 / 拖拽入盘时必须用它，
    /// 而不是各处自己 `strip_prefix`。
    #[allow(dead_code)]
    pub fn relativize(&self, abs: &Path) -> AppResult<String> {
        let canon_root = self.content.canonicalize()?;
        let canon = abs.canonicalize()?;
        let rel = canon
            .strip_prefix(&canon_root)
            .map_err(|_| AppError::PathEscape(abs.display().to_string()))?;
        Ok(to_slash(rel))
    }
}

/// 把前端传来的相对路径清洗成安全的 PathBuf。
pub fn sanitize_rel(rel: &str) -> AppResult<PathBuf> {
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(AppError::PathEscape(rel.to_string()));
    }

    let mut out = PathBuf::new();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return Err(AppError::PathEscape(rel.to_string())),
            // 冒号既能表示盘符（C:）也能表示 NTFS 备用数据流（a.txt:hidden）
            p if p.contains(':') => return Err(AppError::PathEscape(rel.to_string())),
            p => out.push(p),
        }
    }

    if rel.trim().is_empty() {
        return Err(AppError::PathEscape("空路径".into()));
    }
    Ok(out)
}

pub fn to_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 从相对路径推导「从属链」：由外到内的每一级 `(相对路径, 名字)`，**不含根**。
///
/// 纯字符串运算，不查库、不问磁盘。面包屑导航要的是「路径本身表达的层级」，
/// 而不是「库里恰好有哪些祖先节点」—— 前者永远自洽，后者可能因为索引滞后
/// 而缺一级，缺了就会出现「点了面包屑跳不过去」。
pub fn ancestors(rel: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut acc = String::new();
    for seg in rel.split('/') {
        if seg.is_empty() {
            continue;
        }
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(seg);
        out.push((acc.clone(), seg.to_string()));
    }
    out
}

/// 读取卷序列号。
///
/// 这是「认盘不认盘符」的物理依据：盘符会变，卷序列号不会。
/// 取不到时退化为 UNKNOWN，不阻塞启动。
#[cfg(windows)]
fn volume_serial_of(p: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumeInformationW(
            lp_root_path_name: *const u16,
            lp_volume_name_buffer: *mut u16,
            n_volume_name_size: u32,
            lp_volume_serial_number: *mut u32,
            lp_maximum_component_length: *mut u32,
            lp_file_system_flags: *mut u32,
            lp_file_system_name_buffer: *mut u16,
            n_file_system_name_size: u32,
        ) -> i32;
    }

    let mut drive = match p.components().next() {
        Some(Component::Prefix(prefix)) => PathBuf::from(prefix.as_os_str()),
        _ => return None,
    };
    // GetVolumeInformationW 需要 "F:\" 这种带反斜杠的根路径
    drive.push("\\");

    let mut wide: Vec<u16> = drive.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut serial: u32 = 0;
    let ok = unsafe {
        GetVolumeInformationW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };

    if ok == 0 {
        None
    } else {
        Some(format!("{serial:08X}"))
    }
}

#[cfg(not(windows))]
fn volume_serial_of(_p: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal() {
        assert!(sanitize_rel("../etc/passwd").is_err());
        assert!(sanitize_rel("a/../../b").is_err());
        assert!(sanitize_rel("C:/Windows").is_err());
        assert!(sanitize_rel("a.txt:hidden").is_err());
        assert!(sanitize_rel("/abs").is_err());
    }

    #[test]
    fn accepts_normal() {
        assert_eq!(to_slash(&sanitize_rel("a/b/c.md").unwrap()), "a/b/c.md");
        assert_eq!(to_slash(&sanitize_rel("./a//b").unwrap()), "a/b");
        assert_eq!(
            to_slash(&sanitize_rel("中文 目录/x.docx").unwrap()),
            "中文 目录/x.docx"
        );
    }

    #[test]
    fn dir_of_has_no_verbatim_prefix() {
        // 这条测试守的是一个曾经真踩到的坑：`dir_of` 若改回 `resolve()`，
        // Windows 上会带 `\\?\` 前缀，前端拼出的 `<base href>` 让样式与图片
        // 全部静默 404 —— 界面只表现为「没有样式的裸 HTML」。
        let root = std::env::temp_dir().join(format!("pl-dir-of-{}", std::process::id()));
        let ctx = RootCtx::detect_in(Some(root)).expect("构造盘根");

        let dir = to_slash(&ctx.dir_of("a/b/c.md").expect("取目录"));
        assert!(
            dir.starts_with(&to_slash(ctx.content())),
            "越出内容区：{dir}"
        );
        assert!(dir.ends_with("/a/b"), "目录层级不对：{dir}");
        assert!(!dir.contains("//?"), "带了 verbatim 前缀：{dir}");

        assert!(ctx.dir_of("../x").is_err());
        assert!(ctx.dir_of("C:/Windows/x.md").is_err());
    }

    #[test]
    fn breadcrumb_chain() {
        assert_eq!(
            ancestors("工作/2026-Q4/报告.md"),
            vec![
                ("工作".to_string(), "工作".to_string()),
                ("工作/2026-Q4".to_string(), "2026-Q4".to_string()),
                ("工作/2026-Q4/报告.md".to_string(), "报告.md".to_string()),
            ]
        );
        assert!(ancestors("").is_empty());
        assert_eq!(ancestors("a"), vec![("a".to_string(), "a".to_string())]);
    }
}
