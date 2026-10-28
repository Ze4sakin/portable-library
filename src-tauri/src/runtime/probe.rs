//! 组件探测：把清单里声明的组件变成「能不能用」的事实。
//!
//! 探测结果**只缓存一次，且可丢弃**。理由见 `manifest` 模块顶部：
//! 运行时状态不是索引的一部分，它是每次插盘现场问出来的事实。
//! 换台电脑、换块盘、手动放了新组件，缓存立刻过期 —— 所以只缓存到
//! 用户主动「重新探测」或下次启动为止。

use std::time::Duration;

use serde::Serialize;

use crate::paths::{to_slash, RootCtx};
use crate::runtime::env::EnvPlan;
use crate::runtime::launch::{self, Spec};
use crate::runtime::manifest::{Component, Manifest};

/// 6 秒足够任何正常 `--version`。超时基本等于「这个组件是坏的」。
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: String,
    pub kind: String,
    pub declared: String,
    pub exe_rel: String,
    pub exe_abs: String,
    /// 文件在不在
    pub present: bool,
    /// 文件在且能跑出结果
    pub runnable: bool,
    /// 实际探测到的版本行（截断）
    pub detected: Option<String>,
    pub detail: String,
}

pub fn probe_all(root: &RootCtx, manifest: &Manifest) -> Vec<Status> {
    manifest
        .components
        .iter()
        .map(|c| probe_one(root, c))
        .collect()
}

pub fn probe_one(root: &RootCtx, component: &Component) -> Status {
    let mut status = Status {
        id: component.id.clone(),
        kind: component.kind.as_str().to_string(),
        declared: component.version.clone(),
        exe_rel: component.exe.clone(),
        exe_abs: String::new(),
        present: false,
        runnable: false,
        detected: None,
        detail: String::new(),
    };

    let exe = match component.exe_path(&root.runtime_dir()) {
        Ok(p) => p,
        Err(e) => {
            status.detail = e.to_string();
            return status;
        }
    };
    status.exe_abs = to_slash(&exe);

    if !exe.is_file() {
        status.detail = format!("未就位：runtime/{}", component.exe);
        return status;
    }
    status.present = true;

    if component.version_args.is_empty() {
        // 没声明探测参数不等于坏 —— 有些组件（便携 IDE）没有 `--version`
        status.runnable = true;
        status.detail = "已就位（未声明版本探测参数）".into();
        return status;
    }

    let bin_dir = exe
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| root.runtime_dir());
    let plan = EnvPlan::build(root, &[bin_dir], &component.env);
    let spec = Spec {
        program: exe,
        args: component.version_args.clone(),
        cwd: root.root().to_path_buf(),
        plan,
        hide: true,
    };

    match launch::run_capture(&spec, PROBE_TIMEOUT) {
        Ok(out) => {
            if out.timed_out {
                status.detail = format!("探测超时（{} s）", PROBE_TIMEOUT.as_secs());
                return status;
            }

            // 注意借用：这里只借读、不移动，因为下面失败分支还要用完整输出拼错误信息。
            let text: &str = if out.stdout.trim().is_empty() {
                &out.stderr
            } else {
                &out.stdout
            };
            let line = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            if !line.is_empty() {
                status.detected = Some(truncate(&line, 120));
            }

            // 有的组件版本信息走 stderr、退出码非 0 也算正常，
            // 所以判据取「有输出」或「退出码为 0」，两者皆无才算坏
            status.runnable = out.code == Some(0) || status.detected.is_some();
            status.detail = if status.runnable {
                "可执行".into()
            } else {
                format!(
                    "执行失败，退出码 {:?}：{}",
                    out.code,
                    launch::tail(&format!("{}{}", out.stdout, out.stderr), 160)
                )
            };
            status
        }
        Err(e) => {
            status.detail = e.to_string();
            status
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_counts_chars_not_bytes() {
        let s = "便携运行时".repeat(40);
        let out = truncate(&s, 10);
        assert_eq!(out.chars().count(), 11); // 10 + 省略号
    }
}
