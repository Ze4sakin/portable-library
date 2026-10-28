//! 运行时相关命令。
//!
//! 命令层只做「取状态 → 调下层 → 转 DTO」，与 `library` 保持同一形状。
//! 一个刻意的选择：探测结果放在 `AppState` 的缓存里，而不是数据库里。
//! 因为它是**可丢弃的快照** —— 掉了下次探测就有，写进库反而会引入
//! 「缓存与磁盘不一致」这种需要额外对账的问题。

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::paths::to_slash;
use crate::runtime::manifest::Kind;
use crate::runtime::office;
use crate::runtime::probe::{self, Status};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDto {
    pub runtime_dir: String,
    pub manifest_present: bool,
    pub manifest_error: Option<String>,
    /// 是否有可用的便携 Office。前端据此决定要不要显示「转 PDF」
    pub office_available: bool,
    /// 是否有可用的盘内 PDF 阅读器。前端据此决定 PDF 卡片的主按钮文案
    pub pdf_available: bool,
    pub components: Vec<Status>,
}

fn snapshot(state: &AppState, force: bool) -> AppResult<RuntimeDto> {
    let Some(manifest) = state.manifest.as_ref() else {
        return Ok(RuntimeDto {
            runtime_dir: to_slash(&state.root.runtime_dir()),
            manifest_present: false,
            manifest_error: state.manifest_error.clone(),
            office_available: false,
            pdf_available: false,
            components: Vec::new(),
        });
    };

    let mut cache = state.runtime_probe.lock().map_err(|_| AppError::Poisoned)?;
    if force || cache.is_none() {
        *cache = Some(probe::probe_all(&state.root, manifest));
    }
    let components: Vec<Status> = cache.as_ref().cloned().unwrap_or_default();

    let office_available = components
        .iter()
        .any(|c| c.kind == Kind::Office.as_str() && c.runnable);
    let pdf_available = components
        .iter()
        .any(|c| c.kind == Kind::Pdf.as_str() && c.runnable);

    Ok(RuntimeDto {
        runtime_dir: to_slash(&state.root.runtime_dir()),
        manifest_present: true,
        manifest_error: state.manifest_error.clone(),
        office_available,
        pdf_available,
        components,
    })
}

#[tauri::command]
pub fn runtime_status(state: State<'_, AppState>) -> AppResult<RuntimeDto> {
    snapshot(&state, false)
}

/// 强制重新探测。用于「刚把组件拷进盘里」之后刷新状态。
#[tauri::command]
pub fn runtime_probe(state: State<'_, AppState>) -> AppResult<RuntimeDto> {
    snapshot(&state, true)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertDto {
    /// 相对 `.library/cache/` 的路径，供 `open_cached` 打开
    pub rel_in_cache: String,
    pub abs_path: String,
    pub bytes: u64,
    pub duration_ms: u64,
}

/// Office 文档 → PDF（无头 LibreOffice）。
///
/// 这是「编辑接力」链路里的第一跳：先用盘内 Office 转成 PDF 得到高保真页图，
/// 再交给前端或系统阅读器。之所以不自己去渲染 docx，见架构文档第 6 章。
#[tauri::command]
pub fn doc_convert(state: State<'_, AppState>, rel: String) -> AppResult<ConvertDto> {
    let manifest = state
        .manifest
        .as_ref()
        .ok_or_else(|| AppError::Manifest("盘内没有 runtime/manifest.json，无法转换".into()))?;
    let component = manifest
        .get(Kind::Office)
        .ok_or_else(|| AppError::ComponentMissing("清单里未声明 office 组件".into()))?;

    let src = state.root.resolve(&rel)?;
    if !src.is_file() {
        return Err(AppError::NotFound(rel));
    }

    let started = std::time::Instant::now();
    let produced = office::convert_to_pdf(&state.root, component, &src)?;

    let bytes = std::fs::metadata(&produced).map(|m| m.len()).unwrap_or(0);
    let rel_in_cache = produced
        .strip_prefix(state.root.cache_dir())
        .map(to_slash)
        .unwrap_or_else(|_| to_slash(&produced));

    Ok(ConvertDto {
        rel_in_cache,
        abs_path: to_slash(&produced),
        bytes,
        duration_ms: started.elapsed().as_millis() as u64,
    })
}
