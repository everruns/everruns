export const messages: Readonly<Record<string, string>> = {
  "Compaction quality monitoring": "Моніторинг якості ущільнення",
  "Automatically check compaction in real sessions. Operational checks use no judge model; sampled continuity checks use the organization model selected below.":
    "Автоматично перевіряйте ущільнення в реальних сесіях. Операційні перевірки не використовують модель-суддю; вибіркові перевірки збереження контексту використовують вибрану нижче модель організації.",
  "Findings stay in this organization. Sampled excerpts are processed by its selected provider. Monitoring is read-only, and findings are retained for 30 days.":
    "Результати залишаються в цій організації. Вибіркові уривки обробляє вибраний нею постачальник. Моніторинг лише читає дані, а результати зберігаються 30 днів.",
  "Enable automatic monitoring": "Увімкнути автоматичний моніторинг",
  "Monitoring model": "Модель моніторингу",
  "Mechanical checks only": "Лише механічні перевірки",
  "Daily monitoring budget (USD)": "Денний бюджет моніторингу (USD)",
  "Continuity sample (%)": "Вибірка збереження контексту (%)",
  "Excluded session tags": "Виключені теги сесій",
  "A zero budget enables mechanical checks only. The daily UTC cap includes conservative retry reservations; uncertain calls keep their allowance for that day. Disabling monitoring stops new assessments, including queued work.":
    "Нульовий бюджет дозволяє лише механічні перевірки. Денний ліміт за UTC враховує консервативні резерви на повторні спроби; виклики з невідомим результатом зберігають резерв до кінця дня. Вимкнення зупиняє нові перевірки, зокрема роботу в черзі.",
  "Monitoring coverage · 7 days": "Покриття моніторингу · 7 днів",
  "Could not load monitoring coverage.": "Не вдалося завантажити покриття моніторингу.",
  "Today: {reserved} reserved allowance · {reported} provider-reported cost":
    "Сьогодні: {reserved} зарезервовано · {reported} вартість за даними постачальника",
  "No compaction assessments yet.": "Перевірок ущільнення ще немає.",
  "Continuity checks": "Перевірки збереження контексту",
  "Operational checks": "Операційні перевірки",
  "Passed {passed} · Failed {failed} · Insufficient evidence {unknown} · Pending {pending}":
    "Пройдено {passed} · Не пройдено {failed} · Недостатньо доказів {unknown} · Очікується {pending}",
  "Recent findings": "Останні результати",
  "An operational pass confirms the reported execution step. Continuity verdicts apply to sampled evidence and do not prove complete memory preservation or causality.":
    "Успішна операційна перевірка підтверджує повідомлений крок виконання. Оцінки збереження контексту стосуються вибіркових доказів і не доводять повного збереження пам’яті чи причинного зв’язку.",
  "Could not load monitoring findings.": "Не вдалося завантажити результати моніторингу.",
  "Insufficient evidence": "Недостатньо доказів",
  "Select a monitoring model before enabling paid checks.":
    "Виберіть модель моніторингу перед увімкненням платних перевірок.",
  "Compaction monitoring": "Моніторинг ущільнення",
  "System-managed checks configured in organization settings.":
    "Системні перевірки налаштовуються в параметрах організації.",
  "Agree with assessment": "Погодитися з оцінкою",
  "Disagree with assessment": "Не погодитися з оцінкою",
  "Review recorded": "Відгук збережено",
  "Could not save assessment review.": "Не вдалося зберегти відгук про оцінку.",
};
