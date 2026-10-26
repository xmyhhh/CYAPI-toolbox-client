# CYAPI Toolbox Client

独立桌面客户端，采用 **Tauri 2 + React + TypeScript + Vite**。负责 Skill 界面、运行环境检测、下载缓存、真实离线安装与卸载。支持 Windows 10 / 11 x64 与 macOS 11+（Apple Silicon 和 Intel）。服务端地址内置为 `https://api.cyapi.cyou`，桌面版和网页预览均自动连接，无需配置，界面不显示地址设置；客户端通过 HTTPS API 连接，不读取服务端项目目录。旧版 `settings.json` 中保存的地址不再生效。部分网络无法访问 Cloudflare 线路时，客户端自动改用源站 IP（149.104.31.54）直连 `api.cyapi.cyou` 与 `tool.cyapi.cyou`，保持原域名与证书校验，仅替换 DNS 解析结果；模型接入线路（`www.cyapi.cyou` 及服务器分发的备用线路）不参与该回退，详见「Codex 配置 / CYAPI 专属」的线路分发说明。

详细的功能、行为与边界说明见本文；入口、开发与发版流程见根目录 `README.md`。

本文命令均在仓库根目录执行。

主导航依次为「模型接入」「软件接入」「Skill 管理」「MCP 管理」「运行环境」「日志」。软件接入以软件为入口组织教程、版本核对和所需能力，Skill 管理提供任务方法与脚本，MCP 管理维护工具和数据连接。三者使用相同产品用语，并能跳转到对应能力的管理页。

侧栏顶部的「工作区」在 Codex、Claude Code 与 OpenCode 之间切换，模型接入、Skill 管理、MCP 管理、软件接入配置和日志跟随工作区，选择保存在本机。软件版本检测和共享运行环境属于本机状态。Skill 管理内可切换「Skill 广场」和「已安装」，列表和数量按当前 Agent 可读取的 Skill 过滤，安装目标跟随当前工作区。

软件接入明显展示支持范围、本机版本、项目实测版本和官方下载入口。Windows 读取安装记录与文件版本，macOS 读取应用包元数据；自定义位置等检测不到时显示「版本未确认」。符合上游声明范围与经过本项目实测分别展示，安装成功后仍需按教程在新 AI 会话中完成首次任务。

## 官方网站与版权

侧栏底部提供「官方网站」入口，点击后在系统默认浏览器中打开 `https://www.cyapi.cyou`，不会在应用内嵌窗口中加载；同一地址也可在「设置 → 关于」和「使用帮助」中打开。「设置 → 关于」同时显示当前版本、下载页地址 `https://tool.cyapi.cyou/#download` 和版权信息，官网地址可一键复制。

版权信息为 `© <当前年份> CYAPI Toolbox · CYAPI 保留所有权利`，侧栏底部显示简写 `© <当前年份> CYAPI Toolbox`。

桌面版通过 `open-url` 打开工具箱官网链接，只允许 `https` 且主机为 `tool.cyapi.cyou` 或 `www.cyapi.cyou`；软件官方下载安装页使用独立的 `integration-open-download`，仅按接入 ID 打开已校验目录中的 HTTPS 地址。Windows 使用系统 ShellExecute 打开完整 URL。更新包的下载地址另外限制在 `tool.cyapi.cyou`。网页预览使用新标签页。

Claude Code 的全局 Skill 目录遵循 `CLAUDE_CONFIG_DIR`，默认 `~/.claude/skills`。OpenCode 同时显示其兼容读取的 `~/.claude/skills`、`~/.agents/skills` 及已知项目中的对应目录；预览安装时检查所选项目及仓库内的上级目录，阻止重复安装。兼容目录中的 Skill 保留原安装归属，配置和卸载需回到原工作区。OpenCode 的 `OPENCODE_DISABLE_CLAUDE_CODE`、`OPENCODE_DISABLE_CLAUDE_CODE_SKILLS` 和 `OPENCODE_DISABLE_EXTERNAL_SKILLS` 会同步影响兼容目录的显示与重复检测。

目录数据来自服务端，本地准备 106 项、8 个分类，其中 99 项可下载 Skill 文件并离线安装。支持分类和搜索同时筛选；Anthropic 的 `docx`、`pptx`、`xlsx` 仅展示官方来源。安装弹窗显示运行时和额外依赖说明，文件安装完成不代表第三方依赖或账号已配置。

Nature Skills 在 Skill 广场以一个「科研套件」入口展示，展开可查看和单独安装模块，搜索、分类和推荐筛选仍匹配各模块。「管理套件」统一选择 Agent 与全局 / 项目范围，预览每个模块的安装状态，一次补齐缺失模块并更新工具箱管理的旧版本；共享规则优先安装，本机已有的其他版本保留。中途失败会显示已完成数量和停止原因，重试时重新检查状态。已安装页也按套件收拢，不同项目、目录和原安装工作区分别管理；套件管理弹窗可批量卸载当前目录中工具箱管理的模块，并逐项移至备份，外部安装会保留。底层保留各技能的原始目录和按需加载方式。

套件安装与卸载是一个长任务，任务状态由窗口持有而不属于弹窗：弹窗内实时显示「N/M · 模块名 · 阶段」（检查安装位置 / 下载离线包 / 写入文件 / 更新文件 / 移至备份）与已用时秒数，下载或准备环境期间不再只有一行进度。「取消任务」在当前原生调用返回后停止后续模块，已完成的模块保留；「放至后台」或关闭弹窗不会中断任务，任务继续在后台运行，窗口顶部状态栏在任意页面显示进度与取消按钮，「管理套件」可随时重新打开弹窗查看详情。取消与失败后的列表都会重新读取，已安装的模块不会被回退。

## 开发与打包

前端开发需要 Node 22.12+。桌面开发需要 Rust stable：Windows 需要 MSVC、Visual Studio C++ 工具与 Windows SDK；macOS 需要 Xcode Command Line Tools。

```bash
npm ci
npm run dev            # 网页预览：http://127.0.0.1:1420
npm run desktop:dev    # 桌面开发，自动启动前端
npm run build          # TypeScript 检查与前端构建
npm test               # 元数据、路径和原有模拟逻辑回归测试
npm run desktop:build  # 桌面安装包，输出到本目录 release/
npm run desktop:exe    # 仅生成可执行文件
```

`npm run` 脚本跨平台：`scripts/run.mjs` 在 Windows 调用 `scripts/*.ps1`，在 macOS 与 Linux 调用 `scripts/*.sh`。打包目标由平台决定，Windows 生成中文 NSIS 安装包，macOS 生成 `.app` 与 `.dmg`。

项目专属 Rust 通过 `npm run desktop:setup` 安装到本机缓存目录（Windows 为 `%LOCALAPPDATA%\CYAPI-toolbox\build-tools`，macOS 为 `~/Library/Caches/CYAPI-toolbox/build-tools`），不修改系统 PATH。官方源较慢时运行 `scripts/setup-desktop.sh --use-mirror`（Windows 为 `scripts/setup-desktop.ps1 -UseMirror`）。用户运行客户端不需要安装 Rust。

正式发版由 GitHub Actions 完成。`.github/workflows/release.yml` 在推送 `v*` 标签（须与 `package.json` 版本一致）时自动构建 Windows NSIS 安装包与免安装版、macOS Apple Silicon 与 Intel 的 DMG，并把产物发布到 GitHub Releases；`.github/workflows/ci.yml` 在每次推送与提交 PR 时运行前端测试与构建。官网自动部署暂时不做，`Publish the website` 任务有意停用，保留脚本供以后启用；目前官网单独手动同步已发布的 CI 产物，无需配置官网部署 Secrets。

服务端由独立的私有项目 `server/` 管理（未随本仓库发布），启动命令见该项目说明。根目录 `package.json` 中只包含客户端命令。

## 使用 My Image

1. 启动目录服务，打开 `release/CYAPI-Toolbox_0.9.0.exe`（Windows）或 `release/CYAPI Toolbox.app`（macOS），或安装对应安装包。
2. 同步目录，选择 My Image，选择 Codex / Claude Code / OpenCode 的全局或项目目录。
3. 点击「下载离线文件」，下载 Skill 和约 91 MB 的共享 Node 备用包并校验。备用包仅缓存，已有兼容环境时不会重复安装。
4. 下载完成后，可关闭服务端并断网，点击「离线安装」。已有同名 Skill 时显示「已安装」，可查看位置或改用其他目标、项目目录。
5. 点击「配置」，填写 Base URL、API Key 和模型，保存到 `%APPDATA%\my-image\.env`（macOS 为 `~/Library/Application Support/my-image/.env`）。重新打开 Codex 会话并输入 `$my-image`。生成图片仍需要连接上游服务。

Windows 10 / 11 x64 的 EXE 需要 Edge WebView2 Runtime；NSIS 安装包缺少它时会尝试在线安装。macOS 需要 11 或更高版本。Windows 产物未签名；macOS 产物带临时签名（ad-hoc）但未公证。

### macOS 安装与 Gatekeeper

macOS 构建带临时签名：`.app` 包内有 `_CodeSignature/CodeResources`，`codesign --verify --deep --strict` 通过。缺少包级签名时，Gatekeeper 会把应用判为「已损坏，无法打开」；只有 Mach-O 上的链接器临时签名不够。

未公证的应用第一次打开时，Gatekeeper 仍会提示无法验证开发者。在「系统设置 → 隐私与安全性」中点击「仍要打开」，或在终端中移除隔离标记：

```bash
xattr -dr com.apple.quarantine "/Applications/CYAPI Toolbox.app"
```

放行一次之后即可正常启动和自动更新。需要让每台机器都直接双击可用时，应使用 Apple 开发者证书签名并公证（`codesign` 与 `notarytool`），这需要开发者账号与 `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` 或 App Store Connect API 密钥。构建脚本 `scripts/collect-release.sh` 在签名缺失时会补一次临时签名，并在签名校验不通过时拒绝产出发布物。

## 外观与深色模式

界面默认跟随系统外观，Windows、macOS 与 Linux 都会响应系统的浅色 / 深色设置。macOS 的窗口标题栏、边框和背景色同步跟随，切换时不会出现白闪。

「设置 → 外观」可以选择「跟随系统」「浅色」「深色」，选择保存在本机并立即生效。网页预览同样支持。

样式表里的颜色不写死。`scripts/theme.mjs` 在构建时把每个颜色转成 CSS 变量，并按颜色在页面中的角色推导深色变体：背景变成深色表面，正文变成浅色，边框变成深色描边；侧栏、控制台等本来就是深色的面板保持原样，品牌色与图表色单独指定。改完颜色后运行 `npm run theme` 重新生成 `src/app/theme.css`，`npm test` 会检查调色板与样式表是否一致。

## 共享运行环境

- 实际执行 `--version` 检测 Node/Python，显示版本、路径、来源，并对同一可执行文件去重。
- 按最低版本要求依次复用工具箱共享环境、Codex 自带环境、系统环境。Codex 内部路径和预下载状态可能随版本变化，因此采用本机探测。
- My Image 仅需要 Node ≥20，无 npm 依赖，不需要 Python。安装时不会执行 Skill 脚本、npm install 或读取图片 API Key。
- 找不到兼容 Node 时，从已下载的官方 Node 24.14.0 准备一份共享环境。后续兼容 Skill 共用，卸载 Skill 后保留环境。
- 安装时生成 `.cyapi/run-node.ps1`（macOS 与 Linux 为 `.cyapi/run-node.sh`），在 SKILL.md 的 YAML 后加入对应平台的调用说明。Windows 使用子进程执行策略参数，macOS 与 Linux 使用 `sh` 启动；两者都不修改系统 PATH 或全局执行策略；原 Codex 环境路径失效时尝试重新检测。
- Python 支持发现、按离线依赖 ABI 选择已有解释器、创建隔离 venv 与离线安装依赖。相同清单复用环境，wheel 与工具按哈希共享。启动器绑定环境并为子进程添加工具 PATH，不安装新 Python 或修改全局依赖。
- 服务端按平台标识分发二进制产物（`windows-x64`、`macos-arm64`、`macos-x64`、`linux-x64`）。客户端只接受与本机平台一致的产物，其他平台的条目会被跳过而不是报错。
- 离线 Python 依赖按 Python 主次版本 + ABI + 平台准备，客户端按本机平台与解释器版本挑选；软件接入组件按平台提供多份构建，界面显示本机平台的那一份，当前系统没有可用构建时说明已发布的平台。

## macOS 功能边界

macOS 上以下功能与 Windows 一致可用：Skill 安装与卸载、MCP 写入与移除、模型接入（Codex、Claude Code、OpenCode）、用量与日志分析、Codex 排查。

与 Windows 不同的部分：

- 离线 Python 依赖的 macOS 包要求 macOS 13 或更新版本；服务端为 macOS arm64 与 macOS x64 各准备一份。
- 剪映 Skill 的上游依赖里 `uiautomation` 只有 Windows 版本，macOS 不安装该包；它在上游代码中没有引用，其余剪辑能力不受影响。
- FFmpeg / ffprobe 的 macOS 构建来自 `eugeneware/ffmpeg-static` 的官方发布，许可文本随产物一并分发。
- Zotero 接入组件按平台构建；macOS 构建用 `cargo build --target aarch64-apple-darwin` 产出，Rust 源码本身与平台无关。
- macOS 未签名的应用首次启动需要在「系统设置 → 隐私与安全性」中放行；当前产物均未签名。

## 模型接入：Codex

工作区切换到 Codex 后，「模型接入」提供全局 API 地址、API Key、模型 ID、推理强度、上下文窗口和恢复入口。无需启动 Skill 服务端。采用 Responses API，保存后需完全关闭 Codex（包括托盘和后台进程）再重新启动，切换至 API Key 认证；已有会话仍使用开始时读取的 URL，切换线路后需重启 Codex 并新建会话，才会使用新线路。当前不提供登录、代理转发、多供应商方案或联网验证服务商。

Codex 保存前重新读取最新配置，模型字段使用原值、当前值与页面输入进行核对：互不冲突的外部模型变化自动合并；同字段冲突或认证、线路变化时保留输入并在页面提示，不自动弹出错误日志窗口。不会通过更新版本号强行覆盖外部配置；读取后到写入期间的并发修改仍由后端拦截。文件损坏、写入失败及恢复失败仍正常报告。

三个模型接入页的模型字段都有「获取模型列表」。点击后由桌面端直接请求当前 API 地址的 OpenAI 兼容 `/models` 或 Anthropic `/v1/models`，使用当前填写的 API Key；返回的模型在独立添加区搜索、勾选并确认添加，再从已配置模型中选择默认模型，也可手动添加其他 ID。请求不经过目录服务，获取列表不会立即写入配置，点击「保存配置」后才保存所选模型；鉴权失败、接口不存在、超时和网络错误会直接显示在模型字段下方，不自动打开日志窗口。

三个工作区的「CYAPI 专属」在密钥下方显示「余额与额度」。填入密钥并停止输入 600 毫秒后自动查询，读取已有配置、保存配置或切换线路后也会刷新；可点击「刷新余额」重试。桌面端直接向所选线路的 `{baseUrl}/usage` 发送带 Bearer API Key 的 GET 请求，优先显示 `rate_limits` 中的全部周期额度及重置时间，没有周期额度时依次读取 `quota`、`balance` 或 `remaining`。Claude Code 查询使用线路清单中保留 `/v1` 的原始 API 地址。请求不经过目录服务，余额不写入本机存储；失败提示只显示在余额卡片内。

配置目录尊重 `CODEX_HOME`，默认 `~/.codex`。使用 TOML 编辑器保留其他设置、注释及原有供应商定义，新供应商标识为 `cyapi-toolbox`。Key 写入本机 `auth.json`，配置页通过本机 IPC 读取已有 Key 并填入密码框，重启后仍会回读；同时兼容当前供应商的 `experimental_bearer_token`、`env_key` 和 Bearer Authorization 请求头（包括环境变量请求头）。默认掩码显示，点击眼睛可显示或隐藏；不写入浏览器存储、不发给目录服务。清空后保存仍保留原 Key，包括从当前供应商配置中读取的 Key。写入 `cli_auth_credentials_store = "file"` 和 `forced_login_method = "api"`，使 Codex 使用该文件中的 API Key。其他登录字段保留在原 JSON 中，当前认证模式会切换为 `apikey`。

保存前在工具箱数据目录的 `codex-config/previous.json` 备份原始文件；配置和备份均限制为当前用户访问。备份包含原登录凭据，勿分享。恢复会还原上次保存前两个文件的原始字节；首次创建的文件会删除。同值保存保留之前的恢复点。保存校验模型、线路与认证设置的内容版本：后台新增项目记录、修改 MCP、刷新登录令牌或调整文件排版时，保存会保留这些最新内容，不再误报「配置已变化」。模型、上下文窗口、线路、Key、登录方式或账号发生变化时仍拒绝覆盖，需重新读取核对，当前表单输入会保留。提交时再次校验两个文件，恢复仍严格检查完整文件版本，不能覆盖后续外部修改。多文件写入使用暂存和失败回滚，不保证断电事务。

本入口编辑全局 `config.toml` 和 `auth.json`，配置多模型时还会生成下文所述的不可变模型目录文件。新版 Codex 的命名 Profile 使用独立的 `<name>.config.toml` 文件；以 `--profile` 启动时，Profile 中同名字段可以覆盖全局值。

参考 [CC Switch 的 Codex 配置实现](https://github.com/farion1231/cc-switch/blob/caf70b526abf6ebdadebeb8fa140a372ef78f6be/src-tauri/src/codex_config.rs)，许可见 [第三方许可](THIRD-PARTY-NOTICES.md)，同时内置于 EXE 的「使用帮助 → 开源许可」。格式依据 [OpenAI 配置参考](https://developers.openai.com/codex/config-reference)、[高级配置](https://developers.openai.com/codex/config-advanced) 和 [JSON Schema](https://developers.openai.com/codex/config-schema.json)。

## 模型接入：Claude Code

工作区切换到 Claude Code 后，「模型接入」写入全局 `settings.json` 的 `env` 块：`ANTHROPIC_BASE_URL`、`ANTHROPIC_AUTH_TOKEN`，以及可选的 `ANTHROPIC_MODEL`。配置目录尊重 `CLAUDE_CONFIG_DIR`，默认 `~/.claude/settings.json`。模型留空时使用 Claude Code 自带默认模型。

Claude Code 通过 Anthropic Messages API 请求 `{baseUrl}/v1/messages`，因此界面显示和保存的地址不含末尾 `/v1`；从 CYAPI 线路清单或手动粘贴带入的 `/v1` 会被自动去掉。线路选择与 Codex 共用服务器分发的清单，不在本机预设服务地址。

保存管理上述 API 字段、默认模型、模型选择器、思考等级及显式上下文覆盖，具体映射见下文「模型与思考等级」「模型上下文窗口」，保留 `settings.json` 的其他键；文件不是 JSON 对象、`env` 不是对象或存在外部并发修改时拒绝写入。保存前在工具箱数据目录的 `claude-config/previous.json` 备份原始字节，恢复会还原上次保存前的文件（首次创建的文件会删除）。AUTH_TOKEN 只保存在本机，配置页通过本机 IPC 读取并填入密码框，默认掩码显示；不写入浏览器存储、不发给目录服务。Claude Code 会重载多数配置，但默认模型与推理等级在会话启动时读取；请启动新会话使用新的接入设置，当前会话可用 `/model`、`/effort` 切换模型与等级。

状态区通过 `where claude` 检查 PATH 中是否存在命令，并读取 `~/.claude/.credentials.json` 判断当前是 API Token 还是 Claude 登录。「日志」按 Claude Code 的 JSONL、会话索引、子代理与 debug 格式提供会话时间线、用量及错误证据，见 [日志分析](DIAGNOSTICS.md)。

## 模型接入：OpenCode

工作区切换到 OpenCode 后，「模型接入」优先编辑 `OPENCODE_CONFIG` 指定的文件（相对路径按工具箱进程的工作目录解析）。未指定时，配置目录尊重 `XDG_CONFIG_HOME`，默认 `~/.config/opencode`；`opencode.jsonc` 与 `opencode.json` 同时存在时使用前者。页面提供 CYAPI 线路、自定义 API 地址、API Key、模型 ID、CLI 检测和恢复入口。显式配置文件不会改变全局 Skill 的安装目录。

同时识别原生 V2 的 `providers.cyapi` 与兼容配置 `provider.cyapi`：已有条目保持原格式，新条目采用 V2 `package`、`settings.baseURL/apiKey`、`models.<别名>.modelID` 与模型级 `settings.reasoningEffort`。两种 CYAPI provider 同时存在时优先编辑 V2。根 `model` 指向所选默认模型条目，兼容原有别名。API 地址通常以 `/v1` 结尾。OpenCode V2 自动重载配置，必要时使用 `/reload`；会话已选模型与 variant 可覆盖默认设置。保存保留其他 provider、MCP 和权限设置，通过 JSONC CST 编辑保留注释和未修改部分的排版。

保存前在工具箱数据目录的 `opencode-config/previous.json` 备份原始字节，恢复会还原上次保存前的文件（首次创建的文件会删除）。外部程序修改过文件、revision 过期或目标不是 JSON 对象时拒绝写入。API Key 只保存在本机，配置页通过本机 IPC 读取并填入密码框，默认掩码显示；不写入浏览器存储、不发给目录服务。状态区通过 `where opencode` 检查 PATH 中是否存在命令。「日志」兼容 OpenCode 新旧 SQLite、旧 JSON 存储及运行文本，可查看会话、工具状态、用量与错误证据，见 [日志分析](DIAGNOSTICS.md)。

## MCP 管理

「MCP 管理」与 Skill 管理使用相同的顶部标签、搜索工具栏和横向列表，可切换「MCP 广场」与「已安装」。广场按当前软件筛选适配条目，支持名称、说明和连接方式筛选；「已安装」仅展示当前软件配置中检测到的 MCP，包含工具箱管理和外部配置，并显示该软件自己的配置路径。

可安装清单由服务端 `GET /api/mcp` 提供，首次没有缓存或手动「同步广场」时获取。读取已安装列表和「重新检测」只访问本机配置，即使首次离线、尚未下载广场目录，仍可查看和管理已有条目，也可以保存、接入本机自定义条目。切换软件时保留当前标签，清除上一软件的列表和表单，忽略旧请求的返回；自定义表单默认选择当前软件。清单本身不包含任何本机凭据。

- Codex 写入 `~/.codex/config.toml` 的 `[mcp_servers.<id>]`，遵循 `CODEX_HOME`；使用 TOML 编辑器保留其他表、注释和未修改字段。
- Claude Code 写入 `~/.claude.json` 顶层 `mcpServers.<id>`，遵循 `CLAUDE_CONFIG_DIR`；保留该文件的其他配置。它不是 `~/.claude/settings.json`。此处管理全局接入；项目中的启用和停用请在 Claude Code 的 `/mcp` 中操作。
- OpenCode MCP 与模型接入使用同一配置文件，遵循 `OPENCODE_CONFIG` 和 `XDG_CONFIG_HOME`。同时识别 `mcp.<id>` 与 `mcp.servers.<id>`，编辑时保留原格式及对应的 `enabled` / `disabled` 字段；同名条目同时存在于两处时显示冲突，合并重复项后再管理。使用 JSONC CST 保留注释和排版。
- 三个工作区分别读取和写入自己的配置文件，互不串联。只修改目标条目的必要字段，同一条目中的其他 Header 等字段保留。
- Orbit SSH 的访问令牌从本机 `%APPDATA%\OrbitSSH\mcp-token` 读取，端口从 `workspace.json` 的 `preferences.mcpPort` 读取（默认 8765）；令牌只写入当前 Agent 配置，不返回页面、不发给服务端、不写入管理记录。请先启动 Orbit SSH 2.0+ 并开启 MCP。
- 安装成功后写入工具箱数据目录的 `mcp/receipts/<workspace>-<id>.json`，记录配置路径和条目哈希。移除前重新校验；外部修改过的条目不会被静默覆盖或删除。已由其他工具写入且内容一致的条目可「接管」。
- 每次写入前在 `mcp/backups/` 备份原文件。配置冲突、令牌缺失、过期 revision 和清单不可用都会在页面内提示。
- 并发校验只覆盖本次操作真正会写入的那一条目。Agent 会在后台改写自己的配置（例如 Codex 重启运行时后更新 `node_repl` 的管道路径），这类改动属于其他条目，不会再让「接入 / 接管 / 启用 / 停用 / 移除」误报「MCP 配置已变化，请重新检测」；目标条目自身被改动时仍然拒绝写入并要求重新检测。写入瞬间的并发修改仍由提交前的整文件比对拦截。

CodeGraph 需要先安装 Node.js 18+ 和 `npm install -g @colbymchenry/codegraph`，并在项目目录运行 `codegraph init`。服务端目录格式和新增条目步骤见私有服务端仓库中的 MCP 目录分发文档（服务端代码未随本仓库发布）。

## 模型与思考等级

三个工具的「模型接入」均将已配置列表与「添加模型」区域分开。先从 API 列表勾选并确认添加，或手动输入模型 ID，再从已配置模型中选择一个默认模型。添加多个模型用于在工具内切换，不表示同时调用、自动调度或故障转移。当前默认模型不能直接移除，需先切换；Claude Code 也可选择使用工具自身默认模型。所有更改统一保存，接口地址或 Key 变化只清空发现结果，不清空配置草稿。

OpenCode / Claude Code 每个模型都有独立的「默认推理等级」，展开「配置」后明确编辑。未编辑字段不写入；清空只移除该模型的显式等级，不影响其他模型、不等于关闭思考，上层配置仍可能生效。仅切换默认模型或改地址，不再批量统一原有等级。现有未知值可保留，不阻塞无关配置保存。模型列表只提供 ID，不验证等级能力；手动值与候选值不是支持承诺，未确认时保持「不覆盖」。本次不增加付费探测请求。

- Codex 保留独立的「全局推理等级」，写入 `model_reasoning_effort`，不是逐模型默认设置。多模型通过 `model_catalog_json` 指向 `CODEX_HOME/cyapi-models/<内容哈希>.json`。生成时复用已有目录或 `models_cache.json` 元数据，没有可用缓存时尝试隔离环境中的 Codex CLI `debug models --bundled`（离线、最多 5 秒）。失败则保留原配置与凭据。已知模型能力及默认等级保留；不会因全局选择等级而向支持列表追加值，未知模型不再补全所有等级或虚构默认等级。
- Claude Code 写入 `modelPicker.options`（2.1.242+）、同步默认 `model` 与 `ANTHROPIC_MODEL`；只在明确编辑某个模型时修改其 `modelSettings.<规范名称>.effortLevel`（2.1.251+）。可持久化的值是 `low/medium/high/xhigh`，不代表网关全部支持。已有 `CLAUDE_CODE_EFFORT_LEVEL` 和全局 `effortLevel` 默认保留并明确提示，只有点「切换为逐模型配置」并保存才移除本文件这两项。无法可靠解析的内置别名需使用完整模型 ID，不猜测版本。保留其他模型设置、标签、说明及组织限制。
- OpenCode V2 在模型级 `settings.reasoningEffort` 保存；兼容条目保留原有 `options.reasoningEffort` 形式。已有别名、其他选项、variants、JSONC 注释、其他 provider 与 MCP 设置保留；取消选择的 CYAPI 模型条目移除。模型默认推理设置可被当前会话 variant 覆盖，默认模型本身不等于选定 variant。同一 API ID 的多个别名等级不同时显示冲突并分别保留；明确编辑或移除该 ID 的等级会作用于其全部别名。

- OpenCode V2 原生配置另有「可选思考等级」，通过英文逗号分隔的列表明确声明 `capabilities.reasoning.effort`，非空时同时设 `supported: true`，供 OpenCode 生成可选 variants；不猜测模型能力、不自动探测。未编辑不写入，留空只移除本文件的 `effort` 声明，已有 `toggle`、`budget` 和自定义 variants 保留，因此菜单可能仍有其他档位。默认等级不改变菜单列表。同一 API ID 的全部别名在明确编辑时同步，冲突值未编辑则分别保留。兼容旧格式不提供此入口，不自动迁移。

三端保存前均备份，重新读取会回显，恢复还原上次完整配置。Codex 的不可变模型目录保留历史文件，恢复旧引用仍可读取。Codex 配置与目录在新会话启动时加载；Claude Code 的模型和推理等级需在新会话使用或在当前会话用 `/model`、`/effort` 切换；OpenCode V2 自动重载，必要时 `/reload`，不要求关闭后台进程。

## 模型上下文窗口

三个工作区均可在「模型接入」展开配置上下文窗口，单位为 tokens（例如 `200000`，不是字符数或 `200K`）。默认不覆盖工具或模型原值；仅在明确编辑后写入，移除覆盖与撤销本次修改是不同操作。旧配置中的未知值可原样保留，不阻塞只改地址或默认模型。新值须为 `1..=9007199254740991` 内的整数，这是传输安全边界，不是模型能力上限。数值不能扩展服务商实际支持的窗口；本入口不设置最大输出长度、不探测模型能力，也不自动修改压缩阈值。

- **OpenCode：逐模型**写入 `models.<别名>.limit.context`，同时支持原生 V2 和已有兼容格式，保留 `limit.input/output` 和其他模型配置。同一 API ID 的多个别名数值不一致时明确显示冲突；不编辑则分别保留，显式设置或移除该 ID 的覆盖时更新全部对应别名。损坏的非对象 `limit` 不会被静默覆盖。
- **Codex：全局覆盖**写入 `config.toml` 的 `model_context_window`，不是逐模型设置，切换默认模型后仍适用。命名 Profile 的同名字段可能覆盖全局值；不修改 `model_auto_compact_token_limit` 或不可变模型目录中的能力元数据。该字段也纳入保存版本检查，防止覆盖其他窗口的修改。
- **Claude Code：有条件的全局覆盖**写入 `settings.json` 的 `env.CLAUDE_CODE_MAX_CONTEXT_TOKENS` 字符串，不伪装成逐模型配置。通常只有未识别、不以 `claude-` 开头且无 `[1m]` 的自定义 ID 可直接生效；已识别模型或裸 `claude-` ID 还需 `DISABLE_COMPACT`（会关闭全部压缩），未识别的 `[1m]` ID 还涉及 `CLAUDE_CODE_DISABLE_1M_CONTEXT`。具体取决于 Claude Code 版本和模型识别，见[官方生效条件](https://code.claude.com/docs/en/model-config#which-ids-claude_code_max_context_tokens-applies-to)。工具箱不会自动设置这些开关、追加 `[1m]` 或修改模型身份，也不把 `autoCompactWindow` / `CLAUDE_CODE_AUTO_COMPACT_WINDOW` 当成上下文窗口。修改后请重新启动 Claude Code 并新建会话核对；Shell 环境变量或项目配置可能覆盖本文件。

上下文设置与原有推理等级各自独立，保存与恢复沿用各工具的备份、并发检查及生效方式。清除仅删除本文件的显式覆盖，不保证清除其他配置层的设置。

## 软件接入

软件接入目录、Skill、MCP 组件、依赖和接入步骤均从服务端获取，客户端不内置具体软件适配器。MCP 接入会备份注册配置；Skill 接入安装完整文件与运行环境。缓存后可离线安装。

软件接入目录、组件、依赖和接入步骤均从服务端获取，客户端不内置具体软件适配器；接入能力随服务端目录更新，无需重新发布客户端。

服务端 `GET /api/integrations` 提供 schema 3 声明，组件按 SHA256 去重保存，运行时不经过 shell。兼容现有声明格式的新接入无需重新发布客户端；服务端维护方式见私有服务端仓库中的分发说明（服务端代码未随本仓库发布）。客户端可单独构建，无需服务端源码。

## Skill 配置

Skill 广场的配置按钮、已安装记录和安装完成页均可打开配置表单。广场入口也可为已有 Skill 配置用户级文件；写到 Skill 目录的规则需要从本工具管理的安装记录打开。

字段、步骤、默认值、校验和文件映射来自目录中的 `configuration`，客户端没有 My Image 专用配置逻辑。支持文本、URL、密钥、选项、数字、开关，以及 JSON / YAML / `.env`。规则缓存后可离线配置；安装记录另存一份对应源版本的规则。同步目录后，同一源版本使用服务端更新的规则；源版本不同或目录不可用时使用安装记录的快照。

API Key 仅通过本机 IPC 传给 Rust 写入文件，不发给目录服务；读取时只返回是否已配置，留空保留原密钥。新配置文件在写入前设置为当前用户可访问。合并时保留未绑定的数据；JSON/YAML 的排版和 YAML 注释会重新生成。`.env` 保留未绑定行及注释。无效原文件、越界路径、符号链接或目录联接会阻止写入。保存先验证全部内容，再逐文件原子替换，普通错误时尝试回滚；多文件写入不保证断电事务。

当前是声明式文件配置，暂不执行远端脚本、联网验证 API、修改系统环境变量或支持条件分支。My Image 使用上游默认文件位置；若单独设置了上游 `MY_IMAGE_GEN_ENV_FILE`，需同步调整服务端规则中的文件位置。

## 日志（Codex）

Codex 配置新增「CYAPI 专属 / 自定义服务」。CYAPI 默认 API 地址为 `https://www.cyapi.cyou/v1`，备用线路由服务器分发，支持更新、在本机测速并手动选择；测速每条线路发送 3 次不带密钥的请求并显示平均延迟，不切换配置、不发送 API Key。保存线路后，旧会话仍使用原 URL；完全退出并重启 Codex，再新建会话即可使用所选线路。更新失败时使用缓存清单，首次离线只显示官方默认线路。线路的维护说明见私有服务端仓库中的 CYAPI 线路分发文档（服务端代码未随本仓库发布）。

工作区切换到 Codex 后，侧栏「日志」读取 Codex 自身日志；全局「日志窗口」记录工具箱操作。Codex 专用分析会自动关联运行 SQLite、会话索引、CLI / 桌面文本及 JSONL / zst 会话记录，提供回合时间线、用量、请求与故障证据。使用说明见 [DIAGNOSTICS.md](DIAGNOSTICS.md)。

## 缓存与恢复

侧栏「用量统计」随工作区展示 Codex / Claude Code / OpenCode 的本机历史 Token、估算费用、缓存占比、趋势和模型分布，支持时间与模型筛选、会话分析、CSV 导出及本机单价 / 倍率配置。数据来源、去重和计费口径见 [用量统计说明](USAGE.md)。

默认数据目录 `%LOCALAPPDATA%\CYAPI-toolbox\data`，与源码目录无关；移动本项目不会改变已安装 Skill 的位置。

Skill、软件接入与 MCP 目录的本机缓存有效期为 2 小时，重启后继续计时。启动或读取对应目录时，缓存有效则直接复用，过期后自动刷新一次；手动「重新同步」「更新目录」「刷新接入目录」随时强制联网刷新。软件接入页顶部的「刷新接入目录」独立获取软件清单，不依赖 Skill 或 MCP 目录刷新。服务端返回内容未变化也会重新计算有效期，自动刷新失败则保留可用缓存，并避免在 2 小时内反复自动请求。网络请求在安装锁之外进行，服务端缓慢或离线时不阻塞本机配置页读取。

CYAPI 线路清单保存在数据目录的 `cyapi-routes.json`，启动先读取本地缓存并按已配置的 API 地址自动匹配线路，再在后台自动刷新一次；也可手动「更新线路」。自动或手动刷新成功后，完整替换旧缓存并同步三个工作区；失败则保留可用缓存。更新清单不自动切换已选 API 地址，不覆盖未保存的密钥或其他配置。页面/工作区切换复用同一快照，不重复联网；首次离线且没有有效缓存时只显示官方默认线路。

设置页新增「检查更新」：每次进入自动重新联网检查，也可点击按钮手动检查；显示当前版本、最新发布版本、检查时间及更新说明，提供下载或安装更新入口。启动提醒中「跳过此版本」「稍后提醒」不会隐藏设置里的更新信息。检查失败时允许重试；如果使用上次缓存的更新清单，会明确提示未获取到最新信息，不会误报已是最新版本。

```text
catalog.json          离线目录
installed.json        原生安装记录
cache/                按 SHA256 去重的 Skill 与 Node 备用包
python/               离线 wheel、锁文件与共享隔离依赖环境
tools/                按 SHA256 去重的命令行工具
runtimes/node/        按版本共享的 Node
backups/              卸载时保留的完整 Skill
pending.json          中断恢复日志，成功后移除
codex-config/         上次 Codex 配置备份（包含本机凭据）
claude-config/        上次 Claude Code settings.json 备份（包含本机凭据）
opencode-config/      上次 OpenCode 配置备份（包含本机凭据）
integrations/         现有软件接入目录缓存和组件管理记录
components/           从服务端下载、按 SHA256 共享的独立 MCP EXE 与许可
mcp/                  MCP 可安装目录缓存、接管记录与写入前备份
```

客户端验证整包及每个文件的 SHA256，拒绝路径越界、Windows 设备名、大小写重名与文件/目录冲突。安装先暂存，再重命名提交。卸载检查管理标记后移至备份，保留用户新增文件；当前移动要求 Skill 与数据目录位于同一磁盘分区，跨分区或文件被占用时报错并保留原目录。

Codex 全局目录尊重 `CODEX_HOME`，默认 `~/.codex/skills`；项目目录 `.agents/skills`。Claude Code 全局目录 `~/.claude/skills`，项目目录 `.claude/skills`。OpenCode 全局目录遵循 `XDG_CONFIG_HOME`，默认 `~/.config/opencode/skills`；项目目录 `.opencode/skills`。按目录名或 `SKILL.md` 的 `name`（忽略大小写）识别同名 Skill，不要求工具箱安装记录；目录必须有 `SKILL.md`。自动检测上述全局目录、Codex 的 `.system`、用户级 `~/.agents/skills` 和工具箱已有记录的项目目录；安装弹窗另查当前选择的项目，不遍历磁盘搜索项目或插件缓存。「已安装」包含本机已有 Skill；本机已有项只读展示路径，工具箱管理项才提供卸载。不同目标和项目分别判断，可安装到另一位置。

## 原生层验证

以下命令使用项目专属 Rust，也可直接使用已在 PATH 中的 cargo：

```powershell
$env:CARGO_HOME = Join-Path $env:LOCALAPPDATA 'CYAPI-toolbox/build-tools/cargo'
$env:RUSTUP_HOME = Join-Path $env:LOCALAPPDATA 'CYAPI-toolbox/build-tools/rustup'
& "$env:CARGO_HOME/bin/cargo.exe" test --lib --manifest-path src-tauri/Cargo.toml
# 服务端启动后，包含真实下载、离线安装、环境复用与启动器验证：
& "$env:CARGO_HOME/bin/cargo.exe" test --lib --manifest-path src-tauri/Cargo.toml -- --include-ignored
```

测试使用隔离临时目录。手动调试可设置进程环境变量 `CYAPI_TOOLBOX_DATA_DIR`、`CYAPI_TOOLBOX_HOME_DIR`，不要写入系统环境。

主要代码：`src/app/App.tsx` 页面，`src/app/ConfigurationDialog.tsx` 通用配置表单，`src/app/McpManager.tsx` MCP 管理页面，`src/lib/api/native.ts` 原生接口，`src-tauri/src/configuration.rs` 文件配置引擎，`src-tauri/src/mcp.rs` 三端 MCP 检测与写入，`src-tauri/src/service.rs` 安装服务，`src-tauri/src/runtime.rs` 环境检测；`src/app/Integrations.tsx` 声明式软件接入界面，`src-tauri/src/integrations.rs` 通用组件下载校验、MCP 调用与注册。具体软件适配器在独立的 `server/components/native-mcp/` 项目。原有模拟安装模块保留供回归参考，不出现在主安装流程。

本地开发可设置 `CYAPI_TOOLBOX_SERVER_URL=http://127.0.0.1:47821` 后启动 EXE；网页开发使用 `VITE_CYAPI_SERVER_URL`。不设置时保留现有线上服务地址。

本地测试可先启动 server，再双击 release/CYAPI-Toolbox_0.9.0-local.cmd；该启动器只为本次客户端进程使用本地地址。
