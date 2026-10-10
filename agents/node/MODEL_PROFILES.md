# Managed model profiles

Contributors still select and download their own model. The agent selects a
generation profile from the actual model identifier at execution time; it does
not replace the model, quantization, tokenizer, chat template, or resource limits.

Qwen3-Coder Instruct variants use temperature 0.7, top-p 0.8, top-k 20, and
repetition penalty 1.05. Other models retain the existing 0.2/0.9 sampling defaults.
Explicit temperature/top-p values are preserved. Native OpenAI tool turns also
preserve explicit top-k and repetition-penalty values received in their payload.

Profiles apply to managed MLX and llama.cpp execution and compatible model-server
requests. llama.cpp uses its repeat_penalty field and --repeat-penalty CLI flag;
MLX and vLLM use repetition_penalty. Unsupported local MLX installations return an
actionable update error rather than silently dropping profile parameters.

Version 0.2.43 changes the agent version and therefore the qualification
fingerprint. The updated configuration is requalified using the existing policy.
This first version has built-in profiles; contributor-defined profile files and
additional generation parameters on the legacy /v1/jobs endpoint are not added.
