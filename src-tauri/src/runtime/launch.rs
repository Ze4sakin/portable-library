//! 进程启动：两种形态，一个入口。
//!
//! - `spawn_detached`：交给用户的 GUI 程序（LibreOffice、终端），启动即返回
//! - `run_capture`：需要读输出的（版本探测、格式转换），带超时与输出上限
//!
//! 两个约束值得单独写下来：
//!
//! 1. **必须能超时**。可移动硬盘上跑的进程，很容易因为盘速或组件损坏卡死；
//!    没有超时的 `wait()` 会让整个 UI 挂在一个永远不返回的命令上。
//! 2. **必须限输出**。`--version` 只有一行，但 `--help` 可能几 MB。
//!    不限量的 `read_to_string` 就是一次可复现的内存炸弹。

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult};
use crate::runtime::env::EnvPlan;

/// 单个流最多读 64 KB。超出的部分丢弃，只用于「看一眼版本号」这类场景。
const CAPTURE_CAP: u64 = 64 * 1024;

pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub plan: EnvPlan,
    /// 是否隐藏控制台窗口。
    ///
    /// 默认 `true`：探测版本、无头转换都不该在用户眼前闪一个黑框。
    /// 但**打开终端时必须设 `false`** —— 给 `cmd.exe` 加 `CREATE_NO_WINDOW`
    /// 会得到一个看不见的、只有 Ctrl+Alt+Del 才能关掉的进程，
    /// 这是这类启动器最经典的自伤方式。
    pub hide: bool,
}

impl Spec {
    fn command(&self) -> AppResult<Command> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        cmd.current_dir(&self.cwd);
        cmd.stdin(Stdio::null());
        self.plan.apply(&mut cmd)?;
        if self.hide {
            hide_window(&mut cmd);
        }
        Ok(cmd)
    }
}

#[derive(Debug, Clone)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// 启动后不管。用于 GUI 程序 —— 它们的生命周期由用户决定，不该被启动器握着。
pub fn spawn_detached(spec: &Spec) -> AppResult<()> {
    let mut cmd = spec.command()?;
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| AppError::Launch(format!("{} 启动失败：{e}", spec.program.display())))?;
    Ok(())
}

/// 启动并等待，带超时与输出上限。
pub fn run_capture(spec: &Spec, timeout: Duration) -> AppResult<Output> {
    let started = Instant::now();

    let mut cmd = spec.command()?;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::Launch(format!("{} 启动失败：{e}", spec.program.display())))?;

    // 必须用独立线程读管道：管道缓冲区写满后子进程会阻塞，
    // 而我们在主线程里等它退出 —— 单线程实现会直接死锁。
    let out_pipe = child.stdout.take();
    let err_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || read_limited(out_pipe));
    let err_handle = std::thread::spawn(move || read_limited(err_pipe));

    let mut timed_out = false;
    let mut code: Option<i32> = None;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                code = status.code();
                break;
            }
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(AppError::Launch(format!("等待进程失败：{e}"))),
        }
    }

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();

    Ok(Output {
        code,
        stdout,
        stderr,
        timed_out,
    })
}

fn read_limited<R: Read + Send + 'static>(pipe: Option<R>) -> String {
    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = pipe.take(CAPTURE_CAP).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// 取尾部若干字符，用于把错误信息塞进 IPC 而不炸掉前端。
pub fn tail(text: &str, max: usize) -> String {
    let cleaned = text.trim();
    let count = cleaned.chars().count();
    if count <= max {
        return cleaned.to_string();
    }
    let skipped: String = cleaned.chars().skip(count - max).collect();
    format!("…{skipped}")
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_cmd: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end() {
        assert_eq!(tail("abcdef", 3), "…def");
        assert_eq!(tail("  ab  ", 10), "ab");
    }
}
