//! 盘内 PDF 阅读器（SumatraPDF）的参数构造。
//!
//! 和 LibreOffice 一样，「配置不落宿主」在这里也只能靠组件自带的命令行开关，
//! 但两者的机制完全不同，值得并排列出来：
//!
//! | | LibreOffice | SumatraPDF |
//! | --- | --- | --- |
//! | 开关 | `-env:UserInstallation=file:///…` | `-appdata <目录>` |
//! | 取值形态 | **必须是 `file://` URL** | **必须是本地路径**，不是 URL |
//! | 管到的东西 | profile 目录（配置 + 锁文件） | `SumatraPDF-settings.txt` + 缩略图缓存 |
//!
//! 这两格差别不是趣味点：把 `-appdata` 按 LibreOffice 的写法填成
//! `file:///F:/…`，SumatraPDF 会当成一个不存在的目录、**静默回落**到
//! `%APPDATA%\SumatraPDF`，界面上没有任何报错。等到某天在宿主 C 盘看到
//! 那个 settings 文件，才知道隔离从来没生效过。
//!
//! 第二个决定：**不传 `-reuse-instance`**。
//! 复用已有实例（DDE）会把文件交给「先到的那一个」进程 —— 而那个进程
//! 未必是本盘启动的（用户可能自己装过 SumatraPDF），于是文件被开到宿主的
//! 实例里，用的是宿主的配置文件，本盘的 `-appdata` 形同虚设。
//! 隔离在这里比「少开一个窗口」重要，所以宁可每次都起一个新进程。

use std::path::Path;

/// 阅读器能接的扩展名。前列是本职，后面几个是 SumatraPDF 顺带支持、
/// 而宿主上未见得有程序能开的格式（电子书、漫画包、帮助文件）。
///
/// 只列**不会与盘内 Office 抢**的格式：`.xps` 与 `.oxps` 是固定版式文档，
/// 和 PDF 同族；`.epub/.mobi/.cbz/.chm/.djvu` 都没有更合适的便携查看器。
pub const EXTS: &[&str] = &["pdf", "xps", "oxps", "epub", "mobi", "cbz", "chm", "djvu"];

pub fn matches_ext(ext: Option<&str>) -> bool {
    ext.is_some_and(|e| EXTS.contains(&e))
}

/// 打开一个文档的参数。
///
/// `state_dir` 是盘内私有状态目录（`.library/state/pdf`），由根推导，
/// 不写死在清单里 —— 换盘符后仍然指向当前这块盘。
pub fn open_args(state_dir: &Path, file: &Path) -> Vec<String> {
    vec![
        "-appdata".to_string(),
        state_dir.to_string_lossy().into_owned(),
        // 每个文件独立窗口：避免被已有实例（可能是宿主那份）接管成标签页，
        // 也避免用户关掉一个文档时发现另外三个一起没了。
        "-new-window".to_string(),
        file.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn appdata_uses_plain_path_not_file_url() {
        // 这条断言守的是本文档开头那个坑：写成 file:/// 会静默失效。
        let args = open_args(
            &PathBuf::from("F:/盘/.library/state/pdf"),
            &PathBuf::from("F:/盘/content/a.pdf"),
        );
        assert_eq!(args[0], "-appdata");
        assert_eq!(args[1], "F:/盘/.library/state/pdf");
        assert!(!args[1].starts_with("file:"));
    }

    #[test]
    fn file_comes_last() {
        // SumatraPDF 把「不认识的参数」当文件路径，顺序错了会把目录当文档打开。
        let args = open_args(&PathBuf::from("/d/state"), &PathBuf::from("/d/a.pdf"));
        assert_eq!(args.last().map(String::as_str), Some("/d/a.pdf"));
        assert_eq!(args.iter().filter(|a| a.ends_with(".pdf")).count(), 1);
    }

    #[test]
    fn matches_case_insensitively_after_lowercasing() {
        // 上游已把扩展名转小写；这里只确认集合本身不漏。
        assert!(matches_ext(Some("pdf")));
        assert!(matches_ext(Some("epub")));
        assert!(!matches_ext(Some("docx")));
        assert!(!matches_ext(None));
    }
}
