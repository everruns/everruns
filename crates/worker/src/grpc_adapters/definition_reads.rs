use super::*;

// These wire values are decoded only while resolving a source read. The phase
// memo stores portable definitions and neutral blockers, never private DTOs.
fn resolve_agent_record(
    record: Option<proto::Agent>,
) -> (
    Result<Option<AgentDefinition>>,
    Option<everruns_core::DependencyBlocker>,
) {
    let blocker = match &record {
        Some(agent) => match agent.status.to_lowercase().as_str() {
            "archived" => Some(everruns_core::DependencyBlocker::AgentArchived),
            "deleted" => Some(everruns_core::DependencyBlocker::AgentDeleted),
            _ => None,
        },
        None => Some(everruns_core::DependencyBlocker::AgentDeleted),
    };
    (record.map(proto_agent_to_definition).transpose(), blocker)
}

impl GrpcOrgAdapter {
    pub(crate) async fn resolve_agent_read(
        &self,
        agent_id: AgentId,
    ) -> Result<(
        Result<Option<AgentDefinition>>,
        Option<everruns_core::DependencyBlocker>,
    )> {
        Ok(resolve_agent_record(
            self.fetch_agent_record(agent_id).await?,
        ))
    }
}

fn resolve_harness_record(
    record: Option<proto::Harness>,
) -> (
    Result<Option<HarnessDefinition>>,
    Option<everruns_core::DependencyBlocker>,
) {
    let blocker = match &record {
        Some(harness) => match harness.status.to_lowercase().as_str() {
            "archived" => Some(everruns_core::DependencyBlocker::HarnessArchived),
            "deleted" => Some(everruns_core::DependencyBlocker::HarnessDeleted),
            _ => None,
        },
        None => Some(everruns_core::DependencyBlocker::HarnessDeleted),
    };
    (record.map(proto_harness_to_definition).transpose(), blocker)
}

impl GrpcOrgAdapter {
    pub(crate) async fn resolve_harness_read(
        &self,
        harness_id: everruns_contracts::typed_id::HarnessId,
    ) -> Result<(
        Result<Option<HarnessDefinition>>,
        Option<everruns_core::DependencyBlocker>,
    )> {
        Ok(resolve_harness_record(
            self.fetch_harness_record(harness_id).await?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_lifecycle_and_projection_keep_independent_error_semantics() {
        for (status, expected) in [
            ("active", None),
            (
                "ArChIvEd",
                Some(everruns_core::DependencyBlocker::AgentArchived),
            ),
            (
                "deleted",
                Some(everruns_core::DependencyBlocker::AgentDeleted),
            ),
        ] {
            let record = proto::Agent {
                id: Some(uuid_to_proto(Uuid::now_v7())),
                status: status.into(),
                ..Default::default()
            };
            let expected_projection = proto_agent_to_definition(record.clone());
            let (projection, blocker) = resolve_agent_record(Some(record));
            assert_eq!(
                blocker.map(everruns_core::DependencyBlocker::message),
                expected.map(everruns_core::DependencyBlocker::message)
            );
            assert_eq!(
                projection.as_ref().err().map(ToString::to_string),
                expected_projection.as_ref().err().map(ToString::to_string)
            );
            assert_eq!(projection.is_ok(), status == "active");
        }
        let (projection, blocker) = resolve_agent_record(None);
        assert!(projection.unwrap().is_none());
        assert!(matches!(
            blocker,
            Some(everruns_core::DependencyBlocker::AgentDeleted)
        ));
        // A successful active lifecycle probe must not swallow wire decoding or
        // capability projection errors, or make the dependency probe fail.
        for record in [
            proto::Agent {
                status: "active".into(),
                ..Default::default()
            },
            proto::Agent {
                id: Some(uuid_to_proto(Uuid::now_v7())),
                status: "active".into(),
                capabilities: vec!["invalid JSON".into()],
                ..Default::default()
            },
        ] {
            let original = proto_agent_to_definition(record.clone()).unwrap_err();
            let (projection, blocker) = resolve_agent_record(Some(record));
            assert_eq!(projection.unwrap_err().to_string(), original.to_string());
            assert!(blocker.is_none());
        }
    }

    #[test]
    fn harness_lifecycle_and_projection_keep_independent_error_semantics() {
        for (status, expected) in [
            ("active", None),
            (
                "ARCHIVED",
                Some(everruns_core::DependencyBlocker::HarnessArchived),
            ),
            (
                "deleted",
                Some(everruns_core::DependencyBlocker::HarnessDeleted),
            ),
        ] {
            let record = proto::Harness {
                id: Some(uuid_to_proto(Uuid::now_v7())),
                status: status.into(),
                ..Default::default()
            };
            let original = proto_harness_to_definition(record.clone());
            let (projection, blocker) = resolve_harness_record(Some(record));
            assert_eq!(
                blocker.map(everruns_core::DependencyBlocker::message),
                expected.map(everruns_core::DependencyBlocker::message)
            );
            assert_eq!(
                projection.as_ref().err().map(ToString::to_string),
                original.as_ref().err().map(ToString::to_string)
            );
            assert_eq!(projection.is_ok(), status == "active");
        }
        let (projection, blocker) = resolve_harness_record(None);
        assert!(projection.unwrap().is_none());
        assert!(matches!(
            blocker,
            Some(everruns_core::DependencyBlocker::HarnessDeleted)
        ));
        for record in [
            proto::Harness {
                status: "active".into(),
                ..Default::default()
            },
            proto::Harness {
                id: Some(uuid_to_proto(Uuid::now_v7())),
                status: "active".into(),
                capabilities: vec!["invalid JSON".into()],
                ..Default::default()
            },
        ] {
            let original = proto_harness_to_definition(record.clone()).unwrap_err();
            let (projection, blocker) = resolve_harness_record(Some(record));
            assert_eq!(projection.unwrap_err().to_string(), original.to_string());
            assert!(blocker.is_none());
        }
    }
}
