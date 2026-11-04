// Product-owned copy. Preserve interpolation tokens and stable identifiers.
export const messages: Readonly<Record<string, string>> = {
  "(archived)": "(archivado)",
  about: "acerca de",
  "Archive harness": "Archivo entorno de ejecución",
  "Back to harnesses": "Volver a entornos de ejecución",
  "Back to Harnesses": "Volver a entornos de ejecución",
  "Base instructions for the AI model (supports Markdown). Leave empty to contribute no base prompt; the parent harness, agent, session, and capabilities still apply.":
    "Instrucciones básicas para el modelo AI (compatible con Markdown). Déjelo vacío para no contribuir con ningún mensaje base; el entorno de ejecución principal, el agente, la sesión y las capacidades aún se aplican.",
  "Base instructions for this harness. Leave empty to contribute no prompt.":
    "Instrucciones base para este entorno de ejecución. Déjelo vacío para no contribuir con ningún mensaje.",
  Behavior: "Comportamiento",
  "Create Harness": "Crear entorno de ejecución",
  "Create your first harness": "Crea tu primer entorno de ejecución",
  "Declared capabilities": "Capacidades declaradas",
  "Define a reusable execution baseline for agents and sessions.":
    "Definir una línea base de ejecución reutilizable para agentes y sesiones.",
  "Delete harness": "Eliminar entorno de ejecución",
  "Describe what this harness does...": "Describe qué hace este entorno de ejecución...",
  "Direct parent harness": "Entorno de ejecución principal directo",
  "Example harnesses": "Ejemplo de entornos de ejecución",
  "Example Harnesses": "Ejemplo de entornos de ejecución",
  "Files copied into each new session created from this harness.":
    "Archivos copiados en cada nueva sesión creada desde este entorno de ejecución.",
  "Harness configuration": "configuración del entorno de ejecución",
  "Harness not found": "entorno de ejecución no encontrado",
  "How the harness appears in the runtime catalog.":
    "Cómo aparece el entorno de ejecución en el catálogo de tiempo de ejecución.",
  "How the harness is named and found": "Cómo se nombra y encuentra el entorno de ejecución",
  "Inherit system prompt, capabilities, initial files, and default model from another harness.":
    "Heredar el indicador del sistema, las capacidades, los archivos iniciales y el modelo predeterminado de otro entorno de ejecución.",
  "Inheritance, model, and base instructions": "Herencia, modelo e instrucciones base",
  "Inherits from": "Hereda de",
  "Local capabilities": "Capacidades locales",
  "More details": "Más detalles",
  "My Harness": "Mi entorno de ejecución",
  "my-harness": "mi-entorno de ejecución",
  Network: "Red",
  "New harness": "Nuevo entorno de ejecución",
  "New Harness": "Nuevo entorno de ejecución",
  "No harnesses match your filters.": "Ningún entorno de ejecución coincide con tus filtros.",
  "No harnesses yet": "Aún no hay entornos de ejecución",
  "No local capabilities. Inherited ones show in Preview.":
    "Sin capacidades locales. Los heredados se muestran en Vista previa.",
  "No parent harness": "No hay entorno de ejecución principal",
  "One line in simplified Markdown, shown below the chat title once the intro hides. The agent's value wins.":
    "Una línea en Markdown simplificado, que se muestra debajo del título del chat una vez que se oculta la introducción. El valor del agente gana.",
  "Optional human-readable label shown in the UI. Defaults to name if empty.":
    "Etiqueta opcional legible por humanos que se muestra en la interfaz de usuario. El valor predeterminado es el nombre si está vacío.",
  "Parent harness": "Entorno de ejecución principal",
  "Parent Harness": "Entorno de ejecución principal",
  "Permanently delete the archived harness “{value0}”? Existing references will render as deleted tombstones.":
    "¿Eliminar permanentemente el entorno de ejecución archivado “{value0}”? Las referencias existentes se representarán como lápidas eliminadas.",
  "Press Enter or comma to add a tag. Backspace removes the last tag.":
    "Presione Entrar o coma para agregar una etiqueta. La tecla de retroceso elimina la última etiqueta.",
  "Search harnesses…": "Buscar entornos de ejecución…",
  "Seed starter files into every session created from this harness.":
    "Semilla los archivos iniciales en cada sesión creada a partir de este entorno de ejecución.",
  "Select a specific model or leave empty to use the default":
    "Seleccione un modelo específico o déjelo vacío para usar el predeterminado",
  "Set the baseline network policy. Agents and sessions can only narrow it, never widen it.":
    "Establezca la política de red básica. Los agentes y las sesiones sólo pueden limitarlo, nunca ampliarlo.",
  "Shared runtime configuration: base prompt, capabilities, and policies sessions inherit.":
    "Configuración de tiempo de ejecución compartido: las sesiones de avisos básicos, capacidades y políticas heredan.",
  "Show harness inheritance chain": "Mostrar cadena de herencia del entorno de ejecución",
  "Shown as an intro box on a fresh thread when the bound agent leaves the field empty. Images are allowed. Hidden once the user types.":
    "Se muestra como un cuadro de introducción en un hilo nuevo cuando el agente vinculado deja el campo vacío. Se permiten imágenes. Oculto una vez que el usuario escribe.",
  "Shown in the harness picker. Say when someone should choose this harness.":
    "Se muestra en el selector de entorno de ejecución. Decir cuándo alguien debería elegir este entorno de ejecución.",
  "Starter files": "Archivos de inicio",
  "System prompt": "Aviso del sistema",
  "The complete system prompt including capability additions":
    "El mensaje completo del sistema, incluidas las adiciones de capacidades",
  "This harness is read-only.": "Este entorno de ejecución es de solo lectura.",
  "This harness may have been deleted, moved to another organization, or the URL may be wrong.":
    "Es posible que este entorno de ejecución se haya eliminado, se haya movido a otra organización o que la URL sea incorrecta.",
  "Tools and specialized behaviors": "Herramientas y comportamientos especializados",
  "Tools available from the enabled capabilities":
    "Herramientas disponibles desde las capacidades habilitadas",
  "Triage incidents, dig through logs, draft the update.":
    "Clasificar incidentes, examinar registros, redactar la actualización.",
  "Unique identifier used in URLs and API. Lowercase letters, numbers, and hyphens.":
    "Identificador único utilizado en URL y API. Letras minúsculas, números y guiones.",
  "use this harness. The Stats tab breaks usage down over time.":
    "utilice este entorno de ejecución. La pestaña Estadísticas desglosa el uso a lo largo del tiempo.",
  "When to choose this harness": "Cuándo elegir este entorno de ejecución",
  "Which harness should I use?": "¿Qué entorno de ejecución debo utilizar?",
  "You are a helpful assistant...": "Eres un asistente útil...",
  Unrestricted: "Sin restricciones",
  "{value0} allowed": "{value0} permitido",
  "{value0} blocked": "{value0} bloqueado",
};
