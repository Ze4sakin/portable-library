/**
 * 目录视图 —— 中间栏的「结构页」。
 *
 * 这个视图回答的是三个问题，顺序也是界面的顺序：
 *   1. 我在哪？        → 面包屑（从属关系）
 *   2. 这一层分成了几块？→ 子目录列表，各带子树规模与占比（分类）
 *   3. 本级还有哪些东西？→ 文件列表（不按扩展名分组，按归属分组）
 *
 * 刻意**不**按文件类型分组。类型是文件的属性，不是它的归属；
 * 用类型切分会把「同一件事的散件」拆到不同格子里，反而看不出结构。
 */

import { api } from "../api/client";
import type { ChildOverview, DirOverview, NodeDto, TreeStats } from "../api/types";
import { formatBytes, formatShare, formatTime } from "../core/format";

export interface DirViewHandlers {
  /** 进入某个目录（`null` 为内容区根） */
  onEnter: (rel: string | null) => void;
  /** 打开一个文件（切到预览） */
  onOpen: (node: NodeDto) => void;
  /**
   * 双击文件：交给盘内编辑器（缺失时系统默认程序）打开。
   *
   * 和 `onOpen` 分开是因为这两件事的「代价」不同：单击是零风险的本地渲染，
   * 双击会**离开本程序**、起一个进程。混成一个 handler 就只能靠猜用户意图。
   */
  onOpenExternal: (node: NodeDto) => void;
  /** 在资源管理器中定位 */
  onReveal: (rel: string) => void;
}

export class DirectoryView {
  private host: HTMLElement;
  private handlers: DirViewHandlers;

  constructor(host: HTMLElement, handlers: DirViewHandlers) {
    this.host = host;
    this.handlers = handlers;
  }

  async show(rel: string | null): Promise<void> {
    this.host.replaceChildren(placeholder("读取目录结构…"));
    try {
      this.render(await api.dirOverview(rel));
    } catch (err) {
      this.host.replaceChildren(placeholder((err as Error).message, true));
    }
  }

  private render(o: DirOverview): void {
    const wrap = document.createElement("div");
    wrap.className = "dirview";

    wrap.append(this.crumbs(o), this.head(o), this.summary(o));

    const dirs = o.children.filter((c) => c.node.kind === "dir");
    const files = o.children.filter((c) => c.node.kind !== "dir");

    if (dirs.length > 0) {
      wrap.append(this.section("子目录", dirs.length, this.branchList(dirs, o.totals)));
    }
    if (files.length > 0) {
      wrap.append(this.section("本级文件", files.length, this.fileList(files)));
    }
    if (o.children.length === 0) {
      wrap.append(placeholder("这一层还没有东西。往盘里放文件，界面会自动跟上。"));
    }

    this.host.replaceChildren(wrap);
  }

  private crumbs(o: DirOverview): HTMLElement {
    const nav = document.createElement("nav");
    nav.className = "crumbs";

    o.breadcrumb.forEach((c, i) => {
      const isLast = i === o.breadcrumb.length - 1;
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = `crumb${isLast ? " is-current" : ""}`;
      btn.textContent = c.name;
      btn.disabled = isLast;
      btn.addEventListener("click", () => this.handlers.onEnter(c.rel || null));
      nav.append(btn);

      if (!isLast) {
        const sep = document.createElement("span");
        sep.className = "crumb-sep";
        sep.textContent = "/";
        nav.append(sep);
      }
    });

    return nav;
  }

  private head(o: DirOverview): HTMLElement {
    const row = document.createElement("div");
    row.className = "dirview-head";

    const left = document.createElement("div");
    const title = document.createElement("div");
    title.className = "dirview-title";
    title.textContent = o.name;
    left.append(title);

    const path = document.createElement("div");
    path.className = "dirview-path";
    path.textContent = o.rel ? `content/${o.rel}` : "content/";
    left.append(path);

    row.append(left);

    if (o.rel) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn btn-tiny";
      btn.textContent = "在资源管理器中定位";
      btn.addEventListener("click", () => this.handlers.onReveal(o.rel as string));
      row.append(btn);
    }

    return row;
  }

  private summary(o: DirOverview): HTMLElement {
    const row = document.createElement("div");
    row.className = "dirview-summary";

    row.append(
      chip(`${o.dirsDirect} 个子目录`, `${o.filesDirect} 个文件`),
      chip(
        `整枝 ${o.totals.files} 个文件`,
        `${o.totals.dirs} 个目录 · ${formatBytes(o.totals.bytes)}`,
      ),
      chip("最近更新", formatTime(o.totals.latestMtime)),
    );

    return row;
  }

  private section(title: string, count: number, body: HTMLElement): HTMLElement {
    const sec = document.createElement("section");
    sec.className = "dirview-section";

    const head = document.createElement("div");
    head.className = "dirview-section-head";
    const t = document.createElement("span");
    t.textContent = title;
    const c = document.createElement("span");
    c.className = "dirview-count";
    c.textContent = String(count);
    head.append(t, c);

    sec.append(head, body);
    return sec;
  }

  /** 子目录列表：规模条按同级最大值归一，一眼看出这一层是怎么被分掉的。 */
  private branchList(items: ChildOverview[], totals: TreeStats): HTMLElement {
    const list = document.createElement("div");
    list.className = "branch-list";

    const peak = items.reduce((m, c) => Math.max(m, c.subtree.bytes), 0);

    for (const item of items) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "branch";

      const top = document.createElement("div");
      top.className = "branch-top";

      const name = document.createElement("span");
      name.className = "branch-name";
      name.textContent = item.node.name;

      const size = document.createElement("span");
      size.className = "branch-size";
      const share = formatShare(item.subtree.bytes, totals.bytes);
      size.textContent = `${formatBytes(item.subtree.bytes)} · ${share}`;

      top.append(name, size);

      const track = document.createElement("div");
      track.className = "branch-track";
      const bar = document.createElement("div");
      bar.className = "branch-bar";
      const ratio = peak > 0 ? Math.max(0.02, item.subtree.bytes / peak) : 0.02;
      bar.style.width = `${(ratio * 100).toFixed(2)}%`;
      track.append(bar);

      const meta = document.createElement("div");
      meta.className = "branch-meta";
      meta.textContent = `${item.subtree.files} 个文件 · ${item.subtree.dirs} 个子目录 · ${formatTime(item.subtree.latestMtime)}`;

      btn.append(top, track, meta);
      btn.addEventListener("click", () => this.handlers.onEnter(item.node.relPath));
      list.append(btn);
    }

    return list;
  }

  private fileList(items: ChildOverview[]): HTMLElement {
    const list = document.createElement("div");
    list.className = "file-list";

    for (const item of items) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "file-row";

      const mark = document.createElement("span");
      mark.className = "file-mark";
      mark.textContent = extMark(item.node.ext);

      const name = document.createElement("span");
      name.className = "file-name";
      name.textContent = item.node.name;

      const size = document.createElement("span");
      size.className = "file-size";
      size.textContent = formatBytes(item.node.size);

      const time = document.createElement("span");
      time.className = "file-time";
      time.textContent = formatTime(item.node.mtime);

      btn.append(mark, name, size, time);
      btn.addEventListener("click", () => this.handlers.onOpen(item.node));
      // 双击 = 用外部程序打开（Windows 的老习惯）。单击预览已经换了右栏内容，
      // 双击只是接着把文件交给编辑器，两者不冲突。
      btn.title = "单击预览，双击用盘内编辑器打开";
      btn.addEventListener("dblclick", () => this.handlers.onOpenExternal(item.node));
      list.append(btn);
    }

    return list;
  }
}

function chip(label: string, value: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "chip";

  const l = document.createElement("span");
  l.className = "chip-label";
  l.textContent = label;

  const v = document.createElement("span");
  v.className = "chip-value";
  v.textContent = value;

  el.append(l, v);
  return el;
}

/** 扩展名只做一个小小的标记，不参与分组。 */
function extMark(ext: string | null): string {
  if (!ext) return "·";
  return ext.slice(0, 4).toUpperCase();
}

function placeholder(text: string, error = false): HTMLElement {
  const div = document.createElement("div");
  div.className = `placeholder${error ? " error" : ""}`;
  div.textContent = text;
  return div;
}
