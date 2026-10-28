# 第三方组件声明

本项目自身代码采用 [MIT](./LICENSE)。下列第三方组件**不在** MIT 范围内，
各自的许可证条款单独适用。

| 组件 | 路径 | 许可证 | 来源 |
|---|---|---|---|
| SumatraPDF | `runtime/pdf/SumatraPDF.exe` | **AGPLv3** | https://www.sumatrapdfreader.org/ |

## 分发时要求

`runtime/pdf/` 下的三个文件必须一起分发，缺一不可：

- `SumatraPDF.exe` —— 程序本体，未做任何修改
- `AGPL-3.0.txt` —— 许可证全文
- `NOTICE.md` —— 版本号、来源地址、SHA-256 校验值、对应源码 tag

AGPLv3 第 6 条要求：分发二进制时须提供对应源码的获取方式。`NOTICE.md`
中已记录版本号与源码 tag，按该信息可在上游取得完全相同的源码。

## 其他

`runtime/python/`、`runtime/node/`、`runtime/office/` 目录若存在，其中内容
同样为第三方软件，遵循各自上游许可证（Python PSF、Node.js MIT、
LibreOffice MPL-2.0）。本仓库不包含这些目录。
