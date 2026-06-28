# macOS Device Identity Policy

This policy defines when macOS contributor nodes may use Keychain-backed encrypted identity storage, when local encrypted fallback is acceptable, and what operators should expect from diagnostics.

## Identity Modes

| Mode | Trust path | Description |
| --- | --- | --- |
| Non-exportable OS-backed signing | `secure-enclave` or future non-exportable provider | Target enterprise posture. The private key cannot be exported to the CLI, node-agent, or worker, and signing happens through the OS-backed provider. |
| Keychain encrypted fallback | `keychain` | Current accepted macOS UAT and enterprise-pilot mode. The private key is encrypted at rest in the local identity record, and the decrypting secret is stored in macOS Keychain. |
| Local encrypted fallback | `local-encrypted-fallback` | Local preview and recovery-only mode. The private key remains encrypted at rest, but the decrypting secret is derived locally when Keychain access is unavailable. |
| Legacy file identity | `legacy-file` | Not allowed for macOS. It remains only for non-macOS development paths until Linux protected storage lands. |

## Environment Rules

| Environment | Allowed modes | Required behavior |
| --- | --- | --- |
| Local preview | `keychain`, `local-encrypted-fallback` | Allow startup so contributors can test on constrained local machines. Diagnostics must report the effective trust path. |
| UAT | `keychain`, `local-encrypted-fallback` | Allow both modes, but operators should treat `local-encrypted-fallback` as a visible warning and prefer Keychain-backed nodes for sustained testing. |
| Enterprise | `keychain` today; non-exportable OS-backed signing when implemented | Do not accept plaintext identity. Report `local-encrypted-fallback` as below enterprise target so operators can require re-enrollment or host remediation. |
| Production | `keychain` today only by explicit policy exception; non-exportable OS-backed signing is the target | Production readiness requires an operator decision before accepting fallback storage. Once non-exportable macOS signing exists, production should require it unless a documented break-glass exception is active. |

## Diagnostics And Enforcement

- CLI status and node-agent registration must continue to send `identityTrustPath` so operators can see `keychain` versus `local-encrypted-fallback`.
- macOS nodes must not persist raw `private_key_hex` material.
- `local-encrypted-fallback` is acceptable for local preview and UAT smoke testing, but it should be surfaced as a warning in enterprise or production reviews.
- Future non-exportable macOS signing work should add a distinct trust path instead of overloading `keychain`.
- If Keychain is unavailable on a managed Mac, remediation should be to repair Keychain access or re-enroll the node, not to hide the fallback mode.

## Operator Decision

The current macOS implementation is allowed for UAT and enterprise pilots when `identityTrustPath` reports `keychain`. `local-encrypted-fallback` is deliberately still available so local and UAT smoke flows keep working, but it is not the enterprise target. Production rollout remains gated on either non-exportable macOS signing or an explicit documented exception accepting Keychain encrypted fallback for the deployment.
