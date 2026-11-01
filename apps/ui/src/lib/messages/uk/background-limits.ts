// Ukrainian product copy for background limits (Settings → Limits, schedule limits).
export const messages: Readonly<Record<string, string>> = {
  Limits: "Обмеження",
  "Limit what schedules and other unattended work may run and spend":
    "Обмежте, скільки можуть працювати й витрачати розклади та інша робота без нагляду",
  "Unable to load limits.": "Не вдалося завантажити обмеження.",
  "Unattended work": "Робота без нагляду",
  "Limits for work no person started: schedules agents create, trigger runs and automatic retries. A person can still extend a schedule from its session.":
    "Обмеження для роботи, яку не запускала людина: розкладів, створених агентами, запусків тригерів і автоматичних повторів. Людина й надалі може продовжити розклад зі своєї сесії.",
  "Longest an agent's schedule may run": "Найдовший строк дії розкладу агента",
  "Up to 7 days. The agent must set an end, and it cannot ask for more than this.":
    "До 7 днів. Агент мусить задати завершення і не може попросити більше.",
  "Most runs per agent schedule": "Найбільше запусків на розклад агента",
  "Up to 96 runs. The schedule stops at this count or at its end time, whichever is first.":
    "До 96 запусків. Розклад зупиняється на цій кількості або в час завершення, залежно від того, що настане раніше.",
  "Default budget per schedule": "Типовий бюджет на розклад",
  "What a schedule may spend in total when the agent does not ask for a budget.":
    "Скільки загалом може витратити розклад, якщо агент не просить бюджету.",
  "Largest budget an agent may give a schedule": "Найбільший бюджет, який агент може дати розкладу",
  "The most an agent may ask for. A person can raise a single schedule past it.":
    "Найбільше, що може попросити агент. Людина може підняти бюджет окремого розкладу понад це.",
  "Daily cap on unattended work (schedules, triggers, retries)":
    "Денний ліміт на роботу без нагляду (розклади, тригери, повтори)",
  "What all unattended work in this organization may spend per day, together.":
    "Скільки разом може витратити за день уся робота без нагляду в цій організації.",
  "No cap": "Без ліміту",
  "Only organization owners and admins can change these limits.":
    "Лише власники й адміністратори організації можуть змінювати ці обмеження.",
  "Could not save limits.": "Не вдалося зберегти обмеження.",
  runs: "запусків",
  "Schedule lifetime must be a whole number of days from 1 to 7.":
    "Строк дії розкладу має бути цілою кількістю днів від 1 до 7.",
  "Runs per schedule must be a whole number from 1 to 96.":
    "Кількість запусків на розклад має бути цілим числом від 1 до 96.",
  "Default budget per schedule must be more than $0.":
    "Типовий бюджет на розклад має бути більшим за $0.",
  "Largest schedule budget must be more than $0.":
    "Найбільший бюджет розкладу має бути більшим за $0.",
  "Default budget per schedule cannot be more than the largest budget.":
    "Типовий бюджет на розклад не може перевищувати найбільший бюджет.",
  "Daily cap must be more than $0, or turn on No cap.":
    "Денний ліміт має бути більшим за $0, або увімкніть «Без ліміту».",
  "Unattended work (daily)": "Робота без нагляду (за день)",
  "Budget used up": "Бюджет вичерпано",
  " · Run {value0} of {value1}": " · запуск {value0} з {value1}",
  " · Ends {value0}": " · завершується {value0}",
  " · {value0} of {value1}": " · {value0} з {value1}",
  "Stops when: {value0}": "Зупиняється, коли: {value0}",
  Extend: "Продовжити",
  "Re-enable": "Увімкнути знову",
  "Could not re-enable the schedule.": "Не вдалося знову увімкнути розклад.",
  "Extend schedule": "Продовжити розклад",
  "Give this schedule more time, runs or budget. You can go past the limits agents are held to.":
    "Дайте цьому розкладу більше часу, запусків або бюджету. Ви можете вийти за межі обмежень, що діють для агентів.",
  "Ends at": "Завершується",
  "Most runs": "Найбільше запусків",
  "Runs so far: {value0}": "Запусків досі: {value0}",
  "Budget (USD)": "Бюджет (USD)",
  "Could not extend the schedule.": "Не вдалося продовжити розклад.",
  "The end time must be in the future.": "Час завершення має бути в майбутньому.",
  "Runs must be a whole number above 0.": "Кількість запусків має бути цілим числом, більшим за 0.",
  "The budget must be more than $0.": "Бюджет має бути більшим за $0.",
  "Change at least one value to extend.": "Змініть хоча б одне значення, щоб продовжити.",
};
