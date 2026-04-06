//! Embedded Daytona OpenAPI specification for the API calling feature.
//!
//! Decision: Embed a curated subset of the Daytona OpenAPI spec as a const string.
//! This covers the Management API and Toolbox API endpoints that agents can call
//! via `daytona_api_call`. The spec is mounted into the session filesystem at
//! `/daytona/openapi.yaml` when API calling is enabled.

/// Curated Daytona OpenAPI spec covering Management and Toolbox APIs.
///
/// This is a subset focused on the endpoints most useful for agent automation.
/// The full Daytona API docs are at <https://www.daytona.io/docs/api>.
pub const DAYTONA_OPENAPI_SPEC: &str = r##"openapi: 3.0.3
info:
  title: Daytona API
  description: |
    Daytona cloud sandbox Management and Toolbox APIs.
    This spec covers the endpoints available via the `daytona_api_call` tool.

    **Two API tiers:**
    - **Management API** — sandbox lifecycle (create, start, stop, delete, list).
      Paths start with `/sandbox`.
    - **Toolbox API** — in-sandbox operations (exec, files).
      Paths start with `/toolbox/{sandbox_id}/...`.

    Authentication is handled automatically by the tool (Bearer token).
    Do NOT pass auth headers manually.
  version: "1.0"

servers:
  - url: https://app.daytona.io/api
    description: Management API
  - url: https://proxy.app.daytona.io/toolbox
    description: Toolbox API (prefix sandbox_id in path)

paths:
  # ── Management API ────────────────────────────────────────────
  /sandbox:
    get:
      operationId: listSandboxes
      summary: List all sandboxes
      description: Returns all sandboxes owned by the authenticated user.
      responses:
        "200":
          description: Array of sandbox objects
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Sandbox"
    post:
      operationId: createSandbox
      summary: Create a new sandbox
      description: |
        Creates a sandbox from a snapshot. Use `daytona_create_sandbox` tool
        instead for automatic resource tracking and session state management.
        Only use this endpoint directly if you need advanced create options
        not exposed by the tool.
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/CreateSandboxRequest"
      responses:
        "200":
          description: Created sandbox
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Sandbox"

  /sandbox/{sandboxId}:
    get:
      operationId: getSandbox
      summary: Get sandbox details
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Sandbox details
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Sandbox"
    delete:
      operationId: deleteSandbox
      summary: Delete a sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      responses:
        "204":
          description: Sandbox deleted

  /sandbox/{sandboxId}/start:
    post:
      operationId: startSandbox
      summary: Start a stopped sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Sandbox started

  /sandbox/{sandboxId}/stop:
    post:
      operationId: stopSandbox
      summary: Stop a running sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Sandbox stopped

  /sandbox/{sandboxId}/autostop/{interval}:
    post:
      operationId: setAutostop
      summary: Set auto-stop interval
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: interval
          in: path
          required: true
          description: Minutes of inactivity before auto-stop (0 = disable)
          schema:
            type: integer
      responses:
        "200":
          description: Auto-stop updated

  /sandbox/{sandboxId}/labels:
    put:
      operationId: setSandboxLabels
      summary: Set sandbox labels (key-value metadata)
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              additionalProperties:
                type: string
      responses:
        "200":
          description: Labels updated

  # ── Toolbox API (in-sandbox operations) ───────────────────────
  /toolbox/{sandboxId}/process/execute:
    post:
      operationId: executeCommand
      summary: Execute a command synchronously in the sandbox
      description: |
        Runs a shell command and returns the result. For long-running commands
        prefer `daytona_exec` which provides streaming output.
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/ExecRequest"
      responses:
        "200":
          description: Execution result
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/ExecResponse"

  /toolbox/{sandboxId}/files:
    get:
      operationId: listFiles
      summary: List files in a directory
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          description: Directory path to list
          schema:
            type: string
      responses:
        "200":
          description: Array of file entries
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/FileEntry"
    delete:
      operationId: deleteFile
      summary: Delete a file or directory
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          schema:
            type: string
      responses:
        "204":
          description: File deleted

  /toolbox/{sandboxId}/files/download:
    get:
      operationId: downloadFile
      summary: Download a file from the sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Raw file content
          content:
            application/octet-stream:
              schema:
                type: string
                format: binary

  /toolbox/{sandboxId}/files/upload:
    post:
      operationId: uploadFile
      summary: Upload a file to the sandbox
      description: Uses multipart/form-data. Not available via daytona_api_call — use daytona_write_file instead.
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          description: Destination path in sandbox
          schema:
            type: string
      requestBody:
        required: true
        content:
          multipart/form-data:
            schema:
              type: object
              properties:
                file:
                  type: string
                  format: binary
      responses:
        "200":
          description: File uploaded

  /toolbox/{sandboxId}/files/folder:
    post:
      operationId: createFolder
      summary: Create a directory
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          schema:
            type: string
        - name: mode
          in: query
          required: true
          description: Unix permissions (e.g. "0755")
          schema:
            type: string
      responses:
        "200":
          description: Directory created

  /toolbox/{sandboxId}/files/search:
    get:
      operationId: searchFiles
      summary: Search for files by name pattern
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          description: Root directory to search from
          schema:
            type: string
        - name: pattern
          in: query
          required: true
          description: Search pattern (glob)
          schema:
            type: string
      responses:
        "200":
          description: Matching file paths
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/FileEntry"

  /toolbox/{sandboxId}/files/replace:
    post:
      operationId: replaceInFiles
      summary: Find and replace text in files
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/ReplaceRequest"
      responses:
        "200":
          description: Replacement results
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/ReplaceResult"

  /toolbox/{sandboxId}/ports:
    get:
      operationId: listPorts
      summary: List open network ports in sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      responses:
        "200":
          description: Array of open ports
          content:
            application/json:
              schema:
                type: array
                items:
                  type: object
                  properties:
                    port:
                      type: integer
                    url:
                      type: string
                      description: Public URL for the port (if available)

  /toolbox/{sandboxId}/process/session/{sessionId}/exec:
    post:
      operationId: sessionExec
      summary: Execute command in a persistent shell session
      description: |
        Runs a command asynchronously in a persistent session. Used internally
        by `daytona_exec` for streaming output. Prefer `daytona_exec` tool
        for most use cases.
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: sessionId
          in: path
          required: true
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                command:
                  type: string
                async:
                  type: boolean
                  default: true
      responses:
        "200":
          description: Exec started (async) or completed (sync)

  /toolbox/{sandboxId}/git/clone:
    post:
      operationId: gitClone
      summary: Clone a git repository into sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                url:
                  type: string
                  description: Repository URL
                branch:
                  type: string
                  description: Branch to clone
                path:
                  type: string
                  description: Destination path
                username:
                  type: string
                password:
                  type: string
                  description: Token or password
      responses:
        "200":
          description: Clone result

  /toolbox/{sandboxId}/git/status:
    get:
      operationId: gitStatus
      summary: Get git status of a repository in sandbox
      parameters:
        - name: sandboxId
          in: path
          required: true
          schema:
            type: string
        - name: path
          in: query
          required: true
          description: Path to git repository
          schema:
            type: string
      responses:
        "200":
          description: Git status info

components:
  schemas:
    Sandbox:
      type: object
      properties:
        id:
          type: string
        name:
          type: string
        state:
          type: string
          enum: [creating, starting, started, stopping, stopped, destroying, error, unknown]
        snapshot:
          type: string
        labels:
          type: object
          additionalProperties:
            type: string
        autoStopInterval:
          type: integer
          description: Minutes until auto-stop
        autoArchiveInterval:
          type: integer
        autoDeleteInterval:
          type: integer
        resources:
          type: object
          properties:
            cpu:
              type: integer
            memory:
              type: integer
            disk:
              type: integer
        public:
          type: boolean
        target:
          type: string

    CreateSandboxRequest:
      type: object
      properties:
        name:
          type: string
        snapshot:
          type: string
          description: Snapshot name (e.g. "daytona-small", "daytona-medium", "daytona-large")
        autoStopInterval:
          type: integer
          description: Minutes until auto-stop (default 5)
        autoArchiveInterval:
          type: integer
          description: Minutes until auto-archive (default 30)
        autoDeleteInterval:
          type: integer
          description: Minutes until auto-delete (default 60)
        public:
          type: boolean
          default: false
        labels:
          type: object
          additionalProperties:
            type: string
        resources:
          type: object
          properties:
            cpu:
              type: integer
            memory:
              type: integer
            disk:
              type: integer
        env:
          type: object
          additionalProperties:
            type: string
          description: Environment variables to set in sandbox
        user:
          type: string
          description: User to run sandbox as

    ExecRequest:
      type: object
      required:
        - command
      properties:
        command:
          type: string
          description: Shell command to execute
        cwd:
          type: string
          description: Working directory
        timeout:
          type: integer
          description: Timeout in milliseconds

    ExecResponse:
      type: object
      properties:
        code:
          type: integer
          description: Exit code
        result:
          type: string
          description: Combined stdout + stderr output

    FileEntry:
      type: object
      properties:
        name:
          type: string
        isDir:
          type: boolean
        size:
          type: integer
        mode:
          type: string
        modTime:
          type: string
          format: date-time

    ReplaceRequest:
      type: object
      required:
        - files
        - pattern
        - newValue
      properties:
        files:
          type: array
          items:
            type: string
          description: File paths to search
        pattern:
          type: string
          description: Regex pattern to find
        newValue:
          type: string
          description: Replacement string

    ReplaceResult:
      type: object
      properties:
        file:
          type: string
        replacements:
          type: integer

  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
      description: Daytona API key (automatically injected by daytona_api_call)

security:
  - bearerAuth: []
"##;
