// Spanish product copy for background limits (Settings → Limits, schedule limits).
export const messages: Readonly<Record<string, string>> = {
  Limits: "Límites",
  "Limit what schedules and other unattended work may run and spend":
    "Limita lo que las programaciones y otro trabajo desatendido pueden ejecutar y gastar",
  "Unable to load limits.": "No se pudieron cargar los límites.",
  "Unattended work": "Trabajo desatendido",
  "Limits for work no person started: schedules agents create, trigger runs and automatic retries. A person can still extend a schedule from its session.":
    "Límites para el trabajo que ninguna persona inició: programaciones que crean los agentes, ejecuciones de disparadores y reintentos automáticos. Una persona aún puede ampliar una programación desde su sesión.",
  "Longest an agent's schedule may run": "Duración máxima de la programación de un agente",
  "Up to 7 days. The agent must set an end, and it cannot ask for more than this.":
    "Hasta 7 días. El agente debe fijar un final y no puede pedir más que esto.",
  "Most runs per agent schedule": "Máximo de ejecuciones por programación de agente",
  "Up to 96 runs. The schedule stops at this count or at its end time, whichever is first.":
    "Hasta 96 ejecuciones. La programación se detiene en este número o en su hora de fin, lo que ocurra primero.",
  "Default budget per schedule": "Presupuesto predeterminado por programación",
  "What a schedule may spend in total when the agent does not ask for a budget.":
    "Lo que una programación puede gastar en total cuando el agente no pide un presupuesto.",
  "Largest budget an agent may give a schedule":
    "Presupuesto máximo que un agente puede dar a una programación",
  "The most an agent may ask for. A person can raise a single schedule past it.":
    "Lo máximo que un agente puede pedir. Una persona puede subir una programación concreta por encima.",
  "Daily cap on unattended work (schedules, triggers, retries)":
    "Límite diario del trabajo desatendido (programaciones, disparadores, reintentos)",
  "What all unattended work in this organization may spend per day, together.":
    "Lo que todo el trabajo desatendido de esta organización puede gastar al día, en conjunto.",
  "No cap": "Sin límite",
  "Only organization owners and admins can change these limits.":
    "Solo los propietarios y administradores de la organización pueden cambiar estos límites.",
  "Could not save limits.": "No se pudieron guardar los límites.",
  runs: "ejecuciones",
  "Schedule lifetime must be a whole number of days from 1 to 7.":
    "La duración de la programación debe ser un número entero de días entre 1 y 7.",
  "Runs per schedule must be a whole number from 1 to 96.":
    "Las ejecuciones por programación deben ser un número entero entre 1 y 96.",
  "Default budget per schedule must be more than $0.":
    "El presupuesto predeterminado por programación debe ser mayor que 0 $.",
  "Largest schedule budget must be more than $0.":
    "El presupuesto máximo de programación debe ser mayor que 0 $.",
  "Default budget per schedule cannot be more than the largest budget.":
    "El presupuesto predeterminado por programación no puede superar el presupuesto máximo.",
  "Daily cap must be more than $0, or turn on No cap.":
    "El límite diario debe ser mayor que 0 $, o activa Sin límite.",
  "Unattended work (daily)": "Trabajo desatendido (diario)",
  "Budget used up": "Presupuesto agotado",
  "Stopped: repeating": "Detenido: se repite",
  " · Run {value0} of {value1}": " · Ejecución {value0} de {value1}",
  " · Ends {value0}": " · Termina {value0}",
  " · {value0} of {value1}": " · {value0} de {value1}",
  "Stops when: {value0}": "Se detiene cuando: {value0}",
  Extend: "Ampliar",
  "Re-enable": "Volver a activar",
  "Could not re-enable the schedule.": "No se pudo volver a activar la programación.",
  "Extend schedule": "Ampliar programación",
  "Give this schedule more time, runs or budget. You can go past the limits agents are held to.":
    "Da a esta programación más tiempo, ejecuciones o presupuesto. Puedes superar los límites que se aplican a los agentes.",
  "Ends at": "Termina el",
  "Most runs": "Máximo de ejecuciones",
  "Runs so far: {value0}": "Ejecuciones hasta ahora: {value0}",
  "Budget (USD)": "Presupuesto (USD)",
  "Could not extend the schedule.": "No se pudo ampliar la programación.",
  "The end time must be in the future.": "La hora de fin debe estar en el futuro.",
  "Runs must be a whole number above 0.": "Las ejecuciones deben ser un número entero mayor que 0.",
  "The budget must be more than $0.": "El presupuesto debe ser mayor que 0 $.",
  "Change at least one value to extend.": "Cambia al menos un valor para ampliar.",
};
