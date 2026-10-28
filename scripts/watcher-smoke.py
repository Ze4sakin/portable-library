"""监听实时性冒烟测试（需要先 `npm run tauri build -- --no-bundle`）。

为什么要这样一个脚本：单测与 e2e 都在进程内跑，覆盖不了三件事 ——
真实文件系统事件、Tauri 的事件推送、以及「前端真的加载了」。
这个脚本起真进程、改真磁盘、查真索引，是这一段的唯一验证手段。

用法：
    npm run tauri build -- --no-bundle
    python scripts/watcher-smoke.py
"""

import os
import shutil
import sqlite3
import subprocess
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ROOT = REPO / ".smoke"
EXE = REPO / "src-tauri" / "target" / "release" / "portable-library.exe"
DB = ROOT / ".library" / "catalog.db"


def q(sql, args=()):
    con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True, timeout=3)
    try:
        return con.execute(sql, args).fetchall()
    finally:
        con.close()


def count():
    try:
        return q("SELECT COUNT(*) FROM node")[0][0]
    except Exception:
        return -1


def wait_for(pred, label, timeout=25.0):
    t0 = time.time()
    last = None
    while time.time() - t0 < timeout:
        try:
            last = pred()
            if last:
                return time.time() - t0
        except Exception as exc:  # 库还没建好 / 正在提交
            last = f"ERR {exc}"
        time.sleep(0.25)
    raise AssertionError(f"{label} 超时（最后状态 {last!r}）")


def main():
    if not EXE.exists():
        raise SystemExit(f"找不到 {EXE}，请先 npm run tauri build -- --no-bundle")

    if ROOT.exists():
        shutil.rmtree(ROOT)
    (ROOT / "content" / "项目A" / "子目录").mkdir(parents=True)
    (ROOT / "content" / "项目B").mkdir(parents=True)
    (ROOT / "content" / "项目A" / "说明.md").write_text("# 甲\n", encoding="utf-8")
    (ROOT / "content" / "项目A" / "子目录" / "明细.txt").write_text("乙\n", encoding="utf-8")
    (ROOT / "content" / "项目B" / "main.py").write_text("print(1)\n", encoding="utf-8")

    env = dict(os.environ, PL_ROOT=str(ROOT))
    proc = subprocess.Popen([str(EXE)], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"已启动 PID={proc.pid}")

    try:
        # 3 个目录 + 3 个文件 = 6 条
        wait_for(lambda: count() >= 6, "首扫建立索引")
        print(f"[OK] 首扫完成：{count()} 个节点")

        # ① 新建
        (ROOT / "content" / "项目A" / "新建.md").write_text("新增\n", encoding="utf-8")
        dt = wait_for(
            lambda: q("SELECT COUNT(*) FROM node WHERE name='新建.md'")[0][0] == 1,
            "新建文件被索引",
        )
        print(f"[OK] 新建 → {dt:.2f}s 后进索引")

        # ② 改写（长度变化）
        target = ROOT / "content" / "项目A" / "说明.md"
        target.write_text("# 甲\n内容变长了\n", encoding="utf-8")
        # 从磁盘读实际字节数：Windows 文本模式写会把 \n 翻成 \r\n，
        # 自己数 UTF-8 长度会少算，测试就会「报一个不存在的 bug」
        want = target.stat().st_size
        dt = wait_for(
            lambda: q("SELECT size FROM node WHERE name='说明.md'")[0][0] == want,
            "改写被识别",
        )
        print(f"[OK] 改写 → {dt:.2f}s 后 size 更新为 {want}")

        # ③ 一次建出两层新目录
        (ROOT / "content" / "项目B" / "包" / "深层").mkdir(parents=True)
        (ROOT / "content" / "项目B" / "包" / "深层" / "x.py").write_text("x=1\n", encoding="utf-8")
        dt = wait_for(
            lambda: q("SELECT COUNT(*) FROM node WHERE name='x.py'")[0][0] == 1,
            "新建子树被索引",
        )
        print(f"[OK] 新建两层子树 → {dt:.2f}s 后最深层文件进索引")

        # ④ 删除文件
        (ROOT / "content" / "项目B" / "main.py").unlink()
        dt = wait_for(
            lambda: q("SELECT COUNT(*) FROM node WHERE name='main.py'")[0][0] == 0,
            "删除被识别",
        )
        print(f"[OK] 删除 → {dt:.2f}s 后节点消失")

        # ⑤ 删除整棵子树（靠外键级联）
        shutil.rmtree(ROOT / "content" / "项目B" / "包")
        dt = wait_for(
            lambda: q("SELECT COUNT(*) FROM node WHERE rel_path LIKE '项目B/包%'")[0][0] == 0,
            "删除子树被识别",
        )
        print(f"[OK] 删除子树 → {dt:.2f}s 后 3 个节点全部消失")

        # ⑥ 目录重命名：旧路径必须消失，新路径必须连子项一起出现
        (ROOT / "content" / "项目A" / "子目录").rename(ROOT / "content" / "项目A" / "改名后")
        dt = wait_for(
            lambda: q("SELECT COUNT(*) FROM node WHERE rel_path LIKE '项目A/改名后%'")[0][0] == 2
            and q("SELECT COUNT(*) FROM node WHERE rel_path LIKE '项目A/子目录%'")[0][0] == 0,
            "目录重命名被识别",
        )
        print(f"[OK] 目录重命名 → {dt:.2f}s 后旧路径清空、新路径就位")

        print("\n最终索引：")
        for rel, kind, size in q("SELECT rel_path, kind, size FROM node ORDER BY rel_path"):
            print(f"  {kind:4} {size:>5}  {rel}")
        print("\n全部通过。")
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=6)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    main()
