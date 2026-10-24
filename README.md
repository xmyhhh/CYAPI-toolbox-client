# CYAPI Toolbox Client

CYAPI Toolbox 的桌面客户端，面向 Codex / Claude Code / OpenCode 的本地 AI 工具箱。Tauri 2 + React + TypeScript + Vite。

**本仓库公开源码仅供审查与信任评估**（许可见 [LICENSE](LICENSE)，禁止修改与商用）；构建、运行与发版均不依赖服务端源码。

功能入口：**软件接入**（教程、版本核对与首次使用引导）、**Skill 管理**（目录同步、离线安装）、**MCP 管理**（本机配置写入，保存前备份）、**模型接入**（API / Key / 线路）、**运行环境**（Node / Python 检测与共享隔离环境）、**日志**（本机日志与用量统计）。

支持 Windows 10 / 11 x64 与 macOS 11+（Apple Silicon 与 Intel）。目录骨架、接入组件、依赖与数据均由服务端分发，客户端不内置具体软件。

## 下载

普通用户从官网下载：<https://tool.cyapi.cyou/#download>。GitHub Releases 保留各版本安装包与免构建产物。

如果旧版点击「立即更新」后一直显示「正在下载更新安装包」，可退出客户端，用系统浏览器打开上述官网，下载对应系统的安装包后手动安装。无需先卸载或清除工具箱数据。

更新下载会显示百分比与已下载大小，日志中记录连接、下载和校验阶段。主线路下载的超时上限为 20 秒，失败后自动尝试直连源站（超时上限 120 秒）；等待期间仍可使用「从官网下载」，更新下载不会占用本地配置服务锁。

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

推送与 `package.json` 版本一致的 `v*` 标签（如 `v1.1.0`）触发 `.github/workflows/release.yml`：自动构建 Windows NSIS 安装包与免安装 EXE、macOS 双架构（Apple Silicon + Intel）DMG,发布到 GitHub Releases（含 SHA256SUMS.txt）,并以私钥部署官网下载区。测试构建用 `ci-*` 标签或 workflow_dispatch `dry-run`,只产 artifacts 不发版。

## 官网自动部署的 Secrets

发版流程的最后一步会检出私有官网源码、按 Release 产物重新生成下载清单与校验值并部署官网。需要在 Settings → Secrets and variables → Actions 配置：

| Secret | 内容 |
| --- | --- |
| `MONOREPO_TOKEN` | 可读取私有仓库 `xmyhhh/CYAPI-toolbox` 的 access token（官网源码所在） |
| `SITE_SSH_HOST` | 官网服务器地址 |
| `SITE_SSH_USER` | 部署 SSH 用户 |
| `SITE_DEPLOY_KEY` | 部署专用私钥（公钥加入服务器 `authorized_keys`） |

正式发布必须配置上述 Secrets；缺失时 GitHub Release 仍可生成，但官网部署会明确失败，不再静默跳过。CI 在发布与官网构建前均检查 Windows 安装版／免安装版及 macOS ARM64／Intel x64 双架构 DMG，缺失或空文件会阻止发布。

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


### 使用统计回执

桌面客户端会为公开 Skill 的下载校验、安装、更新/重装，以及公开 MCP 的配置接入/接管排入结果回执。仅包含条目 ID/版本、平台、目标软件、耗时、结果分类、缓存标志和随机设备 ID，不发送 API Key、路径、命令、模型内容或原始错误。网络发送由独立后台 SQLite 队列执行，失败退避重试，最多保留 30 天 / 50,000 条，只删除服务端确认的事件。

可在设置中的「使用统计」关闭；关闭后不再新增或发送回执，并清除待发送队列。也支持环境变量 `CYAPI_TELEMETRY_DISABLED=1` 或数据目录内 `telemetry-disabled` 文件。已在发送中的 HTTP 请求可能完成。设备标识是可关联的假名标识；重装软件但保留数据目录时沿用。MCP 配置写入不代表第三方包已安装或连接已验证，当前不采集这两类结果。需要服务端支持 `POST /api/client-events`，旧服务不可用时操作本身不受影响。
