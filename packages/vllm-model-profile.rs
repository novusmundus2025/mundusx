//! Model-specific defaults shared by downloading and serving.
pub const MUSE_GLIMMER_MODEL: &str = "meta-models/Muse-Glimmer-30B";
// vllm/vllm-openai:v0.28.0 multi-platform manifest (linux/arm64 and linux/amd64).
pub const MUSE_GLIMMER_IMAGE: &str =
    "vllm/vllm-openai@sha256:61fc8a896b0a4fbbbdc063bc4b0dbc25ce98e02b5050c24aeb7830ac02039b14";

pub fn is_muse_glimmer(model: &str) -> bool {
    model == MUSE_GLIMMER_MODEL
}

pub fn image_for(
    model: &str,
    override_image: Option<String>,
    configured: Option<String>,
) -> Option<String> {
    override_image
        .filter(|image| !image.trim().is_empty())
        .or_else(|| is_muse_glimmer(model).then(|| MUSE_GLIMMER_IMAGE.to_string()))
        .or(configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muse_uses_compatible_image_instead_of_older_installer_default() {
        assert_eq!(
            image_for(MUSE_GLIMMER_MODEL, None, Some("old-nvidia-image".into())).as_deref(),
            Some(MUSE_GLIMMER_IMAGE)
        );
        assert_eq!(
            image_for(MUSE_GLIMMER_MODEL, Some("custom-image".into()), None).as_deref(),
            Some("custom-image")
        );
    }

    #[test]
    fn other_models_preserve_existing_image_selection() {
        assert_eq!(
            image_for(
                "Qwen/Qwen3-Coder-30B-A3B-Instruct",
                None,
                Some("existing-image".into())
            )
            .as_deref(),
            Some("existing-image")
        );
        assert_eq!(
            image_for("Qwen/Qwen3-Coder-30B-A3B-Instruct", None, None),
            None
        );
    }
}
