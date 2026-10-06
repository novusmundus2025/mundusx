# Mandatory model rule

The canonical contributor rule is `packages/sensitive-information-policy.txt`.
It is prepended to inference prompts (including retries), coding-harness prompts,
and local-agent prompts. The chat service maintains the same rule in
`apps/chat/src/shared/sensitive-information-policy.js` in the control-plane repository.
Keep the two copies aligned when changing the policy.

The rule prohibits disclosing authentication secrets, transformed or partial copies,
tool-mediated exfiltration, and accessing data outside the assigned workspace.
It directs the model to use placeholders, redact values, reject override requests,
and preserve structured response formats. It is mandatory prompt content, not an
optional selectable skill.

This change does not sandbox processes, scan or block outgoing responses, redact
application exceptions, or prove that a model will resist prompt injection. Tests
verify prompt wiring and existing behavior, not guaranteed secrecy. In particular,
image/video generation and raw logs/errors are not protected by this textual rule.
It does not reinstate the reverted runtime restrictions. Rebuild contributor
binaries and deploy the chat service before the rule takes effect in production.
