export const messages: Readonly<Record<string, string>> = {
  "External agent": "外部智能体",
  "Run this agent's turns on your own HTTPS endpoint.":
    "在你自己的 HTTPS 端点上运行此智能体的轮次。",
  "Your endpoint runs this agent's turns.": "你的端点运行此智能体的轮次。",
  "Your code runs each turn with its own system prompt, model and tools. Sessions, Playground, Trace and channels keep working. While an endpoint is connected, the page hides the settings it owns.":
    "你的代码使用自己的系统提示词、模型和工具运行每个轮次。会话、Playground、Trace 和渠道照常工作。端点连接期间，页面会隐藏由它管理的设置。",
  "Your endpoint runs every turn with its own system prompt, model and tools. Everruns keeps the sessions, channels, traces and usage for this agent.":
    "你的端点使用自己的系统提示词、模型和工具运行每个轮次。Everruns 保留此智能体的会话、渠道、追踪和用量。",
  "Harness, capabilities, model, MCP servers, credentials, files, network access, sandbox and token limits are hidden: your endpoint owns them.":
    "Harness、能力、模型、MCP 服务器、凭据、文件、网络访问、沙箱和令牌上限已隐藏：由你的端点管理。",
  "Deploy your endpoint": "部署你的端点",
  "Any HTTPS service that implements the endpoint protocol.": "任何实现端点协议的 HTTPS 服务。",
  "Cloudflare agent guide": "Cloudflare 智能体指南",
  "Install an endpoint secret": "安装端点密钥",
  "Set it in your endpoint's environment before you connect, for example with wrangler secret put EVERRUNS_ENDPOINT_SECRET. At least 32 characters. Everruns stores it encrypted and does not show it again.":
    "连接前先在端点环境中设置它，例如运行 wrangler secret put EVERRUNS_ENDPOINT_SECRET。至少 32 个字符。Everruns 加密存储，之后不再显示。",
  "Endpoint URL": "端点 URL",
  "Everruns sends a signed challenge to check that the endpoint holds the secret. After you connect, messages and session history for this agent go to the endpoint. Running turns must finish first.":
    "Everruns 会发送签名质询，确认端点持有该密钥。连接后，此智能体的消息和会话历史会发送到该端点。正在运行的轮次需先结束。",
  "Verify and connect": "验证并连接",
  "Could not connect the endpoint.": "无法连接端点。",
  "Could not disconnect the endpoint.": "无法断开端点。",
  Disconnected: "已断开",
  Queue: "队列",
  "New messages wait until an endpoint is connected again.": "新消息会等待，直到重新连接端点。",
  "The last call failed": "上一次调用失败",
  "Open the session's trace for details.": "详情请查看该会话的追踪。",
  "Change the endpoint or rotate the secret": "更换端点或轮换密钥",
  "Install the new secret in your endpoint first, then verify again. The current connection keeps working until the new one is verified.":
    "先在端点中安装新密钥，然后重新验证。新连接验证通过前，当前连接继续工作。",
  "Update connection": "更新连接",
  "Disconnect endpoint": "断开端点",
  "New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "新消息会在队列中等待，直到重新连接端点。Everruns 不会自行运行它们。",
  "Disconnect the endpoint? New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "断开端点？新消息会在队列中等待，直到重新连接端点。Everruns 不会自行运行它们。",
  "Connecting an endpoint requires permission to change dangerous agent settings.":
    "连接端点需要修改智能体敏感设置的权限。",
};
