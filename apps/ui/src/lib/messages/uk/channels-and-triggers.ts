// uk product copy for the agent's Channels and triggers tab, Pause agent and the GitHub page.
export const messages: Readonly<Record<string, string>> = {
  "Channels and triggers": "Канали й тригери",
  "How people and software reach this agent, and when it starts on its own.":
    "Як люди та програми звертаються до цього агента і коли він запускається сам.",
  "When this agent starts on its own: schedules, webhooks, GitHub events and MCP server events.":
    "Коли цей агент запускається сам: розклади, вебхуки, події GitHub і події MCP-серверів.",
  "Add a schedule, a webhook or an event to start this agent on its own.":
    "Додайте розклад, вебхук або подію, щоб цей агент запускався сам.",
  "Delete this trigger? It stops starting the agent.":
    "Видалити цей тригер? Він більше не запускатиме агента.",
  "Turn off": "Вимкнути",
  "Turn on": "Увімкнути",
  "Pause agent": "Призупинити агента",
  "Resume agent": "Відновити агента",
  "Pause {value0}? Its channels stop accepting messages and its triggers and schedules stop starting it until you resume it. Sessions already running continue.":
    "Призупинити {value0}? Його канали перестануть приймати повідомлення, а тригери й розклади не запускатимуть його, доки ви його не відновите. Сесії, що вже виконуються, тривають.",
  "This agent is paused": "Цей агент призупинено",
  "Its channels do not accept messages and its triggers and schedules do not start it. Each channel and trigger keeps its own setting for when you resume.":
    "Його канали не приймають повідомлень, а тригери й розклади не запускають його. Кожен канал і тригер зберігає власне налаштування на час відновлення.",
  "Publish opens this channel to callers and Unpublish closes it. Pausing the agent overrides both.":
    "«Опублікувати» відкриває цей канал для викликів, а «Зняти з публікації» закриває його. Призупинення агента має пріоритет над обома.",
  "Add a channel from an agent's Channels and triggers tab. New channels start as drafts and take no traffic until you publish them.":
    "Додайте канал на вкладці «Канали й тригери» агента. Нові канали створюються як чернетки й не приймають трафік, доки ви їх не опублікуєте.",
  "Need another system to start the agent with an HTTP request?":
    "Потрібно, щоб інша система запускала агента HTTP-запитом?",
  "Add a webhook trigger": "Додати тригер вебхука",
  "Trigger type": "Тип тригера",
  Token: "Токен",
  Generate: "Згенерувати",
  "Leave empty to keep the current token": "Залиште порожнім, щоб зберегти поточний токен",
  "Copy it now: it is not shown again. Callers send it as Authorization: Bearer <token> or X-Everruns-Webhook-Token.":
    "Скопіюйте його зараз: його більше не буде показано. Виклики надсилають його як Authorization: Bearer <token> або X-Everruns-Webhook-Token.",
  "Recent runs": "Останні запуски",
  "Loading runs…": "Завантаження запусків…",
  "Starts when {value0} sends {value1}.": "Запускається, коли {value0} надсилає {value1}.",
  "MCP event triggers are created and changed through the API and CLI.":
    "Тригери подій MCP створюються та змінюються через API і CLI.",
  "Who {value0} is on GitHub when it reads code and comments on pull requests.":
    "Ким є {value0} на GitHub, коли читає код і коментує пул-реквести.",
  "Not connected": "Не під’єднано",
  "Not installed": "Не встановлено",
  "Own GitHub App": "Власний GitHub App",
  "Shared Everruns App": "Спільний Everruns App",
  Kind: "Вид",
  trigger: "тригер",
  "The agent's GitHub tools, while the GitHub capability is on":
    "Інструменти GitHub агента, поки ввімкнено можливість GitHub",
  "It may have been deleted, or you may not have access to it.":
    "Можливо, його видалено або у вас немає доступу.",
};
