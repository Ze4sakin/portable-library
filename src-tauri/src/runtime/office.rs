//! 便携 LibreOffice 的两种用法：接管编辑、无头转换。
//!
//! 这一节是全系统「隔离」最典型的一个反例，值得记下来：
//!
//! **LibreOffice 不看 `%APPDATA%`。** 它有自己的 `UserInstallation` 概念，
//! 默认指向 `%APPDATA%\LibreOffice\4\user`，路径由程序内部拼出，环境变量无效。
//! 唯一的重定向手段是命令行参数：
//!
//! ```text
//! soffice "-env:UserInstallation=file:///F:/盘/.library/state/libreoffice" ...
//! ```
//!
//! 这一条同时解决了三个问题：
//! 1. 配置不落到宿主 C 盘（便携的本意）
//! 2. 用户机器上开着 LibreOffice 时，无头转换不会因为 profile 被锁而
//!    **静默退出、退出码还是 0**（这是社区里最常见的「转换没反应」根因）
//! 3. 多个转换进程可以并行，各自持独立 profile
//!
//! 第二个坑必须用代码兜住：`--convert-to` 的退出码不可信，成功与否只能
//! 看输出文件是否存在且非空。本模块因此不检查退出码，只检查产物。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::paths::RootCtx;
use crate::runtime::env::EnvPlan;
use crate::runtime::launch::{self, Spec};
use crate::runtime::manifest::Component;

/// 单文档转换上限。低于 30 s 会误杀大 PPT，高于 90 s 用户已经在骂了。
pub const CONVERT_TIMEOUT: Duration = Duration::from_secs(90);

/// 接管编辑：带独立 profile 打开文件。
pub fn open_args(state_dir: &Path, file: &Path) -> Vec<String> {
    vec![
        format!("-env:UserInstallation={}", to_file_url(state_dir)),
        file.to_string_lossy().into_owned(),
    ]
}

/// 无头转 PDF，产物落在 `.library/cache/pdf/<源路径哈希>/` 下。
///
/// 用哈希分目录而不是直接按文件名：`content/a/report.docx` 与
/// `content/b/report.docx` 同名，平铺会互相覆盖，而覆盖是**静默**的错误。
pub fn convert_to_pdf(root: &RootCtx, component: &Component, src: &Path) -> AppResult<PathBuf> {
    let exe = component.exe_path(&root.runtime_dir())?;
    if !exe.is_file() {
        return Err(AppError::ComponentMissing(component.exe.clone()));
    }

    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".to_string());

    let out_dir = root
        .cache_dir()
        .join("pdf")
        .join(short_hash(&src.to_string_lossy()))
        .join(&stem);
    std::fs::create_dir_all(&out_dir)?;

    let state_dir = root.state_component_dir(&component.id);
    let bin_dir = exe
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| root.runtime_dir());
    let plan = EnvPlan::build(root, &[bin_dir], &component.env);

    let args = vec![
        format!("-env:UserInstallation={}", to_file_url(&state_dir)),
        "--headless".to_string(),
        "--norestore".to_string(),
        "--nolockcheck".to_string(),
        "--nodefault".to_string(),
        "--nofirststartwizard".to_string(),
        "--convert-to".to_string(),
        "pdf".to_string(),
        "--outdir".to_string(),
        out_dir.to_string_lossy().into_owned(),
        src.to_string_lossy().into_owned(),
    ];

    let spec = Spec {
        program: exe,
        args,
        cwd: root.root().to_path_buf(),
        plan,
        hide: true,
    };

    let out = launch::run_capture(&spec, CONVERT_TIMEOUT)?;
    if out.timed_out {
        return Err(AppError::Timeout(CONVERT_TIMEOUT.as_millis() as u64));
    }

    let produced = out_dir.join(format!("{stem}.pdf"));
    match std::fs::metadata(&produced) {
        Ok(meta) if meta.len() > 0 => Ok(produced),
        _ => Err(AppError::Launch(format!(
            "转换未产出文件（退出码 {:?}）。注意：LibreOffice 退出码 0 并不代表成功。{}",
            out.code,
            launch::tail(&format!("{}{}", out.stdout, out.stderr), 300)
        ))),
    }
}

/// 本地路径 → `file://` URL。
///
/// 三个必须处理的点：
/// - 反斜杠必须换成 `/`，否则 LibreOffice 可能把 `\` 当转义
/// - 盘符路径要补成第三根斜杠：`F:/a` → `file:///F:/a`
/// - 空格 / `%` / `#` / `?` 必须百分号编码，否则带空格的目录会解析错
///
/// 非 ASCII 字符保持原样（LibreOffice 按 UTF-8 解析），不额外转义 ——
/// 转义成 `%E4%BE%BF` 反而在部分版本上会导致路径找不到。
pub fn to_file_url(p: &Path) -> String {
    let raw = p.to_string_lossy().replace('\\', "/");

    if let Some(rest) = raw.strip_prefix("//") {
        // UNC：\\server\share\x → file://server/share/x
        format!("file://{}", encode_path(rest))
    } else if raw.starts_with('/') {
        format!("file://{}", encode_path(&raw))
    } else {
        format!("file:///{}", encode_path(&raw))
    }
}

fn encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            c => out.push(c),
        }
    }
    out
}

/// FNV-1a 32 位。只用来做缓存分目录，不承担任何安全职责。
fn short_hash(s: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for b in s.as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_url_for_drive_path() {
        assert_eq!(
            to_file_url(Path::new("F:/盘/.library/state/libreoffice")),
            "file:///F:/盘/.library/state/libreoffice"
        );
    }

    #[test]
    fn file_url_encodes_spaces() {
        assert_eq!(
            to_file_url(Path::new("F:/my disk/office")),
            "file:///F:/my%20disk/office"
        );
    }

    #[test]
    fn file_url_normalizes_backslashes() {
        assert_eq!(
            to_file_url(Path::new(r"F:\work\state")),
            "file:///F:/work/state"
        );
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(short_hash("content/a.md"), short_hash("content/a.md"));
        assert_ne!(short_hash("content/a.md"), short_hash("content/b.md"));
    }
}
