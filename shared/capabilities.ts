/** Product vocabulary shared by the desktop app and website. */
export const capabilities = [
  {
    id: "integrations",
    title: "软件接入",
    purpose: "让 AI 使用常用软件",
    description: "选择软件，按教程准备所需能力，再完成第一次使用。",
    detail: "把软件、Skill、工具连接和运行环境组织成一套接入步骤。",
    example: "例如：检索文献、生成视频草稿。",
    guide: "integrations",
  },
  {
    id: "skills",
    title: "Skill 管理",
    purpose: "扩展 AI 的任务能力",
    description: "安装工作方法、配套脚本和资源，让 AI 按流程完成任务。",
    detail: "按写作、设计、开发等任务挑选技能，管理安装、配置和更新。",
    example: "例如：论文配图、PDF 处理、系统化排错。",
    guide: "skills",
  },
  {
    id: "mcp",
    title: "MCP 管理",
    purpose: "连接 AI 的工具与数据",
    description: "配置 AI 可以调用的工具服务，管理连接与启停。",
    detail: "查看接入要求，管理当前 AI 工具的 MCP 配置和本机自定义服务。",
    example: "例如：浏览器操作、SSH、文档检索。",
    guide: "mcp",
  },
] as const;

export type CapabilityId = (typeof capabilities)[number]["id"];
export const capability = (id: CapabilityId) =>
  capabilities.find((item) => item.id === id)!;
