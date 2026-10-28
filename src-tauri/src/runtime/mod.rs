//! 盘内运行时层（P2）。
//!
//! 这一层的职责只有一句话：**把「清单里写着的组件」变成「能跑的进程」**。
//!
//! 五个子模块，依赖严格单向：
//!
//! ```text
//! manifest ──► paths          （只读文件、校验、给路径）
//! env      ──► paths          （构造子进程环境，不碰宿主）
//! launch   ──► env            （真正 spawn）
//! probe    ──► manifest / env / launch
//! office   ──► manifest / env / launch   （LibreOffice 专用参数）
//! pdf      ──► manifest / env / launch   （SumatraPDF 专用参数）
//! ```
//!
//! 两条不变量：
//! 1. **本层不写数据库**。运行时状态是现场探测出来的，不是索引出来的。
//! 2. **本层不改宿主环境**。所有环境改动只作用于 `Command`，从不调用
//!    `std::env::set_var`。
//!
//! 为什么把运行时单独成层、而不是塞进 `open`：`open` 的语义是「把这个文件
//! 交给一个程序」，只管一次；而运行时是**有状态的资产**（存在与否、能不能跑、
//! 版本对不对），它的生命周期比一次打开长得多，也独立得多。

pub mod env;
pub mod launch;
pub mod manifest;
pub mod office;
pub mod pdf;
pub mod probe;
