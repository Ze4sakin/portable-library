//! 打开器路由 —— 对应讲解文档里的 `open(target, mode)` 动作模型。
//!
//! P1 只有两种外部打开方式（系统默认程序 / 资源管理器定位）。
//! P2 接入便携运行时后，在这里追加实现即可，命令层不用改：
//!
//! ```ignore
//! registry.register(Box::new(LibreOfficeOpener::new(&root, "runtime/office/soffice.exe")));
//! registry.register(Box::new(PortablePythonOpener::new(&root, "runtime/python/python.exe")));
//! ```
//!
//! 关键点：`Opener` 拿到的是**已解析的绝对路径**和 `RootCtx`，
//! 因此便携组件的可执行文件路径同样由根推导，不会写死盘符。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::paths::RootCtx;
use crate::runtime::env::EnvPlan;
use crate::runtime::launch::{self, Spec};
use crate::runtime::manifest::{Component, Kind, Manifest};
use crate::runtime::notepadpp;
use crate::runtime::office;
use crate::runtime::pdf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// 内置预览（前端渲染），不启动外部进程
    Preview,
    /// 用外部编辑器打开
    Edit,
    /// 格式转换（P2：LibreOffice headless）
    Convert,
    /// 在资源管理器中定位
    Reveal,
}

impl OpenMode {
    pub fn parse(s: &str) -> AppResult<Self> {
        match s.to_ascii_lowercase().as_str() {
            "preview" => Ok(OpenMode::Preview),
            "edit" => Ok(OpenMode::Edit),
            "convert" => Ok(OpenMode::Convert),
            "reveal" => Ok(OpenMode::Reveal),
            other => Err(AppError::Unimplemented(match other {
                "open" => "无 mode 的裸打开",
                _ => "未知的打开模式",
            })),
        }
    }
}

/// 打开结果：前端据此决定是自行渲染还是提示「已交给外部程序」。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum OpenOutcome {
    Internal,
    External { program: String },
}

pub trait Opener: Send + Sync {
    fn id(&self) -> &'static str;
    /// 是否能处理该扩展名（不含点，已转小写）
    fn matches(&self, ext: Option<&str>) -> bool;
    fn launch(&self, abs: &Path) -> AppResult<()>;
}

pub struct SystemDefaultOpener;

impl Opener for SystemDefaultOpener {
    fn id(&self) -> &'static str {
        "system-default"
    }
    fn matches(&self, _ext: Option<&str>) -> bool {
        true
    }
    fn launch(&self, abs: &Path) -> AppResult<()> {
        shell_open(abs)
    }
}

pub struct RevealOpener;

impl Opener for RevealOpener {
    fn id(&self) -> &'static str {
        "reveal"
    }
    fn matches(&self, _ext: Option<&str>) -> bool {
        true
    }
    fn launch(&self, abs: &Path) -> AppResult<()> {
        shell_reveal(abs)
    }
}

/// 便携 Office 接管编辑。
///
/// 与 `SystemDefaultOpener` 的差别只有两点，但那两点恰好是「便携」的全部：
/// 1. 用盘内 `soffice.exe`，不依赖宿主的文件关联（宿主可能根本没装 Office）
/// 2. 带 `-env:UserInstallation`，把配置钉在盘内，不写宿主的 `%APPDATA%`
///
/// 注意它不是「更好的打开方式」而是「更稳的打开方式」：
/// WPS / Office 的界面体验确实更好，但它们的配置一定落在 C 盘。
pub struct OfficeOpener {
    root: RootCtx,
    exe: PathBuf,
    state_dir: PathBuf,
}

impl OfficeOpener {
    pub fn new(root: &RootCtx, component: &Component) -> AppResult<Self> {
        let exe = component.exe_path(&root.runtime_dir())?;
        Ok(Self {
            root: root.clone(),
            exe,
            state_dir: root.state_component_dir(&component.id),
        })
    }
}

impl Opener for OfficeOpener {
    fn id(&self) -> &'static str {
        "portable-office"
    }

    fn matches(&self, ext: Option<&str>) -> bool {
        matches!(
            ext,
            Some("doc" | "docx" | "rtf" | "odt" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp")
        )
    }

    fn launch(&self, abs: &Path) -> AppResult<()> {
        let bin_dir = self
            .exe
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.root.runtime_dir());
        let plan = EnvPlan::build(&self.root, &[bin_dir], &BTreeMap::new());
        let spec = Spec {
            program: self.exe.clone(),
            args: office::open_args(&self.state_dir, abs),
            cwd: self.root.root().to_path_buf(),
            plan,
            hide: true,
        };
        launch::spawn_detached(&spec)
    }
}

/// 盘内 PDF 阅读器接管查看。
///
/// 与 `OfficeOpener` 的区别是链路更短：PDF 是**已经排好版**的页面，
/// 不需要「转一道再看」，直接交给阅读器就行。所以这里不给「转 PDF」入口，
/// 也不做任何缓存 —— 中间产物在这里没有存在的理由。
pub struct PdfOpener {
    root: RootCtx,
    exe: PathBuf,
    state_dir: PathBuf,
}

impl PdfOpener {
    pub fn new(root: &RootCtx, component: &Component) -> AppResult<Self> {
        let exe = component.exe_path(&root.runtime_dir())?;
        Ok(Self {
            root: root.clone(),
            exe,
            state_dir: root.state_component_dir(&component.id),
        })
    }
}

impl Opener for PdfOpener {
    fn id(&self) -> &'static str {
        "portable-pdf"
    }

    fn matches(&self, ext: Option<&str>) -> bool {
        pdf::matches_ext(ext)
    }

    fn launch(&self, abs: &Path) -> AppResult<()> {
        // `-appdata` 指向的目录不存在时，SumatraPDF 的行为是「当作没设」，
        // 又静默回到宿主 %APPDATA%。建目录是最便宜的一步防线，
        // 而且失败要在这里暴露，而不是变成「设置某天没生效」。
        std::fs::create_dir_all(&self.state_dir)?;

        let bin_dir = self
            .exe
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.root.runtime_dir());
        let plan = EnvPlan::build(&self.root, &[bin_dir], &BTreeMap::new());
        let spec = Spec {
            program: self.exe.clone(),
            args: pdf::open_args(&self.state_dir, abs),
            cwd: self.root.root().to_path_buf(),
            plan,
            hide: true,
        };
        launch::spawn_detached(&spec)
    }
}

/// 盘内编辑器接管 `.md` / `.html` 的查看与编辑。
///
/// 它和 `PdfOpener` 的差别只有一处，但那处决定了「预览」这件事归谁：
/// PDF 的渲染由阅读器完成，我们只要把文件递过去；而 HTML/Markdown 的渲染
/// 由 Notepad++ 的 PreviewHTML **插件**完成，插件在进程内、没有命令行入口，
/// 所以这里能做的最多是「把文件正确地在盘内那份编辑器里打开」——
/// 让渲染发生的那一步必须由用户在界面里点（详见 `runtime::notepadpp`）。
///
/// 这不是偷懒：Notepad++ 没有「启动时执行某个插件命令」的机制，
/// 硬凑（比如自己发窗口消息模拟点击）会把一个稳定的启动路径绑死在
/// 某个 UI 布局上，插件一改版就断。
pub struct NotepadOpener {
    root: RootCtx,
    exe: PathBuf,
}

impl NotepadOpener {
    pub fn new(root: &RootCtx, component: &Component) -> AppResult<Self> {
        let exe = component.exe_path(&root.runtime_dir())?;
        Ok(Self {
            root: root.clone(),
            exe,
        })
    }
}

impl Opener for NotepadOpener {
    fn id(&self) -> &'static str {
        "portable-editor"
    }

    fn matches(&self, ext: Option<&str>) -> bool {
        notepadpp::matches_ext(ext)
    }

    fn launch(&self, abs: &Path) -> AppResult<()> {
        let bin_dir = self
            .exe
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| self.root.runtime_dir());
        let plan = EnvPlan::build(&self.root, &[bin_dir], &BTreeMap::new());
        let spec = Spec {
            program: self.exe.clone(),
            args: notepadpp::open_args(abs),
            cwd: self.root.root().to_path_buf(),
            plan,
            hide: true,
        };
        launch::spawn_detached(&spec)
    }
}

pub struct OpenerRegistry {
    openers: Vec<Box<dyn Opener>>,
    reveal: RevealOpener,
}

impl OpenerRegistry {
    pub fn default_set() -> Self {
        Self {
            // 顺序即优先级：越靠前越优先命中
            openers: vec![Box::new(SystemDefaultOpener)],
            reveal: RevealOpener,
        }
    }

    /// 按盘内清单装配打开器。
    ///
    /// 一个前提：**便携组件只在「文件真的存在」时才注册**（实现见
    /// `register_if_present`）。所以「盘里放了 SumatraPDF」这件事本身
    /// 就能让 PDF 的默认打开方式变成盘内阅读器，不需要任何配置项 ——
    /// 反过来，没放就自动回落到系统默认程序，行为永远可用。
    pub fn for_root(root: &RootCtx, manifest: Option<&Manifest>) -> Self {
        let mut registry = Self::default_set();

        let Some(m) = manifest else {
            return registry;
        };

        registry.register_if_present(root, m, Kind::Office, |r, c| {
            Ok(Box::new(OfficeOpener::new(r, c)?))
        });
        registry.register_if_present(root, m, Kind::Pdf, |r, c| {
            Ok(Box::new(PdfOpener::new(r, c)?))
        });
        registry.register_if_present(root, m, Kind::Editor, |r, c| {
            Ok(Box::new(NotepadOpener::new(r, c)?))
        });

        registry
    }

    /// 「组件真的在位才注册」这条规则的唯一实现处。
    ///
    /// 抽出来的原因不是少写几行，而是让规则只有一份：两处各自判断，
    /// 迟早会出现一个忘记判 `is_file()` 的分支，表现为「点开报启动失败」——
    /// 而真正的原因（组件没装）被那句启动失败彻底掩盖。
    fn register_if_present<F>(&mut self, root: &RootCtx, m: &Manifest, kind: Kind, make: F)
    where
        F: Fn(&RootCtx, &Component) -> AppResult<Box<dyn Opener>>,
    {
        let Some(component) = m.get(kind) else {
            return;
        };

        let exists = component
            .exe_path(&root.runtime_dir())
            .map(|p| p.is_file())
            .unwrap_or(false);
        if !exists {
            return;
        }

        if let Ok(opener) = make(root, component) {
            self.register(opener);
        }
    }

    pub fn register(&mut self, opener: Box<dyn Opener>) {
        self.openers.insert(0, opener);
    }

    /// 按扩展名选出打开器。
    ///
    /// 单独抽出来是为了让「PDF 会走盘内阅读器」这条路由能被测试断言 ——
    /// 测路由结果，而不是「进程有没有真的起来」。后者在 CI 上既慢又不稳。
    fn resolve(&self, ext: Option<&str>) -> Option<&dyn Opener> {
        // 顺序即优先级；`SystemDefaultOpener::matches` 恒真，所以它一定兜底。
        self.openers
            .iter()
            .find(|o| o.matches(ext))
            .map(|b| b.as_ref())
    }

    pub fn dispatch(
        &self,
        abs: &Path,
        ext: Option<&str>,
        mode: OpenMode,
    ) -> AppResult<OpenOutcome> {
        match mode {
            // P1：预览完全在前端做。P2 在这里先查「精确预览」打开器
            // （LibreOffice → PDF），查不到再回落到 Internal。
            OpenMode::Preview => Ok(OpenOutcome::Internal),

            OpenMode::Reveal => {
                self.reveal.launch(abs)?;
                Ok(OpenOutcome::External {
                    program: "explorer".into(),
                })
            }

            OpenMode::Edit => {
                let opener = self
                    .resolve(ext)
                    .ok_or(AppError::Unimplemented("没有可用的打开器"))?;
                opener.launch(abs)?;
                Ok(OpenOutcome::External {
                    program: opener.id().to_string(),
                })
            }

            OpenMode::Convert => Err(AppError::Unimplemented(
                "格式转换将在 P2 接入便携 LibreOffice 后实现",
            )),
        }
    }
}

#[cfg(windows)]
fn shell_open(abs: &Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("cmd")
        .arg("/C")
        .arg("start")
        .arg("") // start 的第一个引号参数是窗口标题，必须占位
        .arg(abs.as_os_str())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| AppError::Launch(e.to_string()))?;
    Ok(())
}

#[cfg(windows)]
fn shell_reveal(abs: &Path) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // explorer 的 /select 参数必须是一个整体，路径带空格时要自行加引号，
    // 用 raw_arg 绕开 Rust 的参数转义，否则 explorer 会解析失败。
    let mut cmd = std::process::Command::new("explorer");
    cmd.raw_arg(format!("/select,\"{}\"", abs.display()));
    cmd.creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| AppError::Launch(e.to_string()))?;
    Ok(())
}

#[cfg(not(windows))]
fn shell_open(abs: &Path) -> AppResult<()> {
    std::process::Command::new("xdg-open")
        .arg(abs)
        .spawn()
        .map_err(|e| AppError::Launch(e.to_string()))?;
    Ok(())
}

#[cfg(not(windows))]
fn shell_reveal(abs: &Path) -> AppResult<()> {
    let parent = abs.parent().unwrap_or(abs);
    std::process::Command::new("xdg-open")
        .arg(parent)
        .spawn()
        .map_err(|e| AppError::Launch(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::manifest::SCHEMA_VERSION;
    use std::fs;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut p = std::env::temp_dir();
        p.push(format!("pl-reg-{tag}-{}-{nanos}", std::process::id()));
        p
    }

    fn pdf_component() -> Component {
        Component {
            id: "pdf".into(),
            kind: Kind::Pdf,
            version: "3.6.1".into(),
            exe: "pdf/SumatraPDF.exe".into(),
            version_args: Vec::new(),
            env: BTreeMap::new(),
            note: String::new(),
        }
    }

    fn manifest_with(components: Vec<Component>) -> Manifest {
        Manifest {
            schema: SCHEMA_VERSION,
            components,
        }
    }

    #[test]
    fn pdf_routes_to_portable_reader_when_file_exists() {
        let dir = temp_root("present");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("runtime/pdf")).expect("建 runtime/pdf 失败");
        // 注册判据是「exe 文件存在」，内容是不是真的 exe 无所谓 —— 这个测试不启动进程。
        fs::write(dir.join("runtime/pdf/SumatraPDF.exe"), b"stub").expect("写假 exe 失败");

        let root = RootCtx::detect_in(Some(dir.clone())).expect("构造盘根失败");
        let m = manifest_with(vec![pdf_component()]);
        let reg = OpenerRegistry::for_root(&root, Some(&m));

        assert_eq!(
            reg.resolve(Some("pdf")).map(|o| o.id()),
            Some("portable-pdf")
        );
        assert_eq!(
            reg.resolve(Some("epub")).map(|o| o.id()),
            Some("portable-pdf")
        );
        // 没声明 office 组件，docx 必须回落到系统默认程序
        assert_eq!(
            reg.resolve(Some("docx")).map(|o| o.id()),
            Some("system-default")
        );
        assert_eq!(reg.resolve(None).map(|o| o.id()), Some("system-default"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_pdf_file_falls_back_instead_of_registering() {
        // 清单写了 pdf、但文件没放进来：不能注册一个指向空气的打开器，
        // 否则用户点开得到「启动失败」，而真正的原因（没装）被掩盖。
        let dir = temp_root("absent");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("runtime/pdf")).expect("建 runtime/pdf 失败");

        let root = RootCtx::detect_in(Some(dir.clone())).expect("构造盘根失败");
        let m = manifest_with(vec![pdf_component()]);
        let reg = OpenerRegistry::for_root(&root, Some(&m));

        assert_eq!(
            reg.resolve(Some("pdf")).map(|o| o.id()),
            Some("system-default")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    fn editor_component() -> Component {
        Component {
            id: "notepadpp".into(),
            kind: Kind::Editor,
            version: "8.9.8.1".into(),
            // 目录名里同时有空格和大写字母，正好顺带验证路径不会被改写。
            exe: "Notepad++/notepad++.exe".into(),
            version_args: Vec::new(),
            env: BTreeMap::new(),
            note: String::new(),
        }
    }

    #[test]
    fn editor_routes_markdown_and_html_when_file_exists() {
        let dir = temp_root("editor");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("runtime/Notepad++")).expect("建 runtime/Notepad++ 失败");
        fs::write(dir.join("runtime/Notepad++/notepad++.exe"), b"stub").expect("写假 exe 失败");

        let root = RootCtx::detect_in(Some(dir.clone())).expect("构造盘根失败");
        let m = manifest_with(vec![editor_component()]);
        let reg = OpenerRegistry::for_root(&root, Some(&m));

        for ext in ["md", "markdown", "html", "htm", "xhtml"] {
            assert_eq!(
                reg.resolve(Some(ext)).map(|o| o.id()),
                Some("portable-editor"),
                "{ext} 应交给盘内编辑器"
            );
        }

        // 关键边界：编辑器只接管「需要渲染的源文本」，不抢内置预览已经够用的格式。
        for ext in ["txt", "json", "rs", "log"] {
            assert_eq!(
                reg.resolve(Some(ext)).map(|o| o.id()),
                Some("system-default"),
                "{ext} 不该被编辑器抢走"
            );
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_and_reader_do_not_overlap() {
        // 两个打开器同时存在时，集合必须仍然互不重叠：pdf 归阅读器，md 归编辑器。
        // 一旦有人往任一 EXTS 里加了对方的格式，这里会先红。
        let dir = temp_root("both");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("runtime/Notepad++")).expect("建目录失败");
        fs::create_dir_all(dir.join("runtime/pdf")).expect("建目录失败");
        fs::write(dir.join("runtime/Notepad++/notepad++.exe"), b"stub").expect("写 stub 失败");
        fs::write(dir.join("runtime/pdf/SumatraPDF.exe"), b"stub").expect("写 stub 失败");

        let root = RootCtx::detect_in(Some(dir.clone())).expect("构造盘根失败");
        let m = manifest_with(vec![pdf_component(), editor_component()]);
        let reg = OpenerRegistry::for_root(&root, Some(&m));

        assert_eq!(
            reg.resolve(Some("pdf")).map(|o| o.id()),
            Some("portable-pdf")
        );
        assert_eq!(
            reg.resolve(Some("md")).map(|o| o.id()),
            Some("portable-editor")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
