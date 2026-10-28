// Product-owned tracing copy. Keep credentials and authored destination names outside translation.
export const messages: Readonly<Record<string, string>> = {
  Tracing: "Trazado",
  "Add destination": "Añadir destino",
  "Edit tracing destination": "Editar destino de trazado",
  "Add tracing destination": "Añadir destino de trazado",
  Destination: "Destino",
  "Destination name": "Nombre del destino",
  Content: "Contenido",
  Delivery: "Entrega",
  "No tracing destinations": "No hay destinos de trazado",
  "Send agent traces to your Braintrust projects and OTLP collectors.":
    "Envía las trazas de los agentes a tus proyectos de Braintrust y colectores OTLP.",
  "Connect Braintrust or an OpenTelemetry collector, then choose which agents export traces.":
    "Conecta Braintrust o un colector de OpenTelemetry y elige qué agentes exportan trazas.",
  "Loading tracing…": "Cargando trazado…",
  "Could not load tracing": "No se pudo cargar el trazado",
  "Could not save tracing": "No se pudo guardar el trazado",
  "Could not save destination": "No se pudo guardar el destino",
  "Tracing operation failed": "La operación de trazado falló",
  "Organization defaults": "Valores predeterminados de la organización",
  "Agents inherit these destinations unless they customize routing. Personal Chat is excluded.":
    "Los agentes heredan estos destinos, salvo que personalicen el enrutamiento. El chat personal queda excluido.",
  "Platform telemetry is configured separately. Exports are best effort; delivery failures do not stop agents.":
    "La telemetría de la plataforma se configura por separado. La exportación no garantiza la entrega; los fallos de entrega no detienen a los agentes.",
  "Destination created. Choose defaults or assign it from an agent's Tracing settings.":
    "Destino creado. Elige los valores predeterminados o asígnalo desde la configuración de trazado de un agente.",
  "Metadata only": "Solo metadatos",
  "Messages and tools": "Mensajes y herramientas",
  Paused: "En pausa",
  "Delivery failing": "Fallo de entrega",
  "Export accepted": "Exportación aceptada",
  "No exports yet": "Aún no hay exportaciones",
  "Dropped events: {value0}": "Eventos descartados: {value0}",
  Manage: "Gestionar",
  "Edit destination": "Editar destino",
  "Send test trace": "Enviar traza de prueba",
  "Synthetic trace accepted by receiver": "El receptor aceptó la traza sintética",
  "Test export failed; review destination delivery status":
    "La exportación de prueba falló; revisa el estado de entrega del destino",
  "Pause destination": "Pausar destino",
  "Resume destination": "Reanudar destino",
  "Delete destination": "Eliminar destino",
  "Edit defaults": "Editar valores predeterminados",
  "View defaults": "Ver valores predeterminados",
  "Organization health": "Estado de la organización",
  "Remove this destination from all routing policies first. Existing exports at the provider are not deleted.":
    "Primero elimina este destino de todas las políticas de enrutamiento. Las exportaciones existentes en el proveedor no se eliminan.",
  "Credentials stay encrypted and are never exposed to agents or Chat.":
    "Las credenciales permanecen cifradas y nunca se exponen a los agentes ni al chat.",
  Endpoint: "Punto de conexión",
  "HTTPS only. Private collectors must be reachable through the deployment's approved network boundary.":
    "Solo HTTPS. Los colectores privados deben ser accesibles a través del límite de red aprobado del despliegue.",
  "Project ID": "ID del proyecto",
  "Replace authentication": "Reemplazar autenticación",
  "Authentication header": "Cabecera de autenticación",
  "Header value (optional)": "Valor de la cabecera (opcional)",
  "API key": "Clave de API",
  "Include messages and tool content": "Incluir mensajes y contenido de herramientas",
  "Prompts, tool results, and detailed errors will leave Everruns.":
    "Los prompts, los resultados de las herramientas y los errores detallados saldrán de Everruns.",
  "Include reasoning content": "Incluir contenido de razonamiento",
  "Starts with metadata only. Creating a destination does not assign it to agents.":
    "Comienza solo con metadatos. Crear un destino no lo asigna a los agentes.",
  "Create destination": "Crear destino",
  "Organization admins manage trace exports.":
    "Los administradores de la organización gestionan las exportaciones de trazas.",
  Routing: "Enrutamiento",
  "Use organization defaults": "Usar valores predeterminados de la organización",
  "Add to organization defaults": "Añadir a los valores predeterminados de la organización",
  "Use selected destinations": "Usar destinos seleccionados",
  "Turn off external tracing": "Desactivar trazado externo",
  "Selected destinations": "Destinos seleccionados",
  "Connect a destination before selecting defaults.":
    "Conecta un destino antes de seleccionar los valores predeterminados.",
  "Effective destinations": "Destinos efectivos",
  "No external destinations.": "No hay destinos externos.",
  "Destination unavailable": "Destino no disponible",
  Inherited: "Heredado",
  "Agent selection": "Selección del agente",
  "Changes apply to new turns in existing and new sessions. Personal Chat is excluded.":
    "Los cambios se aplican a los nuevos turnos de las sesiones existentes y nuevas. El chat personal queda excluido.",
  "Routing changed elsewhere. Discard this draft and reload before saving.":
    "El enrutamiento cambió en otro lugar. Descarta este borrador y vuelve a cargar antes de guardar.",
  "Manage organization destinations": "Gestionar destinos de la organización",
  "External destinations for this agent’s traces.":
    "Destinos externos para las trazas de este agente.",
  "Tracing changes apply to new turns after Save.":
    "Los cambios de trazado se aplican a los nuevos turnos después de guardar.",
  Destinations: "Destinos",
  "Manage agent trace exports": "Gestionar exportaciones de trazas de agentes",
  "Organization default": "Valor predeterminado de la organización",
  "Agent selections: {value0}": "Selecciones de agentes: {value0}",
  Unassigned: "Sin asignar",
};
