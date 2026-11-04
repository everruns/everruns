export const messages: Readonly<Record<string, string>> = {
  "Changes affect only this estimate.": "Зміни впливають лише на цю оцінку.",
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
  "Smart estimate": "Розумна оцінка",
  "Manual calculator": "Ручний калькулятор",
  "What should we plan for? (optional)": "Що плануємо? (необов’язково)",
  "Infer typical tasks from this agent": "Визначити типові завдання агента",
  "Update estimate": "Оновити оцінку",
  "AI suggests tasks from your agent. Usage sliders update instantly without another AI call.":
    "ШІ пропонує завдання за налаштуваннями агента. Повзунки миттєво оновлюють прогноз без нового виклику ШІ.",
  "Understanding your agent and estimating typical tasks…":
    "Аналізуємо агента й оцінюємо типові завдання…",
  "Smart estimates are unavailable right now. Retry or use the manual calculator.":
    "Розумна оцінка зараз недоступна. Повторіть спробу або скористайтеся ручним калькулятором.",
  "Cost projection": "Прогноз витрат",
  "Average per task": "У середньому за завдання",
  "Daily cost": "Вартість на день",
  "30-day month · Each task starts a new conversation":
    "30 днів на місяць · Кожне завдання починає нову розмову",
  "Check task assumptions and select at least one task. Oversized tasks cannot be priced.":
    "Перевірте припущення й виберіть хоча б одне завдання. Завеликі завдання не можна оцінити.",
  "Usage projection": "Прогноз використання",
  "By tasks": "За завданнями",
  "By users": "За користувачами",
  "Active users": "Активні користувачі",
  "Tasks per user per day": "Завдань на користувача на день",
  "Enter a whole number within the displayed limits.": "Введіть ціле число в зазначених межах.",
  "Task mix": "Розподіл завдань",
  "Typical tasks for your agent": "Типові завдання вашого агента",
  "Suggested by AI, not measured usage. Equal mix to start; adjust relative frequency or set a task to zero.":
    "Пропозиції ШІ, а не виміряне використання. Спочатку рівні частки; змінюйте відносну частоту або встановіть нуль.",
  Frequency: "Частота",
  "Monthly spend as usage grows": "Місячні витрати зі зростанням використання",
  "Monthly planning range: {low} – {high}": "Місячний діапазон: {low} – {high}",
  "Largest cost driver: {task}": "Найбільша стаття витрат: {task}",
  "Edit task": "Редагувати завдання",
  "Task example": "Приклад завдання",
  "Changing the example does not recalculate assumptions. Use the planning input above to generate a new estimate.":
    "Зміна прикладу не перераховує припущення. Введіть побажання вище для нової оцінки.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Current catalog prices and model limits apply. Cache discounts are not assumed.":
    "Інструкції й інструменти враховуються в кожному виклику; попередні відповіді й результати накопичуються. Застосовуються поточні ціни каталогу й ліміти моделі. Знижки за кеш не враховані.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies task complexity, not statistical confidence. AI token costs only: paid tools, hosting, media, other agents and service fees are extra. No tasks are executed and your agent is not changed.":
    "Сценарії планування, не цінові пропозиції чи ліміти витрат. Затінений діапазон змінює складність завдань, а не статистичну впевненість. Лише вартість токенів ШІ: платні інструменти, хостинг, медіа, інші агенти та збори оплачуються окремо. Завдання не виконуються, агент не змінюється.",
  "{label} slider": "Повзунок: {label}",
  "Monthly cost projection with lighter and heavier task scenarios":
    "Прогноз місячних витрат для легших і складніших завдань",
  "{count} tasks / day": "{count} завдань / день",
  "At current usage": "За поточного використання",
  "/ month": "/ місяць",
  "Sandbox runtime": "Час роботи пісочниці",
  "Paid tools": "Платні інструменти",
  "Media generation": "Генерація медіа",
  "Subagent work": "Робота субагентів",
  "Hosting and platform fees": "Хостинг і збори платформи",
  "Other operating costs": "Інші витрати на роботу",
  "Cost category": "Категорія витрат",
  "Add cost": "Додати витрати",
  "Paid from credits": "Оплата з кредитного балансу",
  "Billed externally": "Оплата зовнішньому постачальнику",
  "Quantity per month": "Кількість на місяць",
  "Quantity per task": "Кількість на завдання",
  "USD per {unit}": "USD за {unit}",
  "Enter provider price": "Введіть ціну постачальника",
  "Estimated operating cost · USD": "Орієнтовні витрати на роботу · USD",
  "External bills": "Зовнішні рахунки",
  "Loading service prices…": "Завантаження цін послуг…",
  "Service prices are unavailable. Reopen the estimator to retry.":
    "Ціни послуг недоступні. Відкрийте оцінювач знову, щоб повторити.",
  "Prices needed for: {items}": "Потрібні ціни для: {items}",
  "Check resource quantities and prices. Use nonnegative numbers.":
    "Перевірте кількості ресурсів і ціни. Числа мають бути невід’ємними.",
  "Largest model cost driver: {task}": "Найбільша складова витрат моделі: {task}",
  "Extra model spend for retries (%)": "Додаткові витрати моделі на повтори (%)",
  "Cached input reads (%)": "Читання вхідних токенів з кешу (%)",
  "Cache writes (%)": "Запис у кеш (%)",
  "Monthly estimate: {amount}": "Оцінка на місяць: {amount}",
  "Model usage": "Використання моделі",
  minute: "хвилина",
  call: "виклик",
  item: "елемент",
  task: "завдання",
  month: "місяць",
  cpu_second: "CPU-секунда",
  search: "пошук",
  request: "запит",
  MB: "МБ",
  "Include resources used by an average task. External providers bill separately from prepaid credits.":
    "Додайте ресурси для середнього завдання. Зовнішні постачальники виставляють рахунки окремо від кредитного балансу.",
  "Use the full sandbox instance price, including memory. For subagents, enter the cost of one child task. Include idle time and repeated tool work in quantities; monthly fees are added once.":
    "Вкажіть повну ціну пісочниці, включно з пам’яттю. Для субагентів — ціну одного дочірнього завдання. Врахуйте простій і повторні виклики; місячні збори додаються один раз.",
  "Per-task cost excludes fixed monthly fees. Daily cost includes one thirtieth of monthly fees. Only included resources are estimated.":
    "Вартість завдання не включає фіксовані місячні збори. Денна вартість включає 1/30 місячних зборів. Оцінюються лише додані ресурси.",
  "Cache shares apply to input tokens across all tasks and must total at most 100%. Retry allowance increases model spend only; repeated tools and sandbox time belong in resource quantities.":
    "Частки кешу застосовуються до вхідних токенів усіх завдань і разом не можуть перевищувати 100%. Запас на повтори збільшує лише витрати моделі; повторні інструменти й час пісочниці врахуйте в ресурсах.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Catalog prices and model limits apply. Cache savings require explicit assumptions.":
    "Інструкції та інструменти враховуються в кожному виклику; попередні відповіді й результати накопичуються. Діють ціни каталогу й ліміти моделі. Економія кешу потребує явних припущень.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies model workload; resource assumptions stay fixed. Add paid tools, sandbox time, media, subagents and fees to include them. No tasks are executed and your agent is not changed.":
    "Це сценарії планування, а не цінові пропозиції чи ліміти витрат. Затінений діапазон змінює навантаження моделі; припущення ресурсів незмінні. Додайте платні інструменти, пісочниці, медіа, субагентів і збори. Завдання не виконуються, агент не змінюється.",
};
