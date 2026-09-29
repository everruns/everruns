use std::fmt;

use everruns_host::InProcessRuntimeBuilder;

use super::FunctionTool;

#[derive(Clone)]
pub(super) enum CapabilityImplementation {
    Function(FunctionTool),
    #[cfg(feature = "builtins")]
    AskUser(everruns_builtins::AskUserCapability),
    #[cfg(feature = "builtins")]
    Approval(everruns_builtins::ToolApprovalCapability),
    #[cfg(feature = "capabilities")]
    Definition(crate::capability::Definition),
}

impl CapabilityImplementation {
    pub(super) fn register(&self, builder: InProcessRuntimeBuilder) -> InProcessRuntimeBuilder {
        match self {
            Self::Function(tool) => builder.capability(tool.clone().into_capability()),
            #[cfg(feature = "builtins")]
            Self::AskUser(capability) => builder.capability(capability.clone()),
            #[cfg(feature = "builtins")]
            Self::Approval(capability) => builder.capability(capability.clone()),
            #[cfg(feature = "capabilities")]
            Self::Definition(definition) => {
                builder.capability(crate::capability::runtime_adapter(definition))
            }
        }
    }
}

impl fmt::Debug for CapabilityImplementation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Function(tool) => formatter
                .debug_tuple("Function")
                .field(&tool.name())
                .finish(),
            #[cfg(feature = "builtins")]
            Self::AskUser(_) => formatter.write_str("AskUser"),
            #[cfg(feature = "builtins")]
            Self::Approval(_) => formatter.write_str("Approval"),
            #[cfg(feature = "capabilities")]
            Self::Definition(definition) => formatter
                .debug_tuple("Definition")
                .field(&definition.id())
                .finish(),
        }
    }
}
