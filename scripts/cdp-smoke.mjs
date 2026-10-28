/**
 * 内置渲染的可见性冒烟（截图版）。
 *
 * 注意：iframe 用了空 `sandbox`，它形成的是**不透明源**，父页面拿不到
 * `contentDocument` —— 这是隔离生效的直接证据，也意味着自动化探针
 * 只能靠「截图 + 肉眼」或 CDP 的 frame tree，不能直接查 DOM。
 */
import { writeFileSync } from "node:fs";

const OUT = process.argv[2] ?? ".";
const targets = await (await fetch("http://127.0.0.1:9333/json/list")).json();
const page = targets.find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
ws.addEventListener("message", (ev) => {
  const m = JSON.parse(ev.data);
  const p = pending.get(m.id);
  if (p) {
    pending.delete(m.id);
    p(m.result);
  }
});
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
const send = (method, params = {}) =>
  new Promise((res) => {
    const i = ++id;
    pending.set(i, res);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
const ev = async (e) => (await send("Runtime.evaluate", { expression: e, awaitPromise: true, returnByValue: true })).result?.value;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const shot = async (name) => {
  const { data } = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(`${OUT}/${name}.png`, Buffer.from(data, "base64"));
};
const click = (sel, text) => `
  (() => {
    const el = [...document.querySelectorAll(${JSON.stringify(sel)})]
      .find((e) => (e.textContent || "").includes(${JSON.stringify(text)}));
    if (!el) return "missing";
    el.click();
    return "ok";
  })()`;

await send("Page.enable");
await send("Page.setDeviceMetricsOverride", { width: 1280, height: 820, deviceScaleFactor: 1, mobile: false });

// 当前可能停在上一次的位置：先回根再进 _smoke
await ev(`[...document.querySelectorAll(".crumb")].find(b=>b.textContent.includes("全部内容"))?.click()`);
await sleep(600);
console.log("进入 _smoke:", await ev(click("button.branch", "_smoke")));
await sleep(900);

console.log("demo.md:", await ev(click("button.file-row", "demo.md")));
await sleep(1200);
// 「源码 / 预览」是跨文件保持的，上一轮若停在源码，这里先切回来
await ev(`[...document.querySelectorAll(".seg-btn")].find((b) => b.textContent === "预览")?.click()`);
await sleep(700);
await shot("01-md-render");

console.log(
  "外壳状态:",
  JSON.stringify(
    await ev(`({
      iframe: !!document.querySelector("iframe.preview-frame"),
      sandbox: document.querySelector("iframe.preview-frame")?.getAttribute("sandbox"),
      sandboxBlocksParent: (() => { try { return document.querySelector("iframe.preview-frame")?.contentDocument === null } catch { return "throw" } })(),
      seg: [...document.querySelectorAll(".seg-btn")].map(b => b.textContent + (b.classList.contains("is-active") ? "*" : "")),
      tips: [...document.querySelectorAll(".preview-tip")].map(t => t.textContent.slice(0, 40))
    })`),
  ),
);

// 抓子 frame 的 frame tree（sandbox iframe 在 CDP 里有自己的 frameId）
const tree = await send("Page.getFrameTree");
const flat = [];
const walk = (n) => {
  flat.push({ id: n.frame.id, url: n.frame.url.slice(0, 60), origin: n.frame.securityOrigin });
  (n.childFrames ?? []).forEach(walk);
};
walk(tree.frameTree);
console.log("frames:", JSON.stringify(flat));

// 单击文件后中间栏已被预览占满，要回目录页才能点下一个文件
await ev(`[...document.querySelectorAll("button.crumb")].find(b=>b.textContent.includes("_smoke"))?.click()`);
await sleep(800);

console.log("demo.html:", await ev(click("button.file-row", "demo.html")));
await sleep(1300);
await ev(`[...document.querySelectorAll(".seg-btn")].find((b) => b.textContent === "预览")?.click()`);
await sleep(700);
await shot("02-html-render");

console.log(
  "html 外壳:",
  JSON.stringify(
    await ev(`({
      seg: [...document.querySelectorAll(".seg-btn")].map(b => b.textContent + (b.classList.contains("is-active") ? "*" : "")),
      tips: [...document.querySelectorAll(".preview-tip")].map(t => t.textContent.slice(0, 60))
    })`),
  ),
);

await ev(`[...document.querySelectorAll(".seg-btn")].find((b) => b.textContent === "源码")?.click()`);
await sleep(500);
await shot("03-source-mode");
console.log(
  "源码态:",
  JSON.stringify(
    await ev(`({
      iframe: !!document.querySelector("iframe.preview-frame"),
      pre: document.querySelectorAll("pre.code-block").length,
      seg: [...document.querySelectorAll(".seg-btn")].map(b => b.textContent + (b.classList.contains("is-active") ? "*" : ""))
    })`),
  ),
);

ws.close();
