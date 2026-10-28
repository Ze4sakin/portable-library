/**
 * 唯一的后端边界。
 *
 * 规矩：视图层与组件层**不得**直接 import `@tauri-apps/api`。
 * 想加新能力，就在这里加一个方法，别处一律通过 `api` 对象调用。
 *
 * 这样做换来三件事：
 * 1. 后端命令清单只有一处，审查「前端能干什么」时看这一个文件就够
 * 2. 错误在这里统一归一化成 Error，视图层不必处理 Tauri 的裸字符串
 * 3. 将来换 IPC 机制（或做 Web 版 mock）只需替换本文件
 */

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ConvertResult,
  DirOverview,
  NodeDto,
  OpenMode,
  OpenResult,
  ProjectDto,
  RootInfo,
  RuntimeStatus,
  ScanReport,
  StatsDto,
  TextPreview,
  WatchInfo,
} from "./types";

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (raw) {
    const message = typeof raw === "string" ? raw : raw instanceof Error ? raw.message : String(raw);
    throw new Error(message);
  }
}

export const api = {
  libInit: () => call<RootInfo>("lib_init"),

  scanFull: () => call<ScanReport>("scan_full"),

  listChildren: (parent: string | null) => call<NodeDto[]>("list_children", { parent }),

  /** 结构概览。`null` 表示内容区根 —— 右侧「分类全景」与中间目录页共用它。 */
  dirOverview: (rel: string | null) => call<DirOverview>("dir_overview", { rel }),

  stats: () => call<StatsDto>("library_stats"),

  watchInfo: () => call<WatchInfo>("watch_info"),

  openTarget: (rel: string, mode: OpenMode) => call<OpenResult>("open_target", { rel, mode }),

  /** 打开缓存区的派生产物（转换出来的 PDF）。与 openTarget 是两个作用域。 */
  openCached: (rel: string) => call<OpenResult>("open_cached", { rel }),

  readText: (rel: string, maxBytes?: number) =>
    call<TextPreview>("read_text", { rel, maxBytes: maxBytes ?? null }),

  /**
   * 内置渲染的基准目录（绝对路径）。
   *
   * 只有「要渲染的文档」需要它：iframe 里的 `<base href>` 指到这个目录，
   * 文档中 `./style.css`、`img/a.png` 才会落回盘里。取不到不是错误 ——
   * 没有相对资源的文档（纯内联样式）照样能看。
   */
  previewBase: (rel: string) => call<string>("preview_base", { rel }),

  /**
   * 盘内绝对路径 → asset 协议 URL。
   *
   * 包一层而不是让视图直接 import `@tauri-apps/api`：这条 IPC/协议相关的
   * 知识只应该存在于本文件（见文件头）。
   */
  assetUrl: (absPath: string) => convertFileSrc(absPath),

  runtimeStatus: () => call<RuntimeStatus>("runtime_status"),

  /** 强制重新探测：刚把组件拷进盘里时用 */
  runtimeProbe: () => call<RuntimeStatus>("runtime_probe"),

  projectList: (limit?: number) => call<ProjectDto[]>("project_list", { limit: limit ?? null }),

  /** 在项目目录开终端，环境已指向盘内运行时 */
  projectTerminal: (rel: string) => call<void>("project_terminal", { rel }),

  docConvert: (rel: string) => call<ConvertResult>("doc_convert", { rel }),
};

/**
 * 索引因监听而发生变化的推送。
 *
 * 事件名与后端 `lib::EVENT_LIBRARY_CHANGED` 必须一致。
 * 只有**真的改动了索引**才会推送 —— 一次保存引出十几条文件事件，
 * 若逐条通知，界面会自己刷新个不停。
 */
export function onLibraryChanged(cb: (report: ScanReport) => void): Promise<UnlistenFn> {
  return listen<ScanReport>("library:changed", (e) => cb(e.payload));
}

/** 增量对账失败（例如库被占用）。不致命，但必须让用户看见。 */
export function onLibraryError(cb: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("library:error", (e) => cb(e.payload));
}

export type { UnlistenFn };
