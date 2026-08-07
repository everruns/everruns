// Plugin subsystem for everruns-core.
//
// This module implements the plugin compiler: loading a plugin directory
// (PluginFileSet), parsing its manifest, and compiling the result into a
// DeclarativeCapabilityDefinition (CompiledPlugin). The compiled definition is
// then registered with `plugin:{name}` as its capability ID and executes
// through the same declarative runtime path.
//
// See knowledge/integrations/plugins.md for the full specification.

pub mod compiler;
pub mod file_set;
pub mod manifest;

pub use compiler::{CompiledPlugin, compile_plugin};
pub use file_set::{
    MAX_PLUGIN_FILE_BYTES, MAX_PLUGIN_FILES, MAX_PLUGIN_TOTAL_BYTES, PluginFileSet,
};
pub use manifest::{McpServersField, PluginAuthor, PluginManifest, StringOrArray};
