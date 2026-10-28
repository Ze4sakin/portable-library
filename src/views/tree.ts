/**
 * 文件树视图。
 *
 * 关键决策：**懒加载**。
 * 索引里可能有几十万节点，但一次只取一层目录的子项（`list_children`），
 * 因此前端内存占用与目录深度成正比，与文件总数无关。
 *
 * 另两条为「实时同步」服务的能力：
 * - `refresh()` 原地重建已展开的层级，**保留展开状态**。整棵树推倒重来会
 *   让用户正在看的目录突然折叠，比不刷新还烦人。
 * - `reveal()` 把某个路径逐级展开并选中，供面包屑与结构面板跳转使用，
 *   否则「中间栏跳到某处、左栏还停在别处」会让人失去位置感。
 */

import { api } from "../api/client";
import type { NodeDto } from "../api/types";

export class TreeView {
  private host: HTMLElement;
  private onSelect: (node: NodeDto) => void;
  private activeRel: string | null = null;

  constructor(host: HTMLElement, onSelect: (node: NodeDto) => void) {
    this.host = host;
    this.onSelect = onSelect;
  }

  async load(): Promise<void> {
    this.host.replaceChildren();
    this.activeRel = null;

    const ul = document.createElement("ul");
    ul.className = "tree-list tree-root";
    ul.dataset.parent = "";
    this.host.append(ul);
    await this.fill(ul, null);
  }

  /**
   * 原地刷新：为每个已展开的层重新取一次子项，保留展开状态与选中项。
   *
   * 只重建「看得见」的部分。折叠着的分支下次展开时自然会取到最新数据，
   * 没必要为它们付出查询代价。
   */
  async refresh(): Promise<void> {
    const root = this.host.querySelector<HTMLUListElement>("ul.tree-root");
    if (!root) return;
    await this.rebuild(root);
    if (this.activeRel) this.highlight(this.activeRel);
  }

  /** 逐级展开到 `rel` 并选中。路径不在索引里时安静放弃。 */
  async reveal(rel: string): Promise<void> {
    const segs = rel.split("/").filter(Boolean);
    let ul = this.host.querySelector<HTMLUListElement>("ul.tree-root");
    if (!ul) return;

    for (let i = 0; i < segs.length; i += 1) {
      const target = segs.slice(0, i + 1).join("/");
      const row = findRow(ul, target);
      if (!row) return;

      if (i === segs.length - 1) {
        this.select(row, target);
        row.scrollIntoView({ block: "nearest" });
        return;
      }

      const li = row.parentElement as HTMLLIElement;
      const child = await this.ensureExpanded(li, row, target);
      ul = child;
    }
  }

  private async rebuild(ul: HTMLUListElement): Promise<void> {
    const parentRel = ul.dataset.parent ?? "";
    const expanded: string[] = [];

    for (const childUl of Array.from(
      ul.querySelectorAll<HTMLUListElement>(":scope > li > ul.tree-list"),
    )) {
      if (childUl.classList.contains("collapsed")) continue;
      const li = childUl.parentElement as HTMLLIElement;
      const row = li.querySelector<HTMLElement>(":scope > .tree-row");
      if (row?.dataset.rel) expanded.push(row.dataset.rel);
    }

    ul.replaceChildren();
    await this.fill(ul, parentRel === "" ? null : parentRel);

    for (const rel of expanded) {
      const row = findRow(ul, rel);
      if (!row) continue;
      const li = row.parentElement as HTMLLIElement;
      const child = await this.ensureExpanded(li, row, rel);
      await this.rebuild(child);
    }
  }

  private async fill(ul: HTMLUListElement, parentRel: string | null): Promise<void> {
    let nodes: NodeDto[];
    try {
      nodes = await api.listChildren(parentRel);
    } catch (err) {
      ul.append(this.hint((err as Error).message));
      return;
    }

    if (nodes.length === 0) {
      if (parentRel !== null) ul.append(this.hint("（空目录）"));
      return;
    }

    for (const node of nodes) {
      ul.append(this.makeRow(node));
    }
  }

  private makeRow(node: NodeDto): HTMLLIElement {
    const li = document.createElement("li");
    li.className = "tree-item";

    const row = document.createElement("div");
    row.className = `tree-row ${node.kind === "dir" ? "is-dir" : "is-file"}`;
    row.dataset.rel = node.relPath;

    const caret = document.createElement("span");
    caret.className = "tree-caret";
    // 只有真的有子项才给展开箭头 —— 空目录给箭头会点出一片空白
    caret.textContent = node.hasChildren ? "▸" : "";
    row.append(caret);

    const label = document.createElement("span");
    label.className = "tree-label";
    label.textContent = node.name;
    label.title = node.relPath;
    row.append(label);

    if (node.kind === "dir") {
      const badge = document.createElement("span");
      badge.className = "tree-badge";
      badge.textContent = "DIR";
      row.append(badge);
    }

    row.addEventListener("click", () => {
      this.select(row, node.relPath);
      this.onSelect(node);
      if (node.kind === "dir") void this.toggle(li, row, caret, node);
    });

    li.append(row);
    return li;
  }

  private async toggle(
    li: HTMLLIElement,
    row: HTMLElement,
    caret: HTMLElement,
    node: NodeDto,
  ): Promise<void> {
    const existing = li.querySelector<HTMLUListElement>(":scope > ul.tree-list");

    if (existing) {
      const collapsed = existing.classList.toggle("collapsed");
      caret.textContent = collapsed ? "▸" : "▾";
      return;
    }

    if (!node.hasChildren) return;
    caret.textContent = "▾";
    await this.ensureExpanded(li, row, node.relPath, caret);
  }

  private async ensureExpanded(
    li: HTMLLIElement,
    row: HTMLElement,
    rel: string,
    caret?: HTMLElement,
  ): Promise<HTMLUListElement> {
    const existing = li.querySelector<HTMLUListElement>(":scope > ul.tree-list");
    if (existing) {
      existing.classList.remove("collapsed");
      if (caret) caret.textContent = "▾";
      return existing;
    }

    const ul = document.createElement("ul");
    ul.className = "tree-list";
    ul.dataset.parent = rel;
    li.append(ul);

    row.classList.add("loading");
    await this.fill(ul, rel);
    row.classList.remove("loading");

    const arrow = caret ?? row.querySelector<HTMLElement>(".tree-caret");
    if (arrow) arrow.textContent = "▾";
    return ul;
  }

  private select(row: HTMLElement, rel: string): void {
    this.host.querySelector(".tree-row.is-active")?.classList.remove("is-active");
    row.classList.add("is-active");
    this.activeRel = rel;
  }

  private highlight(rel: string): void {
    const root = this.host.querySelector<HTMLUListElement>("ul.tree-root");
    if (!root) return;
    this.host.querySelector(".tree-row.is-active")?.classList.remove("is-active");
    // 路径可能已被折叠，找不到就不强求 —— 中间栏才是当前位置的权威显示
    for (const row of Array.from(root.querySelectorAll<HTMLElement>(".tree-row"))) {
      if (row.dataset.rel === rel) {
        row.classList.add("is-active");
        return;
      }
    }
  }

  private hint(text: string): HTMLLIElement {
    const li = document.createElement("li");
    li.className = "tree-hint";
    li.textContent = text;
    return li;
  }
}

function findRow(ul: HTMLUListElement, rel: string): HTMLElement | null {
  for (const li of Array.from(ul.children)) {
    const row = li.querySelector<HTMLElement>(":scope > .tree-row");
    if (row?.dataset.rel === rel) return row;
  }
  return null;
}
