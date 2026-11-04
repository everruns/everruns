export const messages: Readonly<Record<string, string>> = {
  "External agent": "Agente externo",
  "Run this agent's turns on your own HTTPS endpoint.":
    "Ejecuta los turnos de este agente en tu propio endpoint HTTPS.",
  "Your endpoint runs this agent's turns.": "Tu endpoint ejecuta los turnos de este agente.",
  "Your code runs each turn with its own system prompt, model and tools. Sessions, Playground, Trace and channels keep working. While an endpoint is connected, the page hides the settings it owns.":
    "Tu código ejecuta cada turno con su propio prompt de sistema, modelo y herramientas. Las sesiones, Playground, Trace y los canales siguen funcionando. Mientras haya un endpoint conectado, la página oculta los ajustes que este gestiona.",
  "Your endpoint runs every turn with its own system prompt, model and tools. Everruns keeps the sessions, channels, traces and usage for this agent.":
    "Tu endpoint ejecuta cada turno con su propio prompt de sistema, modelo y herramientas. Everruns conserva las sesiones, los canales, las trazas y el uso de este agente.",
  "Harness, capabilities, model, MCP servers, credentials, files, network access, sandbox and token limits are hidden: your endpoint owns them.":
    "El harness, las capacidades, el modelo, los servidores MCP, las credenciales, los archivos, el acceso a la red, el sandbox y los límites de tokens están ocultos: los gestiona tu endpoint.",
  "Deploy your endpoint": "Despliega tu endpoint",
  "Any HTTPS service that implements the endpoint protocol.":
    "Cualquier servicio HTTPS que implemente el protocolo del endpoint.",
  "Cloudflare agent guide": "Guía del agente de Cloudflare",
  "Install an endpoint secret": "Instala un secreto del endpoint",
  "Set it in your endpoint's environment before you connect, for example with wrangler secret put EVERRUNS_ENDPOINT_SECRET. At least 32 characters. Everruns stores it encrypted and does not show it again.":
    "Configúralo en el entorno del endpoint antes de conectar, por ejemplo con wrangler secret put EVERRUNS_ENDPOINT_SECRET. Al menos 32 caracteres. Everruns lo guarda cifrado y no vuelve a mostrarlo.",
  "Endpoint URL": "URL del endpoint",
  "Everruns sends a signed challenge to check that the endpoint holds the secret. After you connect, messages and session history for this agent go to the endpoint. Running turns must finish first.":
    "Everruns envía un desafío firmado para comprobar que el endpoint tiene el secreto. Tras conectar, los mensajes y el historial de sesiones de este agente van al endpoint. Los turnos en curso deben terminar primero.",
  "Verify and connect": "Verificar y conectar",
  "Could not connect the endpoint.": "No se pudo conectar el endpoint.",
  "Could not disconnect the endpoint.": "No se pudo desconectar el endpoint.",
  Disconnected: "Desconectado",
  Queue: "Cola",
  "New messages wait until an endpoint is connected again.":
    "Los mensajes nuevos esperan hasta que se vuelva a conectar un endpoint.",
  "The last call failed": "La última llamada falló",
  "Open the session's trace for details.": "Consulta la traza de la sesión para ver los detalles.",
  "Change the endpoint or rotate the secret": "Cambiar el endpoint o renovar el secreto",
  "Install the new secret in your endpoint first, then verify again. The current connection keeps working until the new one is verified.":
    "Instala primero el nuevo secreto en tu endpoint y vuelve a verificar. La conexión actual sigue funcionando hasta que se verifique la nueva.",
  "Update connection": "Actualizar conexión",
  "Disconnect endpoint": "Desconectar endpoint",
  "New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "Los mensajes nuevos esperan en la cola hasta que se vuelva a conectar un endpoint. Everruns no los ejecuta por sí mismo.",
  "Disconnect the endpoint? New messages wait in the queue until an endpoint is connected again. Everruns does not run them itself.":
    "¿Desconectar el endpoint? Los mensajes nuevos esperan en la cola hasta que se vuelva a conectar un endpoint. Everruns no los ejecuta por sí mismo.",
  "Connecting an endpoint requires permission to change dangerous agent settings.":
    "Conectar un endpoint requiere permiso para cambiar los ajustes sensibles del agente.",
};
