// zh-CN product copy for the agent's Channels and triggers tab, Pause agent and the GitHub page.
export const messages: Readonly<Record<string, string>> = {
  "Channels and triggers": "渠道和触发器",
  "How people and software reach this agent, and when it starts on its own.":
    "人员和软件如何访问此智能体，以及它何时自行启动。",
  "When this agent starts on its own: schedules, webhooks, GitHub events and MCP server events.":
    "此智能体何时自行启动：计划、Webhook、GitHub 事件和 MCP 服务器事件。",
  "Add a schedule, a webhook or an event to start this agent on its own.":
    "添加计划、Webhook 或事件，让此智能体自行启动。",
  "Delete this trigger? It stops starting the agent.": "删除此触发器？它将不再启动智能体。",
  "Turn off": "关闭",
  "Turn on": "开启",
  "Pause agent": "暂停智能体",
  "Resume agent": "恢复智能体",
  "Pause {value0}? Its channels stop accepting messages and its triggers and schedules stop starting it until you resume it. Sessions already running continue.":
    "暂停 {value0}？在你恢复之前，它的渠道将不再接收消息，它的触发器和计划也不会启动它。正在运行的会话会继续。",
  "This agent is paused": "此智能体已暂停",
  "Its channels do not accept messages and its triggers and schedules do not start it. Each channel and trigger keeps its own setting for when you resume.":
    "它的渠道不接收消息，它的触发器和计划不会启动它。每个渠道和触发器都会保留各自的设置，恢复后照常生效。",
  "Publish opens this channel to callers and Unpublish closes it. Pausing the agent overrides both.":
    "发布会向调用方开放此渠道，取消发布会关闭它。暂停智能体优先于这两者。",
  "Add a channel from an agent's Channels and triggers tab. New channels start as drafts and take no traffic until you publish them.":
    "在智能体的“渠道和触发器”标签页中添加渠道。新渠道以草稿开始，发布前不接收流量。",
  "Need another system to start the agent with an HTTP request?":
    "需要让其他系统通过 HTTP 请求启动智能体？",
  "Add a webhook trigger": "添加 Webhook 触发器",
  "Trigger type": "触发器类型",
  Token: "令牌",
  Generate: "生成",
  "Leave empty to keep the current token": "留空以保留当前令牌",
  "Copy it now: it is not shown again. Callers send it as Authorization: Bearer <token> or X-Everruns-Webhook-Token.":
    "请立即复制：它不会再次显示。调用方以 Authorization: Bearer <token> 或 X-Everruns-Webhook-Token 发送。",
  "Recent runs": "最近运行",
  "Loading runs…": "正在加载运行记录…",
  "Starts when {value0} sends {value1}.": "当 {value0} 发送 {value1} 时启动。",
  "MCP event triggers are created and changed through the API and CLI.":
    "MCP 事件触发器通过 API 和 CLI 创建和修改。",
  "Who {value0} is on GitHub when it reads code and comments on pull requests.":
    "{value0} 在 GitHub 上读取代码和评论拉取请求时的身份。",
  "Not connected": "未连接",
  "Not installed": "未安装",
  "Own GitHub App": "自有 GitHub App",
  "Shared Everruns App": "共享的 Everruns App",
  Kind: "类型",
  trigger: "触发器",
  "The agent's GitHub tools, while the GitHub capability is on":
    "智能体的 GitHub 工具（在 GitHub 能力开启时）",
  "It may have been deleted, or you may not have access to it.":
    "它可能已被删除，或者你没有访问权限。",
};
