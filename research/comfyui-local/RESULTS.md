# 方案 A：本机引擎验证结果

> **当前架构已变更（2026-10-27）：** 以下为原始与逐任务 / 检查副本阶段记录。最新体验使用[独立常驻测试服务](../workflow-service/README.md)，验证与启动时延见[新服务结果](../workflow-service/RESULTS.md)。旧双引擎和关闭客户端释放引擎不再代表当前行为。

日期：2026-10-26。结论：**本机真实 ComfyUI 可以在 CPU 模式下编排 HTTP 图片节点，方案 A 值得继续验证。** 这不是实际 CYAPI 成图记录，也不代表客户端集成、跨平台和正式分发已经完成。

## 1. 环境与范围

| 项目 | 本次实际环境 |
| --- | --- |
| 系统 | Windows 11 x64，build 26300 |
| Python | 3.13.13，仓库忽略目录内的独立虚拟环境 |
| ComfyUI | `v0.42.0`，`355cd6b14c92c19ac3188fe528e47a3f21ef5178` |
| PyTorch / torchvision | `2.14.1+cpu` / `0.29.1+cpu`，无 CUDA 构建 |
| 官方前端包 | `1.55.18`，只检查 HTML 可加载，没有操作画布 |
| 节点 | 自有 V3 异步 HTTP 节点 `CYAPIResearchMockImage` |
| 模型 | 无本地模型、无真实模型请求；本机 aiohttp 服务返回固定测试 PNG |

未修改上游核心源码，`git -C output/workflow-research/comfyui status --short` 为空。实验使用独立 base / user 目录、回环地址与临时假凭据，关闭 Partner Nodes、assets、PNG 工作流元数据和自动打开浏览器，只放行自有自定义节点。

## 2. 已跑通的链路

```text
独立实验客户端
  → POST /prompt + WebSocket /ws
  → 真实 ComfyUI 执行 CYAPIResearchMockImage
  → HTTP 请求本机模拟图片 API（假凭据来自进程环境）
  → base64 PNG → IMAGE tensor
  → 原生 SaveImage
  → GET /history/{id} → GET /view
  → 取回 192 × 128 PNG
```

图片明确写有 `LOCAL MOCK API / NO AI MODEL`。不依赖浏览器节点回调，不使用官方 Comfy 付费能力，也没有使用已有 Agent 凭据。

## 3. 验收结果

完整脚本连续两次成功，每次报告均为 `passed: true`，各 13 项检查通过：

| 检查 | 观察结果 |
| --- | --- |
| CPU 启动 | `/system_stats` 返回 CPU 设备，日志为 `Device: cpu`、`VRAM: DISABLED` |
| 自定义节点 | `/object_info` 可查到自有节点，真实引擎执行成功 |
| 原生编辑器入口 | 根页面返回 HTML；不等同于画布交互通过 |
| 图片管线 | 通过 `/view` 取回有效 PNG，尺寸正确，未保存 prompt / workflow 元数据 |
| 相同图再次提交 | 收到 `execution_cached`，模拟 API 请求数仍为 1 |
| 明确重新生成 | 改变 `run_id` 后恰好新增一次模拟请求 |
| HTTP 503 | 收到 `execution_error`，包含 HTTP 状态，无自动重试 |
| 无效图片 | 合法 base64 包裹无效图片数据，收到明确的图片错误，无自动重试 |
| 超时 | 模拟服务延迟 5 秒，节点 3 秒超时后报错，无自动重试 |
| 无效图 | 不存在的节点类型导致 HTTP 400，没有访问模拟 API |
| 错误后恢复 | 后续正常图仍可成功；每轮模拟 API 合计 6 次请求 |
| 网络监听 | 对引擎启动器及子进程检查，监听地址仅 `127.0.0.1` |
| 假凭据边界 | 图、事件、历史与 PNG 元数据均无假凭据；报告另外确认引擎日志也无假凭据 |

两轮均确认引擎进程树退出，随后进程列表核对无本实验引擎残留。模拟 API 在同一脚本中关闭。退出验证针对正常完成 / Python 异常清理，不代表脚本被强杀时也能保证清理。

## 4. 时间与资源成本

| 指标 | 本机观察值 |
| --- | --- |
| 首次启动至健康检查通过 | 60.051 秒；该轮之后在客户端取图参数处失败，不是完整通过记录 |
| 随后两轮全新进程启动 | 6.709 秒 / 7.702 秒；可能已有文件与字节码缓存，不当作冷机性能承诺 |
| 完整通过轮的引擎进程树 RSS 采样峰值 | 499.27 MiB / 492.02 MiB，间隔 0.2 秒；不含独立实验客户端 |
| 虚拟环境文件总大小 | 1,879,355,680 字节，约 1.75 GiB |
| 上游源码目录文件总大小 | 76,468,680 字节，约 72.93 MiB，包含浅克隆 `.git` 与运行产生的缓存 |
| CPU Torch 安装 | 69.73 秒，Torch wheel 下载显示约 118.3 MiB |
| 其余完整依赖安装 | 167.93 秒；最终 85 个包，`uv pip check` 通过 |

大小按文件逻辑长度累加，**不是最终安装包 / 下载体积**；不含共享 uv 缓存、基础 Python 安装等。该环境保留上游完整依赖，没有做最小化。上游模板与媒体包约 512.59 MiB，Torch 包目录约 474.28 MiB，是后续分析与精简候选，尚未验证能否安全移除。

## 5. 本轮发现与处理

1. **产物描述不是 `/view` 查询参数。** 当前版本的 `images[]` 含嵌套 `metadata`，首轮脚本直接传整个对象导致取图失败。已在自有客户端侧只映射 `filename / subfolder / type`，未改引擎。
2. **Windows 虚拟环境有启动器与真实子进程。** 首轮只采启动器内存得到约 4 MiB，不能作为引擎内存数据。已改为进程树采样、监听核对和退出清理，完整通过轮均观察到 2 个进程。
3. **原生超时错误缺少可读文字。** 当前可从 `exception_type: TimeoutError` 识别，但异常文字可能为空。真实适配器需要区分超时与结果未知，不能直接把原生异常文字作为用户提示，更不能自动重交付费请求。
4. **CPU 编排不等于轻量安装。** 本轮完整依赖仍含 Torch、推理支持库和上游模板媒体。建议独立按需准备引擎，后续再验证裁剪，而非现在改核心或把依赖塞进默认客户端包。

## 6. 证据与复跑

原始产物留在本地忽略目录，不随客户端发布：

- 首轮失败：`output/workflow-research/runs/a3940df19ad64d4d9338be0a47a11c78/`。
- 完整通过 1：`output/workflow-research/runs/8b91170cf9fb4586ab7d8d451bde73b6/`。
- 完整通过 2：`output/workflow-research/runs/e15eac020df245019faa9638c60f8482/`。
- 各目录包含 `report.json`、`comfyui.log`、按 prompt ID 保存的事件 / 历史；成功取图后有 `retrieved-mock.png`。
- [重跑命令与版本快照](README.md)保留在仓库；依赖快照与实际 freeze 一致，但不是带哈希的生产锁定，也未重建第二个环境验证安装复现。

客户端回归：`npm test` 为 72 通过、1 跳过、0 失败；`npm run build` 通过，仅有现有的大 chunk 提示。没有增加正式客户端功能、改变 `1.2.5` 或发布配置。

## 7. 决策与未验证边界

第 1 步原建议为继续第 2 步；按用户随后提出的客户端体验需求，**现调整为先交付独立模拟桌面版，再继续真实协议核对。** 真实 API 仍需先确认模型 ID、返回形式、计费与少量测试预算，不自动读取旧凭据或发起付费调用。

本轮不覆盖：真实 API 的鉴权 / 限流 / 余额、图片 URL 下载、安全鉴权、任意节点沙箱、断网恢复、持久幂等、重启续跑、并发、取消、人工确认、原生画布操作、Tauri 集成、自动安装、macOS 与 GPL 分发审查。`run_id` 只用于改变缓存输入；ComfyUI 内存缓存不是防重复计费方案。回环监听与凭据检查也不等于对第三方依赖的抓包或全面隐私审计。

## 8. 后续：独立客户端模拟接入

以上第 1–7 节是原始引擎实验记录；桌面体验说明另见 [CLIENT.md](CLIENT.md)。桌面适配器使用 768 × 512 测试 PNG，色彩和指纹随输入变化，不理解提示词，也不是 AI 生图。

- Python desktop adapter：正常出图、HTTP 503、坏图、超时、执行期停止均通过，确认进程树退出；错误场景均只有一次模拟请求。最新报告：`output/workflow-research/desktop-tests/d0001caf94254ce693a08c65e146cc2b/report.json`。
- Rust bridge：3 个测试通过（含需要本机环境的 ignored 测试），覆盖真实引擎出图、重复提交拒绝、历史重载、启动期停止。
- 修复 Windows stdin 缓冲锁导致的子进程启动 / 退出阻塞；关闭管道监控使用 `os.read`，git 子进程显式使用空 stdin。进度输出断开也会继续清理引擎。
- 前端回归：76 通过、1 因缺少私有服务端数据跳过、0 失败；正式与研究前端构建通过，正式 feature 关闭的 Rust check 通过。保留原有 chunk 大小与两个 Rust 警告，没有顺手修改无关代码。
- 研究构建使用独立前端输出、Cargo target、应用标识与数据目录，禁止正式更新；当前只有本地 EXE，不打包第三方引擎，不是可移机安装器。

独立 EXE 已构建并通过真实桌面操作验收（WebView2 CDP，没有注入假 IPC）：

- 11 项 UI 检查通过：入口 / 环境、真实引擎出图预览、复制反馈、三种错误、停止、更新前后端拦截、切页继续、深色截图、无横向溢出 / 未处理前端异常。
- 图片节点执行中通过 `CloseMainWindow` 正常关闭研究窗口，观察到适配器和引擎进程树退出。进程退出后记录可能停留在执行阶段；重启统一转为「已中断」，不伪称上次正常完成或自动续跑。
- 重启的 4 项检查通过：入口 / 环境、上次未完成记录转为中断且没有活跃任务、成功记录保留、没有横向溢出 / 前端异常。
- 本机报告和截图：`output/workflow-research/client-tests/report.json`、`restart-report.json`、`exit-report.json`、`success.png`、`timeout.png`、`dark.png`。最初测试脚本误用异步 `waitForFunction` 导致切页断言提前返回，已改为显式等待真实 snapshot 并完整重跑通过，未将失败运行算作通过。
- 原表单版 EXE：14,771,712 字节，SHA-256 `2c2e17353b37da83e92439d25de1c0467518230f4096ff5f44fb946926b6ae2d`，现已由下述原生画布版替换。未创建安装包、未推送或发布，运行依赖构建机的独立引擎环境。

## 9. 原生画布检查（2026-10-27）

用户反馈表单无法核对实际图，因此补独立的原生 ComfyUI WebView 窗口，不再把运行状态卡片当画布。只复用本机已安装官方前端 `1.55.18`，通过自有扩展调用 `app.loadApiJson` 还原节点；没有修改第三方核心或将其源码加入 React 包。

- 草稿图：包含当前提示词，`run_id=draft-not-executed`，明确未执行，不创建任务记录。
- 任务图：读取选中任务自己的 `workflow-api.json`，保留原提示词、`run_id`、节点 ID / 类型和 IMAGE 连线。早期记录缺图时明确拒绝，不生成替代历史。
- 检查副本：移动 / 编辑不回写表单或历史，「恢复原始图」重新载入；前端禁用运行按钮和提交方法，独立画布引擎拒绝 `/prompt` 与 `/api/prompt`，不把它当双向编辑器。
- 生命周期：最多一个画布会话；与单任务引擎独立。关闭画布释放检查引擎；关闭主客户端释放画布与任务引擎。两个引擎并存会增加资源占用，尚未重新做峰值基准。
- 边界：原生 WebView 不获客户端 IPC 权限，只允许同源导航；回环监听 + 上游 `--offline` CSP，不代表生产鉴权或全链路隐私审计。

真实 EXE 测试（不是网页仿制或注入假 IPC）：

1. 原生画布显示两个实际节点和一条 IMAGE 连线；从画布 `graphToPrompt()` 重新导出的图与草稿 / 历史保存的 API JSON 一致。
2. 选中旧任务时输入和 `run_id` 与旧任务完全匹配，运行新任务也不改变旧快照。
3. 画布副本编辑不回写，恢复按钮正确还原；引擎禁止提交且队列为空，远程 WebView 调用客户端 IPC 被拒绝。
4. 客户端关闭画布、画布原生关闭按钮、启动期取消和主窗口退出分别检查，避免遗留窗口 / 引擎。
5. 主窗口退出测试在图片节点实际执行、另一个画布保持打开时发送正常 `WM_CLOSE`；确认客户端及两组 Python / ComfyUI 进程全部退出。

报告 / 截图：`output/workflow-research/canvas-tests/{report.json,native-close-report.json,exit-report.json,draft.png,historical.png,client.png}`。适配器独立检查为 `research/comfyui-local/test_inspector.py`，真实桌面为 `test_canvas.mjs`；主窗口退出为 `test_canvas_exit.py`。

回归：原图片适配器五场景通过（`desktop-tests/f21beccb4f1e49b2a4afa519d5957684/report.json`）；Rust bridge 3 项通过；原桌面 11 项回归通过；`npm test` 为 77 通过、1 缺私有数据跳过、0 失败，正式与研究前端构建及 feature 关闭的 Rust check 通过。仍未调用真实 CYAPI、读取 Agent 凭据、下载本地模型或发布。

画布检查共 9 项通过，独立适配器的就绪 / 启动期取消两项通过（`inspector-tests/7b12b8cb119a482283209e10756a5d1a/report.json`）。最新画布 EXE：`output/workflow-research/client/CYAPI-Toolbox-Workflows-Research.exe`，14,838,272 字节，SHA-256 `8e164bcefcc0f93f9e81b6a8eb5702ba24fc5ca1ef5bf0361ac04d44bc758154`。首轮最终复测因按重复提示词定位历史行选错任务，已改为按已核对的行索引定位并等待任务 ID 更新后重跑全部通过；不是修改图来迁就断言。
