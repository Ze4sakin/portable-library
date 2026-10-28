//! 盘内编辑器（Notepad++ + PreviewHTML 插件）的参数构造。
//!
//! 它和 SumatraPDF 解决的是同一件事（「配置不落宿主」），但手段完全不同，
//! 并排列出来才看得出哪一格是厂商给的、哪一格是我们自己补的：
//!
//! | | SumatraPDF | Notepad++ |
//! | --- | --- | --- |
//! | 配置隔离 | `-appdata <本地路径>` | **没有命令行开关**，靠 `doLocalConf.xml` 标记文件 |
//! | 实例复用 | 不传 `-reuse-instance`（默认就复用） | 传 `-multiInst`（默认会复用） |
//! | 会话 | 无此概念 | 传 `-nosession` |
//!
//! ## 为什么没有 `-settingsDir`
//!
//! Notepad++ 在 8.x 里**确实没有**这个开关 —— 在二进制里搜过，只有 `-multiInst`。
//! 想让它不写 `%APPDATA%\Notepad++`，唯一途径是「exe 同目录存在 `doLocalConf.xml`」
//! 这个厂商自带的便携标记。这是好消息（不需要我们传参），也是唯一的坏消息：
//! 配置只能落在 `runtime/Notepad++/` 里，没法像 LibreOffice 那样外移到 `.library/`。
//!
//! 所以 `runtime/` 的「只读资产」这条规矩在这里有一个**由厂商设计决定的例外**：
//! 它会被写入 `config.xml` / `session.xml` / `langs.xml` / `stylers.xml`。
//! 这些文件因此不该进版本库（见 `.gitignore`），而标记文件本身必须随盘 ——
//! 少了它，程序会静默退回宿主配置目录，界面上没有任何提示。
//!
//! ## 为什么必须给 `-multiInst`
//!
//! Notepad++ 的默认行为是「再开一个文件就交给已在跑的那个实例」，通过窗口消息
//! 找窗口，**不区分那个实例是从哪个 exe 启动的**。后果是：宿主机自己装过
//! Notepad++ 且正在运行时，我们从盘里启动的进程会把文件转交给宿主那个窗口 ——
//! 用的是宿主的配置、宿主的插件集，本盘的 `doLocalConf.xml` 和 PreviewHTML
//! 全都形同虚设，而且用户完全看不出发生了什么。
//!
//! `-multiInst` 强制起新进程，把「一定是本盘这一份」变成结构保证。
//! 代价是每点一次多一个窗口，而不是多一个标签页 —— 这里选了隔离。

use std::path::Path;

/// 接管「查看 + 编辑」的格式。
///
/// 只列 `.md` / `.html` 这两种：它们的共同点是「源文本可读，但排版效果需要渲染」
/// —— 正好是编辑器的强项（源码）+ PreviewHTML 的强项（渲染）叠加的地方。
///
/// 刻意**不收**的格式：
/// - `.txt` `.json` `.log` 等：内置预览已经够用，拉进编辑器只是多一次启动
/// - `.docx` `.xlsx`：流式排版，该走 LibreOffice
/// - `.pdf`：已排好版，该走阅读器
pub const EXTS: &[&str] = &[
    "md", "markdown", "mdown", "mkd", "html", "htm", "xhtml", "shtml",
];

pub fn matches_ext(ext: Option<&str>) -> bool {
    ext.is_some_and(|e| EXTS.contains(&e))
}

/// 打开一个文件的参数。
///
/// 顺序有讲究：Notepad++ 把不认识的参数当文件路径，所以文件必须放最后，
/// 开关在前 —— 与 `pdf::open_args` 同一条规矩。
pub fn open_args(file: &Path) -> Vec<String> {
    vec![
        // 见模块注释：不开这个，文件可能被宿主那份实例接走。
        "-multiInst".to_string(),
        // session.xml 记的是绝对路径，换盘符/换机器后是一堆打不开的标签页。
        "-nosession".to_string(),
        file.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn isolates_instance_and_skips_session() {
        // 这两条是「便携」能不能成立的全部：没有 -multiInst 会被宿主实例接走，
        // 没有 -nosession 会拖着上一个盘符的会话进来。
        let args = open_args(&PathBuf::from("F:/盘/content/a.md"));
        assert!(args.contains(&"-multiInst".to_string()), "必须隔离实例");
        assert!(args.contains(&"-nosession".to_string()), "必须跳过会话");
    }

    #[test]
    fn file_comes_last() {
        // 认不出的参数会被当成路径；顺序错了会把开关当文件打开。
        let args = open_args(&PathBuf::from("F:/盘/content/a.html"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("F:/盘/content/a.html")
        );
        assert!(!args[0].starts_with("F:"));
    }

    #[test]
    fn matches_only_editor_formats() {
        assert!(matches_ext(Some("md")));
        assert!(matches_ext(Some("html")));
        assert!(matches_ext(Some("htm")));
        // 内置预览够用的纯文本不该抢
        assert!(!matches_ext(Some("txt")));
        // 排版格式走自己的链路
        assert!(!matches_ext(Some("docx")));
        assert!(!matches_ext(Some("pdf")));
        assert!(!matches_ext(None));
    }

    #[test]
    fn path_with_space_and_plus_is_passed_verbatim() {
        // runtime 目录名是 `Notepad++`，盘根也可能带空格；这里确认不做任何改写。
        let args = open_args(&PathBuf::from("F:/我的 盘/content/说明 文档.md"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("F:/我的 盘/content/说明 文档.md")
        );
    }
}
