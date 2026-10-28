//! 打开相关命令：`open(target, mode)` 与内置预览的取数接口。

use std::io::Read;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::open::{OpenMode, OpenOutcome};
use crate::state::AppState;

/// 预览读取上限：256 KB。超过就截断并在前端提示，
/// 避免一次误点视频文件把 WebView 撑爆。
const PREVIEW_LIMIT: u64 = 256 * 1024;
const PREVIEW_HARD_CAP: u64 = 2 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResultDto {
    pub outcome: String,
    pub program: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPreviewDto {
    pub content: String,
    pub truncated: bool,
    pub bytes: u64,
    pub encoding: String,
}

#[tauri::command]
pub fn open_target(
    state: State<'_, AppState>,
    rel: String,
    mode: String,
) -> AppResult<OpenResultDto> {
    let open_mode = OpenMode::parse(&mode)?;
    let abs = state.root.resolve(&rel)?;

    if !abs.exists() {
        return Err(AppError::NotFound(rel));
    }

    let ext = abs
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());

    Ok(to_dto(state.registry.dispatch(
        &abs,
        ext.as_deref(),
        open_mode,
    )?))
}

/// 打开缓存区里的派生产物（目前只有转换出来的 PDF）。
///
/// 单独开一个命令而不是放宽 `open_target` 的作用域，是因为这两件事的
/// 信任级别不同：`open_target` 只能碰用户内容，缓存区是系统区的产物，
/// 一旦混在一起，`content/` 的边界就不再是硬边界了。
#[tauri::command]
pub fn open_cached(state: State<'_, AppState>, rel: String) -> AppResult<OpenResultDto> {
    let abs = state.root.cache_path(&rel)?;
    if !abs.is_file() {
        return Err(AppError::NotFound(rel));
    }

    let ext = abs
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());

    Ok(to_dto(state.registry.dispatch(
        &abs,
        ext.as_deref(),
        OpenMode::Edit,
    )?))
}

fn to_dto(outcome: OpenOutcome) -> OpenResultDto {
    match outcome {
        OpenOutcome::Internal => OpenResultDto {
            outcome: "internal".into(),
            program: None,
        },
        OpenOutcome::External { program } => OpenResultDto {
            outcome: "external".into(),
            program: Some(program),
        },
    }
}

#[tauri::command]
pub fn read_text(
    state: State<'_, AppState>,
    rel: String,
    max_bytes: Option<u64>,
) -> AppResult<TextPreviewDto> {
    let limit = max_bytes.unwrap_or(PREVIEW_LIMIT).min(PREVIEW_HARD_CAP);

    let abs = state.root.resolve(&rel)?;
    let meta = std::fs::metadata(&abs).map_err(|_| AppError::NotFound(rel.clone()))?;
    if !meta.is_file() {
        return Err(AppError::NotText);
    }

    let mut reader = std::fs::File::open(&abs)?.take(limit + 1);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;

    let truncated = buf.len() as u64 > limit;
    if truncated {
        buf.truncate(limit as usize);
    }

    // 前 8 KB 出现 NUL 就判定为二进制，不做无意义的文本预览
    if buf.iter().take(8192).any(|b| *b == 0) {
        return Err(AppError::NotText);
    }

    let (content, encoding) = match String::from_utf8(buf) {
        Ok(s) => (s, "utf-8".to_string()),
        Err(e) => (
            String::from_utf8_lossy(e.as_bytes()).to_string(),
            "utf-8-lossy".to_string(),
        ),
    };

    Ok(TextPreviewDto {
        content,
        truncated,
        bytes: meta.len(),
        encoding,
    })
}

/// 内置渲染的基准目录（绝对路径，正斜杠）。
///
/// 前端拿它配合 `convertFileSrc` 拼出 `http://asset.localhost/<目录>/`，
/// 写进 iframe `srcdoc` 的 `<base href>`。**这不是可选的**：只把 HTML 文本
/// 塞进 srcdoc，文档里的 `./style.css`、`img/a.png` 会以父页面为基准解析，
/// 打到前端资源里，结果是「样式全丢」。渲染保真靠的就是这一行 base。
///
/// 返回目录而不是文件，是因为 md 引用的图片常散在子目录里；`<base>` 本身
/// 只能是一个目录。
#[tauri::command]
pub fn preview_base(state: State<'_, AppState>, rel: String) -> AppResult<String> {
    Ok(crate::paths::to_slash(&state.root.dir_of(&rel)?))
}
