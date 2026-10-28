//! 子进程环境沙箱。
//!
//! **必须先说清这一层的真实能力边界**（结论来自上一轮的技术核实）：
//! 环境变量不是隔离手段，只是「默认值提示」。大量 Windows 程序绕开 `%APPDATA%`
//! 直接用 `SHGetKnownFolderPath` 问注册表；注册表写入更是任何环境变量都拦不住。
//!
//! 所以本模块只解决「愿意读环境变量的那部分程序」，对不读的必须靠组件自带的
//! 私有状态参数。典型反例就是 LibreOffice：它根本不看 `%APPDATA%`，而是锁自己的
//! `UserInstallation` 目录，只能用命令行参数重定向 —— 见 `office::convert_to_pdf`。
//!
//! 一条设计纪律：**沙箱是按进程构造的，绝不改宿主环境**。
//! 这里全程只用 `Command::env` / `env_remove`，不碰 `std::env::set_var`，
//! 因为后者改的是当前进程乃至子进程继承的全局状态，插一次盘就会留下痕迹。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

use crate::error::AppResult;
use crate::paths::RootCtx;

pub const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

pub struct EnvPlan {
    sets: Vec<(String, String)>,
    removes: Vec<String>,
    prepend: Vec<PathBuf>,
    original_path: Option<String>,
    dirs: Vec<PathBuf>,
}

impl EnvPlan {
    /// `prepend` 是盘内组件的 bin 目录，会排在宿主 `PATH` 之前 ——
    /// 这让 `python` / `node` 优先解析到盘内版本，而宿主既有工具仍然可用。
    /// 刻意不「清空 PATH」：清空会让 DLL 搜索和系统命令全部失效，
    /// 换来的隔离收益远小于维护成本。
    pub fn build(root: &RootCtx, prepend: &[PathBuf], extra: &BTreeMap<String, String>) -> Self {
        let home = root.home_dir();
        let temp = home.join("temp");
        let roaming = home.join("appdata").join("roaming");
        let local = home.join("appdata").join("local");

        let mut sets: Vec<(String, String)> = Vec::new();
        for (key, dir) in [
            ("USERPROFILE", &home),
            ("HOME", &home),
            ("APPDATA", &roaming),
            ("LOCALAPPDATA", &local),
            ("TEMP", &temp),
            ("TMP", &temp),
        ] {
            sets.push((key.to_string(), dir.to_string_lossy().into_owned()));
        }

        // 语言工具链的私有落点：把缓存、全局包、pip 的 user base 都拽回盘内，
        // 否则 `pip install --user` 会写到宿主 C:\Users\<用户>\AppData。
        for (key, dir) in [
            ("PYTHONUSERBASE", home.join("python")),
            ("PIP_CACHE_DIR", home.join("pip-cache")),
            ("NPM_CONFIG_CACHE", home.join("npm-cache")),
            ("NPM_CONFIG_PREFIX", home.join("npm-global")),
        ] {
            sets.push((key.to_string(), dir.to_string_lossy().into_owned()));
        }

        // 这几条是纯值，不是路径
        sets.push(("PYTHONIOENCODING".to_string(), "utf-8".to_string()));
        sets.push((
            "NPM_CONFIG_UPDATE_NOTIFIER".to_string(),
            "false".to_string(),
        ));

        // git 的全局配置也重定向：否则用户在盘内仓库里改 global config 会改宿主
        sets.push((
            "GIT_CONFIG_GLOBAL".to_string(),
            home.join(".gitconfig").to_string_lossy().into_owned(),
        ));

        // 宿主的这些变量会直接破坏盘内运行时，必须清掉：
        // PYTHONHOME 会让 embeddable 版 python 直接找错标准库；
        // VIRTUAL_ENV / CONDA_PREFIX 会把盘外环境激活进来；
        // NODE_OPTIONS / NODE_PATH 会把盘外模块路径注入。
        let removes = [
            "PYTHONHOME",
            "PYTHONPATH",
            "PYTHONSTARTUP",
            "VIRTUAL_ENV",
            "CONDA_PREFIX",
            "NODE_OPTIONS",
            "NODE_PATH",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let mut dirs: Vec<PathBuf> = vec![
            home.clone(),
            home.join("appdata"),
            home.join("appdata").join("local"),
            home.join("appdata").join("roaming"),
            temp,
            home.join("python"),
            home.join("pip-cache"),
            home.join("npm-cache"),
            home.join("npm-global"),
        ];
        dirs.push(root.state_dir().join("tmp"));

        // 组件自定义变量最后压入，因此可以覆盖上面的默认值
        for (k, v) in extra {
            sets.push((k.clone(), v.clone()));
        }

        Self {
            sets,
            removes,
            prepend: prepend.to_vec(),
            original_path: std::env::var("PATH").ok(),
            dirs,
        }
    }

    /// 创建沙箱目录并写入子进程环境。只影响这一个子进程。
    pub fn apply(&self, cmd: &mut Command) -> AppResult<()> {
        for d in &self.dirs {
            std::fs::create_dir_all(d)?;
        }
        cmd.env("PATH", self.path_value());
        for (k, v) in &self.sets {
            cmd.env(k, v);
        }
        for k in &self.removes {
            cmd.env_remove(k);
        }
        Ok(())
    }

    fn path_value(&self) -> OsString {
        let mut parts: Vec<OsString> = self
            .prepend
            .iter()
            .map(|p| p.as_os_str().to_os_string())
            .collect();

        if let Some(orig) = &self.original_path {
            if !orig.trim().is_empty() {
                parts.push(OsString::from(orig));
            }
        }

        let mut joined = OsString::new();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                joined.push(PATH_SEP.to_string());
            }
            joined.push(p);
        }
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(prepend: Vec<PathBuf>, original: Option<&str>) -> EnvPlan {
        EnvPlan {
            sets: Vec::new(),
            removes: Vec::new(),
            prepend,
            original_path: original.map(|s| s.to_string()),
            dirs: Vec::new(),
        }
    }

    #[test]
    fn disk_dirs_come_first() {
        let p = plan(vec![PathBuf::from("disk-bin")], Some("host-bin"));
        assert_eq!(
            p.path_value().to_string_lossy(),
            format!("disk-bin{PATH_SEP}host-bin")
        );
    }

    #[test]
    fn empty_original_path_is_skipped() {
        let p = plan(vec![PathBuf::from("disk-bin")], Some("   "));
        assert_eq!(p.path_value().to_string_lossy(), "disk-bin");
    }
}
