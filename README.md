# CYAPI Toolbox Client

面向 **Codex、Claude Code 和 OpenCode** 的本地 AI 桌面工具箱，将常用配置与能力管理集中到一个界面。支持 Windows 10 / 11 x64 和 macOS 11+（Apple 芯片 / Intel），提供浅色、深色及跟随系统外观。

可管理 API 与模型、安装 Skill、配置 MCP、准备 Node / Python 运行环境，并查看本机日志与用量统计。三个工作区分别管理，配置写入前备份；软件接入提供教程、版本核对与首次使用引导。

**下载：** [官方网站](https://tool.cyapi.cyou/#download) · [GitHub Releases](https://github.com/xmyhhh/CYAPI-toolbox-client/releases)。详细说明见 [功能介绍](docs/FEATURES.md)、[日志排查](docs/DIAGNOSTICS.md) 和 [用量统计](docs/USAGE.md)。

**隐私：** API Key 与工具配置保存在本机。客户端可发送 Skill / MCP 操作结果统计，包含随机设备标识，不发送 API Key、本机路径、命令或模型内容；可在「设置 → 使用统计」关闭并清空待发送队列。

本项目使用 Tauri 2、React、TypeScript 和 Vite。公开源码仅供查阅、本地编译与审查评估，不授权修改、衍生、再分发或商业使用，详见 [LICENSE](LICENSE)；第三方组件许可见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。
