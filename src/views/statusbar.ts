import type { RootInfo, StatsDto, WatchInfo } from "../api/types";
import { formatBytes } from "../core/format";

/**
 * 状态栏：只读展示，不做任何业务判断。
 *
 * 三段信息来源不同、更新时机也不同（盘根在启动时确定、统计随扫描变化、
 * 监听状态可能中途才就绪），所以这里保留三份数据、统一重绘，
 * 而不是让调用方自己拼字符串 —— 拼字符串的写法迟早会出现「两个字段各说各话」。
 */
export class StatusBar {
  private host: HTMLElement;
  private info: RootInfo | null = null;
  private stats: StatsDto | null = null;
  private watch: WatchInfo | null = null;
  private msgEl: HTMLElement | null = null;

  constructor(host: HTMLElement) {
    this.host = host;
  }

  render(info: RootInfo, stats: StatsDto): void {
    this.info = info;
    this.stats = stats;
    this.paint();
  }

  setWatch(watch: WatchInfo): void {
    this.watch = watch;
    this.paint();
  }

  private paint(): void {
    this.host.replaceChildren();
    const info = this.info;
    const stats = this.stats;
    if (!info || !stats) return;

    this.host.append(this.seg(`内容区 ${info.contentRoot}`, info.contentRoot));
    this.host.append(this.seg(`卷 ${info.volumeSerial}`, `卷序列号 ${info.volumeSerial}`));
    this.host.append(
      this.seg(`${stats.files} 文件 · ${stats.dirs} 目录 · ${formatBytes(stats.bytes)}`),
    );

    if (this.watch) {
      const label = this.watch.active ? "实时同步 开" : "实时同步 关";
      const title = this.watch.active
        ? "文件改动会自动进索引"
        : (this.watch.error ?? "监听不可用，索引只在扫描时更新");
      const el = this.seg(label, title);
      el.classList.add(this.watch.active ? "status-watch-on" : "status-watch-off");
      this.host.append(el);
    }

    this.msgEl = document.createElement("span");
    this.msgEl.className = "status-msg";
    this.host.append(this.msgEl);
  }

  notice(text: string, tone: "info" | "error" = "info"): void {
    if (!this.msgEl) return;
    this.msgEl.textContent = text;
    this.msgEl.className = `status-msg is-${tone}`;
  }

  private seg(text: string, title?: string): HTMLElement {
    const el = document.createElement("span");
    el.className = "status-seg";
    el.textContent = text;
    el.title = title ?? text;
    return el;
  }
}
