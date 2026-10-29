# CYAPI Toolbox Client

面向 **Codex、Claude Code 和 OpenCode** 的本地 AI 桌面工具箱，将常用配置与能力管理集中到一个界面。支持 Windows 10 / 11 x64 和 macOS 11+（Apple 芯片 / Intel），提供浅色、深色及跟随系统外观。

可管理 API 与模型、安装 Skill、配置 MCP、准备 Node / Python 运行环境，并查看本机日志与用量统计。三个工作区分别管理，配置写入前备份；软件接入提供教程、版本核对与首次使用引导。

**下载：** [官方网站](https://tool.cyapi.cyou/#download) · [GitHub Releases](https://github.com/xmyhhh/CYAPI-toolbox-client/releases)。详细说明见 [功能介绍](docs/FEATURES.md)、[日志排查](docs/DIAGNOSTICS.md) 和 [用量统计](docs/USAGE.md)。

**隐私：** 工具接入的 API Key 与工具配置保存在本机；工作流 Key 由用户手动填写，只保存在客户端 `localStorage`（可选择仅本次使用），服务端只按每次请求的 `Authorization: Bearer` 校验，不从 Agent 读取凭据。客户端不额外上报 Skill / MCP 操作结果，也不为此生成设备标识或统计队列。下载量可由服务端根据下载请求统计；本机日志与模型用量统计保留在本地。

**工作流：** 与「工具接入」平级，是客户端内的原生 React 页面，支持「工作流库 → 草稿 → 队列 → 结果」和真实 ComfyUI 原生画布。画布是唯一从服务端加载的界面，通过短时 canvas token 嵌入 iframe；其余数据都用 Bearer 调 `/api/workflows/*`。当前执行为参考图拼贴模拟，非 AI 生图、不计费；无需在本机安装 Python、ComfyUI 或模型。关闭客户端不停止服务端已提交任务。详细说明见 [工作流](docs/WORKFLOWS.md)。`research/` 仅保留历史文档，不再包含运行代码或构建入口。

本项目使用 Tauri 2、React、TypeScript 和 Vite。公开源码仅供查阅、本地编译与审查评估，不授权修改、衍生、再分发或商业使用，详见 [LICENSE](LICENSE)；第三方组件许可见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
