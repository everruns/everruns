// Ukrainian product copy for customer tracing.
export const messages: Readonly<Record<string, string>> = {
  Tracing: "Трасування",
  "Add destination": "Додати призначення",
  "Edit tracing destination": "Змінити призначення трасування",
  "Add tracing destination": "Додати призначення трасування",
  Destination: "Призначення",
  "Destination name": "Назва призначення",
  Content: "Вміст",
  Delivery: "Доставка",
  "No tracing destinations": "Немає призначень трасування",
  "Send agent traces to your Braintrust projects and OTLP collectors.":
    "Надсилайте траси агентів до проєктів Braintrust і колекторів OTLP.",
  "Connect Braintrust or an OpenTelemetry collector, then choose which agents export traces.":
    "Підключіть Braintrust або колектор OpenTelemetry, а потім виберіть агентів для експорту трас.",
  "Loading tracing…": "Завантаження трасування…",
  "Could not load tracing": "Не вдалося завантажити трасування",
  "Could not save tracing": "Не вдалося зберегти трасування",
  "Could not save destination": "Не вдалося зберегти призначення",
  "Tracing operation failed": "Не вдалося виконати операцію трасування",
  "Organization defaults": "Типові налаштування організації",
  "Agents inherit these destinations unless they customize routing. Personal Chat is excluded.":
    "Агенти успадковують ці призначення, якщо не мають власних правил маршрутизації. Особистий чат виключено.",
  "Platform telemetry is configured separately. Exports are best effort; delivery failures do not stop agents.":
    "Телеметрія платформи налаштовується окремо. Доставка експорту не гарантується; збої доставки не зупиняють агентів.",
  "Destination created. Choose defaults or assign it from an agent's Tracing settings.":
    "Призначення створено. Виберіть типові призначення або призначте його в налаштуваннях трасування агента.",
  "Metadata only": "Лише метадані",
  "Messages and tools": "Повідомлення й інструменти",
  Paused: "Призупинено",
  "Delivery failing": "Помилка доставки",
  "Export accepted": "Експорт прийнято",
  "No exports yet": "Експорту ще не було",
  "Dropped events: {value0}": "Відкинуті події: {value0}",
  Manage: "Керувати",
  "Edit destination": "Змінити призначення",
  "Send test trace": "Надіслати тестову трасу",
  "Synthetic trace accepted by receiver": "Отримувач прийняв синтетичну трасу",
  "Test export failed; review destination delivery status":
    "Тестовий експорт не вдався; перевірте стан доставки призначення",
  "Pause destination": "Призупинити призначення",
  "Resume destination": "Відновити призначення",
  "Delete destination": "Видалити призначення",
  "Edit defaults": "Змінити типові налаштування",
  "View defaults": "Переглянути типові налаштування",
  "Organization health": "Стан організації",
  "Remove this destination from all routing policies first. Existing exports at the provider are not deleted.":
    "Спочатку вилучіть це призначення з усіх правил маршрутизації. Наявні експорти у провайдера не видаляються.",
  "Credentials stay encrypted and are never exposed to agents or Chat.":
    "Облікові дані зберігаються зашифрованими й ніколи не розкриваються агентам або чату.",
  Endpoint: "Кінцева точка",
  "HTTPS only. Private collectors must be reachable through the deployment's approved network boundary.":
    "Лише HTTPS. Приватні колектори мають бути доступні через дозволений мережевий канал розгортання.",
  "Project ID": "Ідентифікатор проєкту",
  "Replace authentication": "Замінити автентифікацію",
  "Authentication header": "Заголовок автентифікації",
  "Header value (optional)": "Значення заголовка (необов’язково)",
  "API key": "Ключ API",
  "Include messages and tool content": "Включати повідомлення й вміст інструментів",
  "Prompts, tool results, and detailed errors will leave Everruns.":
    "Запити, результати інструментів і докладні помилки надсилатимуться за межі Everruns.",
  "Include reasoning content": "Включати вміст міркувань",
  "Starts with metadata only. Creating a destination does not assign it to agents.":
    "Спочатку експортуються лише метадані. Створення призначення не призначає його агентам.",
  "Create destination": "Створити призначення",
  "Organization admins manage trace exports.": "Експортом трас керують адміністратори організації.",
  Routing: "Маршрутизація",
  "Use organization defaults": "Використовувати типові налаштування організації",
  "Add to organization defaults": "Додати до типових налаштувань організації",
  "Use selected destinations": "Використовувати вибрані призначення",
  "Turn off external tracing": "Вимкнути зовнішнє трасування",
  "Selected destinations": "Вибрані призначення",
  "Connect a destination before selecting defaults.":
    "Підключіть призначення, перш ніж вибирати типові налаштування.",
  "Effective destinations": "Поточні призначення",
  "No external destinations.": "Немає зовнішніх призначень.",
  "Destination unavailable": "Призначення недоступне",
  Inherited: "Успадковано",
  "Agent selection": "Вибір агента",
  "Changes apply to new turns in existing and new sessions. Personal Chat is excluded.":
    "Зміни застосовуються до нових ходів у наявних і нових сеансах. Особистий чат виключено.",
  "Routing changed elsewhere. Discard this draft and reload before saving.":
    "Маршрутизацію змінено в іншому місці. Відхиліть цю чернетку й перезавантажте дані перед збереженням.",
  "Manage organization destinations": "Керувати призначеннями організації",
  "External destinations for this agent’s traces.": "Зовнішні призначення для трас цього агента.",
  "Tracing changes apply to new turns after Save.":
    "Після збереження зміни трасування застосовуються до нових ходів.",
  Destinations: "Призначення",
  "Manage agent trace exports": "Керування експортом трас агентів",
  "Organization default": "Типове призначення організації",
  "Agent selections: {value0}": "Вибрано агентами: {value0}",
  Unassigned: "Не призначено",
};
