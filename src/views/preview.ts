/**
 * 预览视图。
 *
 * 文本类内容分两种看法，由用户当场选：
 *   - **预览**：md 走 markdown-it 渲染、html 按盘内原样渲染，都装在沙箱 iframe 里
 *   - **源码**：原文照抄，方便复制
 *
 * 为什么渲染一定要 iframe：文档里的相对路径（同目录的 css、图片）需要一个
 * 「以盘内目录为基准」的 URL 才能加载，而 `srcdoc` 的基准是父页面。为此后端
 * 提供 `preview_base`，前端拼成 asset 协议地址写进 `<base href>`。
 * 沙箱里不给 `allow-scripts` —— 盘里的 HTML 是**用户内容**，不是本程序的代码：
 * 看，不需要跑。要看交互效果的，用外部程序打开。
 *
 * 不做的事情（留给 P2）：docx/xlsx 前端渲染、LibreOffice 精确预览、pdf.js。
 * 理由见 ARCHITECTURE.md「P1 裁剪说明」。
 */

import MarkdownIt from "markdown-it";

import { api } from "../api/client";
import type { NodeDto, ProjectDto, TextPreview } from "../api/types";
import { formatBytes, formatTime } from "../core/format";

const TEXT_EXT = new Set([
  "md", "markdown", "txt", "log", "json", "toml", "yaml", "yml", "ini", "conf", "env",
  "csv", "tsv", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "css", "scss",
  "html", "htm", "xml", "py", "sh", "bat", "ps1", "go", "java", "kt", "c", "h",
  "cpp", "hpp", "cs", "rb", "php", "sql", "gitignore",
]);

/** 能渲染成本文之外观感的格式：md 走 markdown-it，html 按原样。 */
const MARKDOWN_EXT = new Set(["md", "markdown", "mdown", "mkd"]);
const HTML_EXT = new Set(["html", "htm", "xhtml", "shtml"]);

/**
 * Markdown 渲染器。
 *
 * `html: false` 是硬要求：md 里夹的原生 HTML 一律转义，不参与解析。
 * 渲染结果随后被丢进 `sandbox=""` 的 iframe，两道防线是重复的 —— 但一份
 * 来自移动盘的文档不该有任何执行机会，重复在这里不算浪费。
 */
const md = new MarkdownIt({ html: false, linkify: false, breaks: false });


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

/**
 * 盘内编辑器（Notepad++）接管的格式。
 *
 * 同样必须与 `src-tauri/src/runtime/notepadpp.rs` 的 `EXTS` 一致。
 * 这里只用于「换个按钮文案、加一句提示」，真正的路由在后端 registry ——
 * 前端不做任何「点下去会交给谁」的判断，否则托盘后会多出一份会过期的真相。
 */
const EDITOR_EXT = new Set(["md", "markdown", "mdown", "mkd", "html", "htm", "xhtml", "shtml"]);

type Tone = "info" | "error";
type Crumb = { name: string; rel: string };

/** 已取回、正在展示的文本。切「预览 / 源码」时用它重画，不必再读盘。 */
type ShownText = {
  node: NodeDto;
  content: string;
  truncated: boolean;
  encoding: string;
  ext: string;
  /** 渲染基准目录（绝对路径）。取不到时为 null，此时只看得到内联样式。 */
  base: string | null;
  crumbs: Crumb[];
};

export class PreviewView {
  private host: HTMLElement;
  private onNotice: (text: string, tone: Tone) => void;
  private onNavigate: (rel: string | null) => void;
  private officeAvailable = false;
  private pdfAvailable = false;
  private editorAvailable = false;
  /**
   * 用户上次选的看法。跨文件保持 —— 习惯看源码的人不该每点一个文件重新切一次。
   * 只影响 md / html，其余格式没有第二种看法。
   */
  private sourceMode = false;
  private current: ShownText | null = null;

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

  /** 盘内编辑器是否可用。决定 md/html 的主按钮是「用盘内 Notepad++ 编辑」还是系统默认程序。 */
  setEditorAvailable(value: boolean): void {
    this.editorAvailable = value;
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
    const renderable = MARKDOWN_EXT.has(ext) || HTML_EXT.has(ext);
    try {
      const [preview, base] = await Promise.all([
        api.readText(node.relPath),
        // 基准目录取不到不影响「看」，不值得把整个预览打断
        renderable ? api.previewBase(node.relPath).catch(() => null) : Promise.resolve(null),
      ]);
      this.showText(node, preview, ext, base, crumbs);
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
    preview: TextPreview,
    ext: string,
    base: string | null,
    crumbs: Crumb[],
  ): void {
    this.current = {
      node,
      content: preview.content,
      truncated: preview.truncated,
      encoding: preview.encoding,
      ext,
      base,
      crumbs,
    };
    this.renderText();
  }

  /**
   * 按当前「预览 / 源码」选择重画。
   *
   * 用 `this.current` 里缓存的正文，而不是再调一次 `readText`：切个看法
   * 不该产生 I/O。文件真被改了，watcher 会推 `library:changed`，
   * 上层重新 `show()` —— 那条路才读盘。
   */
  private renderText(): void {
    const cur = this.current;
    if (!cur) return;

    const { node, ext, crumbs } = cur;
    const renderable = MARKDOWN_EXT.has(ext) || HTML_EXT.has(ext);

    // 能否交给盘内编辑器，取决于「盘里有」×「格式归它管」两件事。
    // 后者必须与后端 registry 的判断同源，否则会出现「文案说 Notepad++、
    // 点下去走系统默认程序」这种最难解释的错位。
    const editable = this.editorAvailable && EDITOR_EXT.has(ext);

    const wrap = document.createElement("div");
    wrap.className = "preview-body";

    const crumbBar = this.crumbs(crumbs);
    if (crumbBar) wrap.append(crumbBar);

    const head = document.createElement("div");
    head.className = "preview-head";
    head.append(
      this.field(node.name, "strong"),
      this.field(`${formatBytes(node.size)} · ${formatTime(node.mtime)} · ${cur.encoding}`),
    );

    if (cur.truncated) {
      const tip = document.createElement("div");
      tip.className = "preview-tip";
      tip.textContent = "文件较大，仅显示前 256 KB";
      head.append(tip);
    }

    const bar = this.actions(node, editable ? "用盘内 Notepad++ 编辑" : undefined);
    if (renderable) bar.append(this.modeSwitch());

    wrap.append(head, bar);

    for (const line of this.modeTips(ext, renderable, editable)) {
      const tip = document.createElement("div");
      tip.className = "preview-tip";
      tip.textContent = line;
      wrap.append(tip);
    }

    wrap.append(this.contentBody(ext, cur.content, cur.base, renderable));
    this.host.replaceChildren(wrap);
  }

  private modeTips(ext: string, renderable: boolean, editable: boolean): string[] {
    const out: string[] = [];
    if (renderable) {
      if (this.sourceMode) {
        out.push("源码模式。切到「预览」看渲染结果。");
      } else if (MARKDOWN_EXT.has(ext)) {
        out.push("内置渲染（只读）：标题、列表、表格、代码块、图片都按 Markdown 语义排版。");
      } else {
        out.push(
          "按盘内原样渲染，同目录的 CSS / 图片会一起加载。为安全起见脚本不执行 —— 要看交互效果，用外部程序打开。",
        );
      }
    }
    if (editable) {
      out.push("「用盘内 Notepad++ 编辑」改完保存，这里会跟着重画。");
    }
    return out;
  }

  private contentBody(
    ext: string,
    content: string,
    base: string | null,
    renderable: boolean,
  ): HTMLElement {
    const body = document.createElement("div");

    if (renderable && !this.sourceMode) {
      body.className = "preview-content is-render";
      body.append(this.frame(ext, content, base));
      return body;
    }

    body.className = "preview-content";
    const pre = document.createElement("pre");
    pre.className = "code-block";
    pre.textContent = content;
    body.append(pre);
    return body;
  }

  private frame(ext: string, content: string, base: string | null): HTMLIFrameElement {
    const iframe = document.createElement("iframe");
    iframe.className = "preview-frame";
    // 空 sandbox = 全禁：不执行脚本、不提交表单、不跳顶层。相对资源照常加载。
    iframe.setAttribute("sandbox", "");
    iframe.setAttribute("referrerpolicy", "no-referrer");
    iframe.srcdoc = MARKDOWN_EXT.has(ext)
      ? markdownDocument(md.render(content), base)
      : htmlDocument(content, base);
    return iframe;
  }

  private modeSwitch(): HTMLElement {
    const seg = document.createElement("div");
    seg.className = "seg";
    const pick = (label: string, source: boolean) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = this.sourceMode === source ? "seg-btn is-active" : "seg-btn";
      btn.textContent = label;
      btn.addEventListener("click", () => {
        if (this.sourceMode === source) return;
        this.sourceMode = source;
        this.renderText();
      });
      return btn;
    };
    seg.append(pick("预览", false), pick("源码", true));
    return seg;
  }

  private showActionCard(node: NodeDto, crumbs: Crumb[]): void {
    this.current = null;
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
    this.current = null;
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

function escapeAttr(src: string): string {
  return src.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");
}

/**
 * `<base href>` —— 渲染保真的关键一行。
 *
 * `convertFileSrc` 把整条路径塞进**一个** path segment（`\` → `%5C`），
 * 所以取自它的 URL 末尾没有 `/`。按 URL 解析规则，「不以斜杠结尾的路径」
 * 最后一段会被相对引用**整个替换掉**（`...%5Cdocs` + `a.png` → `.../a.png`），
 * 目录就此丢掉。补一个 `/` 让基准退回目录层，解码后是 `F:/盘/目录/a.png` ——
 * Windows 的文件 API 认混合分隔符，asset 协议这一侧照常打开。
 */
function baseTag(base: string | null): string {
  if (!base) return "";
  return `<base href="${escapeAttr(`${api.assetUrl(base)}/`)}">`;
}

/** md 渲染结果 → 完整文档。样式内联，跟应用同一套暗色变量（iframe 里没有父页面样式）。 */
function markdownDocument(html: string, base: string | null): string {
  return `<!DOCTYPE html><html lang="zh-CN"><head><meta charset="utf-8">${baseTag(base)}<style>${DOC_CSS}</style></head><body class="doc">${html}</body></html>`;
}

/**
 * 盘里的 HTML → 可渲染文档，只做一件事：插入 `<base>`。
 *
 * 必须插在 `<head>` 之后：`<base>` 只影响**它之后**出现的相对 URL，
 * 插在末尾等于没插。没有 `<head>` 的（片段式 HTML）补一个骨架 —— 否则
 * base 无处安放，CSS 与图片依旧全丢。
 */
function htmlDocument(src: string, base: string | null): string {
  const tag = baseTag(base);
  if (!tag) return src;

  const head = /<head\b[^>]*>/i.exec(src);
  if (head) {
    const at = head.index + head[0].length;
    return src.slice(0, at) + tag + src.slice(at);
  }

  return `<!DOCTYPE html><html><head><meta charset="utf-8">${tag}</head><body>${src}</body></html>`;
}

/**
 * md 文档的排版样式。
 *
 * 不复用应用主样式：那份 CSS 是按三栏布局写的（`.preview-content` 之类），
 * 塞进 iframe 只会带来一堆不生效的规则。这里只留「一份文档该有的样子」。
 */
const DOC_CSS = `
:root { color-scheme: dark; }
html { background: #232322; }
body {
  margin: 0;
  padding: 18px 22px 48px;
  color: #e6e4dd;
  background: #232322;
  font: 13.5px/1.75 "Segoe UI", "Microsoft YaHei UI", system-ui, sans-serif;
  -webkit-font-smoothing: antialiased;
  overflow-wrap: break-word;
}
h1, h2, h3, h4, h5, h6 { margin: 22px 0 10px; font-weight: 600; line-height: 1.35; }
h1 { font-size: 21px; padding-bottom: 7px; border-bottom: 1px solid #3a3a37; }
h2 { font-size: 17px; padding-bottom: 5px; border-bottom: 1px solid #3a3a37; }
h3 { font-size: 15px; }
h4, h5, h6 { font-size: 13.5px; }
p, ul, ol, pre, table, blockquote { margin: 10px 0; }
ul, ol { padding-left: 22px; }
li { margin: 3px 0; }
li > p { margin: 4px 0; }
a { color: #5dcaa5; text-decoration: none; }
a:hover { text-decoration: underline; }
code {
  padding: 1px 5px;
  font-family: ui-monospace, "Cascadia Mono", Consolas, "Sarasa Mono SC", monospace;
  font-size: 12.5px;
  background: #2a2a28;
  border-radius: 4px;
}
pre {
  padding: 12px 14px;
  overflow: auto;
  background: #1b1b1a;
  border: 1px solid #3a3a37;
  border-radius: 8px;
}
pre code { padding: 0; background: none; font-size: 12.5px; line-height: 1.6; }
blockquote {
  padding: 2px 14px;
  color: #a3a099;
  border-left: 3px solid #1d9e75;
}
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th, td { padding: 6px 10px; text-align: left; vertical-align: top; border: 1px solid #3a3a37; }
th { font-weight: 600; background: #2a2a28; }
hr { margin: 20px 0; border: 0; border-top: 1px solid #3a3a37; }
img { max-width: 100%; height: auto; }
input[type="checkbox"] { margin-right: 6px; }
`;

