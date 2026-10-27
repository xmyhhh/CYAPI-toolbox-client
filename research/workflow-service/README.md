# 独立 ComfyUI 常驻测试服务

> 历史归档：本目录现仅保留文档，以下代码、命令与环境路径已退役。工作流已接入现有远程服务，当前客户端见 [工作流说明](../../docs/WORKFLOWS.md)。

这是工作流下一大版本的**本机单用户模拟服务**，与桌面程序分离。服务提供工作流库、草稿、任务队列、结果和真实 ComfyUI 原生画布；同一常驻引擎负责执行，不为每次任务 / 预览重新启动。

不读取 Agent 配置、没有真实 Key 入口、不调用 CYAPI、不下载本地模型。模拟 PNG 明确标记 `NOT AI GENERATED`；会将实际上传的参考图拼贴进结果，指纹包含描述与图片内容，但不理解描述、不生成新产品图。模板封面是 SVG 插画，不是生成结果。

## 当前开发机使用

在仓库根目录执行：

```powershell
npm run workflow:service:start   # 后台常驻，等待 ready 后命令返回
npm run workflow:service:status  # 进程身份 / 启动阶段，不输出控制令牌
npm run workflow:service:stop    # 显式停止服务与该实例的整个引擎进程树
```

打开 **http://127.0.0.1:8188/workspace/**，或打开研究客户端「工作流」（Workflow）。同一 App 顶层入口依次为「工具接入」「工作流」，不需要先配置 Agent。创作四个页面位于桌面侧栏，独立浏览器仍显示页签。切换工作台仅隐藏 iframe，不停止上传或已提交任务；全局外观共用，工具配置与创作数据独立。

关闭网页 / 客户端不停止服务，也不取消已经提交的任务。服务不会跟随客户端启动，不自动重启或重放任务；重启电脑后需再次启动服务。连接与日志入口收在设置中。客户端与服务只通过精确 origin / source 校验的 UI 通道交换固定页面名、外观、页状态与导航锁，不转发原生 IPC 或用户素材。

### 用户体验流程

1. 工作流库选择「产品生图」或「灵感成图」，预览模板和真实节点图；此时不执行。
2. 创建草稿，上传最多 6 张参考图（PNG / JPG / WebP、单张 8 MB），填写描述。产品模板至少需要 1 张参考图。
3. 草稿表单自动保存到服务，也可显式保存。原生画布可编辑草稿；在弹窗右上角点击「保存画布」后，描述 / 参考图同步回表单，revision 阻止旧窗口覆盖新内容。保存状态 / 错误放在标题栏，不在画布底部覆盖节点。
4. 显式点击「启动任务」才提交。重复启动同一草稿返回同一个任务；新任务换唯一 run_id，不依赖 ComfyUI 缓存防重。队列可取消指定任务，已完成任务中预览和下载图片。
5. 「新建工作流」从基础图开始进入原生画布；服务模板需创建副本才可编辑。分享文件导出 / 导入原生 API 图，不含任务描述或参考图；不开放在线多人分享。

工作台内的画布只预览 / 编辑，原生「运行」不提交，返回工作台启动。直接访问 `/` 仍可用于独立技术测试，不计入工作台任务。节点 `scenario` 可测试 `success` / `http-error` / `invalid-image` / `timeout`；失败不自动重试。模拟接口人为延迟 1 秒，取消仅停止本机执行，不代表未来上游已取消 / 退款。

普通页面只使用一个主页面滚动区域。弹窗打开时锁住背景滚动，说明弹窗只滚动内容区；画布弹窗标题固定、iframe 自动填充剩余空间，不再靠固定减去标题高度。工作台画布内收起全局多工作流页签，避免累积旧会话和横向滚动条；画布仍可拖动 / 缩放，必要的节点文本和工具列表滚动保留。保存中禁止关闭弹窗，失败后可保留画布重试。

执行入口只接受本服务图片、SaveImage、PreviewImage 三类节点（1–20 个），不开放任意模型或社区节点。当前工作台表单限制为**一个图片源节点**，可调整连线和输出节点，不是任意 ComfyUI 工作流产品化。原生界面可能列出其他内置节点，但其图提交会被拒绝。

### 文件与持久性

- 环境复用 `output/workflow-research/{venv,comfyui}`，不安装 / 下载依赖，不修改上游核心。
- 服务数据：`output/workflow-research/service/`，含 `service.json`、`service.log`、`comfyui.log`、`runtime/user` 与 `runtime/output`。
- `service.json` 中有**仅本机管理使用的随机合成控制令牌**，不是 CYAPI Key；不要分享此文件。公开状态、执行图、历史、日志不输出令牌。目录应只对运行服务的账户可读。
- 工作流、原生历史与异常日志可能含提示词和本机路径；反馈前检查并脱敏，不要把完整数据目录直接打包分享。
- `workspace/workspace.json` 保存个人模板、草稿、任务快照 / 状态；`workspace/uploads` 保存去元数据参考图，`workspace/results` 保存已收集结果。工作台列表和完成结果跨服务重启保留；原生内存队列 / 历史 / 缓存不恢复。
- 提交意图先落盘，再单次请求 ComfyUI；连接不确定标记状态待核对，不自动重试。服务重启时活跃任务标记中断，不重放。原生输出已落盘但尚未收集的窗口不保证自动找回；这不是生产级事务队列。
- 画布数据以 API 图为准，仅保存原生布局；草稿画布显式保存，不能依赖浏览器自动恢复。快速强退仍可能丢掉尚未保存的输入。
- 旧表单实验任务仍留在 `%LOCALAPPDATA%/CYAPI-toolbox-research/data/workflows/jobs/`，不迁移、不混入新原生历史。
- 日志追加，当前不自动轮转或清理图片；长期运行前需补容量策略。正常 stop 清理该实例进程树；被强杀、断电的异常不保证自动清理，不做全局 Python 杀进程。

## 服务代码独立运行 / 以后迁移

`research/workflow-service/` 自包含服务代码、节点、前端扩展、上游版本和 Windows 依赖快照，不导入桌面端或旧 `comfyui-local/desktop.py`。Node 脚本仅是本仓库快捷入口，不是服务依赖。

在已有兼容的 CPU Python 环境中可直接运行：

```text
python -s service.py serve --comfyui /absolute/path/ComfyUI --data /absolute/path/service-data --port 8188
```

`serve` 前台运行，适合以后交给 systemd / 容器管理；接收 SIGINT / SIGTERM 后清理引擎。也可用同样参数执行 `start` / `status` / `stop`。`start` 脱离启动终端后台运行；OS 文件锁防止同一数据目录重复启动，重复 start 复用已就绪实例。固定端口被占用时报错，不接管已有 ComfyUI。

依赖基线：Python `3.13.13`、ComfyUI `v0.42.0` / `355cd6b14c92c19ac3188fe528e47a3f21ef5178`、Torch `2.14.1+cpu`、torchvision `0.29.1+cpu`、官方前端 `1.55.18`。先从 `upstream.json` 指定仓库 / 提交准备 ComfyUI，再准备 CPU Torch 和依赖。`requirements-windows-py313.lock` 是已测 Windows 环境完整快照（无 wheel 哈希），**不是跨平台生产锁**；CPU Torch 需对应 CPU wheel 索引。Linux / macOS 尚未实机验收，不应直接套用 Windows wheel 或宣称已支持生产。

服务管理进程不 import Torch；只有一套长驻 ComfyUI 初始化 Torch。自有节点复制到独立 runtime 的白名单目录，不改上游核心。默认参数含 CPU、回环监听、禁 Partner Nodes / assets / PNG 元数据、offline CSP，仅放行自有 custom node。`--offline` 不是完整出站网络沙箱。

独立浏览器服务可用 `--port` 更换端口。桌面内嵌工作台目前**精确允许 127.0.0.1:8188**，没有放宽正式版 CSP，也没有给服务 iframe 添加 IPC 或导航权限。快捷脚本支持 `CYAPI_WORKFLOW_RESEARCH_ROOT`；独立 Python CLI 用显式路径迁移。

### 部署到服务器前必须另做

本轮**没有远程部署**。服务强制回环，不提供 `0.0.0.0` 开关；可以在服务器本机准备后通过 SSH 隧道做单人受限测试，但这不等于完成公网服务。

不要直接公开 8188，也不要仅靠反向代理把它当多用户 API。至少需要：

- HTTPS、应用鉴权、WebSocket 同等鉴权、访问控制、速率与请求大小限制；代理下的 Host / Origin 策略需明确设计，不能简单删掉检查。
- 用户 / 工作空间 / 队列 / 文件 / 输出隔离，凭据托管与最小权限；不能把 ComfyUI `--multi-user` 当安全隔离。
- 生产级持久任务库、上游幂等与状态未知核对、事务恢复、配额与真实取消语义；当前 JSON 文件防重仅适用于本机单进程。
- 文件 API 审计、输出路径与体积限制、日志 / 产物轮转、进程监管、健康监控和崩溃清理。
- 真实 CYAPI 协议、模型和费用授权单独确认；当前模拟实现不能直接冒充生产接口。
- ComfyUI / 前端 GPL-3.0 与节点派生 / 分发边界审查。当前不把第三方环境打包进 EXE，也不代表许可审查已完成。

## 验证

```powershell
& output/workflow-research/venv/Scripts/python.exe -s research/workflow-service/test_service.py
& output/workflow-research/venv/Scripts/python.exe -s research/workflow-service/test_workspace.py
```

测试使用独立临时端口 / 数据目录，正常结束停止自己的进程，不停止 8188 体验服务。报告在 `output/workflow-research/service-tests/<id>/`；验证真 CPU 引擎、连续出图 / 缓存、错误不重试、节点限制、Host / Origin、停止鉴权、回环监听、进程树退出与令牌不泄漏。

`test_workspace_client.mjs` 通过真实服务验收用户闭环；`--native` 连接实际研究 EXE 的临时 WebView2 CDP。`test_workbenches.mjs` 检查两个平级入口、导航同步、保存失败、切换中上传、限定 UI 通道、主题 / 最小窗口与导航记忆。`test_workspace_restart.mjs` 检查重开 / 重新下载与远程 iframe 的原生副作用隔离；`test_workspace_inputs.mjs` 检查上传锁定、部分失败保留与导入前保存。UI 测试会创建明确的验收数据，不自动删除未知内容。Playwright 通过 `PLAYWRIGHT_MODULE` 指定，浏览器模式使用本机 Edge。

`test_canvas_layout.mjs`（可加 `--native`）检查普通页 / 弹窗滚动区域、实际最小 / 常规窗口、无底部遮挡条与旧工作流页签、真实画布拖动缩放、标题栏保存 / 失败重试 / revision 冲突、保存中关闭与 Escape 保护、载图失败和限定消息源。报告包含当前服务资源哈希，核对实际加载的是源文件版本；`test_workspace_client.mjs` 可用 `WORKFLOW_TEST_OUTPUT` 单独保留本轮闭环证据。

旧 `test_client.mjs` / `test_restart.mjs` 仅代表上一轮服务连接界面。日常启动不得开启调试端口。实际结果见 [RESULTS.md](RESULTS.md)。
