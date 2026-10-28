// Product-owned copy. Preserve interpolation tokens and stable identifiers.
export const messages: Readonly<Record<string, string>> = {
  "Select an agent": "Seleccione un agente",
  "Select at least two runs from the eval’s Runs tab to compare.":
    "Seleccione al menos dos ejecuciones de la pestaña Ejecuciones de la evaluación para comparar.",
  "Select runs to compare them case by case.":
    "Seleccione ejecuciones para compararlas caso por caso.",
  "Service failing - calls are rejected immediately":
    "Fallo del servicio: las llamadas se rechazan inmediatamente",
  "Session tags": "Etiquetas de sesión",
  Share: "Compartir",
  Skipped: "Saltado",
  "Some runs could not be loaded. Check the selected runs and try again.":
    "No se pudieron cargar algunas ejecuciones. Verifique las ejecuciones seleccionadas e inténtelo nuevamente.",
  standalone: "independiente",
  "Start your first run": "Comienza tu primera carrera",
  "Starter templates": "Plantillas de inicio",
  "Starting...": "Comenzando...",
  Statistics: "Estadísticas",
  "Success Count": "Recuento de éxitos",
  Successful: "Exitoso",
  "System Load": "Carga del sistema",
  "Tags (optional)": "Etiquetas (opcional)",
  "Target Input": "Entrada de destino",
  "Target Input (JSON)": "Entrada de destino (JSON)",
  "Target Name": "Nombre del objetivo",
  "Target override:": "Anulación de objetivo:",
  "Target Type": "Tipo de objetivo",
  task: "tarea",
  "Task completions and failures per interval (last 15 min)":
    "Finalizaciones y fallas de tareas por intervalo (últimos 15 minutos)",
  "Task Queue": "Cola de tareas",
  "Tasks that exhausted all retry attempts. Requeue to retry or delete to discard.":
    "Tareas que agotaron todos los reintentos. Volver a poner en cola para volver a intentarlo o eliminar para descartar.",
  "Tasks that have exhausted all retry attempts": "Tareas que han agotado todos los reintentos",
  "Test case name": "Nombre del caso de prueba",
  "Testing recovery - allowing one call to check if service is back":
    "Prueba de recuperación: permite que una llamada verifique si el servicio ha vuelto",
  Text: "Texto",
  "The durable queues API is not available.": "La API de colas duraderas no está disponible.",
  "The durable schedules API is not available.": "La API de horarios duraderos no está disponible.",
  "The durable workers API is not available. Please ensure the backend is running.":
    "La API de trabajadores duraderos no está disponible. Asegúrese de que el backend se esté ejecutando.",
  "The durable workflows API is not available.":
    "La API de flujos de trabajo duraderos no está disponible.",
  "The schedule could not be found or the API is not available.":
    "No se pudo encontrar el cronograma o la API no está disponible.",
  "The task queue is empty.": "La cola de tareas está vacía.",
  "The task queue is empty. Tasks will appear here when workflows schedule activities.":
    "La cola de tareas está vacía. Las tareas aparecerán aquí cuando los flujos de trabajo programen actividades.",
  "The workflow could not be loaded. It may not exist or the API is unavailable.":
    "No se pudo cargar el flujo de trabajo. Puede que no exista o que la API no esté disponible.",
  "This eval may have been deleted, moved to another organization, or the URL may be wrong.":
    "Es posible que esta evaluación se haya eliminado, se haya movido a otra organización o que la URL sea incorrecta.",
  "This eval run may have been deleted, moved to another organization, or the URL may be wrong.":
    "Es posible que esta ejecución de evaluación se haya eliminado, se haya movido a otra organización o que la URL sea incorrecta.",
  "This feature is currently disabled.": "Esta función está actualmente deshabilitada.",
  "This observer may have been deleted, moved to another organization, or the URL may be wrong.":
    "Es posible que este observador haya sido eliminado, trasladado a otra organización o que la URL sea incorrecta.",
  "This schedule hasn't been triggered yet. Click 'Trigger' to run it manually.":
    "Este cronograma aún no se ha activado. Haga clic en 'Activar' para ejecutarlo manualmente.",
  'This will delete the circuit breaker state for "':
    'Esto eliminará el estado del disyuntor para "',
  'This will immediately block all calls through the "':
    'Esto bloqueará inmediatamente todas las llamadas a través del "',
  Throughput: "Rendimiento",
  "Throughput per interval: started, completed, failed (last 15 min)":
    "Rendimiento por intervalo: iniciado, completado, fallido (últimos 15 min)",
  "Timeline of workflow events": "Cronología de eventos del flujo de trabajo",
  "Timeout:": "Tiempo de espera:",
  "to immediately allow calls after a service recovers.":
    "para permitir llamadas inmediatamente después de que se recupere un servicio.",
  "to proactively block calls when you know a service has issues. Use":
    "para bloquear llamadas de forma proactiva cuando sabes que un servicio tiene problemas. uso",
  "Tool calls ({value0})": "Llamadas a herramientas ({value0})",
  "Tool-level scoring": "Puntuación a nivel de herramienta",
  Total: "Total",
  "Total Executions": "Ejecuciones totales",
  "Trends are computed from recent sampled scores; full aggregation is a follow-up.":
    "Las tendencias se calculan a partir de puntuaciones de muestras recientes; la agregación completa es un seguimiento.",
  Trigger: "Activador",
  "Trigger Now": "Activar ahora",
  "Triggered by:": "Activado por:",
  "Unable to Load Queues": "No se pueden cargar colas",
  "Unable to Load Schedules": "No se pueden cargar los horarios",
  "Unable to Load Workers": "No se pueden cargar trabajadores",
  "Unable to Load Workflows": "No se pueden cargar flujos de trabajo",
  "Under backpressure": "Bajo contrapresión",
  Unlimited: "Ilimitado",
  "Update the schedule configuration.": "Actualizar la configuración del horario.",
  Use: "Uso",
  "Use agent default": "Usar agente predeterminado",
  "Use default": "Usar valor predeterminado",
  "Use org default": "Usar el valor predeterminado de la organización",
  "Use org default model": "Usar el modelo predeterminado de la organización",
  "Value (comma-separated for multiple)": "Valor (separado por comas para múltiples)",
  "View schedule {value0}": "Ver horario {value0}",
  "View Session": "Ver sesión",
  "View Workflow": "Ver flujo de trabajo",
  "View workflow {value0}": "Ver flujo de trabajo {value0}",
  "Waiting for results...": "Esperando resultados...",
  "Weight (optional, default 1.0)": "Peso (opcional, predeterminado 1.0)",
  "What this observer watches for...": "Lo que este observador observa...",
  "What this test verifies": "Lo que verifica esta prueba",
  Worker: "Ejecutor",
  "Worker Pool": "Grupo de trabajadores",
  Workflow: "Flujo de trabajo",
  "Workflow Details": "Detalles del flujo de trabajo",
  "Workflow input parameters": "Parámetros de entrada del flujo de trabajo",
  "Workflow Not Found": "Flujo de trabajo no encontrado",
  "workflow-type-name": "nombre-tipo-flujo de trabajo",
  "Yes (max {value0})": "Sí (máx. {value0})",
};
