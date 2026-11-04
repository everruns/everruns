export const messages: Readonly<Record<string, string>> = {
  "External agent": "Зовнішній агент",
  "Run this agent's turns on your own HTTPS endpoint.":
    "Виконуйте ходи цього агента на власній HTTPS-точці.",
  "Your endpoint runs this agent's turns.": "Ходи цього агента виконує ваша кінцева точка.",
  "Your code runs each turn with its own system prompt, model and tools. Sessions, Playground, Trace and channels keep working. While an endpoint is connected, the page hides the settings it owns.":
    "Ваш код виконує кожен хід із власними системними інструкціями, моделлю та інструментами. Сеанси, Playground, Trace і канали працюють як зазвичай. Поки кінцеву точку підключено, сторінка приховує налаштування, якими вона керує.",
  "Your endpoint runs every turn with its own system prompt, model and tools. Everruns keeps the sessions, channels, traces and usage for this agent.":
    "Ваша кінцева точка виконує кожен хід із власними системними інструкціями, моделлю та інструментами. Everruns зберігає сеанси, канали, трасування та використання для цього агента.",
  "Harness, capabilities, model, MCP servers, credentials, files, network access, sandbox and token limits are hidden: your endpoint owns them.":
    "Обв'язку, можливості, модель, MCP-сервери, облікові дані, файли, мережевий доступ, пісочницю й ліміти токенів приховано: ними керує ваша кінцева точка.",
  "Deploy your endpoint": "Розгорніть кінцеву точку",
  "Any HTTPS service that implements the endpoint protocol.":
    "Будь-який HTTPS-сервіс, що реалізує протокол кінцевої точки.",
  "Cloudflare agent guide": "Посібник з агента Cloudflare",
  "Install an endpoint secret": "Додайте секрет кінцевої точки",
  "Set it in your endpoint's environment before you connect, for example with wrangler secret put EVERRUNS_ENDPOINT_SECRET. At least 32 characters. Everruns stores it encrypted and does not show it again.":
    "Задайте його в середовищі кінцевої точки до підключення, наприклад командою wrangler secret put EVERRUNS_ENDPOINT_SECRET. Щонайменше 32 символи. Everruns зберігає його зашифрованим і більше не показує.",
  "Endpoint URL": "URL кінцевої точки",
  "Everruns sends a signed challenge to check that the endpoint holds the secret. After you connect, messages and session history for this agent go to the endpoint. Running turns must finish first.":
    "Everruns надсилає підписаний виклик, щоб перевірити, що кінцева точка має секрет. Після підключення повідомлення та історія сеансів цього агента надходять до кінцевої точки. Спершу мають завершитися поточні ходи.",
  "Verify and connect": "Перевірити й підключити",
  "Could not connect the endpoint.": "Не вдалося підключити кінцеву точку.",
  "Could not disconnect the endpoint.": "Не вдалося відключити кінцеву точку.",
  Disconnected: "Відключено",
  Queue: "Черга",
  "New messages wait until an endpoint is connected again.":
    "Нові повідомлення чекають, доки кінцеву точку не буде підключено знову.",
  "The last call failed": "Останній виклик не вдався",
  "Open the session's trace for details.": "Подробиці дивіться в трасуванні сеансу.",
  "Change the endpoint or rotate the secret": "Змінити кінцеву точку або оновити секрет",
  "Install the new secret in your endpoint first, then verify again. The current connection keeps working until the new one is verified.":
    "Спершу додайте новий секрет до кінцевої точки, потім перевірте знову. Поточне підключення працює, доки нове не буде перевірено.",
  "Update connection": "Оновити підключення",
  "Disconnect endpoint": "Відключити кінцеву точку",
  "New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Нові повідомлення чекають у черзі, доки кінцеву точку не буде підключено знову. Everruns не виконує їх сам.",
  "Disconnect the endpoint? New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Відключити кінцеву точку? Нові повідомлення чекають у черзі, доки кінцеву точку не буде підключено знову. Everruns не виконує їх сам.",
  "Connecting an endpoint requires permission to change dangerous agent settings.":
    "Для підключення кінцевої точки потрібен дозвіл на зміну небезпечних налаштувань агента.",
};
