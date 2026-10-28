/**
 * 与 Rust 侧 DTO 一一对应。
 *
 * 约定：Rust 结构体统一 `#[serde(rename_all = "camelCase")]`，
 * 因此这里全部是 camelCase。改字段时两边必须同步，
 * 这是前端唯一的「类型契约」来源。
 */

export type NodeKind = "dir" | "file" | "link";
export type OpenMode = "preview" | "edit" | "reveal" | "convert";
export type RuntimeKind = "python" | "node" | "office" | "pdf" | "tool";

export interface RootInfo {
  root: string;
  contentRoot: string;
  runtimeDir: string;
  stateDir: string;
  volumeSerial: string;
  dbPath: string;
  schemaVersion: number;
}

export interface NodeDto {
  id: number;
  relPath: string;
  name: string;
  ext: string | null;
  kind: NodeKind;
  size: number;
  mtime: number;
  hasChildren: boolean;
}

export interface ScanReport {
  scanned: number;
  added: number;
  updated: number;
  removed: number;
  unchanged: number;
  skippedDirs: number;
  durationMs: number;
}

export interface StatsDto {
  total: number;
  dirs: number;
  files: number;
  bytes: number;
}

export interface OpenResult {
  outcome: "internal" | "external";
  program?: string | null;
}

export interface TextPreview {
  content: string;
  truncated: boolean;
  bytes: number;
  encoding: string;
}

/** 一个运行时组件的现场状态。`present` 是文件在不在，`runnable` 是能不能跑。 */
export interface RuntimeComponent {
  id: string;
  kind: RuntimeKind;
  declared: string;
  exeRel: string;
  exeAbs: string;
  present: boolean;
  runnable: boolean;
  detected: string | null;
  detail: string;
}

export interface RuntimeStatus {
  runtimeDir: string;
  manifestPresent: boolean;
  manifestError: string | null;
  officeAvailable: boolean;
  /** 盘内 PDF 阅读器是否可用。决定 PDF 卡片主按钮的文案与提示。 */
  pdfAvailable: boolean;
  components: RuntimeComponent[];
}

export interface ProjectDto {
  nodeId: number;
  relPath: string;
  name: string;
  lang: string;
  markers: string[];
  runtimeKind: string | null;
  runtimeReady: boolean;
}

export interface ConvertResult {
  relInCache: string;
  absPath: string;
  bytes: number;
  durationMs: number;
}

/** 一棵子树的规模。`dirs`/`files`/`bytes` 均**不含自身**。 */
export interface TreeStats {
  dirs: number;
  files: number;
  bytes: number;
  latestMtime: number;
}

/** 面包屑一节。`rel` 为空串表示内容区根。 */
export interface CrumbDto {
  name: string;
  rel: string;
}

/** 一个子项 + 它自己的规模。结构视图的基本单元。 */
export interface ChildOverview {
  node: NodeDto;
  subtree: TreeStats;
}

/** 当前这一层「长什么样」——分类与从属关系的完整描述。 */
export interface DirOverview {
  rel: string | null;
  name: string;
  breadcrumb: CrumbDto[];
  totals: TreeStats;
  children: ChildOverview[];
  dirsDirect: number;
  filesDirect: number;
}

/** 文件监听是否在跑。为 false 时索引只在扫描时更新。 */
export interface WatchInfo {
  active: boolean;
  error: string | null;
}
