/**
 * 项目列表。
 *
 * 数据来源是索引上的一次查询（后端 `project_list`），不是单独的项目表 ——
 * 所以这里不存在「列表与磁盘不一致」这种状态，扫描完就是新的。
 *
 * 一个显示上的取舍：只标「运行时是否就绪」，不标「能不能编译」。
 * 后者需要真的跑一次构建，成本高且会污染盘（生成 target/、node_modules/）。
 */

import { api } from "../api/client";
import type { ProjectDto } from "../api/types";

const LANG_LABEL: Record<string, string> = {
  python: "py",
  node: "node",
  rust: "rs",
  go: "go",
  java: "java",
  php: "php",
  ruby: "rb",
  cpp: "c++",
};

export class ProjectsView {
  private host: HTMLElement;
  private onSelect: (project: ProjectDto) => void;

  constructor(host: HTMLElement, onSelect: (project: ProjectDto) => void) {
    this.host = host;
    this.onSelect = onSelect;
  }

  async load(): Promise<void> {
    this.host.replaceChildren(this.note("加载中…"));
    try {
      this.render(await api.projectList());
    } catch (err) {
      this.host.replaceChildren(this.note((err as Error).message, "error"));
    }
  }

  private render(items: ProjectDto[]): void {
    const frag = document.createDocumentFragment();

    const head = document.createElement("div");
    head.className = "rail-head";
    const title = document.createElement("span");
    title.textContent = "项目";
    const count = document.createElement("span");
    count.className = "rail-count";
    count.textContent = String(items.length);
    head.append(title, count);
    frag.append(head);

    if (items.length === 0) {
      frag.append(
        this.note("未识别到项目。需要目录里有 package.json / pyproject.toml 等标记文件。"),
      );
      this.host.replaceChildren(frag);
      return;
    }

    for (const p of items) {
      frag.append(this.row(p));
    }

    this.host.replaceChildren(frag);
  }

  private row(p: ProjectDto): HTMLElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "proj";

    const badge = document.createElement("span");
    badge.className = `proj-lang lang-${p.lang}`;
    badge.textContent = LANG_LABEL[p.lang] ?? p.lang;

    const main = document.createElement("span");
    main.className = "proj-main";

    const name = document.createElement("span");
    name.className = "proj-name";
    name.textContent = p.name;

    const path = document.createElement("span");
    path.className = "proj-path";
    path.textContent = p.relPath;

    main.append(name, path);

    const dot = document.createElement("span");
    const ready = p.runtimeReady;
    dot.className = `dot ${ready ? "ok" : "bad"}`;
    dot.title = p.runtimeKind
      ? ready
        ? `盘内 ${p.runtimeKind} 运行时可用`
        : `盘内缺少可用的 ${p.runtimeKind} 运行时`
      : "静态语言，无需盘内运行时";

    btn.append(badge, main, dot);
    btn.addEventListener("click", () => {
      for (const el of Array.from(this.host.querySelectorAll(".proj.is-active"))) {
        el.classList.remove("is-active");
      }
      btn.classList.add("is-active");
      this.onSelect(p);
    });
    return btn;
  }

  private note(text: string, tone: "info" | "error" = "info"): HTMLElement {
    const el = document.createElement("div");
    el.className = `rail-note${tone === "error" ? " is-error" : ""}`;
    el.textContent = text;
    return el;
  }
}
