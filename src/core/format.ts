/** 纯展示层工具函数，不含任何业务与 IO。 */

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / Math.pow(1024, i);
  return `${value >= 100 || i === 0 ? Math.round(value) : value.toFixed(1)} ${units[i]}`;
}

export function formatTime(mtimeNs: number): string {
  if (!mtimeNs) return "—";
  const d = new Date(mtimeNs / 1_000_000);
  if (Number.isNaN(d.getTime())) return "—";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * 相对路径 → 面包屑（含根）。
 *
 * 与后端 `paths::ancestors` 同构。这里复制一份纯字符串逻辑，是为了让
 * 「选中一个文件」时不必为了显示它的位置而多跑一次 IPC；
 * 目录页用后端返回的那一份，两边结果一致的保证由 e2e 测试覆盖。
 */
export function pathCrumbs(rel: string): { name: string; rel: string }[] {
  const out = [{ name: "全部内容", rel: "" }];
  let acc = "";
  for (const seg of rel.split("/")) {
    if (!seg) continue;
    acc = acc ? `${acc}/${seg}` : seg;
    out.push({ name: seg, rel: acc });
  }
  return out;
}

/** 把 0~1 的比例变成可读的百分比，极端值不显示成 0% / 100% 骗人。 */
export function formatShare(part: number, whole: number): string {
  if (whole <= 0 || part <= 0) return "0%";
  const pct = (part / whole) * 100;
  if (pct > 0 && pct < 1) return "<1%";
  return `${Math.round(pct)}%`;
}
