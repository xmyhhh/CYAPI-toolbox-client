# CYAPI Toolbox Client

CYAPI Toolbox 的桌面客户端，面向 Codex / Claude Code / OpenCode 的本地 AI 工具箱。Tauri 2 + React + TypeScript + Vite。

**本仓库公开源码仅供审查与信任评估**（许可见 [LICENSE](LICENSE)，禁止修改与商用）；构建、运行与发版均不依赖服务端源码。

功能入口：**软件接入**（教程、版本核对与首次使用引导）、**Skill 管理**（目录同步、离线安装）、**MCP 管理**（本机配置写入，保存前备份）、**模型接入**（API / Key / 线路）、**运行环境**（Node / Python 检测与共享隔离环境）、**日志**（本机日志与用量统计）。

支持 Windows 10 / 11 x64 与 macOS 11+（Apple Silicon 与 Intel）。目录骨架、接入组件、依赖与数据均由服务端分发，客户端不内置具体软件。

## 下载

普通用户从官网下载：<https://tool.cyapi.cyou/#download>。GitHub Releases 保留各版本安装包与免构建产物。

## 开发

Node 22.12+ 与 Rust stable（Windows 需 MSVC + SDK，macOS 需 Xcode CLT）。

```bash
npm ci
npm run dev            # 网页预览 http://127.0.0.1:1420
npm run desktop:dev    # 桌面开发
npm test               # Node 回归测试
npm run build          # TypeScript 检查与前端构建
npm run desktop:build  # 桌面安装包，输出到 release/
```

桌面 Rust 工具链可通过 `npm run desktop:setup` 装入本机缓存目录，不改系统 PATH。

## 发版

推送与 `package.json` 版本一致的 `v*` 标签（如 `v1.1.0`）触发 `.github/workflows/release.yml`：自动构建 Windows NSIS 安装包与免安装 EXE、macOS 双架构（Apple Silicon + Intel）DMG 与免安装 ZIP,并发布到 GitHub Releases。日常检查由 `ci.yml` 完成。

## 目录结构

```text
src/         React 界面与原生接口封装（features/ 按能力拆分,app/ 装配,lib/ API 与工具）
src-tauri/   Rust 侧:环境检测、下载缓存、离线安装、配置写入与日志分析
shared/      与服务端复用的类型与产品用语
docs/        功能与运维文档（FEATURES / DIAGNOSTICS / USAGE）
scripts/     构建、打包与图标脚本（Windows 用 .ps1，macOS / Linux 用 .sh）
public/      静态资源
release/     本地构建产物（不提交）
tests/       Node 原生测试
```

## 许可

[LICENSE](LICENSE):本仓库仅提供源码查阅与本地审查,允许查看与本地编译评估;禁止修改、衍生、再分发与任何商业用途。第三方组件许可见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
