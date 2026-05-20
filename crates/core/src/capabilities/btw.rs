// BTW capability
//
// Decision: `/btw` is its own capability so harnesses and agents can opt into
// the side-question command explicitly instead of depending on a generic bucket.

use super::{Capability, CapabilityStatus};
use crate::command::{CommandArg, CommandDescriptor, CommandSource};

pub const BTW_CAPABILITY_ID: &str = "btw";

pub struct BtwCapability;

impl Capability for BtwCapability {
    fn id(&self) -> &str {
        BTW_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "BTW"
    }

    fn description(&self) -> &str {
        "Ephemeral side-question command for the current session."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("message-circle")
    }

    fn category(&self) -> Option<&str> {
        Some("System")
    }

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![CommandDescriptor {
            name: "btw".to_string(),
            description:
                "Ask a side question about the current session without interrupting the main task."
                    .to_string(),
            source: CommandSource::System,
            args: vec![CommandArg {
                name: "question".to_string(),
                description: "The side question to answer.".to_string(),
                required: true,
                suggestions: vec![],
            }],
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_btw_capability_metadata() {
        let cap = BtwCapability;
        assert_eq!(cap.id(), BTW_CAPABILITY_ID);
        assert_eq!(cap.name(), "BTW");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("message-circle"));
        assert_eq!(cap.category(), Some("System"));
    }

    #[test]
    fn test_btw_capability_registers_command() {
        let cap = BtwCapability;
        let commands = cap.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "btw");
        assert_eq!(commands[0].source, CommandSource::System);
        assert_eq!(commands[0].args.len(), 1);
        assert_eq!(commands[0].args[0].name, "question");
        assert!(commands[0].args[0].required);
    }
}
