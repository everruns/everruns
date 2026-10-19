import wasm from "../pkg/everruns_durable_object_bg.wasm";
import { initSync, session_command, session_command_key } from "../pkg/everruns_durable_object.js";
import { installSessionCommands } from "./session-command.js";
initSync({ module: wasm });
installSessionCommands(session_command,session_command_key);
