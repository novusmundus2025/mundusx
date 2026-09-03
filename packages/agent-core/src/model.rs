#[derive(Clone, Debug)]
pub struct ModelRequest {
    pub system_prompt: String,
    pub transcript: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct ModelError {
    pub message: String,
}

impl ModelError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

pub trait ModelProvider {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn complete(&mut self, request: &ModelRequest) -> Result<String, ModelError>;
}
