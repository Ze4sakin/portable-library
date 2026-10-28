/**
 * 预览视图。
 *
 * P1 的边界：文本类内容内置渲染，其余格式给出操作卡片交给外部程序。
 * 不做的事情（留给 P2）：docx/xlsx 前端渲染、LibreOffice 精确预览、pdf.js。
 * 理由见 ARCHITECTURE.md「P1 裁剪说明」。
 */

import { api } from "../api/client";
import type { NodeDto, ProjectDto } from "../api/types";
import { formatBytes, formatTime } from "../core/format";

const TEXT_EXT = new Set([
  "md", "markdown", "txt", "log", "json", "toml", "yaml", "yml", "ini", "conf", "env",
  "csv", "tsv", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "css", "scss",
  "html", "htm", "xml", "py", "sh", "bat", "ps1", "go", "java", "kt", "c", "h",
  "cpp", "hpp", "cs", "rb", "php", "sql", "gitignore",
]);

/** 能交给盘内 LibreOffice 转 PDF 的格式 */
const OFFICE_EXT = new Set([
  "doc", "docx", "rtf", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp",
]);

/**
 * 盘内阅读器（SumatraPDF）能接的格式。
 *
 * 这份清单必须与 `src-tauri/src/runtime/pdf.rs` 的 `EXTS` 一致：
 * 前端只决定「说什么话」，后端才决定「交给谁」。两边不一致的表现是
 * 提示写着能用、点下去却走了系统默认程序（或反过来）。
 */
const PDF_EXT = new Set(["pdf", "xps", "oxps", "epub", "mobi", "cbz", "chm", "djvu"]);

type Tone = "info" | "error";
type Crumb = { name: string; rel: string };

export class PreviewView {
  private host: HTMLElement;
  private onNotice: (text: string, tone: Tone) => void;
  private onNavigate: (rel: string | null) => void;
  private officeAvailable = false;
  private pdfAvailable = false;

  constructor(
    host: HTMLElement,
    onNotice: (text: string, tone: Tone) => void = () => {},
    onNavigate: (rel: string | null) => void = () => {},
  ) {
    this.host = host;
    this.onNotice = onNotice;
    this.onNavigate = onNavigate;
  }

  /** 由运行时面板在探测完成后注入 —— 预览视图不自己发起探测。 */
  setOfficeAvailable(value: boolean): void {
    this.officeAvailable = value;
  }

  /** 同上。两个能力来自同一次探测快照，但分别注入，因为这个视图只关心各自的有无。 */
  setPdfAvailable(value: boolean): void {
    this.pdfAvailable = value;
  }

  empty(text = "从左侧选择一个文件"): void {
    const div = document.createElement("div");
    div.className = "placeholder";
    div.textContent = text;
    this.host.replaceChildren(div);
  }

  async show(node: NodeDto, crumbs: Crumb[] = []): Promise<void> {
    if (node.kind === "dir") {
      this.empty(`「${node.name}」是一个目录`);
      return;
    }

    const ext = (node.ext ?? "").toLowerCase();
    if (!TEXT_EXT.has(ext)) {
      this.showActionCard(node, crumbs);
      return;
    }

    this.empty("读取中…");
    try {
      const preview = await api.readText(node.relPath);
      this.showText(node, preview.content, preview.truncated, preview.encoding, crumbs);
    } catch (err) {
      this.showError((err as Error).message);
    }
  }

  /** 文件的位置链。点任一级就跳回那个目录 —— 「从属关系」要能走。 */
  private crumbs(crumbs: Crumb[]): HTMLElement | null {
    const usable = crumbs.filter((c) => c.rel !== "" || crumbs.length > 1);
    if (usable.length === 0) return null;

    const nav = document.createElement("nav");
    nav.className = "crumbs";

    crumbs.forEach((c, i) => {
      const isLast = i === crumbs.length - 1;
      if (isLast) {
        const span = document.createElement("span");
        span.className = "crumb is-current";
        span.textContent = c.name;
        nav.append(span);
      } else {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = "crumb";
        btn.textContent = c.name;
        btn.addEventListener("click", () => this.onNavigate(c.rel || null));
        nav.append(btn);

        const sep = document.createElement("span");
        sep.className = "crumb-sep";
        sep.textContent = "/";
        nav.append(sep);
      }
    });

    return nav;
  }

  private showText(
    node: NodeDto,
    content: string,
    truncated: boolean,
    encoding: string,
    crumbs: Crumb[],
  ): void {
    const wrap = document.createElement("div");
    wrap.className = "preview-body";

    const crumbBar = this.crumbs(crumbs);
    if (crumbBar) wrap.append(crumbBar);

    const head = document.createElement("div");
    head.className = "preview-head";
    head.append(
      this.field(node.name, "strong"),
      this.field(`${formatBytes(node.size)} · ${formatTime(node.mtime)} · ${encoding}`),
    );

    if (truncated) {
      const tip = document.createElement("div");
      tip.className = "preview-tip";
      tip.textContent = "文件较大，仅显示前 256 KB";
      head.append(tip);
    }

    const body = document.createElement("div");
    body.className = "preview-content";
    const ext = (node.ext ?? "").toLowerCase();
    if (ext === "md" || ext === "markdown") {
      body.innerHTML = renderMarkdown(content);
      body.classList.add("is-markdown");
    } else {
      const pre = document.createElement("pre");
      pre.className = "code-block";
      pre.textContent = content;
      body.append(pre);
    }

    wrap.append(head, this.actions(node), body);
    this.host.replaceChildren(wrap);
  }

  private showActionCard(node: NodeDto, crumbs: Crumb[]): void {
    const wrap = document.createElement("div");
    wrap.className = "preview-body";

    const crumbBar = this.crumbs(crumbs);
    if (crumbBar) wrap.append(crumbBar);

    const head = document.createElement("div");
    head.className = "preview-head";
    head.append(
      this.field(node.name, "strong"),
      this.field(`${formatBytes(node.size)} · ${formatTime(node.mtime)}`),
    );

    const ext = (node.ext ?? "").toLowerCase();
    // 两个集合不重叠（OFFICE_EXT ∩ PDF_EXT = ∅），所以不必互相排除。
    const convertible = this.officeAvailable && OFFICE_EXT.has(ext);
    const usePortablePdf = this.pdfAvailable && PDF_EXT.has(ext);

    const note = document.createElement("div");
    note.className = "preview-tip";
    if (usePortablePdf) {
      note.textContent =
        "固定版式文档不做内置渲染。下面的按钮会用盘内阅读器打开 —— 阅读位置与缩放记在盘内，不写这台机器。";
    } else if (convertible) {
      note.textContent = `该格式不做内置渲染。可用盘内 LibreOffice 无头转成 PDF，再用${
        this.pdfAvailable ? "盘内阅读器" : "系统阅读器"
      }看。`;
    } else {
      note.textContent = "该格式暂不支持内置预览，可用系统程序打开";
    }

    const bar = this.actions(node, usePortablePdf ? "用盘内阅读器打开" : undefined);
    if (convertible) {
      bar.insertBefore(this.convertButton(node, crumbs), bar.firstChild);
    }

    wrap.append(head, note, bar);
    this.host.replaceChildren(wrap);
  }

  /** 编辑接力的第一跳：Office → PDF。渲染保真交给 LibreOffice，不自己实现。 */
  private convertButton(node: NodeDto, crumbs: Crumb[]): HTMLButtonElement {
    return this.actionButton("用盘内 Office 转 PDF", "primary", async () => {
      this.empty("正在转换（首次启动 LibreOffice 会慢几秒）…");
      const result = await api.docConvert(node.relPath);
      await api.openCached(result.relInCache);
      const reader = this.pdfAvailable ? "盘内阅读器" : "系统阅读器";
      this.onNotice(`已转换 ${result.durationMs} ms → ${result.relInCache}（已交给${reader}）`, "info");
      this.showActionCard(node, crumbs);
    });
  }

  /**
   * 项目卡片。
   *
   * 这里刻意只给「开终端」而不给「运行」按钮：`npm run dev` / `pytest main.py`
   * 的形态千差万别，一个通用启动器必然要写一套项目配置文件（属于 P3）。
   * 而终端 + 已就绪的环境，覆盖了 90% 的实际操作，且零配置。
   */
  showProject(p: ProjectDto): void {
    const wrap = document.createElement("div");
    wrap.className = "preview-body";

    const head = document.createElement("div");
    head.className = "preview-head";
    head.append(
      this.field(p.name, "strong"),
      this.field(p.relPath),
      this.field(`主语言 ${p.lang}`),
    );

    const body = document.createElement("div");
    body.className = "preview-content";
    const detail = document.createElement("pre");
    detail.className = "code-block";
    const runtimeLine = p.runtimeKind
      ? `${p.runtimeKind}（${p.runtimeReady ? "已就绪" : "盘内缺失"}）`
      : "无需盘内运行时";
    detail.textContent = [
      `路径     ${p.relPath}`,
      `主语言   ${p.lang}`,
      `标记文件 ${p.markers.join("  ")}`,
      `运行时   ${runtimeLine}`,
    ].join("\n");
    body.append(detail);

    const bar = document.createElement("div");
    bar.className = "preview-actions";
    bar.append(
      this.actionButton("在此目录打开终端", "primary", async () => {
        await api.projectTerminal(p.relPath);
        this.onNotice(`已打开终端：${p.relPath}`, "info");
      }),
      this.actionButton("在资源管理器中定位", "ghost", async () => {
        await api.openTarget(p.relPath, "reveal");
      }),
    );

    const tip = document.createElement("div");
    tip.className = "preview-tip";
    tip.textContent = "终端的工作目录与 PATH 已指向盘内运行时，无需额外激活环境。";

    wrap.append(head, bar, tip, body);
    this.host.replaceChildren(wrap);
  }

  private actions(node: NodeDto, editLabel = "用外部程序打开"): HTMLElement {
    const bar = document.createElement("div");
    bar.className = "preview-actions";

    bar.append(
      this.actionButton(editLabel, "primary", async () => {
        const r = await api.openTarget(node.relPath, "edit");
        if (r.outcome === "external") {
          bar.dataset.last = `已交给 ${r.program ?? "外部程序"}`;
        }
      }),
      this.actionButton("在资源管理器中定位", "ghost", async () => {
        await api.openTarget(node.relPath, "reveal");
      }),
    );

    return bar;
  }

  private actionButton(
    text: string,
    kind: "primary" | "ghost",
    run: () => Promise<void>,
  ): HTMLButtonElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `btn ${kind === "primary" ? "btn-primary" : ""}`;
    btn.textContent = text;
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      try {
        await run();
      } catch (err) {
        this.showError((err as Error).message);
      } finally {
        btn.disabled = false;
      }
    });
    return btn;
  }

  private field(text: string, weight?: "strong"): HTMLElement {
    const el = document.createElement("div");
    el.className = weight === "strong" ? "preview-title" : "preview-meta";
    el.textContent = text;
    return el;
  }

  private showError(message: string): void {
    const div = document.createElement("div");
    div.className = "placeholder error";
    div.textContent = message;
    this.host.replaceChildren(div);
  }
}

function escapeHtml(src: string): string {
  return src.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/**
 * 极简 Markdown 渲染。
 * 安全前提：**先整体转义，再做结构替换**，因此不会引入脚本。
 * 完整渲染（表格、脚注、任务列表）留给 P2 接 markdown-it。
 */
function renderMarkdown(src: string): string {
  const lines = escapeHtml(src).split(/\r?\n/);
  const out: string[] = [];
  let inCode = false;
  let inList = false;

  const closeList = () => {
    if (inList) {
      out.push("</ul>");
      inList = false;
    }
  };

  for (const line of lines) {
    if (line.startsWith("```")) {
      closeList();
      if (inCode) {
        out.push("</code></pre>");
        inCode = false;
      } else {
        out.push('<pre class="code-block"><code>');
        inCode = true;
      }
      continue;
    }

    if (inCode) {
      out.push(line);
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading && heading[1] !== undefined && heading[2] !== undefined) {
      const level = heading[1].length;
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }

    const bullet = /^\s*[-*]\s+(.*)$/.exec(line);
    if (bullet && bullet[1] !== undefined) {
      if (!inList) {
        out.push("<ul>");
        inList = true;
      }
      out.push(`<li>${inline(bullet[1])}</li>`);
      continue;
    }

    closeList();

    if (line.trim() === "") {
      continue;
    }
    out.push(`<p>${inline(line)}</p>`);
  }

  closeList();
  if (inCode) out.push("</code></pre>");
  return out.join("\n");
}

function inline(src: string): string {
  return src
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
}
