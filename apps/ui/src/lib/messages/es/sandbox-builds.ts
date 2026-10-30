export const messages: Readonly<Record<string, string>> = {
  "Build snapshot": "Crear instantánea",
  "Run an install script once, then reuse the resulting provider snapshot.":
    "Ejecuta un script de instalación una vez y reutiliza la instantánea del proveedor resultante.",
  Provider: "Proveedor",
  "Base snapshot": "Instantánea base",
  "Provider default": "Valor predeterminado del proveedor",
  "Source revision": "Revisión de origen",
  "Select and save a named Organization provider account before building.":
    "Selecciona y guarda una cuenta de proveedor de la organización con nombre antes de crear la instantánea.",
  "Save your configuration changes before building.":
    "Guarda los cambios de configuración antes de crear la instantánea.",
  "Install script": "Script de instalación",
  "Verification and build access": "Verificación y acceso de compilación",
  "Verification command (optional)": "Comando de verificación (opcional)",
  "Metadata file (optional)": "Archivo de metadatos (opcional)",
  "Capture up to 16 KiB of non-secret software notes written by your script.":
    "Captura hasta 16 KiB de notas de software sin secretos escritas por tu script.",
  "Total build timeout (seconds)": "Tiempo máximo total de compilación (segundos)",
  "Build network": "Red de compilación",
  "Loading…": "Cargando…",
  "Build result": "Resultado de la compilación",
  Step: "Paso",
  "Resource cleanup is pending and will be retried.":
    "La limpieza de recursos está pendiente y se volverá a intentar.",
  "Captured software notes": "Notas de software capturadas",
  "Build log": "Registro de compilación",
  "Waiting for output…": "Esperando salida…",
  "Save new revision": "Guardar nueva revisión",
  "Discard snapshot": "Descartar instantánea",
  "The source revision changed. Build again from the current template.":
    "La revisión de origen ha cambiado. Vuelve a compilar desde la plantilla actual.",
  Queued: "En cola",
  Running: "En ejecución",
  Ready: "Lista",
  Failed: "Error",
  Cancelled: "Cancelada",
  Saved: "Guardada",
  Discarded: "Descartada",
  "Starting build compute": "Iniciando el entorno de compilación",
  "Running install script": "Ejecutando el script de instalación",
  "Verifying installation": "Verificando la instalación",
  "Creating snapshot": "Creando instantánea",
  "Verifying fresh snapshot": "Verificando un nuevo entorno desde la instantánea",
  "Cancellation requested": "Cancelación solicitada",
  "Cancel build": "Cancelar compilación",
  "Built snapshot": "Instantánea creada",
  "Sandbox Template sections": "Secciones de la plantilla de sandbox",
  Configuration: "Configuración",
  "Snapshot builds currently support Daytona. Other template targets remain configurable.":
    "La creación de instantáneas admite actualmente Daytona. Los demás destinos de plantilla siguen siendo configurables.",
  "Runs in fresh isolated build compute. Launch bootstrap commands stay separate. Do not put credentials in scripts.":
    "Se ejecuta en un entorno de compilación nuevo y aislado. Los comandos de inicio se mantienen separados. No incluyas credenciales en los scripts.",
  "Runs after installation and again on a fresh sandbox from the snapshot. Only what this command checks is verified.":
    "Se ejecuta después de la instalación y de nuevo en una sandbox nueva desde la instantánea. Solo se verifica lo que comprueba este comando.",
  "Allow package sources during installation without expanding the finished sandbox’s runtime policy.":
    "Permite fuentes de paquetes durante la instalación sin ampliar la política de ejecución de la sandbox final.",
  "Build compute and snapshot storage are billed to the selected provider account. Runtime credentials and private workspace data are not attached.":
    "El cómputo y el almacenamiento de instantáneas se facturan a la cuenta del proveedor seleccionada. No se adjuntan credenciales de ejecución ni datos privados del espacio de trabajo.",
  "Author-supplied reference data. These notes do not grant capabilities or change runtime policy.":
    "Datos de referencia del autor. Estas notas no conceden capacidades ni cambian la política de ejecución.",
  "Discard this unused provider snapshot? Applied template revisions are kept.":
    "¿Descartar esta instantánea del proveedor sin usar? Se conservan las revisiones de plantilla aplicadas.",
  "Older builds": "Compilaciones anteriores",
  "Reuse script": "Reutilizar script",
};
