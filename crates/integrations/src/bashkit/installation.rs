//! Install the authorized host CLI and Knowledge retrieval builtins.
use super::*;
/// Register the `everruns` command tree, by whichever route this host can serve.
///
/// Two routes, because two hosts differ in what they can see:
///
///   * A host that links its commands in inserts a `CliCommandSourceHandle` and
///     gets the local tree: parsing, help, and dispatch without leaving the
///     process. This is the Framework application's route.
///   * A hosted worker cannot rebuild that tree (`CliRoute` is `&'static`, and
///     the commands live behind the control plane), so it forwards instead: a
///     registered tool that declares a `CliSpelling` receives the rendered
///     command line. This is the server's route.
///
/// Both are gated on something the session already has, never on a config flag:
/// no source and no spelled tool means no builtin, so the shell can never
/// promise a surface it cannot serve, nor one the harness withheld.
pub(super) fn install_cli_tree(builder: BashBuilder, context: &ToolContext) -> BashBuilder {
    let builder = if let Some(service) =
        context.extension::<everruns_contracts::knowledge::KnowledgeRuntimeExt>()
    {
        builder.builtin(
            "knowledge",
            Box::new(knowledge::KnowledgeBuiltin {
                service: service.0.clone(),
                session: context.session_id,
            }),
        )
    } else {
        builder
    };

    let builder = tools_in_shell::install(builder, context);
    if let Some(handle) = context
        .extensions
        .get::<crate::bashkit::cli::CliCommandSourceHandle>()
    {
        // Registered under the source's own root token, not a fixed name: the
        // commands belong to the host, so the word that introduces them does too.
        let builtin = crate::bashkit::cli::EverrunsBuiltin::new(handle.0.clone());
        let root = builtin.root().to_string();
        return builder.builtin(root, Box::new(builtin));
    }

    if let Some(builtin) = crate::bashkit::cli::forwarding_builtin_for(context) {
        let root = builtin.root().to_string();
        return builder.builtin(root, Box::new(builtin));
    }

    builder
}
