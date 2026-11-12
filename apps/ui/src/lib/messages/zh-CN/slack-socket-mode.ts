// zh-CN product copy for Slack Socket Mode: how Slack reaches a channel.
export const messages: Readonly<Record<string, string>> = {
  "Private approval cards": "私密审批卡片",
  "Show approval cards only to the requester. If a card disappears, answer the pending request in Everruns. Agent replies remain visible in the conversation.":
    "仅向请求者显示审批卡片。如果卡片消失，请在 Everruns 中回复待处理的请求。智能体的回复仍会在对话中显示。",

  "Socket Mode": "Socket Mode",
  "How Slack reaches Everruns": "Slack 如何连接 Everruns",
  "Everruns connects out to Slack. Works without a public address, such as on your own computer.":
    "Everruns 主动连接 Slack。无需公网地址，例如在你自己的电脑上也能使用。",
  "Slack sends each message to this server. Needs a public HTTPS address.":
    "Slack 将每条消息发送到此服务器。需要公网 HTTPS 地址。",
  "App-level token": "应用级令牌",
  "In your Slack app, open Basic Information, then App-Level Tokens, and generate one with the connections:write scope.":
    "在你的 Slack 应用中打开 Basic Information，再打开 App-Level Tokens，生成一个带 connections:write 权限的令牌。",
  "Open the app's settings in Slack": "在 Slack 中打开应用设置",
  "These fields are optional when saving. Everruns connects to Slack once an app-level token is saved; a bot token is needed to send replies.":
    "保存时这些字段可留空。保存应用级令牌后 Everruns 即连接 Slack；发送回复需要机器人令牌。",
  "Create an app from this channel’s manifest in Slack and install it to your workspace. Then generate an app-level token and enter it with the bot token below.":
    "在 Slack 中用此频道的清单创建应用并安装到你的工作区。然后生成应用级令牌，并在下方与机器人令牌一起填写。",
  "Everruns is connected to Slack. Invite the bot to a channel, then @mention it to test.":
    "Everruns 已连接 Slack。将机器人邀请到频道，然后 @提及它进行测试。",
};
