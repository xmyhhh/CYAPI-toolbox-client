# 工作流

工作流是桌面客户端内的原生 React 页面，不是服务端托管的网站。客户端使用标准 `npm run desktop:dev` / `npm run desktop:exe` 构建，无预研开关、独立身份或本机引擎。

## 使用

1. 在同一 App 选择「工作流」。页面默认连接 `https://api.cyapi.cyou`，业务接口为 `/api/workflows/*`；模板库可先浏览，不需要 Key。
2. 在「激活 Key」中手动填写 CYAPI API Key。Key 只写入客户端 `localStorage`（`cyapi.workflows.keys.v1`），也可以选择仅本次使用；不会读取工具接入或 Agent 配置中的凭据。服务端只按每次请求的 `Authorization: Bearer` 校验，不签发 Cookie、不建立会话。
3. 选择模板创建草稿，准备描述与参考图。草稿、任务与结果按 Key 隔离；同一 Key 重新激活即可看到原历史。
4. 画布是唯一从服务端加载的界面。点击「画布」时客户端用 Bearer 换取 10 分钟有效的 canvas token，再以 iframe 打开 ComfyUI；API Key 不进入 iframe URL。画布修改需点击「保存画布」。
5. 显式确认报价并启动任务，在队列查看进度，在结果中预览或下载产物。当前是参考图拼贴模拟，非 AI 生成、不计费，不下载模型。

工作流的 Key 与工具接入完全独立。执行模式和费用以页面实际显示为准；测试报价不是账户余额或真实账单。遇到 `unknown` 或待核对状态时，请先检查队列和结果，不要通过重复启动来恢复。

## 页面结构

- **工作流库**：分类、搜索、模板预览、示例、新建、导入、个人模板导出/删除。
- **草稿**：列表与编辑器；编辑器内可上传参考图、打开画布、查看报价。
- **队列**：进行中与历史任务，可取消或查看详情。
- **结果**：媒体预览与下载；产物超过 30 天后服务端会不定时清理。

编辑器、任务详情、Key、指南、设置、报价、画布都是页面内对话框，不打开新窗口，也不启动第二个 WebView。

## 认证与安全边界

- 主 UI 全程本地；只有 ComfyUI 画布在 sandboxed iframe 中从服务加载。
- 服务端工作流接口只接受 `Authorization: Bearer` 或画布接口的 `Authorization: Canvas <hmac>`。旧 `/api/workflows/session`、`/api/workflows/keys*` 已删除并返回 404。
- 画布 token 只能读写被签发的 graph 和只读画布代理，不能提交任务、读取状态或续期；每次打开画布重新签发。
- 图片、视频、音频与下载都用带 Bearer 的 `fetch` 取回 Blob，再 `URL.createObjectURL` 展示或在内存中下载；不使用媒体 URL token，也不把 Key 放进 URL。
- CORS 仅允许 `tauri://localhost`、`http://tauri.localhost`、开发源 `http://127.0.0.1:1420` 与服务自身 origin，且不启用凭据。
- 远程内容没有任何本机 IPC 权限；原生命令入口校验 caller label 与本地 UI origin。
- Key 在激活时会调用 `GET /api/workflows/state` 校验；无效 Key 会显示错误且不会保存。

## 构建与验收

`npm test`、`npm run build`、`cargo test --manifest-path src-tauri/Cargo.toml`。Windows 便携版使用 `npm run desktop:exe`，只编译及收集本地产物，不自动上传或发布更新。

编译后可运行 `npm run test:workflows:desktop`，在隔离临时用户目录启动真实 Windows EXE，验证：工作流是主 WebView 内的原生页面、没有第二个页面或子 WebView、模板库可浏览、无效 Key 被拒绝、`workflows` / `reload_workflows` 命令已删除、外观切换不破坏页面。测试仅为该进程开启 WebView2 调试端口，结束后关闭，不把调试开关写入正式配置。

服务端的 `server/tests/workflows-browser.mjs` 是另一层显式本机集成测试：用隔离 fixture 校验器创建草稿、签发 canvas token、在真实浏览器加载固定版 ComfyUI 画布、保存 graph、通过 API 执行并带 Bearer 下载产物。它不代表生产真实 Key 登录通过，也不可作为生产启动器。

有效生产 Key 的完整上传、报价、执行和结果下载仍需使用者验收。
