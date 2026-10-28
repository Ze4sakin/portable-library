//! 宿主 WebView2 运行时的检测（发行策略 A：依赖系统 Evergreen，不捆绑）。
//!
//! ## 为什么是 A
//!
//! Tauri 在 Windows 上的界面就是一个 WebView2 实例。这个运行时有三条路：
//!
//! | 方案 | 代价 | 结论 |
//! | --- | --- | --- |
//! | A 依赖宿主 Evergreen | 0 MB；缺了打不开 | **采用** |
//! | B 盘内 Fixed Version | +120~180 MB；需自己跟安全更新 | 不采用 |
//! | C 启动前下载安装 | 需要联网 + 安装程序 + 管理员权限 | 不采用 |
//!
//! 决定性理由：本盘是**便携资产**，不是发行渠道。B 会让盘多背一个浏览器内核，
//! 而且内核版本从此与系统脱钩 —— 用户半年后插上这块盘，拿到的是一个带已知
//! 漏洞的旧内核。C 在离线场景（这块盘最典型的使用场景）根本不成立，
//! 而它要解决的「没装怎么办」，A 用一次提示就覆盖了：Windows 10/11 上
//! 装了 Edge 就自带 WebView2，真正缺的人是极少数。
//!
//! ## 那缺了怎么办
//!
//! 什么都不做的结果是最糟的一种失败：双击 exe，窗口不出现，没有任何提示。
//! 所以这里只做一件很轻的事 —— 启动前查注册表，缺了就弹一句人话，然后退出。
//! 这不是 C（不下载、不安装、不提权），只是给 A 补上失败路径。
//!
//! ## 检测为什么这么写
//!
//! WebView2 的注册位置是「每机器 + 每用户」两档，64 位系统上还落在
//! `WOW6432Node` 下。四个组合都要查，否则会出现「明明装了却说没装」——
//! 这是这个检测最常见的错法：只查 HKLM 64 位视图。
//!
//! 另外 `pv` 的值可能是 `0.0.0.0`（占位、实际未安装），必须当成没装。

/// 微软官方的 Evergreen 引导链接（会自动选 x86/x64/ARM64）。
pub const DOWNLOAD_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

#[cfg(windows)]
pub use imp::ensure;

/// 只在测试里用：把「本机到底装没装」打出来，方便排查「检测说没有」的现场。
#[cfg(all(test, windows))]
pub use imp::detect;

#[cfg(not(windows))]
pub fn detect() -> Option<String> {
    None
}

#[cfg(not(windows))]
pub fn ensure() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
        RRF_SUBKEY_WOW6432KEY,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    /// WebView2 Evergreen Runtime 的固定产品 GUID，微软文档明确要求按它查。
    const CLIENT_KEY: &str =
        r"SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// 读一个位置的 `pv`。`view32` 为真时走 WOW6432 视图。
    fn read_pv(root: HKEY, view32: bool) -> Option<String> {
        let sub = wide(CLIENT_KEY);
        let name = wide("pv");
        let mut buf = [0u16; 64];
        let mut size = (buf.len() * 2) as u32;
        let mut kind: u32 = 0;
        let flags = RRF_RT_REG_SZ | if view32 { RRF_SUBKEY_WOW6432KEY } else { 0 };

        let rc = unsafe {
            RegGetValueW(
                root,
                sub.as_ptr(),
                name.as_ptr(),
                flags,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }

        // size 是字节数且含结尾 NUL；截断到缓冲区长度，避免异常数据越界。
        let chars = (size as usize / 2).saturating_sub(1).min(buf.len());
        let text = String::from_utf16_lossy(&buf[..chars]);
        let text = text.trim().to_string();

        // 空串和 0.0.0.0 都表示「登记了但没有真的安装」。
        if text.is_empty() || text == "0.0.0.0" {
            None
        } else {
            Some(text)
        }
    }

    pub fn detect() -> Option<String> {
        read_pv(HKEY_LOCAL_MACHINE, true)
            .or_else(|| read_pv(HKEY_CURRENT_USER, true))
            .or_else(|| read_pv(HKEY_LOCAL_MACHINE, false))
            .or_else(|| read_pv(HKEY_CURRENT_USER, false))
    }

    /// 缺失时弹窗说明并返回 `Err`，由调用方决定退出。
    ///
    /// 弹窗在 GUI 子系统下是**唯一**用户能看到的通道：发行版没有控制台，
    /// `eprintln!` 写出去也没人看得到。
    pub fn ensure() -> Result<(), String> {
        if detect().is_some() {
            return Ok(());
        }

        let text = format!(
            "便携资料库需要 Microsoft Edge WebView2 运行时，但本机未检测到。\n\n\
             请安装 Evergreen Runtime（免费，约 2 MB 在线安装包）：\n{}\n\n\
             安装完成后重新打开本程序即可。\n\n\
             说明：本程序不把 WebView2 打进盘里，因为它必须跟随系统更新 —— \
             便携盘上冻结一个旧内核，等于随身带一个不再修补的浏览器。",
            super::DOWNLOAD_URL
        );
        message_box("便携资料库 · 缺少 WebView2 运行时", &text);

        Err("缺少 WebView2 运行时".to_string())
    }

    fn message_box(title: &str, text: &str) {
        let t = wide(text);
        let c = wide(title);
        // 返回值忽略：此刻唯一能做的就是把话说出来。
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                t.as_ptr(),
                c.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn detects_webview2_on_this_machine() {
        // 不断言「一定有」——CI 或精简系统可能确实没有。
        // 但检测函数本身必须能跑通、不 panic、不返回空串。
        if let Some(v) = detect() {
            assert!(!v.trim().is_empty());
            assert_ne!(v, "0.0.0.0");
            println!("检测到 WebView2：{v}");
        }
    }
}
