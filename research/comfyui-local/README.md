# 本机 ComfyUI 预研（方案 A）

> 历史归档：本目录现仅保留文档，以下代码、命令与环境路径已退役。当前客户端与服务见 [工作流说明](../../docs/WORKFLOWS.md)。

> **2026-10-27 更新：** 当前入口已切换为[独立常驻测试服务](../workflow-service/README.md)。本目录保留原始引擎与逐任务 / 检查副本实验，以下旧流程不再是客户端主架构。

对应[下一大版本方案](../../docs/plans/NEXT-MAJOR-WORKFLOWS-COMFYUI.md)的第 1 步与提前进行的第 3 步模拟客户端。验证真实引擎的 CPU 编排与本机模拟 HTTP 图片接口，**不是实际 CYAPI 生图，不调用付费模型，也不是生产安装器**。

**当前结果（2026-10-26）：Windows 模拟链路连续两次通过，每次 13 项检查。** 资源占用、兼容性发现和未验证事项见 [RESULTS.md](RESULTS.md)。

**需要在软件里体验：见 [CLIENT.md](CLIENT.md)**，包含独立预研 EXE 路径、测试顺序与反馈方式。

## 文件

- `upstream.json`：实验的 ComfyUI 固定提交与 CPU Torch 版本。
- `nodes/__init__.py`：独立 V3 实验节点，只接受 `127.0.0.1` 模拟接口，不修改 ComfyUI 核心。
- `smoke.py`：启动模拟 API 和 ComfyUI，提交工作流，验证结果 / 缓存 / 失败行为，结束时停止自建服务。
- `desktop.py`：旧逐任务 stdio 适配器，按任务启动真实引擎；`test_desktop.py` 验证历史五种场景。当前客户端不再嵌入 / 调用。
- `test_client.mjs`：通过临时 WebView2 CDP 对真实研究桌面版操作验收，需要外部 Playwright。
- `requirements-windows-py313.lock`：完成环境验证后记录的依赖快照（仅对应本次平台，不代表跨平台锁定）。
- `RESULTS.md`：本轮实验结论、数据及后续决策依据。

第三方源码、虚拟环境和脚本测试产物放在已忽略的 `output/workflow-research/` 下；旧桌面任务数据在独立预研 data 中。不要把 Key 放入这些文件；实验只使用临时生成的假凭据。当前研究构建只连接独立服务，不嵌入适配器 / 节点，不分发第三方引擎，不接入正式发布流程。

## 重跑（仓库根目录，PowerShell）

前置：Python 3.13、Git、uv（以下示例用 PATH 中的 `uv`）。若没有 uv，可使用已有独立的 uv 绝对路径；不必修改系统 PATH。

```powershell
git clone --depth 1 --branch v0.42.0 https://github.com/Comfy-Org/ComfyUI.git output/workflow-research/comfyui
uv venv --python 3.13 output/workflow-research/venv
uv pip install --python output/workflow-research/venv/Scripts/python.exe --index-url https://download.pytorch.org/whl/cpu 'torch==2.14.1+cpu' 'torchvision==0.29.1+cpu'
uv pip install --python output/workflow-research/venv/Scripts/python.exe -r research/comfyui-local/requirements-windows-py313.lock --extra-index-url https://download.pytorch.org/whl/cpu --index-strategy unsafe-best-match
& output/workflow-research/venv/Scripts/python.exe research/comfyui-local/smoke.py --comfyui output/workflow-research/comfyui --output output/workflow-research/runs
```

依赖源策略仅用于本次固定版本实验；此快照不是带哈希的生产分发清单，后续安装器必须另做来源与完整性校验。目录已存在时复用，不要覆盖用户现有环境；脚本会检查引擎提交和 CPU Torch。

每次执行写入独立 `runs/<id>/`，包含 `report.json`、引擎日志、执行事件 / 历史和 `retrieved-mock.png`。报告列出启动时间、引擎进程树 RSS 采样峰值、环境体积、检查项和失败原因；正常完成或测试失败时都尝试停止自建进程树，强杀测试脚本本身不在此保证范围内。假图片上明确标注 `LOCAL MOCK API / NO AI MODEL`。

## 验证边界

- 不安装本地模型，不启用 Manager / 官方付费节点，不接触现有 Agent 或已有 ComfyUI。
- `run_id` 只是显式触发再次运行的实验输入，**不是**持久幂等键；缓存也不保证跨重启防重复计费。
- 原始 `smoke.py` 不覆盖客户端；独立桌面模拟任务的停止与历史另由 adapter / bridge / UI 测试验证，不代表真实 API 取消或崩溃恢复。人工确认、真实 CYAPI、自动安装、macOS 仍未覆盖。
- 引擎只监听回环地址，但本实验没有实现生产级鉴权或任意节点沙箱；不要把它暴露到局域网或公网。
- 假凭据检查覆盖图、事件、历史、图片元数据和日志；不是对所有第三方依赖的网络抓包或隐私审计。
- 引擎原生编辑器仅检查 HTML 可加载，画布交互与嵌入仍需后续实测。
- 当前不分发第三方引擎 / 前端，也不修改客户端许可；进入节点包发布或打包阶段前另行核对 GPL 边界。
