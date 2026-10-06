//! Versioned contribution selection and execution capabilities.
//! Keep the control-plane copy wire-compatible. Selection never grants execution readiness.
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    #[default]
    Llm,
    TextToImage,
    ImageEdit,
    TextToVideo,
    ImageToVideo,
}

impl Operation {
    pub fn is_llm(&self) -> bool {
        *self == Self::Llm
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::TextToImage => "text_to_image",
            Self::ImageEdit => "image_edit",
            Self::TextToVideo => "text_to_video",
            Self::ImageToVideo => "image_to_video",
        }
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Operation {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "llm" => Ok(Self::Llm),
            "image" | "text_to_image" => Ok(Self::TextToImage),
            "image-edit" | "image_edit" => Ok(Self::ImageEdit),
            "video" | "text_to_video" => Ok(Self::TextToVideo),
            "image-to-video" | "image_to_video" => Ok(Self::ImageToVideo),
            _ => Err(format!(
                "unknown workload {value:?}; use llm,image,image-edit,video,image-to-video or all"
            )),
        }
    }
}

pub fn legacy_operations() -> Vec<Operation> {
    vec![Operation::Llm]
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContributionSelection {
    #[serde(default = "legacy_operations")]
    pub operations: Vec<Operation>,
    /// Explicit endpoint only; no credentials, network scanning, or runtime ownership implied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comfyui_url: Option<String>,
}

impl Default for ContributionSelection {
    fn default() -> Self {
        Self {
            operations: legacy_operations(),
            comfyui_url: None,
        }
    }
}

impl ContributionSelection {
    pub fn llm_enabled(&self) -> bool {
        self.operations.contains(&Operation::Llm)
    }
    pub fn media_enabled(&self) -> bool {
        self.operations.iter().any(|operation| !operation.is_llm())
    }
}

/// Positive execution support, distinct from installation preferences and current free slots.
/// Absent on legacy nodes. Unknown contract versions must not grant new capabilities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecutionCapabilities {
    pub contract_version: u32,
    pub operations: Vec<Operation>,
}

impl ExecutionCapabilities {
    pub fn llm_only(enabled: bool) -> Self {
        Self {
            contract_version: 1,
            operations: if enabled { legacy_operations() } else { vec![] },
        }
    }
    pub fn supports(&self, operation: Operation) -> bool {
        self.contract_version == 1 && self.operations.contains(&operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_selection_remains_llm_only() {
        assert_eq!(
            serde_json::from_str::<ContributionSelection>("{}").unwrap(),
            ContributionSelection::default()
        );
    }
    #[test]
    fn explicit_empty_selection_is_not_legacy() {
        let selection: ContributionSelection =
            serde_json::from_str(r#"{"operations":[]}"#).unwrap();
        assert!(!selection.llm_enabled());
    }
    #[test]
    fn future_operations_do_not_silently_become_llm() {
        assert!(serde_json::from_str::<Operation>(r#""future_media""#).is_err());
        assert!(!ExecutionCapabilities {
            contract_version: 99,
            operations: legacy_operations()
        }
        .supports(Operation::Llm));
    }
    #[test]
    fn selection_never_implies_media_execution() {
        assert!(!ExecutionCapabilities::llm_only(true).supports(Operation::TextToImage));
    }
}
