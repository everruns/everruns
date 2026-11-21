// es product copy for MCP Apps views from connected MCP servers
// (knowledge/integrations/mcp-apps-host.md).
export const messages: Readonly<Record<string, string>> = {
  "The view could not be loaded": "No se pudo cargar la vista",
  "interactive view from {value0}": "vista interactiva de {value0}",
  "Retry view": "Reintentar vista",
  "{value0} view: {value1}": "Vista de {value0}: {value1}",
  "The interactive view didn't load ({value0}). Showing the text result.":
    "La vista interactiva no se cargó ({value0}). Se muestra el resultado en texto.",
  "Loading view from {value0}…": "Cargando la vista de {value0}…",
  "Let the {value0} view run {value1}?": "¿Permitir que la vista de {value0} ejecute {value1}?",
  "The server does not mark this tool as read-only, so it may change something.":
    "El servidor no marca esta herramienta como de solo lectura, así que puede cambiar algo.",
  "Run tool": "Ejecutar herramienta",
  "Interactive views": "Vistas interactivas",
  "On, with a picture and link in Slack": "Activadas, con imagen y enlace en Slack",
  "On, text only in Slack": "Activadas, solo texto en Slack",
  "Show the charts and screens this server provides under its tool calls, in a sandboxed frame. A view reaches only the sites the server declares; tools that are not read-only ask before they run.":
    "Muestra los gráficos y pantallas de este servidor bajo sus llamadas a herramientas, en un marco aislado. Una vista solo llega a los sitios que declara el servidor; las herramientas que no son de solo lectura preguntan antes de ejecutarse.",
  "the view did not start": "la vista no se inició",
};
