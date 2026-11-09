// uk product copy for Slack Socket Mode: how Slack reaches a channel.
export const messages: Readonly<Record<string, string>> = {
  "Socket Mode": "Socket Mode",
  "How Slack reaches Everruns": "Як Slack досягає Everruns",
  "Everruns connects out to Slack. Works without a public address, such as on your own computer.":
    "Everruns сам підключається до Slack. Працює без публічної адреси, наприклад на вашому комп’ютері.",
  "Slack sends each message to this server. Needs a public HTTPS address.":
    "Slack надсилає кожне повідомлення на цей сервер. Потрібна публічна адреса HTTPS.",
  "App-level token": "Токен рівня застосунку",
  "In your Slack app, open Basic Information, then App-Level Tokens, and generate one with the connections:write scope.":
    "У вашому застосунку Slack відкрийте Basic Information, потім App-Level Tokens, і створіть токен з дозволом connections:write.",
  "Open the app's settings in Slack": "Відкрити налаштування застосунку в Slack",
  "These fields are optional when saving. Everruns connects to Slack once an app-level token is saved; a bot token is needed to send replies.":
    "Ці поля необов’язкові під час збереження. Everruns підключається до Slack, щойно збережено токен рівня застосунку; для відповідей потрібен токен бота.",
  "Create an app from this channel’s manifest in Slack and install it to your workspace. Then generate an app-level token and enter it with the bot token below.":
    "Створіть у Slack застосунок із маніфесту цього каналу та встановіть його у свій робочий простір. Потім створіть токен рівня застосунку та введіть його нижче разом із токеном бота.",
  "Everruns is connected to Slack. Invite the bot to a channel, then @mention it to test.":
    "Everruns підключено до Slack. Запросіть бота в канал і згадайте його через @, щоб перевірити.",
};
