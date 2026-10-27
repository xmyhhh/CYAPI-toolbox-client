# CYAPI 多模态售卖方案：ComfyUI + 上游 API 转卖 + 统一计费（v1 草案）

> 状态：设计稿（未实施）
> 前提：sub2api 在售语言模型 API；Bridge 部署在 ComfyUI 同机或同内网；产物存储用 S3/R2（与 sub2api 异步图像任务共用一套）。
> 目标：图像 / 视频 / 语音 / 3D / ComfyUI 任意 workflow，一把 key，统一余额、统一账单、统一缓存。

---

## 0. 一句话架构

```
用户（sub2api key，一套余额）
   │
   ├─ 文本对话 ────────────────► sub2api 原生管道（现有业务，不动）
   ├─ OpenAI 标准形状的请求 ────► sub2api ──► 伪 OpenAI 账户（base_url=Bridge）
   │     /v1/images/*  /v1/audio/*  /v1/videos/*(OpenAI组,需实测)
   │
   └─ 自定义任务型请求 ─────────► Bridge 自有路由（/images /videos /audio /3d /comfy）
          Bridge = 任务引擎 + 计费桥 + 缓存层 + 上游适配器
             │
             ├─ 本地 ComfyUI（小模型自跑，按 GPU 秒计费）
             ├─ 上游厂商 API（转卖，按上游成本 × 倍率计费）
             └─ 产物 → S3/R2 → 签名短链 → 缓存索引
```

---

## 1. 计费模型：两种计费器覆盖全部业务

### 1.1 计费器 A：伪 usage（同步请求，宝贝在响应里）

适用：图像同步接口、ASR、TTS 等"一次请求一次响应"的场景。

- Bridge 在**成功**响应里返回伪造 `usage`：
  ```json
  { "usage": { "input_tokens": 0, "output_tokens": 1234, "total_tokens": 1234 } }
  ```
- sub2api 按 `output_tokens × 渠道 rate_multiplier × 单价` 记流水。
- **失败必须返回 4xx/5xx**（任何 200+空结果都会等于白扣用户钱，是红线）。
- 价目表长在 Bridge 配置里，按 `model` 查积分价。

### 1.2 计费器 B：计费桥（异步任务 + ComfyUI 任意 workflow）

适用：视频、3D、图像 async、以及"用户/站内提交任意 ComfyUI workflow"。

sub2api 的 billing 只认"响应里的 usage"，管不了几分钟的异步跑到一半失败。所以异步任务由 Bridge 自己记账，任务终态决定扣不扣：

```
pending（不入账）→ running（不入账）
   ├─ succeeded   → 扣费（冻结时预检余额，成功时实扣）
   ├─ failed / canceled / timeout → 不扣费（记 0 元失败流水，后台可查）
   └─ expired     → 不扣费
```

**ComfyUI 完整能力支持 = 计费不再依赖枚举模型，而是两大模式：**

#### 模式一：打包定价（运营主流，推荐起步）

把常用能力封装成**可发布的"业务节点"（Workflow Package）**：

```
product_code         例如 img.t2i.flux-dev
workflow_id          对应一个 ComfyUI workflow JSON（模板）
input_schema         对外暴露哪些节点为参数（prompt / size / seed / steps / 图入）
output_spec          输出节点及产物类型（png → S3, mesh → glb, audio → mp3）
pricing              base_price + 超参数加减价（如 upscale = base × 2.5）
engine               comfy | vendor:<reco> | vendor:<triposg>（见 §2）
```

发布一个新玩法 = 新增一行产品表 + 一个 workflow JSON，计费/缓存/前端自动生效。你的 workflow 库和草稿页（research 分支已做的 UI）就是朝着这个方向。

#### 模式二：GPU 秒计费（兜底"所有功能"）

对没打包、自由提交的 workflow 图（也就是 ComfyUI /prompt 的任意 JSON，做白名单校验后透传）：
按**实测执行时长**计费：埋点 ComfyUI 执行生命周期

- 后端资源按每分钟执行时长分成执行计费，历史 /history 返回执行耗时
- 价目：`GPU_MINUTE × card_multiplier`（RTX4090 本地 ×1，租卡 ×N）
- 用途：高级用户直跑任意节点组合（通过**预校验白名单**，禁止undeclared伤害性节点/子进程/联网节点）

两个模式共用同一套终态扣费逻辑和失败不扣费原则。

### 1.3 价目表（config 结构示例）

```yaml
pricing:
  local_gpu:
    minute_cost: 0.02            # 每 GPU 秒 ×60 折美元，自跑硬件时用
  vendors:                       # 转卖型：上游价 × 倍率（含平台毛利）
    kling:      { margin: 1.3,  per_s: { "720p-5s": 0.50  } }
    wanxiang:   { margin: 1.35, per_s: { "720p-5s": 0.42  } }
    hunyuan3d:  { margin: 1.3,  per_mesh: 0.55 }
    tripo:      { margin: 1.3,  per_mesh: 0.40 }
    gpt_sovits: { margin: 1.25, per_min: 0.10 }   # 语音转写
  products:                      # 打包定价（覆盖本地/上游的成品玩法）
    img.t2i.flux-dev:      { cost: 0.05,  engine: comfy,   workflow: t2i_flux }
    img.t2i.flux-dev-fast: { cost: 0.02,  engine: vendor:siliconflow, model: FLUX.1-schnell }
    vid.t2v.wan25-720p-5s: { cost: 0.60,  engine: vendor:kling-or-wan }
    mesh.text2mesh-1M:     { cost: 0.55,  engine: vendor:tripo }
    asr.whisper-lg:        { cost_tokens: 250,  engine: local:faster-whisper }  # 同步型走计费器A
```

---

## 2. 上游适配器（engine=vendor）与本地引擎（engine=comfy）

统一任务引擎不关心内容从哪来，只调用统一接口：

```python
class Engine:
    async def submit(self, task) -> Handle        # 提交上游 / 建 workflow
    async def poll(self, handle) -> TaskStatus    # 轮询
    async def fetch(self, handle) -> list[bytes]  # 结果取回
    def price(self, task) -> Money                # 该任务应收
```

- **vendor 适配器**（一层薄封装，失败重试≤2，上游 429/超时→任务 `failed` 不扣费再换线路）
  - 视频：即梦 AI / 可灵 / 通义万相 API
  - 图像：Stable Diffusion 系 / Flux 厂商 API（硅基流动等）＋ Nano Banana / gpt-image-1（转卖）
  - 语音：MiniMax 语音 / 硅基流动 ASR·TTS / OpenAI Whisper API
  - 3D：Tripo API / Meshy API / Hunyuan3D API
- **comfy 本地引擎**：复用你 `research/workflow-service/` 已有的实例锁 / stop-token / start/stop，加队列分发与状态上报，改造成常驻生产服务（loopback-only 保留，但允许 Bridge 内网夹带）。

**转卖 vs 自跑的仲裁规则**（写进 engine chooser）：
1. 有打包产品优先走产品定义
2. 自跑 GPU 忙碌 > 阈值 或 卡型不满足（视频大模型本地跑不动）→ 自动落上游 vendor
3. 上游不可用 → fallback 本地（配了的话）
4. 都不行 → `failed`，不扣费

这样小模型自跑省成本、大模型转卖保可用，一个开关每类业务可配置。

---

## 3. 缓存层（全模态统一）

缓存是计费的附属品（同一输入白嫖会伤卖家，所以要明确"是否免费命中"）。

### 3.1 缓存键（规范化哈希）

```
key = sha256( scope + kind + normalized_payload )

scope : user 或 global（运营配，默认 user 以下才共享）
kind  : products:{model, params, seed, workflow_hash}
        （ComfyUI 自由图：workflow 的规范 JSON 哈希，包含所有节点参数与模型名）
```

- `seed=0`（用户没指定）视作不进缓存键 → CD 走一次缓存可能就不妙了；**默认：没指定 seed 时生成随机 seed 并**写进缓存键**，保证同参同结果可复现
- 值：产物对象 key 列表（S3/R2），不存二进制

### 3.2 读流程

```
查缓存
 ├─ HIT  → 校验产物仍存在 → 签名短签返回
 │         计费：按产品价 × cache_discount（自由配：全部 免/半价/全价，默认用户级缓存=免，全球共享=半价）
 └─ MISS → single-flight（同 key 并发只发一个 upstream，请求在 Redis 上挂锁排队）
            完成后写缓存、其余等待者共享同一产物
```

- L1 本地 NFS 热缓存（跑图机器磁盘），L2 S3/R2；存不到本地时回传从 S2 拉
- TTL 按 kind 配置（图像 7 天 / 视频 30 天 / mesh 90 天 / asr 不缓存）
- **绝不缓存**昨天喜欢以 tf32 之外的那些不可复现输入（如 `random()` 节点 / 非 seeded 随机噪声）→ 通过标记 workflow 里含有"未 seed 随机性"就整体不缓存该图

### 3.3 与 sub2api 异步图像的协同

sub2api 自带的 `/v1/images/tasks/{id}` 已有 S3 转存逻辑。我们的缓存索引在 Bridge 层再建一层（用同一桶），保证：转存图 URL 也可命中缓存；产物 URL 统一带 `Cache-Control` 和过期签名。

---

## 4. 模块明细（业务流逐一）

### 4.1 图像（文生图 / 改图 / 补图）

| 路径 | 引擎 | 计费器 | 备注 |
|---|---|---|---|
| `POST /v1/images/generations`（标 OpenAI 格式） | 伪账户 → Bridge SYNC  | A（伪 usage） | 同步小图；客户端 SDK 兼容性最好 |
| `POST /v1/images/generations/async` | sub2api 原生 → Bridge Async | B（任务） | 转卖高分辨 gpt-image；S3 开启后走 §3 |
| `POST /bridge/images/edit` | Bridge | B | inpaint / 局部重绘（ComfyUI 节点流） |
| `POST /bridge/comfy`（自由图） | comfy 白名单 | B（GPU 秒） | §1.2 模式二 |

### 4.2 视频

| 路径 | 说明 |
|---|---|
| `POST /bridge/videos/txt2video`   | 提交 wan/kling 任务；产物 mp4 上 S3；**转卖（自跑视频可先不启用）** |
| `POST /bridge/videos/img2video`  | 图入图出 |
| `POST /bridge/videos/extension` | 续写延长 |
| `GET  /bridge/videos/tasks/{id}` | pending → running → succeeded/failed，成功返回 S3 直链 |

计费细粒度按 `分辨率 × 秒数` 分档，PRICING 表里是明确货币价、非伪 token。转卖是默认路线：自跑视频对显存要求太高，等 GPU 预算充足再开本地档（价目表里区分 `local_wan` / `vendor_wan`）。

### 4.3 语音

| 能力 | 路径 | 引擎/计费 | 时长计入 |
|---|---|---|---|
| ASR（录音转文字） | `POST /v1/audio/transcriptions`（OpenAI 形态） | 本地 faster-whisper（计费器A，按音频秒折 token）或 vendor | `cost = ceil(秒/60) × per_min` |
| TTS（语音合成） | `POST /bridge/audio/speech` | MiniMax / GPT-SoVITS 自跑 | 按字数/次数 |
| 翻译/字幕 | `POST /bridge/audio/transcribe` 扩展参数 | 同 ASR | 加工价 |

**先实测**：sub2api 的 OpenAI 组是否转发 `/v1/audio/*` 并计费（文档没写；用伪账户实测一发便知）。不行就走 `/bridge/*` + 计费器 B。

### 4.4 3D mesh

| 路径 | 说明 |
|---|---|
| `POST /bridge/3d/text2mesh` | Tripo / Hunyuan3D，产物 glb+贴图上 S3 |
| `POST /bridge/3d/image2mesh` | 单图重建 3D |
| `POST /bridge/3d/refine`     | 精修/补面（第一次产物 task_id 作输入） |

计费按**次**，价随**面数档位**（1万/10万/百万面）。ComfyUI 也有 Hunyuan3D 节点可自跑小档位：same engine chooser 自动二选一。格式转换、贴图烘培等衍生操作按"子操作加价"叠加。

### 4.5 ComfyUI 全能力（工作流工作室接入）

research 分支规划中的 UI 是公司封装，计费侧接入就绪：

```
前端工作流画布 →  你已做的 workflow 草稿/模板
     ↓ "连接 CYAPI"
POST /bridge/comfy（打包型 → 走产品价目；自由型 → GPU 秒计费）
   ↓
缓存命中（同 seed 同参数）→ 秒回旧图，按 cache_discount 收费（帮用户省钱还省你 GPU）
   ↓
异步任务 → UI 轮询收图 → 后台写账单（含 workflow 摘要做账单描述，用户能看懂烧钱在哪个节点）
```

**管理员能力**（后台配）：发布/下架产品、调价、每用户日限额（防恶意烧余额）、单 workflow 过大（>500 节点）拒绝。

---

## 4.6 LoRA / 权重资产库（用户自带模型）

背景（对标 RunningHub 生态）：ComfyUI 出图质量大头取决于"底模 + LoRA 配方"，社区 LoRA 是生态刚需。RunningHub 把用户上传的 LoRA 做成云端托管资产；我们走**本地优先**路线——用户自己电脑里本来就有 `models/loras/`，直接读，不上传、不托管、无审核合规负担。

### 三层资产策略

| 层级 | 形态 | 说明 |
|---|---|---|
| L1 本地库（默认） | 扫描用户本地 ComfyUI 的 `models/loras/`、`checkpoints/`、`vae/` | 工具箱"资产"页展示清单：名称/大小/base model/触发词(可解析 civitai 元数据)；运行 workflow 时直接引用，零上传 |
| L2 打包模板 | 发布"工作流产品"时自带 `asset_manifest`（该 workflow 依赖的 LoRA 清单 + 推荐下载源 civitai 链接/hash 校验） | 用户一键安装模板 → 客户端校验本地缺失 → 提示一键补齐下载（下载到本地，不进我们服务器） |
| L3 云共享库（远期，可选） | 若做"云端转卖档"，仅上架**合规且我们有权分发**的少量 LoRA | 默认不做用户上传托管（审核/版权/NSFW 风险高，国内合规成本大）；云档应用遇到用户私有 LoRA 时明确降级为"该 LoRA 仅本地档可用" |

### 与引擎仲裁（§2）的联动

```
engine chooser 增加 asset 检查：
workflow 依赖 resource ∈ {checkpoint, lora, vae, upscale_model} 
 → 本地库有 → 可跑本地档（GPU 弹性规则不变）
 → 本地缺  → 提示补齐；用户拒绝补齐但可用云档替代 → 引导云档（按 product 价计费）
 → 云档也依赖（如 LoRA 叠加合成）上游 API 不支持 → 标记"仅本地"，产品页明确说明
```

### 计费相关

- L1 本地库 = 纯本地执行 → 按 GPU 秒计费（§1.2 模式二）或免费档，**不因用了 LoRA 加价**（LoRA 本身对推理几乎无额外开销）
- L2 打包模板含 LoRA → 不额外收费（LoRA 下载免费走 civitai 原链）；我们卖的是"调好的配方"，价格就是 workflow 产品价
- L 云共享库 → 若上架，按"含版权内容的产品"单独定价，收益与作者分成挂钩（对应"作者分成"空档策略）

### 技术要点

- 扫描不用 ComfyUI server 维护的 metadata，自己读 safetensors 头（lightweight, `safetensors` 库 0 spark）解析 trigger words / base model / license 字段
- MD5/hash 校验和统一为 `asset_id`（从 RunningHub 页面细节见的做法一致，便于模板之 cross-machine 复现）
- 客户端 asset 页复用现有"技能库/集成"UI 模式（installer/install 交互），陶集成不新增交互惯例
- 版权红线：L2 manifest 只放引用链接，**不给用户 rehost 未授权权重**；显示 license 字段（civitai 元数据有），商业不允许的要标注"仅个人使用"

## 5. 数据库 schema（新增表，独立 Postgres schema 或库）

```sql
-- 产品（打包定价的玩法）
products(id, code UNIQUE, name, kind, engine, workflow_ref,
         input_schema jsonb, output_spec jsonb,
         cost numeric, cost_type per_run|per_gpu_s|per_second|per_minute|per_char,
         cache_policy, enabled, created_at)

-- 任务（所有异步业务，含 ComfyUI 自由图）
bridge_tasks(id, user_id, api_key_id, product_code NULL, kind,
             engine, workflow_id NULL, params jsonb,           -- 规范化入参（缓存键来源之一）
             cost_estimate numeric, cost_final numeric NULL,    -- 终态实扣，NULL=未扣
             status pending|running|succeeded|failed|canceled|expired,
             gpu_seconds numeric NULL,                          -- comfy 引擎实测
             artifact jsonb, cache_key NULL,                    -- 产物 S3 键 + 缓存索引引用
             error jsonb, submitted_at, finished_at,
             billing_mode pseudo_usage|ledger,                  -- 计费器Aのもので终态由上游流水体现
             created_at)

-- 缓存索引
cache_entries(cache_key PK, kind, scope, object_keys jsonb,
              size_bytes, hits int, created_at, expires_at)

-- 账单流水摘要（sub2api 已有 token 流水；此处只记 Bridge 维度，便于对账 + 给用户展示"烧在哪"）
bridge_bill_items(id, task_id, user_id, amount, desc, billed_at)
```

✅ **已源码验证（M0 完成，2026-10-27，基于 Wei-Shaw/sub2api 最新源码）**：
1. **伪 usage 计费成立**：网关转发后会解析上游响应体里的 `usage` 对象（`internal/service/openai_gateway_response_handling.go:1350`，路径 `usage` / `response.usage` / `data.usage` 等），字段支持 `input_tokens|prompt_tokens`、`output_tokens|completion_tokens`、`total_tokens`、cache/reasoning/image 细分。Bridge 成功返回伪 usage → 自动按 token 计费 ✓
2. **图像另有按张计费**：组级 1K/2K/4K 分档单价（`image_billing_size.go:28`），无 usage 时按张数兜底扣费（`ImageCountBilling` 开关）；所以图像更建议走"按张定价"而不是伪 token。
3. **余额调整 admin API 存在**：`POST /api/v1/admin/users/:id/balance`（`admin/user_handler.go:405`），支持 set/add/subtract + 备注幂等写入 → **计费器 B（异步任务成功后扣款）直接调它即可，无需自建子账本**。
4. **base_url 自定义无域名校验**（默认只查格式，`security.url_allowlist` 未开时不查域名；生产建议开启 allowlist 包含 Bridge 地址）。
5. **图像模型名会被校验**：images 端点要求模型名能被识别为 gpt-image-* / gemini-*-image*（`validateOpenAIImagesModel`）——自定模型名需用复合组"显式路由"把公开模型名映射到已识别的上游模型名，或在 Bridge 端忽略名字按 product_code 定价。
6. 成本头（x-openai-*）不参与计费，只看响应体。

**推论修正**：§1.1 伪 usage 适用于 audio/chat 类；**图像类直接用组级按张定价更干净**（configure ImagePrice1K/2K/4K），Bridge 不必伪造 usage。

---

## 6. 里程碑（每步可独立收款）

| # | 交付 | 计费 | 依赖 | 目标 |
|---|---|---|---|---|
| M0 | PoC：伪账户 + 伪 usage 计费验证（半天） | A | sub2api 后台 | 验证计费链路（最高风险先破） |
| M0.5 | PoC：sub2api 用户余额被外部扣款的方式敲定 | B | admin API 侦查 | §5 的 ⚠️ 点闭环 |
| M1 | 图像同步（文生图 · 3-4 个打包产品） | A | ComfyUI 常驻化 | 第一批可卖 |
| M2 | 图像 async + S3 + 缓存 L2 | A/B | R2 账号 | 高分辨率可卖、成本降 |
| M3 | ASR + TTS | A/B | whisper 部署或 API | 语音可卖 |
| M4 | 视频全套（vendor 为主） | B | 厂商开通 | 视频可卖 |
| M5 | 3D mesh | B | Tripo/智浦 开通 | 3D 可卖 |
| M6 | ComfyUI 自由图（白名单 + GPU 秒计费）+ 工作流 UI 正式接入 | B | security review | "全能力"卖点收宫 |

---

## 7. 合规与安全红线

- 凭据净化：产物 PNG metadata、webhook、日志一律不带 sub2api key
- 上游 API key 只存 Bridge 服务端（环境变量 / secrets），绝不随 workflow JSON 下发
- ComfyUI 自由图必须过节点白名单 + 资源上限（进程数、显存、时长），防止 `%/` 溢出等
- 用量侧账单可解释：每条流水带"业务 carga 描述"（哪个产品 / 哪个 workflow / 多少秒），用户的 tech-closure 体验是你的差异化
- `allow_insecure_http=false`，Bridge 走内网 → sub2api 出口，公网只露 sub2api 域名
