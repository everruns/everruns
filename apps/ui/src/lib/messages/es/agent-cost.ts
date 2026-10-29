export const messages: Readonly<Record<string, string>> = {
  "/ task": "/ tarea",
  "Advanced assumptions": "Supuestos avanzados",
  "Agent model": "Modelo del agente",
  "Allow for variation": "Prever variaciones",
  "Answer a question or write a short reply":
    "Responder una pregunta o redactar una respuesta breve",
  "Calculating your estimate…": "Calculando la estimación…",
  "Check the numbers under Advanced assumptions, or choose a task to reset them.":
    "Revisa los números en Supuestos avanzados o elige una tarea para restablecerlos.",
  "Choose a model for this agent before estimating its cost.":
    "Elige un modelo para este agente antes de estimar su coste.",
  "Choose the kind of work and how often you’ll use this agent.":
    "Elige el tipo de trabajo y con qué frecuencia usarás este agente.",
  "Cost breakdown": "Desglose de costes",
  "Deep research": "Investigación profunda",
  "Enter a whole number from 0 to 1,000,000.": "Introduce un número entero de 0 a 1.000.000.",
  "Enter a whole number from {min} to {max}.": "Introduce un número entero de {min} a {max}.",
  "Estimate cost": "Estimar coste",
  "Estimated AI cost · USD": "Coste estimado de IA · USD",
  "Explore a topic and compare several sources": "Explorar un tema y comparar varias fuentes",
  "Find information and put together an answer": "Buscar información y preparar una respuesta",
  "For simpler or more involved tasks": "Para tareas más sencillas o más complejas",
  "Harness model": "Modelo del arnés",
  "Include follow-up calls after tools.":
    "Incluye las llamadas posteriores al uso de herramientas.",
  "Include reasoning and tool-call arguments.":
    "Incluye el razonamiento y los argumentos de las llamadas a herramientas.",
  "Input · {count} tokens": "Entrada · {count} tokens",
  "Larger tasks may need a different model.":
    "Las tareas más grandes pueden necesitar otro modelo.",
  "Long-context pricing applies to one or more calls.":
    "Se aplican tarifas de contexto largo a una o más llamadas.",
  "Model calls per task": "Llamadas al modelo por tarea",
  "Monthly cost": "Coste mensual",
  "Monthly cost assumes 30 days of use. Each task starts a new conversation.":
    "El coste mensual supone 30 días de uso. Cada tarea inicia una conversación nueva.",
  "Multi-step task": "Tarea de varios pasos",
  "New tool results added before the next call.":
    "Nuevos resultados de herramientas añadidos antes de la siguiente llamada.",
  "One task is one request, such as “research this company.”":
    "Una tarea es una solicitud, como «investiga esta empresa».",
  "Output is capped at the configured agent or model limit in affected scenarios.":
    "La salida se limita al máximo configurado del agente o modelo en los escenarios afectados.",
  "Output tokens per call": "Tokens de salida por llamada",
  "Output · {count} tokens": "Salida · {count} tokens",
  "Quick answer": "Respuesta rápida",
  Retry: "Reintentar",
  "Starting input tokens": "Tokens de entrada iniciales",
  "Task assumptions": "Supuestos de la tarea",
  "Tasks per day": "Tareas por día",
  "The task choices fill in these assumptions for you. Adjust them if you know how much work your agent usually does.":
    "Las opciones de tarea completan estos supuestos. Ajústalos si conoces cuánto trabajo suele realizar tu agente.",
  "This task is too large for the selected model. Try a smaller task or adjust Advanced assumptions.":
    "Esta tarea es demasiado grande para el modelo seleccionado. Prueba una tarea más pequeña o ajusta los Supuestos avanzados.",
  "Tool result tokens per call": "Tokens de resultados de herramientas por llamada",
  "Using {model}": "Usando {model}",
  "We can’t estimate this setup yet. Check the model and the agent’s output limit.":
    "Aún no podemos estimar esta configuración. Revisa el modelo y el límite de salida del agente.",
  "We couldn’t load your estimate. Please try again.":
    "No pudimos cargar la estimación. Inténtalo de nuevo.",
  "We don’t have prices for this model yet. Try another model to see an estimate.":
    "Aún no tenemos precios para este modelo. Prueba otro modelo para ver una estimación.",
  "What does a task look like?": "¿Cómo es una tarea?",
  "What will this agent cost?": "¿Cuánto costará este agente?",
  "Where the cost comes from": "De dónde proviene el coste",
  "Your message and any starting context.": "Tu mensaje y el contexto inicial.",
  "{count} tasks per month": "{count} tareas al mes",
  "{range} for lighter to heavier tasks": "{range} para tareas más sencillas o más complejas",
  "Enter tasks per day below": "Introduce las tareas por día a continuación",
  "A guide, not a fixed price or spending limit. Includes AI usage only. Paid tools, hosting and other service fees are extra. Trying this calculator is free and won’t run or change your agent.":
    "Es una orientación, no un precio fijo ni un límite de gasto. Solo incluye el uso de IA. Las herramientas de pago, el alojamiento y otros servicios se cobran aparte. La calculadora es gratuita y no ejecuta ni modifica tu agente.",
  "Includes roughly {count} tokens of instructions, tools and message overhead on every call. Previous outputs and tool results accumulate in the conversation.":
    "Cada llamada incluye aproximadamente {count} tokens de instrucciones, herramientas y estructura de mensajes. Las salidas anteriores y los resultados de herramientas se acumulan en la conversación.",
  "Planning estimate, not a quote or spending limit. Lighter and heavier scenarios use half to twice the model calls, output and tool results. This is a sensitivity range, not a confidence interval or a maximum.":
    "Estimación para planificar, no un presupuesto ni un límite de gasto. Los escenarios más ligeros y más intensos usan entre la mitad y el doble de llamadas, salida y resultados de herramientas. Es un rango de sensibilidad, no un intervalo de confianza ni un máximo.",
  "Uses catalog token prices and approximate text sizing, with no cache discount. Actual costs depend on the task, retries, reasoning, cache reads/writes and provider pricing. Excludes sandbox time, paid tools, media, extra models or subagents, and platform fees.":
    "Usa precios de catálogo y tamaños de texto aproximados, sin descuento de caché. El coste real depende de la tarea, los reintentos, el razonamiento, las lecturas/escrituras de caché y el proveedor. Excluye tiempo de sandbox, herramientas de pago, medios, modelos adicionales, subagentes y tarifas de plataforma.",
  "Uses the current agent draft. Estimator inputs do not change the agent. Run representative tasks in Playground and compare their recorded usage before setting a budget.":
    "Usa el borrador actual del agente. Los datos del estimador no modifican el agente. Ejecuta tareas representativas en Playground y compara su uso registrado antes de fijar un presupuesto.",
  "Smart estimate": "Estimación inteligente",
  "Manual calculator": "Calculadora manual",
  "What should we plan for? (optional)": "¿Qué debemos planificar? (opcional)",
  "Infer typical tasks from this agent": "Inferir tareas habituales de este agente",
  "Update estimate": "Actualizar estimación",
  "AI suggests tasks from your agent. Usage sliders update instantly without another AI call.":
    "La IA sugiere tareas según tu agente. Los controles actualizan la proyección al instante sin otra llamada a la IA.",
  "Understanding your agent and estimating typical tasks…":
    "Analizando tu agente y estimando tareas habituales…",
  "Smart estimates are unavailable right now. Retry or use the manual calculator.":
    "Las estimaciones inteligentes no están disponibles. Reintenta o usa la calculadora manual.",
  "Cost projection": "Proyección de costes",
  "Average per task": "Promedio por tarea",
  "Daily cost": "Coste diario",
  "30-day month · Each task starts a new conversation":
    "Mes de 30 días · Cada tarea inicia una conversación nueva",
  "Check task assumptions and select at least one task. Oversized tasks cannot be priced.":
    "Revisa los supuestos y selecciona al menos una tarea. No se pueden calcular tareas demasiado grandes.",
  "Usage projection": "Proyección de uso",
  "By tasks": "Por tareas",
  "By users": "Por usuarios",
  "Active users": "Usuarios activos",
  "Tasks per user per day": "Tareas por usuario al día",
  "Enter a whole number within the displayed limits.":
    "Introduce un número entero dentro de los límites indicados.",
  "Task mix": "Distribución de tareas",
  "Typical tasks for your agent": "Tareas habituales de tu agente",
  "Suggested by AI, not measured usage. Equal mix to start; adjust relative frequency or set a task to zero.":
    "Sugerencias de IA, no uso medido. Mezcla inicial uniforme; ajusta la frecuencia relativa o pon una tarea a cero.",
  Frequency: "Frecuencia",
  "Monthly spend as usage grows": "Gasto mensual al aumentar el uso",
  "Monthly planning range: {low} – {high}": "Rango mensual previsto: {low} – {high}",
  "Largest cost driver: {task}": "Principal factor de coste: {task}",
  "Edit task": "Editar tarea",
  "Task example": "Ejemplo de tarea",
  "Changing the example does not recalculate assumptions. Use the planning input above to generate a new estimate.":
    "Cambiar el ejemplo no recalcula los supuestos. Usa el campo superior para generar otra estimación.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Current catalog prices and model limits apply. Cache discounts are not assumed.":
    "Las instrucciones y herramientas cuentan en cada llamada; las respuestas y resultados anteriores se acumulan. Se aplican los precios del catálogo y límites del modelo, sin descuentos de caché supuestos.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies task complexity, not statistical confidence. AI token costs only: paid tools, hosting, media, other agents and service fees are extra. No tasks are executed and your agent is not changed.":
    "Escenarios de planificación, no presupuestos ni límites. El área sombreada varía la complejidad, no la confianza estadística. Solo costes de tokens de IA: herramientas de pago, alojamiento, medios, otros agentes y tarifas son adicionales. No se ejecutan tareas ni se modifica tu agente.",
  "{label} slider": "Control de {label}",
  "Monthly cost projection with lighter and heavier task scenarios":
    "Proyección mensual para tareas más sencillas y complejas",
  "{count} tasks / day": "{count} tareas / día",
  "At current usage": "Con el uso actual",
  "/ month": "/ mes",
  "Sandbox runtime": "Tiempo de sandbox",
  "Paid tools": "Herramientas de pago",
  "Media generation": "Generación de medios",
  "Subagent work": "Trabajo de subagentes",
  "Hosting and platform fees": "Alojamiento y tarifas de plataforma",
  "Other operating costs": "Otros costes operativos",
  "Cost category": "Categoría de coste",
  "Add cost": "Añadir coste",
  "Paid from credits": "Pagado con créditos",
  "Billed externally": "Facturado externamente",
  "Quantity per month": "Cantidad por mes",
  "Quantity per task": "Cantidad por tarea",
  "USD per {unit}": "USD por {unit}",
  "Enter provider price": "Introduce el precio del proveedor",
  "Estimated operating cost · USD": "Coste operativo estimado · USD",
  "External bills": "Facturas externas",
  "Loading service prices…": "Cargando precios de servicios…",
  "Service prices are unavailable. Reopen the estimator to retry.":
    "Los precios no están disponibles. Vuelve a abrir el estimador.",
  "Prices needed for: {items}": "Se necesitan precios para: {items}",
  "Check resource quantities and prices. Use nonnegative numbers.":
    "Comprueba cantidades y precios. Usa números no negativos.",
  "Largest model cost driver: {task}": "Mayor coste del modelo: {task}",
  "Extra model spend for retries (%)": "Gasto adicional del modelo por reintentos (%)",
  "Cached input reads (%)": "Lecturas de entrada en caché (%)",
  "Cache writes (%)": "Escrituras en caché (%)",
  "Monthly estimate: {amount}": "Estimación mensual: {amount}",
  "Model usage": "Uso del modelo",
  minute: "minuto",
  call: "llamada",
  item: "elemento",
  task: "tarea",
  month: "mes",
  cpu_second: "segundo CPU",
  search: "búsqueda",
  request: "solicitud",
  MB: "MB",
  "Include resources used by an average task. External providers bill separately from prepaid credits.":
    "Incluye los recursos de una tarea media. Los proveedores externos facturan aparte de los créditos prepagados.",
  "Use the full sandbox instance price, including memory. For subagents, enter the cost of one child task. Include idle time and repeated tool work in quantities; monthly fees are added once.":
    "Usa el precio completo del sandbox, incluida la memoria. Para subagentes, introduce el coste de una tarea hija. Incluye tiempo inactivo y trabajo repetido; las cuotas mensuales se suman una sola vez.",
  "Per-task cost excludes fixed monthly fees. Daily cost includes one thirtieth of monthly fees. Only included resources are estimated.":
    "El coste por tarea excluye cuotas mensuales fijas. El coste diario incluye una trigésima parte de estas cuotas. Solo se estiman los recursos incluidos.",
  "Cache shares apply to input tokens across all tasks and must total at most 100%. Retry allowance increases model spend only; repeated tools and sandbox time belong in resource quantities.":
    "Las proporciones de caché se aplican a tokens de entrada de todas las tareas y deben sumar como máximo el 100 %. El margen de reintentos aumenta solo el gasto del modelo; incluye herramientas repetidas y tiempo de sandbox en las cantidades.",
  "Instructions and tools are counted on every call; earlier outputs and tool results accumulate. Catalog prices and model limits apply. Cache savings require explicit assumptions.":
    "Las instrucciones y herramientas cuentan en cada llamada; las salidas y resultados anteriores se acumulan. Se aplican precios del catálogo y límites del modelo. El ahorro por caché requiere supuestos explícitos.",
  "Planning scenarios, not quotes or spending limits. The shaded range varies model workload; resource assumptions stay fixed. Add paid tools, sandbox time, media, subagents and fees to include them. No tasks are executed and your agent is not changed.":
    "Escenarios de planificación, no presupuestos vinculantes ni límites. El área sombreada varía el trabajo del modelo; los recursos permanecen fijos. Añade herramientas de pago, sandbox, medios, subagentes y cuotas para incluirlos. No se ejecutan tareas ni se modifica el agente.",
};
