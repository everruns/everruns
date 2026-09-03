/**
 * Decisions:
 * - Keep catalogs in code for now so adding new locales is a small, typed change.
 * - Separate UI locale (`en`/`uk`) from backend locale (`en-US`, `uk-UA`, etc.).
 * - Fall back to English for unsupported browser languages until more locales land.
 */

export type SupportedLocale = "en" | "uk";

const messages = {
  en: {
    thinking: "thinking",
    iteration: "Iteration {value}",
    new_messages: "New messages",
    conversation_turns: "Conversation turns",
    jump_to_turn: "Jump to turn",
    no_messages_yet: "No messages yet",
    start_with_prompt: "Start with a prompt, screenshot, or slash command.",
    loading_older_messages: "Loading older messages...",
    waiting_on_tools: "Waiting on tools",
    scheduled: "Scheduled",
    worked_for: "Worked for {duration}",
    type_message_or_commands: "Type a message or / for commands... (Enter to send)",
    type_message: "Type a message... (Enter to send)",
    reply_to: "Reply to {name}... (Enter to send)",
    attach_images: "Attach images (PNG, JPEG, GIF, WebP)",
    model: "Model",
    model_changed: "Model changed from {from} to {to}",
    recent: "Recent",
    reasoning: "Reasoning",
    verbosity: "Verbosity",
    default: "Default",
    default_with_value: "Default ({value})",
    reset_to_default: "Reset to default",
    cancel_current_turn: "Cancel current turn",
    uploading_images: "Uploading images...",
    running: "running",
    waiting: "waiting",
    details: "details",
    hide_details: "hide details",
    current_directory: "current directory",
    shell: "Shell",
    list_files_in: "List files in {value}",
    read_file: "Read file",
    read_named_file: "Read {value}",
    find_value: "Find {value}",
    search_files: "Search files",
    search_web: "Search web",
    search_web_for: "Search web for {value}",
    write_file: "Write file",
    write_named_file: "Write {value}",
    edit_file: "Edit file",
    edit_named_file: "Edit {value}",
    list_secrets: "List secrets",
    secret_store: "Secret Store",
    list_stored_values: "List stored values",
    key_value_store: "Key Value Store",
    working: "Working",
    exploring: "Exploring {value}",
    read_count: "{count} {label}",
    read_label_one: "read",
    read_label_many: "reads",
    search_label_one: "search",
    search_label_many: "searches",
    write_label_one: "write",
    write_label_many: "writes",
    shell_label_one: "shell",
    shell_label_many: "shells",
    tool_label_one: "tool",
    tool_label_many: "tools",
    collapse_tool_activity: "Collapse tool activity",
    expand_tool_activity: "Expand tool activity",
    collapse_activity_group: "Collapse activity group",
    expand_activity_group: "Expand activity group",
    chat_error_title: "Something went wrong",
    chat_error_description:
      "Try sending your message again. If it keeps happening, share feedback so we can investigate.",
    chat_error_fallback: "Message failed.",
    chat_model_loading: "Checking model availability…",
    chat_model_required: "Choose a model, or configure a default model, before sending.",
    voice_error_description: "Voice could not start. Try again, or check voice provider setup.",
    voice_microphone_permission_description:
      "Check your browser microphone permissions, then try starting voice again.",
    voice_microphone_permission_error:
      "Microphone access is blocked. Allow microphone access in your browser settings and try again.",
    voice_service_unavailable_description:
      "The voice service could not start a realtime call. Check provider configuration, then try again.",
    voice_service_unavailable_error: "Voice service is unavailable.",
    error_prefix: "Error: {value}",
    runtime_error_budget_exhausted_reached:
      "Budget exhausted. {spent} {currency} spent reached the {limit} {currency} limit. Increase the budget to continue.",
    runtime_error_budget_exhausted_exceeded:
      "Budget exhausted. {spent} {currency} spent exceeded the {limit} {currency} limit. Increase the budget to continue.",
    runtime_error_budget_exhausted_generic: "Budget exhausted. Increase the budget to continue.",
    runtime_error_budget_paused_reached:
      "Budget paused. {spent} {currency} spent reached the {soft_limit} {currency} soft limit. Increase or resume the budget to continue.",
    runtime_error_budget_paused_exceeded:
      "Budget paused. {spent} {currency} spent exceeded the {soft_limit} {currency} soft limit. Increase or resume the budget to continue.",
    runtime_error_budget_paused_with_spent:
      "Budget paused with {spent} {currency} spent. Increase or resume the budget to continue.",
    runtime_error_budget_paused_generic:
      "Budget paused. Increase or resume the budget to continue.",
    runtime_error_model_unavailable:
      "The model `{value}` is not available. It may have been removed, renamed, or your API key may not have access to it. Please select a different model.",
    runtime_error_model_unavailable_generic:
      "The selected model is not available. Please select a different model.",
    runtime_error_model_not_configured:
      "No model is configured for this chat. Choose a model or configure a default model, then try again.",
    runtime_error_request_too_large:
      "The conversation has become too long for the model to process. Please start a new session or reduce the context size.",
    runtime_error_provider_rate_limited: "Rate limited by the AI provider. Please wait a moment.",
    runtime_error_provider_rate_limited_retry_after:
      "Rate limited by the AI provider. Please wait {retry_after}s and try again.",
    runtime_error_provider_misconfigured:
      "There is a misconfiguration with the AI provider. Please contact support.",
    runtime_error_provider_quota_exhausted:
      "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue.",
    runtime_error_details_label: "Details",
    runtime_error_provider_unavailable:
      "The AI provider is experiencing issues. Please try again shortly.",
    runtime_error_processing_error:
      "I encountered an error while processing your request. Please try again later.",
    runtime_error_dependency_unavailable:
      "Execution stopped because a required dependency is unavailable.",
    runtime_error_invalid_tool_schema:
      "A connected tool uses an input schema that this model provider does not support. Update the integration or choose a different model provider, then try again.",
    binary_file: "binary file{value}",
    image_count_one: "{count} image",
    image_count_few: "{count} images",
    image_count_many: "{count} images",
    read_tool_title: "Read {value}",
    collapse_file_output: "Collapse file output",
    expand_file_output: "Expand file output",
    write_tool_default: "Write",
    write_tool_edit: "Edit",
    write_tool_replace: "Replace in",
    write_tool_append: "Append to",
    write_tool_file_suffix: "file",
    created: "created",
    updated: "updated",
    diff_truncated: "Diff truncated",
    collapse_write_output: "Collapse write output",
    expand_write_output: "Expand write output",
    edit_count_one: "{count} edit",
    edit_count_few: "{count} edits",
    edit_count_many: "{count} edits",
    line_number: "line {value}",
    no_tasks: "No tasks",
    todos_completed_summary: "{completed} of {total} todos completed",
    write_todos_plan_captured: "Plan captured from write_todos",
    collapse_execution_plan: "Collapse execution plan",
    expand_execution_plan: "Expand execution plan",
    waiting_for_client: "Waiting for client...",
    output: "output",
    show_less: "show less",
    see_more: "> see more...",
    executing: "> ... executing ...",
    exit_code: "exit code {value}",
    message_info: "Message info",
    message_info_id: "ID",
    message_info_model: "Model",
    message_info_reasoning: "Reasoning",
    message_info_phase: "Phase",
    message_info_time: "Time",
    message_info_tokens: "Tokens",
    message_info_tokens_value: "{input} in / {output} out",
    trace_view_message: "View this message's trace on the provider",
    trace_view_turn: "View this turn's trace on the provider",
    trace_view_session: "View session trace on the provider",
    trace_label: "Trace",
    validation_required: "This field is required",
    validation_pattern: "Value does not match the expected format",
    validation_enum: "Choose one of the allowed values",
    validation_type: "Value has the wrong type (expected {value})",
    validation_minimum: "Value must be at least {value}",
    validation_maximum: "Value must be at most {value}",
    validation_min_length: "Must be at least {value} characters",
    validation_max_length: "Must be at most {value} characters",
    validation_min_items: "Add at least {value} items",
    validation_max_items: "Use at most {value} items",
    validation_invalid: "Invalid value",
    memory_mounts: "Mounts",
    memory_mounts_help: "Mount org memory into /workspace for sessions using this capability.",
    memory_load_failed: "Failed to load memory. Try again.",
    memory_none_active: "No active memory found in this org. Create a memory first to add a mount.",
    memory_label: "Memory",
    memory_loading: "Loading...",
    memory_select: "Select a memory",
    memory_search: "Search memory...",
    memory_required: "Memory is required",
    memory_invalid_id: "Invalid memory ID",
    memory_unavailable: "Memory is unavailable or archived",
    mount_path_label: "Mount path",
    mount_mode_label: "Mode",
    mount_mode_readonly: "Read only",
    mount_mode_readwrite: "Read / write",
    mount_remove: "Remove mount",
    mount_add: "Add mount",
    mount_path_must_start: "Path must be /workspace or start with /workspace/",
    mount_path_double_slash: "Path must not contain '//'",
    mount_path_null: "Path must not contain null bytes",
    mount_path_dotdot: "Path must not contain '..'",
    mount_path_trailing_slash: "Path must not end with a trailing slash",
    mount_path_duplicate: "Duplicate mount path '{value}'",
    mount_path_overlap: "Path '{value}' overlaps with '{other}'",
    export_session: "Export session",
    export_jsonl: "Export JSONL",
    export_atif: "Export ATIF",
    atif_export_title: "ATIF export",
    atif_export_failed_title: "ATIF export failed",
    atif_export_too_large: "This session is too large for ATIF export.",
    atif_export_segmented_prompt:
      "This session is too large for a single ATIF file. Download it in parts instead?",
    atif_export_download_parts: "Download in parts",
    atif_export_segmented_limit:
      "ATIF export stopped at the {count}-part safety limit; the session is unexpectedly large.",
    atif_parts_exported_one: "Exported {count} ATIF part",
    atif_parts_exported_few: "Exported {count} ATIF parts",
    atif_parts_exported_many: "Exported {count} ATIF parts",
    atif_export_stopped_one: "ATIF export stopped after {count} part",
    atif_export_stopped_few: "ATIF export stopped after {count} parts",
    atif_export_stopped_many: "ATIF export stopped after {count} parts",
    atif_images_omitted_one: "{count} image was omitted from the ATIF export",
    atif_images_omitted_few: "{count} images were omitted from the ATIF export",
    atif_images_omitted_many: "{count} images were omitted from the ATIF export",
  },
  uk: {
    thinking: "думаю",
    iteration: "Ітерація {value}",
    new_messages: "Нові повідомлення",
    conversation_turns: "Ходи розмови",
    jump_to_turn: "Перейти до ходу",
    no_messages_yet: "Повідомлень ще немає",
    start_with_prompt: "Почніть із запиту, скриншота або slash-команди.",
    loading_older_messages: "Завантажую старіші повідомлення...",
    waiting_on_tools: "Очікування інструментів",
    scheduled: "За розкладом",
    worked_for: "Працював {duration}",
    type_message_or_commands: "Введіть повідомлення або / для команд... (Enter, щоб надіслати)",
    type_message: "Введіть повідомлення... (Enter, щоб надіслати)",
    reply_to: "Відповісти {name}... (Enter, щоб надіслати)",
    attach_images: "Додати зображення (PNG, JPEG, GIF, WebP)",
    model: "Модель",
    model_changed: "Модель змінено з {from} на {to}",
    recent: "Нещодавні",
    reasoning: "Міркування",
    verbosity: "Деталізація",
    default: "За замовчуванням",
    default_with_value: "За замовчуванням ({value})",
    reset_to_default: "Скинути до типового",
    cancel_current_turn: "Скасувати поточний хід",
    uploading_images: "Завантажую зображення...",
    running: "виконується",
    waiting: "очікування",
    details: "деталі",
    hide_details: "сховати деталі",
    current_directory: "поточній директорії",
    shell: "Командний рядок",
    list_files_in: "Показати файли у {value}",
    read_file: "Прочитати файл",
    read_named_file: "Прочитати {value}",
    find_value: "Знайти {value}",
    search_files: "Пошук у файлах",
    search_web: "Пошук у вебі",
    search_web_for: "Шукати у вебі: {value}",
    write_file: "Записати файл",
    write_named_file: "Записати {value}",
    edit_file: "Редагувати файл",
    edit_named_file: "Редагувати {value}",
    list_secrets: "Показати секрети",
    secret_store: "Сховище секретів",
    list_stored_values: "Показати збережені значення",
    key_value_store: "Сховище ключів і значень",
    working: "Працюю",
    exploring: "Досліджую: {value}",
    read_count: "{count} {label}",
    read_label_one: "читання",
    read_label_many: "читань",
    search_label_one: "пошук",
    search_label_many: "пошуків",
    write_label_one: "запис",
    write_label_many: "записів",
    shell_label_one: "команда",
    shell_label_many: "команд",
    tool_label_one: "інструмент",
    tool_label_many: "інструментів",
    collapse_tool_activity: "Згорнути активність інструментів",
    expand_tool_activity: "Розгорнути активність інструментів",
    collapse_activity_group: "Згорнути групу активності",
    expand_activity_group: "Розгорнути групу активності",
    chat_error_title: "Щось пішло не так",
    chat_error_description:
      "Спробуйте надіслати повідомлення ще раз. Якщо це повторюється, надішліть відгук, щоб ми могли дослідити проблему.",
    chat_error_fallback: "Повідомлення не вдалося надіслати.",
    chat_model_loading: "Перевіряємо доступність моделі…",
    chat_model_required:
      "Виберіть модель або налаштуйте модель за замовчуванням перед надсиланням.",
    voice_error_description:
      "Не вдалося запустити голос. Спробуйте ще раз або перевірте налаштування голосового провайдера.",
    voice_microphone_permission_description:
      "Перевірте дозволи мікрофона в браузері, а потім спробуйте запустити голос ще раз.",
    voice_microphone_permission_error:
      "Доступ до мікрофона заблоковано. Дозвольте доступ до мікрофона в налаштуваннях браузера й спробуйте ще раз.",
    voice_service_unavailable_description:
      "Голосовий сервіс не зміг запустити realtime-виклик. Перевірте налаштування провайдера й спробуйте ще раз.",
    voice_service_unavailable_error: "Голосовий сервіс недоступний.",
    error_prefix: "Помилка: {value}",
    runtime_error_budget_exhausted_reached:
      "Бюджет вичерпано. Витрачено {spent} {currency}, що досягло ліміту {limit} {currency}. Збільште бюджет, щоб продовжити.",
    runtime_error_budget_exhausted_exceeded:
      "Бюджет вичерпано. Витрачено {spent} {currency}, що перевищило ліміт {limit} {currency}. Збільште бюджет, щоб продовжити.",
    runtime_error_budget_exhausted_generic: "Бюджет вичерпано. Збільште бюджет, щоб продовжити.",
    runtime_error_budget_paused_reached:
      "Бюджет призупинено. Витрачено {spent} {currency}, що досягло м'якого ліміту {soft_limit} {currency}. Збільште бюджет або відновіть його, щоб продовжити.",
    runtime_error_budget_paused_exceeded:
      "Бюджет призупинено. Витрачено {spent} {currency}, що перевищило м'який ліміт {soft_limit} {currency}. Збільште бюджет або відновіть його, щоб продовжити.",
    runtime_error_budget_paused_with_spent:
      "Бюджет призупинено після витрати {spent} {currency}. Збільште бюджет або відновіть його, щоб продовжити.",
    runtime_error_budget_paused_generic:
      "Бюджет призупинено. Збільште бюджет або відновіть його, щоб продовжити.",
    runtime_error_model_unavailable:
      "Модель `{value}` недоступна. Її могли видалити, перейменувати або ваш API-ключ не має до неї доступу. Виберіть іншу модель.",
    runtime_error_model_unavailable_generic: "Вибрана модель недоступна. Виберіть іншу модель.",
    runtime_error_model_not_configured:
      "Для цього чату не налаштовано модель. Виберіть модель або налаштуйте модель за замовчуванням і повторіть спробу.",
    runtime_error_request_too_large:
      "Розмова стала надто довгою для цієї моделі. Почніть нову сесію або зменште розмір контексту.",
    runtime_error_provider_rate_limited:
      "AI-провайдер тимчасово обмежив запити. Зачекайте трохи й спробуйте ще раз.",
    runtime_error_provider_rate_limited_retry_after:
      "AI-провайдер тимчасово обмежив запити. Зачекайте {retry_after} с і спробуйте ще раз.",
    runtime_error_provider_misconfigured:
      "AI-провайдер налаштований некоректно. Зверніться до підтримки.",
    runtime_error_provider_quota_exhausted:
      "На обліковому записі AI-провайдера закінчилися кредити або квота. Поповніть рахунок або збільште ліміти, щоб продовжити.",
    runtime_error_details_label: "Деталі",
    runtime_error_provider_unavailable:
      "AI-провайдер зараз недоступний. Спробуйте ще раз трохи пізніше.",
    runtime_error_processing_error:
      "Під час обробки вашого запиту сталася помилка. Спробуйте ще раз пізніше.",
    runtime_error_dependency_unavailable: "Виконання зупинено, бо потрібна залежність недоступна.",
    runtime_error_invalid_tool_schema:
      "Підключений інструмент використовує схему вводу, яку цей провайдер моделі не підтримує. Оновіть інтеграцію або виберіть іншого провайдера моделі й повторіть спробу.",
    binary_file: "бінарний файл{value}",
    image_count_one: "{count} зображення",
    image_count_few: "{count} зображення",
    image_count_many: "{count} зображень",
    read_tool_title: "Прочитати {value}",
    collapse_file_output: "Згорнути вивід файла",
    expand_file_output: "Розгорнути вивід файла",
    write_tool_default: "Записати",
    write_tool_edit: "Редагувати",
    write_tool_replace: "Замінити у",
    write_tool_append: "Дописати у",
    write_tool_file_suffix: "файл",
    created: "створено",
    updated: "оновлено",
    diff_truncated: "Diff обрізано",
    collapse_write_output: "Згорнути вивід запису",
    expand_write_output: "Розгорнути вивід запису",
    edit_count_one: "{count} зміна",
    edit_count_few: "{count} зміни",
    edit_count_many: "{count} змін",
    line_number: "рядок {value}",
    no_tasks: "Завдань немає",
    todos_completed_summary: "Завершено {completed} із {total} завдань",
    write_todos_plan_captured: "План зафіксовано через write_todos",
    collapse_execution_plan: "Згорнути план виконання",
    expand_execution_plan: "Розгорнути план виконання",
    waiting_for_client: "Очікування клієнта...",
    output: "вивід",
    show_less: "показати менше",
    see_more: "> показати більше...",
    executing: "> ... виконується ...",
    exit_code: "код виходу {value}",
    message_info: "Інформація про повідомлення",
    message_info_id: "ID",
    message_info_model: "Модель",
    message_info_reasoning: "Міркування",
    message_info_phase: "Фаза",
    message_info_time: "Час",
    message_info_tokens: "Токени",
    message_info_tokens_value: "{input} вхід / {output} вихід",
    trace_view_message: "Переглянути трасування цього повідомлення у провайдера",
    trace_view_turn: "Переглянути трасування цього ходу у провайдера",
    trace_view_session: "Переглянути трасування сесії у провайдера",
    trace_label: "Трасування",
    validation_required: "Це поле обов'язкове",
    validation_pattern: "Значення не відповідає очікуваному формату",
    validation_enum: "Виберіть одне з дозволених значень",
    validation_type: "Неправильний тип значення (очікується {value})",
    validation_minimum: "Значення має бути не менше {value}",
    validation_maximum: "Значення має бути не більше {value}",
    validation_min_length: "Має містити щонайменше {value} символів",
    validation_max_length: "Має містити щонайбільше {value} символів",
    validation_min_items: "Додайте щонайменше {value} елементів",
    validation_max_items: "Використовуйте щонайбільше {value} елементів",
    validation_invalid: "Неприпустиме значення",
    memory_mounts: "Монтування",
    memory_mounts_help:
      "Підключайте пам'ять організації у /workspace для сесій із цією можливістю.",
    memory_load_failed: "Не вдалося завантажити пам'ять. Спробуйте ще раз.",
    memory_none_active:
      "В організації немає активної пам'яті. Спочатку створіть пам'ять, щоб додати монтування.",
    memory_label: "Пам'ять",
    memory_loading: "Завантаження...",
    memory_select: "Виберіть пам'ять",
    memory_search: "Пошук пам'яті...",
    memory_required: "Потрібно вибрати пам'ять",
    memory_invalid_id: "Неприпустимий ідентифікатор пам'яті",
    memory_unavailable: "Пам'ять недоступна або архівована",
    mount_path_label: "Шлях монтування",
    mount_mode_label: "Режим",
    mount_mode_readonly: "Лише читання",
    mount_mode_readwrite: "Читання й запис",
    mount_remove: "Видалити монтування",
    mount_add: "Додати монтування",
    mount_path_must_start: "Шлях має бути /workspace або починатися з /workspace/",
    mount_path_double_slash: "Шлях не може містити '//'",
    mount_path_null: "Шлях не може містити нульові байти",
    mount_path_dotdot: "Шлях не може містити '..'",
    mount_path_trailing_slash: "Шлях не може закінчуватися символом '/'",
    mount_path_duplicate: "Повторюваний шлях монтування '{value}'",
    mount_path_overlap: "Шлях '{value}' перетинається з '{other}'",
    export_session: "Експортувати сесію",
    export_jsonl: "Експортувати JSONL",
    export_atif: "Експортувати ATIF",
    atif_export_title: "Експорт ATIF",
    atif_export_failed_title: "Не вдалося експортувати ATIF",
    atif_export_too_large: "Ця сесія завелика для експорту ATIF.",
    atif_export_segmented_prompt:
      "Ця сесія завелика для одного файлу ATIF. Завантажити її частинами?",
    atif_export_download_parts: "Завантажити частинами",
    atif_export_segmented_limit:
      "Експорт ATIF зупинено на межі безпеки в {count} частин; сесія неочікувано велика.",
    atif_parts_exported_one: "Експортовано {count} частину ATIF",
    atif_parts_exported_few: "Експортовано {count} частини ATIF",
    atif_parts_exported_many: "Експортовано {count} частин ATIF",
    atif_export_stopped_one: "Експорт ATIF зупинено після {count} частини",
    atif_export_stopped_few: "Експорт ATIF зупинено після {count} частин",
    atif_export_stopped_many: "Експорт ATIF зупинено після {count} частин",
    atif_images_omitted_one: "{count} зображення пропущено під час експорту ATIF",
    atif_images_omitted_few: "{count} зображення пропущено під час експорту ATIF",
    atif_images_omitted_many: "{count} зображень пропущено під час експорту ATIF",
  },
} as const;

export type MessageKey = keyof typeof messages.en;

export function normalizeBackendLocale(input?: string | null): string {
  const normalized = input?.trim().replace(/_/g, "-");
  return normalized && normalized.length > 0 ? normalized : "en-US";
}

export function detectBrowserLocale(
  navigatorLike?: Pick<Navigator, "language" | "languages">,
): string {
  const candidate =
    navigatorLike?.languages?.find((value) => value && value.trim().length > 0) ??
    navigatorLike?.language;
  return normalizeBackendLocale(candidate);
}

export function getSupportedLocale(locale: string): SupportedLocale {
  return locale.toLowerCase().startsWith("uk") ? "uk" : "en";
}

export function formatMessage(
  locale: SupportedLocale,
  key: MessageKey,
  values?: Record<string, string | number>,
): string {
  const template = messages[locale][key] ?? messages.en[key];
  if (!values) return template;
  return template.replace(/\{(\w+)\}/g, (_, name: string) => String(values[name] ?? `{${name}}`));
}

function getUkrainianPluralForm(count: number): "one" | "few" | "many" {
  const absCount = Math.abs(count);
  const lastTwoDigits = absCount % 100;
  const lastDigit = absCount % 10;

  if (lastTwoDigits >= 11 && lastTwoDigits <= 14) {
    return "many";
  }

  if (lastDigit === 1) {
    return "one";
  }

  if (lastDigit >= 2 && lastDigit <= 4) {
    return "few";
  }

  return "many";
}

export function formatImageCount(locale: SupportedLocale, count: number): string {
  if (locale === "uk") {
    const form = getUkrainianPluralForm(count);
    if (form === "one") {
      return formatMessage(locale, "image_count_one", { count });
    }
    if (form === "few") {
      return formatMessage(locale, "image_count_few", { count });
    }
    return formatMessage(locale, "image_count_many", { count });
  }

  return formatMessage(locale, count === 1 ? "image_count_one" : "image_count_many", { count });
}

export function formatAtifImagesOmitted(locale: SupportedLocale, count: number): string {
  if (locale === "uk") {
    const form = getUkrainianPluralForm(count);
    if (form === "one") {
      return formatMessage(locale, "atif_images_omitted_one", { count });
    }
    if (form === "few") {
      return formatMessage(locale, "atif_images_omitted_few", { count });
    }
    return formatMessage(locale, "atif_images_omitted_many", { count });
  }

  return formatMessage(
    locale,
    count === 1 ? "atif_images_omitted_one" : "atif_images_omitted_many",
    {
      count,
    },
  );
}

export function formatAtifPartsExported(locale: SupportedLocale, count: number): string {
  if (locale === "uk") {
    const form = getUkrainianPluralForm(count);
    if (form === "one") {
      return formatMessage(locale, "atif_parts_exported_one", { count });
    }
    if (form === "few") {
      return formatMessage(locale, "atif_parts_exported_few", { count });
    }
    return formatMessage(locale, "atif_parts_exported_many", { count });
  }

  return formatMessage(
    locale,
    count === 1 ? "atif_parts_exported_one" : "atif_parts_exported_many",
    { count },
  );
}

export function formatAtifExportStopped(locale: SupportedLocale, count: number): string {
  if (locale === "uk") {
    const form = getUkrainianPluralForm(count);
    if (form === "one") {
      return formatMessage(locale, "atif_export_stopped_one", { count });
    }
    if (form === "few") {
      return formatMessage(locale, "atif_export_stopped_few", { count });
    }
    return formatMessage(locale, "atif_export_stopped_many", { count });
  }

  return formatMessage(
    locale,
    count === 1 ? "atif_export_stopped_one" : "atif_export_stopped_many",
    { count },
  );
}

export function formatEditCount(locale: SupportedLocale, count: number): string {
  if (locale === "uk") {
    const form = getUkrainianPluralForm(count);
    if (form === "one") {
      return formatMessage(locale, "edit_count_one", { count });
    }
    if (form === "few") {
      return formatMessage(locale, "edit_count_few", { count });
    }
    return formatMessage(locale, "edit_count_many", { count });
  }

  return formatMessage(locale, count === 1 ? "edit_count_one" : "edit_count_many", { count });
}
