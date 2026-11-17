// uk product copy for tool approval: Settings → Approvals, My approvals, the agent's
// approval settings, the session mode switch and review marks in chat
// (knowledge/execution/tool-approval.md).
export const messages: Readonly<Record<string, string>> = {
  "A reviewer answers first, asks you when unsure":
    "Спершу відповідає рецензент, а коли не впевнений, питає вас",
  "A reviewer answers routine calls; you get the unclear ones":
    "Рецензент відповідає на рутинні виклики, а неочевидні отримуєте ви",
  Action: "Дія",
  "Add a note": "Додати нотатку",
  "Add rule": "Додати правило",
  "Agent default": "Як в агента",
  "Allow at least one mode.": "Дозвольте хоча б один режим.",
  "Allowed modes": "Дозволені режими",
  "Always ask": "Завжди питати",
  "Applies from the next tool call.": "Діє з наступного виклику інструмента.",
  "Applies to sessions you start. You can still switch one session from its header, within what the organization allows.":
    "Стосується сесій, які ви починаєте. Окрему сесію все одно можна перемкнути в її заголовку, у межах дозволеного організацією.",
  "Approval mode": "Режим погодження",
  "Approval mode: {mode}": "Режим погодження: {mode}",
  "Approvals: {mode}": "Погодження: {mode}",
  "Asked a person": "Запитано людину",
  "Asks before any change": "Питає перед будь-якою зміною",
  "Asks before risky calls": "Питає перед ризикованими викликами",
  "Auto review is paused after several blocked calls. Every call now waits for you until you approve one.":
    "Автоперевірку призупинено після кількох заблокованих викликів. Тепер кожен виклик чекає на вас, доки ви не погодите один.",
  "Auto shows only when the organization allows it.":
    "Авто з'являється лише тоді, коли організація його дозволяє.",
  "Auto-allowed": "Дозволено автоматично",
  "Choose which approval modes agents may use and set rules for every agent":
    "Виберіть, які режими погодження можуть використовувати агенти, і задайте правила для всіх агентів",
  "Could not change the mode. You may not be allowed to manage this session.":
    "Не вдалося змінити режим. Можливо, ви не можете керувати цією сесією.",
  "Could not load recent reviews.": "Не вдалося завантажити останні перевірки.",
  "Could not save.": "Не вдалося зберегти.",
  "Each agent's own mode": "Власний режим кожного агента",
  "How agents in this organization get a go-ahead before they use a tool.":
    "Як агенти цієї організації отримують дозвіл, перш ніж використати інструмент.",
  "How tool calls in your own sessions get approved.":
    "Як погоджуються виклики інструментів у ваших сесіях.",
  "Modes agents and people may pick. A mode you turn off is raised to the next more careful one you allow.":
    "Режими, які можуть вибирати агенти й люди. Вимкнений режим підвищується до наступного обережнішого дозволеного.",
  "My approvals": "Мої погодження",
  "My extra rules": "Мої додаткові правила",
  "Never asks": "Ніколи не питає",
  "No auto reviews yet.": "Автоперевірок ще не було.",
  Normal: "Звичайний",
  "Not available: this agent's manager turned approvals on":
    "Недоступно: менеджер цього агента ввімкнув погодження",
  "Note {number}": "Нотатка {number}",
  "Notes for the reviewer": "Нотатки для рецензента",
  "Only organization owners and admins can change these settings.":
    "Лише власники й адміністратори організації можуть змінювати ці налаштування.",
  "Only the prod-* clusters are production.": "Продакшн лише кластери prod-*.",
  "Personal rules can only add caution: Always ask or Never. They apply to calls agents make on your behalf.":
    "Особисті правила можуть лише додати обережності: Завжди питати або Ніколи. Вони діють для викликів, які агенти роблять від вашого імені.",
  "Plain sentences the auto reviewer reads for every agent: your domains, your repositories, which systems are production.":
    "Прості речення, які автоматичний рецензент читає для кожного агента: ваші домени, ваші репозиторії, які системи є продакшном.",
  "Plain sentences the auto reviewer reads, such as which repositories are yours or what counts as production.":
    "Прості речення, які читає автоматичний рецензент: наприклад, які репозиторії ваші або що вважається продакшном.",
  Protective: "Захисний",
  "Recent reviews": "Останні перевірки",
  "Remove note {number}": "Видалити нотатку {number}",
  "Remove rule for {tool}": "Видалити правило для {tool}",
  Rules: "Правила",
  "Rules apply in every mode; the strictest match wins. A tool name can use * as a wildcard.":
    "Правила діють у кожному режимі; перемагає найсуворіший збіг. У назві інструмента можна використовувати * як шаблон.",
  "Rules for every agent": "Правила для всіх агентів",
  "Start my sessions in": "Починати мої сесії в режимі",
  "The approval mode your sessions start in and your own extra rules":
    "Режим погодження, у якому починаються ваші сесії, і ваші додаткові правила",
  "The last calls the auto reviewer decided on, newest first.":
    "Останні виклики, щодо яких вирішив автоматичний рецензент, від найновіших.",
  "These apply to every agent in the organization, in every mode. When rules disagree, the most cautious one wins.":
    "Вони діють для кожного агента організації в кожному режимі. Коли правила суперечать одне одному, перемагає найобережніше.",
  "Tool name or pattern": "Назва інструмента або шаблон",
  "Trust notes for the reviewer": "Нотатки довіри для рецензента",
  "Unable to load approval settings.": "Не вдалося завантажити налаштування погоджень.",
  Verdict: "Рішення",
  When: "Коли",
  "Why you're asked:": "Чому питають вас:",
  "Your own default, else the agent's mode ({mode})":
    "Ваш типовий режим, інакше режим агента ({mode})",
  "github.com/acme is ours; pushing branches there is routine.":
    "github.com/acme наш; пушити туди гілки — рутина.",
  "{mode} (agent default)": "{mode} (як в агента)",
};
