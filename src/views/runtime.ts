/**
 * 运行时面板：显示盘内组件的现场状态。
 *
 * 一条刻意的克制：这里**不提供安装按钮**。
 * 组件的获取（下载解压、选版本、放脚本）是一次性的装配动作，属于构建期而非运行期。
 * 把「装运行时」做成界面按钮，会诱导用户在一块盘上调三个版本，
 * 而每一个版本都要额外维护隔离参数 —— 这正是便携系统最容易失控的地方。
 */

import { api } from "../api/client";
import type { RuntimeComponent, RuntimeStatus } from "../api/types";

type Tone = "info" | "error";

export class RuntimeView {
  private host: HTMLElement;
  private onNotice: (text: string, tone: Tone) => void;
  /**
   * 每次探测成功后回调。
   *
   * 存在的理由只有一个：预览视图的按钮文案依赖「盘内有没有这个组件」，
   * 而用户刚把组件拷进盘、点一下「重新探测」时，预览那边也必须立刻改口。
   * 没有这个回调，就得让 main 在每次 load 后自己再读一遍状态 —— 那是同一份
   * 事实的第二处来源。
   */
  private onStatus: (status: RuntimeStatus) => void;

  constructor(
    host: HTMLElement,
    onNotice: (text: string, tone: Tone) => void,
    onStatus: (status: RuntimeStatus) => void = () => {},
  ) {
    this.host = host;
    this.onNotice = onNotice;
    this.onStatus = onStatus;
  }

  async load(force = false): Promise<void> {
    this.host.replaceChildren(this.note("探测中…"));
    try {
      const status = force ? await api.runtimeProbe() : await api.runtimeStatus();
      this.render(status);
      this.onStatus(status);

      if (force) {
        const ready = status.components.filter((c) => c.runnable).length;
        this.onNotice(
          status.manifestPresent
            ? `运行时探测完成：${ready}/${status.components.length} 可用`
            : "运行时探测完成：盘内无清单",
          "info",
        );
      }
    } catch (err) {
      this.host.replaceChildren(this.note((err as Error).message, "error"));
    }
  }

  private render(status: RuntimeStatus): void {
    const frag = document.createDocumentFragment();

    const head = document.createElement("div");
    head.className = "rail-head";
    const title = document.createElement("span");
    title.textContent = "盘内运行时";
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn btn-tiny";
    btn.textContent = "重新探测";
    btn.addEventListener("click", () => {
      btn.disabled = true;
      void this.load(true).finally(() => {
        btn.disabled = false;
      });
    });
    head.append(title, btn);
    frag.append(head);

    const dir = document.createElement("div");
    dir.className = "rail-path";
    dir.textContent = status.runtimeDir;
    dir.title = status.runtimeDir;
    frag.append(dir);

    if (status.manifestError) {
      frag.append(this.note(status.manifestError, "error"));
    }

    if (!status.manifestPresent) {
      frag.append(
        this.note("未找到 runtime/manifest.json —— 当前仅作文件库使用，无盘内运行时。"),
      );
      this.host.replaceChildren(frag);
      return;
    }

    if (status.components.length === 0) {
      frag.append(this.note("清单为空，未声明任何组件。"));
      this.host.replaceChildren(frag);
      return;
    }

    for (const c of status.components) {
      frag.append(this.componentRow(c));
    }

    this.host.replaceChildren(frag);
  }

  private componentRow(c: RuntimeComponent): HTMLElement {
    const row = document.createElement("div");
    row.className = "comp";
    row.title = c.detail;

    const dot = document.createElement("span");
    dot.className = `dot ${c.runnable ? "ok" : c.present ? "warn" : "bad"}`;

    const main = document.createElement("div");
    main.className = "comp-main";

    const name = document.createElement("div");
    name.className = "comp-name";
    name.append(document.createTextNode(c.id));
    const ver = document.createElement("span");
    ver.className = "comp-ver";
    ver.textContent = c.detected ?? c.declared;
    name.append(ver);

    const sub = document.createElement("div");
    sub.className = "comp-sub";
    sub.textContent = c.detail;

    main.append(name, sub);
    row.append(dot, main);
    return row;
  }

  private note(text: string, tone: Tone = "info"): HTMLElement {
    const el = document.createElement("div");
    el.className = `rail-note${tone === "error" ? " is-error" : ""}`;
    el.textContent = text;
    return el;
  }
}
