/**
 * 应用入口：只做装配与编排，不含任何渲染细节或 IO 细节。
 *
 * 三栏的分工（这是本版的核心结构，不是装饰）：
 *   左：层级树        —— 从属关系，一次一层
 *   中：结构页 / 预览 —— 当前这一层怎么分的、这个文件是什么
 *   右：结构 / 运行时 / 项目 —— 全景与能力
 *
 * 数据流（启动）：
 *   lib_init ──► RootInfo ──┐
 *   library_stats ──────────┤
 *   watch_info ─────────────┼──► 首次无索引？→ scan_full → 重载
 *                           └──► 有索引 → navigate(null)（展示分类全景）
 *
 * 实时同步的数据流是反向的：
 *   watcher（后端线程）→ reconcile_dirty → emit "library:changed" → refreshAll()
 * 前端**不轮询**。没有事件就说明索引没变，界面不必动。
 */

import "./styles.css";

import { api, onLibraryChanged, onLibraryError } from "./api/client";
import type { NodeDto, ProjectDto, RootInfo, ScanReport, StatsDto } from "./api/types";
import { pathCrumbs } from "./core/format";
import { DirectoryView } from "./views/dirview";
import { PreviewView } from "./views/preview";
import { ProjectsView } from "./views/projects";
import { RuntimeView } from "./views/runtime";
import { StatusBar } from "./views/statusbar";
import { StructureView } from "./views/structure";
import { TreeView } from "./views/tree";

function pick<T extends HTMLElement>(selector: string): T {
  const el = document.querySelector<T>(selector);
  if (!el) throw new Error(`缺少必需元素：${selector}`);
  return el;
}

const contentHost = pick<HTMLElement>("#content");
const statusHost = pick<HTMLElement>("#status");
const treeHost = pick<HTMLElement>("#tree");
const runtimeHost = pick<HTMLElement>("#runtime");
const projectsHost = pick<HTMLElement>("#projects");
const structureHost = pick<HTMLElement>("#structure");
const scanBtn = pick<HTMLButtonElement>("#btn-scan");
const tabs = Array.from(document.querySelectorAll<HTMLButtonElement>(".rail-tab"));
const panes: Record<string, HTMLElement | null> = {
  structure: structureHost,
  runtime: runtimeHost,
  projects: projectsHost,
};

/** 当前所在的目录（文件预览时是它所在的目录）——「刷新哪里」的唯一依据。 */
let contextDir: string | null = null;
/** 非空表示中间栏正在看一个文件，刷新时需要重取内容。 */
let selectedFile: NodeDto | null = null;
let rootInfo: RootInfo | null = null;
let booted = false;

const status = new StatusBar(statusHost);
const tree = new TreeView(treeHost, (node: NodeDto) => {
  if (node.kind === "dir") void navigate(node.relPath);
  else void openFile(node);
});
const structure = new StructureView(structureHost, (rel) => void navigate(rel));
const dirview = new DirectoryView(contentHost, {
  onEnter: (rel) => void navigate(rel),
  onOpen: (node) => void openFile(node),
  onOpenExternal: (node) => void openExternal(node),
  onReveal: (rel) => void reveal(rel),
});
const preview = new PreviewView(
  contentHost,
  (text, tone) => status.notice(text, tone),
  (rel) => void navigate(rel),
);
const runtime = new RuntimeView(
  runtimeHost,
  (text, tone) => status.notice(text, tone),
  // 探测结果只有一处来源：面板探完，顺手把「有没有」告诉预览视图。
  // 这样点「重新探测」之后，PDF 卡片的文案会立刻跟着变。
  (s) => {
    preview.setOfficeAvailable(s.officeAvailable);
    preview.setPdfAvailable(s.pdfAvailable);
    preview.setEditorAvailable(s.editorAvailable);
  },
);
const projects = new ProjectsView(projectsHost, (p: ProjectDto) => void showProject(p));

function parentOf(rel: string): string | null {
  const i = rel.lastIndexOf("/");
  return i < 0 ? null : rel.slice(0, i);
}

async function reveal(rel: string): Promise<void> {
  try {
    await api.openTarget(rel, "reveal");
  } catch (err) {
    status.notice((err as Error).message, "error");
  }
}

/** 进入一个目录：中间栏换成结构页，左栏与右栏同步到同一位置。 */
async function navigate(rel: string | null): Promise<void> {
  contextDir = rel;
  selectedFile = null;
  await dirview.show(rel);
  void structure.load(rel);
  if (rel) void tree.reveal(rel);
}

/** 看一个文件：位置链 + 内容。 */
async function openFile(node: NodeDto): Promise<void> {
  contextDir = parentOf(node.relPath);
  selectedFile = node;
  await preview.show(node, pathCrumbs(node.relPath));
  void structure.load(contextDir);
  void tree.reveal(node.relPath);
}

/**
 * 把一个文件交给外部程序（双击走这条）。
 *
 * 路由交给后端：盘内有 Notepad++ 时它接 md/html，没有就落回系统默认程序。
 * 前端只负责说清楚「交给了谁」—— 不自己判断，否则会出现两套会互相过期的真相。
 */
async function openExternal(node: NodeDto): Promise<void> {
  try {
    const r = await api.openTarget(node.relPath, "edit");
    const who = r.outcome === "external" ? (r.program ?? "外部程序") : "内置预览";
    status.notice(`${node.name} → ${who}`, "info");
  } catch (err) {
    status.notice((err as Error).message, "error");
  }
}

async function showProject(p: ProjectDto): Promise<void> {
  contextDir = p.relPath;
  selectedFile = null;
  preview.showProject(p);
  void structure.load(p.relPath);
  void tree.reveal(p.relPath);
}

async function readStats(): Promise<StatsDto> {
  try {
    return await api.stats();
  } catch {
    // 索引还没建立时后端会报错，这里视为空库，不算异常
    return { total: 0, dirs: 0, files: 0, bytes: 0 };
  }
}

/** 把三栏一起拉到最新。位置不变，只是内容重新取一遍。 */
async function refreshAll(): Promise<void> {
  if (!booted) return;
  await tree.refresh();

  if (selectedFile) await preview.show(selectedFile, pathCrumbs(selectedFile.relPath));
  else await dirview.show(contextDir);

  await structure.load(contextDir);
  void projects.load();

  if (rootInfo) status.render(rootInfo, await readStats());
}

async function runScan(): Promise<void> {
  scanBtn.disabled = true;
  const original = scanBtn.textContent;
  scanBtn.textContent = "扫描中…";

  try {
    const report = await api.scanFull();
    await refreshAll();
    if (booted && contextDir === null && !selectedFile) await dirview.show(null);

    status.notice(
      `扫描完成：新增 ${report.added} · 更新 ${report.updated} · 移除 ${report.removed} · 未变 ${report.unchanged} · ${report.durationMs} ms`,
      "info",
    );
  } catch (err) {
    status.notice((err as Error).message, "error");
  } finally {
    scanBtn.disabled = false;
    scanBtn.textContent = original ?? "重新扫描";
  }
}

async function loadRuntimeAndProjects(): Promise<void> {
  // 视图内部已各自兜住异常，这里再包一层只是保证一个面板失败不影响另一个
  try {
    // 能力值（office / pdf 是否可用）由 RuntimeView 通过 onStatus 回灌给预览视图
    await runtime.load();
  } catch {
    /* RuntimeView 会把错误渲染在面板里 */
  }

  try {
    await projects.load();
  } catch {
    /* ProjectsView 同上 */
  }
}

/** 监听推送来了就刷新。同一时刻只跑一次，多来的合并成一次补刷。 */
let refreshing = false;
let queued = false;

async function onChanged(report: ScanReport): Promise<void> {
  if (refreshing) {
    queued = true;
    return;
  }
  refreshing = true;
  try {
    await refreshAll();
    status.notice(
      `已同步：新增 ${report.added} · 更新 ${report.updated} · 移除 ${report.removed}`,
      "info",
    );
  } catch (err) {
    status.notice((err as Error).message, "error");
  } finally {
    refreshing = false;
    if (queued) {
      queued = false;
      void refreshAll();
    }
  }
}

function switchTab(name: string): void {
  for (const t of tabs) t.classList.toggle("is-active", t.dataset.tab === name);
  for (const [key, el] of Object.entries(panes)) {
    el?.classList.toggle("is-active", key === name);
  }
}

async function boot(): Promise<void> {
  for (const t of tabs) {
    t.addEventListener("click", () => switchTab(t.dataset.tab ?? "structure"));
  }
  scanBtn.addEventListener("click", () => void runScan());

  try {
    rootInfo = await api.libInit();
    document.title = `便携资料库 · ${rootInfo.volumeSerial}`;

    const stats = await readStats();
    status.render(rootInfo, stats);

    // 监听状态先问一次：它决定界面要不要提示「手动刷新」
    try {
      status.setWatch(await api.watchInfo());
    } catch {
      /* 拿不到就当未知，不显示这一段 */
    }

    booted = true;

    // 事件订阅必须早于首次扫描：扫描期间产生的变更也要被接住
    await onLibraryChanged((r) => void onChanged(r));
    await onLibraryError((msg) => status.notice(`同步失败：${msg}`, "error"));

    if (stats.total === 0) {
      status.notice("首次使用，正在建立索引…", "info");
      await runScan();
    }
    await navigate(null);
  } catch (err) {
    booted = true;
    contentHost.replaceChildren(
      Object.assign(document.createElement("div"), {
        className: "placeholder error",
        textContent: (err as Error).message,
      }),
    );
    status.notice(`初始化失败：${(err as Error).message}`, "error");
  }

  // 与索引无关，独立于上面的成败
  await loadRuntimeAndProjects();
}

void boot();
