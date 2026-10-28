/**
 * 结构全景 —— 右栏的「分类」页。
 *
 * 展示的是**内容区根这一层怎么被分掉的**：每块多大、装了多少文件、最近动过没有。
 * 它不随浏览位置变化（始终是全景），只把「你现在在哪一支」高亮出来 ——
 * 全景图一移动就失去了参照价值。
 *
 * 与目录视图的分工：目录视图回答「这一层有什么」，全景回答「整块盘是怎么组织的」。
 */

import { api } from "../api/client";
import type { ChildOverview, DirOverview } from "../api/types";
import { formatBytes, formatShare } from "../core/format";

export class StructureView {
  private host: HTMLElement;
  private onNavigate: (rel: string | null) => void;

  constructor(host: HTMLElement, onNavigate: (rel: string | null) => void) {
    this.host = host;
    this.onNavigate = onNavigate;
  }

  async load(currentRel: string | null = null): Promise<void> {
    try {
      this.render(await api.dirOverview(null), currentRel);
    } catch (err) {
      this.host.replaceChildren(note((err as Error).message, true));
    }
  }

  private render(o: DirOverview, currentRel: string | null): void {
    const frag = document.createDocumentFragment();

    const head = document.createElement("div");
    head.className = "rail-head";
    const title = document.createElement("span");
    title.textContent = "结构 / 分类";
    const count = document.createElement("span");
    count.className = "rail-count";
    count.textContent = `${o.dirsDirect} 支 · ${o.filesDirect} 个根层文件`;
    head.append(title, count);
    frag.append(head);

    if (currentRel) {
      const back = document.createElement("button");
      back.type = "button";
      back.className = "btn btn-tiny rail-back";
      back.textContent = "← 回到全部内容";
      back.addEventListener("click", () => this.onNavigate(null));
      frag.append(back);

      const where = document.createElement("div");
      where.className = "rail-path";
      where.textContent = `当前：content/${currentRel}`;
      frag.append(where);
    }

    const branches = o.children.filter((c) => c.node.kind === "dir");
    if (branches.length === 0) {
      frag.append(note("内容区根层还没有子目录。分类靠目录结构表达，先在盘里建几个目录。"));
      this.host.replaceChildren(frag);
      return;
    }

    const peak = branches.reduce((m, c) => Math.max(m, c.subtree.bytes), 0);
    const top = currentRel ? currentRel.split("/")[0] : null;

    for (const b of branches) {
      frag.append(this.branch(b, peak, o.totals.bytes, b.node.relPath === top));
    }

    this.host.replaceChildren(frag);
  }

  private branch(item: ChildOverview, peak: number, whole: number, isActive: boolean): HTMLElement {
    const row = document.createElement("button");
    row.type = "button";
    row.className = `sbranch${isActive ? " is-active" : ""}`;

    const top = document.createElement("div");
    top.className = "sbranch-top";

    const name = document.createElement("span");
    name.className = "sbranch-name";
    name.textContent = item.node.name;

    const stat = document.createElement("span");
    stat.className = "sbranch-stat";
    stat.textContent = `${item.subtree.files} · ${formatBytes(item.subtree.bytes)}`;

    top.append(name, stat);

    const track = document.createElement("div");
    track.className = "sbranch-track";
    const bar = document.createElement("div");
    bar.className = "sbranch-bar";
    const ratio = peak > 0 ? Math.max(0.02, item.subtree.bytes / peak) : 0.02;
    bar.style.width = `${(ratio * 100).toFixed(2)}%`;
    track.append(bar);

    const share = document.createElement("div");
    share.className = "sbranch-share";
    share.textContent = `占 ${formatShare(item.subtree.bytes, whole)}`;

    row.append(top, track, share);
    row.addEventListener("click", () => this.onNavigate(item.node.relPath));
    return row;
  }
}

function note(text: string, error = false): HTMLElement {
  const el = document.createElement("div");
  el.className = `rail-note${error ? " is-error" : ""}`;
  el.textContent = text;
  return el;
}
