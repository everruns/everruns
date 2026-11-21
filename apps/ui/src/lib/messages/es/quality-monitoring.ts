export const messages: Readonly<Record<string, string>> = {
  "Compaction quality monitoring": "Monitoreo de calidad de la compactación",
  "Automatically check compaction in real sessions. Operational checks use no judge model; sampled continuity checks use the organization model selected below.":
    "Comprueba automáticamente la compactación en sesiones reales. Las comprobaciones operativas no usan un modelo evaluador; las comprobaciones de continuidad por muestreo usan el modelo de la organización seleccionado abajo.",
  "Findings stay in this organization. Sampled excerpts are processed by its selected provider. Monitoring is read-only, and findings are retained for 30 days.":
    "Los resultados permanecen en esta organización. Su proveedor seleccionado procesa los fragmentos de muestra. El monitoreo es de solo lectura y los resultados se conservan durante 30 días.",
  "Enable automatic monitoring": "Activar monitoreo automático",
  "Monitoring model": "Modelo de monitoreo",
  "Mechanical checks only": "Solo comprobaciones mecánicas",
  "Daily monitoring budget (USD)": "Presupuesto diario de monitoreo (USD)",
  "Continuity sample (%)": "Muestra de continuidad (%)",
  "Excluded session tags": "Etiquetas de sesión excluidas",
  "A zero budget enables mechanical checks only. The daily UTC cap includes conservative retry reservations; uncertain calls keep their allowance for that day. Disabling monitoring stops new assessments, including queued work.":
    "Un presupuesto de cero permite solo comprobaciones mecánicas. El límite diario UTC incluye reservas conservadoras para reintentos; las llamadas inciertas mantienen su reserva durante ese día. Desactivar detiene nuevas evaluaciones, incluido el trabajo en cola.",
  "Monitoring coverage · 7 days": "Cobertura del monitoreo · 7 días",
  "Could not load monitoring coverage.": "No se pudo cargar la cobertura del monitoreo.",
  "Today: {reserved} reserved allowance · {reported} provider-reported cost":
    "Hoy: {reserved} reservados · {reported} de costo informado por el proveedor",
  "No compaction assessments yet.": "Aún no hay evaluaciones de compactación.",
  "Continuity checks": "Comprobaciones de continuidad",
  "Operational checks": "Comprobaciones operativas",
  "Passed {passed} · Failed {failed} · Insufficient evidence {unknown} · Pending {pending}":
    "Aprobadas {passed} · Fallidas {failed} · Evidencia insuficiente {unknown} · Pendientes {pending}",
  "Recent findings": "Resultados recientes",
  "An operational pass confirms the reported execution step. Continuity verdicts apply to sampled evidence and do not prove complete memory preservation or causality.":
    "Una aprobación operativa confirma el paso de ejecución informado. Los veredictos de continuidad se aplican a evidencia de muestra y no prueban la conservación completa de la memoria ni la causalidad.",
  "Could not load monitoring findings.": "No se pudieron cargar los resultados del monitoreo.",
  "Insufficient evidence": "Evidencia insuficiente",
  "Select a monitoring model before enabling paid checks.":
    "Selecciona un modelo de monitoreo antes de activar comprobaciones de pago.",
  "Compaction monitoring": "Monitoreo de compactación",
  "System-managed checks configured in organization settings.":
    "Comprobaciones gestionadas por el sistema y configuradas en los ajustes de la organización.",
  "Agree with assessment": "Aceptar la evaluación",
  "Disagree with assessment": "Discrepar de la evaluación",
  "Review recorded": "Revisión registrada",
  "Could not save assessment review.": "No se pudo guardar la revisión de la evaluación.",
};
