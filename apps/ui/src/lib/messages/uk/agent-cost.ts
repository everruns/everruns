export const messages: Readonly<Record<string, string>> = {
  "/ task": "/ завдання",
  "Advanced assumptions": "Додаткові припущення",
  "Agent model": "Модель агента",
  "Allow for variation": "Врахуйте коливання",
  "Answer a question or write a short reply":
    "Відповісти на запитання або написати коротку відповідь",
  "Calculating your estimate…": "Розраховуємо оцінку…",
  "Check the numbers under Advanced assumptions, or choose a task to reset them.":
    "Перевірте числа в розділі «Додаткові припущення» або виберіть завдання, щоб скинути їх.",
  "Choose a model for this agent before estimating its cost.":
    "Виберіть модель для цього агента перед оцінкою вартості.",
  "Choose the kind of work and how often you’ll use this agent.":
    "Виберіть тип роботи та частоту використання агента.",
  "Cost breakdown": "Складові вартості",
  "Deep research": "Глибоке дослідження",
  "Enter a whole number from 0 to 1,000,000.": "Введіть ціле число від 0 до 1 000 000.",
  "Enter a whole number from {min} to {max}.": "Введіть ціле число від {min} до {max}.",
  "Estimate cost": "Оцінити вартість",
  "Estimated AI cost · USD": "Орієнтовна вартість ШІ · USD",
  "Explore a topic and compare several sources": "Дослідити тему та порівняти кілька джерел",
  "Find information and put together an answer": "Знайти інформацію та підготувати відповідь",
  "For simpler or more involved tasks": "Для простіших або складніших завдань",
  "Harness model": "Модель обв’язки",
  "Include follow-up calls after tools.": "Врахуйте повторні виклики після інструментів.",
  "Include reasoning and tool-call arguments.":
    "Врахуйте міркування та аргументи викликів інструментів.",
  "Input · {count} tokens": "Вхід · {count} токенів",
  "Larger tasks may need a different model.": "Для більших завдань може знадобитися інша модель.",
  "Long-context pricing applies to one or more calls.":
    "До одного або кількох викликів застосовується тариф для довгого контексту.",
  "Model calls per task": "Викликів моделі на завдання",
  "Monthly cost": "Вартість на місяць",
  "Monthly cost assumes 30 days of use. Each task starts a new conversation.":
    "Місячна вартість передбачає 30 днів використання. Кожне завдання починає нову розмову.",
  "Multi-step task": "Багатокрокове завдання",
  "New tool results added before the next call.":
    "Нові результати інструментів перед наступним викликом.",
  "One task is one request, such as “research this company.”":
    "Одне завдання — це один запит, наприклад «досліди цю компанію».",
  "Output is capped at the configured agent or model limit in affected scenarios.":
    "У відповідних сценаріях вихід обмежено налаштованим лімітом агента або моделі.",
  "Output tokens per call": "Вихідних токенів на виклик",
  "Output · {count} tokens": "Вихід · {count} токенів",
  "Quick answer": "Швидка відповідь",
  Retry: "Спробувати знову",
  "Starting input tokens": "Початкові вхідні токени",
  "Task assumptions": "Припущення щодо завдання",
  "Tasks per day": "Завдань на день",
  "The task choices fill in these assumptions for you. Adjust them if you know how much work your agent usually does.":
    "Вибір завдання заповнює ці припущення. Уточніть їх, якщо знаєте звичайний обсяг роботи агента.",
  "This task is too large for the selected model. Try a smaller task or adjust Advanced assumptions.":
    "Це завдання завелике для вибраної моделі. Спробуйте менше завдання або змініть «Додаткові припущення».",
  "Tool result tokens per call": "Токенів результатів інструментів на виклик",
  "Using {model}": "Використовується {model}",
  "We can’t estimate this setup yet. Check the model and the agent’s output limit.":
    "Поки не можемо оцінити ці налаштування. Перевірте модель і ліміт виходу агента.",
  "We couldn’t load your estimate. Please try again.":
    "Не вдалося завантажити оцінку. Спробуйте знову.",
  "We don’t have prices for this model yet. Try another model to see an estimate.":
    "Цін для цієї моделі ще немає. Спробуйте іншу модель для оцінки.",
  "What does a task look like?": "Яким буде завдання?",
  "What will this agent cost?": "Скільки коштуватиме цей агент?",
  "Where the cost comes from": "З чого складається вартість",
  "Your message and any starting context.": "Ваше повідомлення та початковий контекст.",
  "{count} tasks per month": "{count} завдань на місяць",
  "{range} for lighter to heavier tasks": "{range} для простіших і складніших завдань",
  "Enter tasks per day below": "Введіть кількість завдань на день нижче",
  "A guide, not a fixed price or spending limit. Includes AI usage only. Paid tools, hosting and other service fees are extra. Trying this calculator is free and won’t run or change your agent.":
    "Це орієнтир, а не фіксована ціна чи ліміт витрат. Враховано лише використання ШІ. Платні інструменти, хостинг та інші послуги оплачуються окремо. Калькулятор безкоштовний і не запускає та не змінює агента.",
  "Includes roughly {count} tokens of instructions, tools and message overhead on every call. Previous outputs and tool results accumulate in the conversation.":
    "Кожен виклик містить приблизно {count} токенів інструкцій, інструментів і службових даних. Попередні відповіді та результати інструментів накопичуються в розмові.",
  "Planning estimate, not a quote or spending limit. Lighter and heavier scenarios use half to twice the model calls, output and tool results. This is a sensitivity range, not a confidence interval or a maximum.":
    "Оцінка для планування, а не цінова пропозиція чи ліміт витрат. Легші та важчі сценарії використовують від половини до подвійної кількості викликів, виходу й результатів інструментів. Це діапазон чутливості, а не довірчий інтервал чи максимум.",
  "Uses catalog token prices and approximate text sizing, with no cache discount. Actual costs depend on the task, retries, reasoning, cache reads/writes and provider pricing. Excludes sandbox time, paid tools, media, extra models or subagents, and platform fees.":
    "Використано каталожні ціни токенів і приблизний розмір тексту без знижки за кеш. Фактична вартість залежить від завдання, повторів, міркувань, читання/запису кешу та цін провайдера. Не враховано час пісочниці, платні інструменти, медіа, додаткові моделі чи субагенти та збори платформи.",
  "Uses the current agent draft. Estimator inputs do not change the agent. Run representative tasks in Playground and compare their recorded usage before setting a budget.":
    "Використано поточну чернетку агента. Дані калькулятора не змінюють агента. Перед визначенням бюджету виконайте типові завдання в Playground і порівняйте записане використання.",
};
