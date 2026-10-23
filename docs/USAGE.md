# 用量统计

侧栏「用量统计」随 Codex / Claude Code / OpenCode 工作区切换，离线读取本机历史会话。页面提供今天、近 7 天、近 30 天、全部、自定义日期范围，以及模型与关键词筛选；总览展示 Token、估算费用、缓存读取占比、计费用量记录数、趋势和模型分布。明细与会话列表支持分页，会话可下钻到明细，筛选结果可导出 CSV。

## 数据来源

- Codex：`CODEX_HOME/sessions` 和 `CODEX_HOME/archived_sessions`，默认 `~/.codex`，支持 `.jsonl` 和 `.jsonl.zst`。读取 `session_meta`、`turn_context` 和 `token_count`，优先采用单次用量，累计快照作为兜底。重复通知、相同线程的记录副本以及显式父子会话重放会合并；跨文件续写恢复累计基线，模型切换不清零累计值。
- Claude Code：`CLAUDE_CONFIG_DIR/projects`，默认 `~/.claude/projects`，包含子 Agent 记录。按 `message.id` 合并分段响应，取各项用量最大值，忽略 synthetic 消息。
- OpenCode：`XDG_DATA_HOME/opencode/opencode.db`，默认 `~/.local/share/opencode/opencode.db`，只读打开并保持 WAL 可见。支持 `message` 和 `session_message` 两种表结构，包含新版已完成的 compaction 用量；不统计尚未完成的响应。没有数据库时读取旧版 `storage/message` JSON，避免与已迁移数据库重复累计。

仅返回模型、时间、会话标识、项目路径及用量等元数据，不读取 API Key，也不向服务端上传会话正文。来源路径、读取失败、缺失父会话、损坏尾行和读取上限在页面底部显示；部分读取时会展示提示。日志删除后不再计入下次扫描，统计并非永久账单存档。

未变化的 JSONL / JSON 文件按大小和修改时间复用进程内解析缓存；修改、追加或截断后重新解析。底部「重新扫描全部文件」绕过缓存。SQLite 每次重新查询，只读事务保持查询内一致性。单次上限为每个来源 20,000 份文件、单文件及解压后 512 MB、单行 4 MB、200,000 条记录；合并前原始记录达到 400,000 条会停止读取剩余文件，超限会明确提示结果不完整。

## Token 与费用口径

输入包含缓存读取与写入，输出包含推理。缓存与推理是子集，不再加到总 Token。Claude / OpenCode 的独立缓存字段会归一化到输入中；OpenCode 的独立 reasoning 字段会归一化到输出中。缓存读取占比 = 缓存读取 / 总输入，不代表供应商保证的命中率。计费用量记录数只计带 Token 的模型响应，不包含没有用量的失败请求，也不是用户消息数或完整 HTTP 请求数。

费用优先使用当前工作区自定义单价，其次是 OpenCode 中正数的记录费用，最后是精确模型名称匹配的内置参考单价。不对模型别名猜价；缺失单价时保留 Token 并显示「待配置」，汇总仅相加已知费用，明确列出待定价记录数。零记录费用可能代表未知价格，需配置零单价才能明确表示免费。

在「模型单价」中分别配置普通输入、输出、缓存读取、5 分钟缓存写入、1 小时缓存写入，单位为 USD / 百万 Token；也可设置当前工作区费用倍率。计算时普通输入先扣除缓存读写，1 小时写入从总写入中单独分出，推理输出按输出计费。设置仅保存在本机应用存储，修改后即时重算，零单价和零倍率均有效。

参考单价不是实时报价，精确名称以代码中的 `referencePrices` 为准。实际价格请核对 [OpenAI 定价](https://openai.com/api/pricing/) / [Anthropic 定价](https://platform.claude.com/docs/en/about-claude/pricing) 及所用网关。长上下文阶梯、服务层级、折扣、订阅权益及账户余额不从会话中推断；导出的金额是分析估算，不等同于账户账单。日期范围使用本机时区，近 7 / 30 天包含今天。CSV 时间使用带 `Z` 的 UTC 时间，模型和项目字段已防护表格公式注入。

## 上游参考与验证

参考 [CC Switch](https://github.com/farion1231/cc-switch) 提交 `f92ff0b03964188f1cd011d9302ab8869d7a8219` 的本地会话解析和缓存计费口径，许可保留在 `THIRD-PARTY-NOTICES.md`。

```powershell
npm test
npm run build
# 配好 Rust 工具链后
cargo test --manifest-path src-tauri/Cargo.toml --lib usage::
```
