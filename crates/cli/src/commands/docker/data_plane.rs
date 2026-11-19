//! `--data-plane`: where an instance's new organizations run agents.

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
pub enum DataPlaneChoice {
    /// Cells: the data plane v2 runtime
    V2,
    /// Workers: the original runtime
    V1,
}

impl DataPlaneChoice {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::V2 => "v2",
            Self::V1 => "v1",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "v2" => Some(Self::V2),
            "v1" => Some(Self::V1),
            _ => None,
        }
    }
}

/// The data plane `choice` selects, and how many of its containers run.
///
/// Decision: switching an instance that ran v2 back to v1 keeps the cell
/// running. Its organizations' sessions live there; v1 only means new
/// organizations go to the workers. An instance that never ran v2 runs no
/// data plane containers in v1.
pub(super) fn data_plane_settings(
    choice: Option<DataPlaneChoice>,
    saved: Option<&str>,
    saved_replicas: Option<&str>,
) -> (DataPlaneChoice, &'static str) {
    let saved_choice = saved.and_then(DataPlaneChoice::parse);
    let plane = choice.or(saved_choice).unwrap_or(DataPlaneChoice::V2);
    let ran_v2 = saved_choice == Some(DataPlaneChoice::V2)
        || saved_replicas.is_some_and(|r| r.trim() != "0");
    let replicas = match plane {
        DataPlaneChoice::V2 => "1",
        DataPlaneChoice::V1 if ran_v2 => "1",
        DataPlaneChoice::V1 => "0",
    };
    (plane, replicas)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_plane_defaults_to_v2_and_keeps_a_saved_choice() {
        use DataPlaneChoice::{V1, V2};
        // A new instance, or one from before data plane v2: v2.
        assert_eq!(data_plane_settings(None, None, None), (V2, "1"));
        // A saved choice holds until a flag changes it.
        assert_eq!(data_plane_settings(None, Some("v1"), Some("0")), (V1, "0"));
        assert_eq!(data_plane_settings(None, Some("V2"), Some("1")), (V2, "1"));
        assert_eq!(
            data_plane_settings(Some(V2), Some("v1"), Some("0")),
            (V2, "1")
        );
        // v1 from the start runs no data plane containers.
        assert_eq!(data_plane_settings(Some(V1), None, None), (V1, "0"));
        // An instance that ran v2 keeps its cell: its organizations live there.
        assert_eq!(
            data_plane_settings(Some(V1), Some("v2"), Some("1")),
            (V1, "1")
        );
        assert_eq!(data_plane_settings(None, Some("v1"), Some("1")), (V1, "1"));
        // A value the CLI did not write falls back to the default.
        assert_eq!(data_plane_settings(None, Some("v3"), None), (V2, "1"));
    }
}
