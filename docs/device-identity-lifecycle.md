# Device Identity Lifecycle

This page describes the recommended long-term behavior for contributor device identity.

## Goal

The contributor machine should keep a stable identity across:

- app reinstall
- normal OS updates
- machine restarts

The private key should not be exportable to the CLI, agent, or worker.
Those components should only be able to ask the local system to sign a request.

## Recommended Storage Model

- Use non-exportable storage or an encrypted-at-rest sign-only fallback instead of a plain readable file:

- macOS: current implementation tries to store the machine secret in Keychain and falls back to a machine-local encrypted secret when keychain access is unavailable; in both cases the private key stays encrypted-at-rest in the local identity record so the app never reads raw private-key bytes
- Windows: DPAPI-protected device identity storage, with TPM-backed or CNG / KSP-backed non-exportable keys as the future enterprise target
- Linux: Secret Service-backed encrypted identity storage when available, with a visible local encrypted fallback for constrained local/UAT hosts; TPM / PKCS#11 / kernel keyring remains the future stricter enterprise target

The current macOS implementation keeps the private key encrypted-at-rest inside the local identity record and only exposes sign operations to the CLI and agent. The Rust CLI and agent persist public metadata plus the encrypted key blob and nonce, but they never persist raw private-key bytes. The secret used to decrypt the key is stored in the macOS Keychain when possible and otherwise derived locally as a fallback so signing continues to work in constrained environments.

`opengpu status` now reports the active `identityTrustPath` so you can see whether the machine is using `keychain` or `local-encrypted-fallback` on the current Mac.

The macOS environment policy is defined in [docs/macos-identity-policy.md](macos-identity-policy.md). Local preview and UAT may use `keychain` or `local-encrypted-fallback`; enterprise pilots should require `keychain`; production should require non-exportable OS-backed signing when it lands, or an explicit exception for Keychain encrypted fallback.

Windows now stores the Ed25519 private key as a DPAPI-protected blob in the local identity record. The CLI and node-agent keep `private_key_hex` empty, decrypt the protected blob only for local signing, and report `dpapi://mundusx/device-identity` as the trust path. Existing Windows identity files that still contain plaintext `private_key_hex` are rejected with re-enrollment guidance instead of being reused silently.

Linux now stores the Ed25519 private key as an encrypted blob in the local identity record. The CLI and node-agent keep `private_key_hex` empty, use `secret-tool` / Secret Service for the decrypting machine secret when available, and report `secret-service://mundusx/device-identity` as the trust path. Hosts without Secret Service use `local-encrypted-fallback://mundusx/device-identity` so local and UAT smoke flows still work, but operators should treat that path as below enterprise target. Existing Linux identity files that still contain plaintext `private_key_hex` are rejected with re-enrollment guidance instead of being reused silently.

The old plain file-backed prototype remains only for non-macOS, non-Linux, and non-Windows development paths.

## Lifecycle

### First enrollment

1. The CLI or agent creates a device key locally and immediately stores it in encrypted-at-rest form.
2. The app records the public key, fingerprint, hostname, and a stable machine label.
3. The control plane stores the public key, fingerprint, hostname, `identityTrustPath`, and device metadata.
4. The device uses the same encrypted identity record for future signed requests.

### Normal start

1. `opengpu start` or the agent looks for the existing encrypted identity record using the saved machine label.
2. If the encrypted key is present, it is decrypted locally for signing.
3. The control plane sees the same contributor identity.

### App reinstall

1. The app is removed and installed again.
2. The encrypted identity record remains on disk unless the user explicitly removes it.
3. The CLI and agent reuse the same key on next launch.
4. The device keeps the same identity.

### OS update

1. The OS updates.
2. The encrypted identity record remains available.
3. The agent reuses it on next launch.
4. No manual re-enrollment is needed.

### OS reset or device wipe

1. The key may be lost.
2. The agent detects that the key is missing.
3. The node re-enrolls as a new device.
4. The old device identity can be revoked on the control plane.

### Explicit reset

If the contributor wants to retire the device identity, the product should provide an explicit reset/revoke flow.

## What Is Signed

The device key signs:

- register
- heartbeat
- job claim
- job completion

The signed payload includes:

- node ID
- hostname
- public key fingerprint
- public key
- backend
- contribution percent

## What The Hostname Is For

Hostname is signed contributor metadata, not identity.

Use it for:

- auditing
- operator visibility
- spotting changes over time

Do not use it as proof of uniqueness or as a payout target.

## Current Prototype Status

- macOS: the app-visible identity record omits raw private-key bytes, and the secret uses Keychain when available with a local encrypted fallback when it is not.
- Windows: the app-visible identity record omits raw private-key bytes and uses DPAPI-protected key material for CLI and node-agent signing.
- Linux: the app-visible identity record omits raw private-key bytes and uses Secret Service-backed encrypted key material when available, with a visible local encrypted fallback.
- Other platforms: still use the file-backed prototype for development convenience.
- The long-term design is still non-exportable OS-backed storage everywhere.
- macOS fallback policy is explicit: `local-encrypted-fallback` is acceptable for local preview and UAT, visible as below enterprise target, and not the default production posture.

## Open Identity Follow-Ups

- Linux stricter non-exportable signing through TPM, PKCS#11, or kernel keyring remains the future enterprise target beyond the current Secret Service encrypted path.
- macOS non-exportable identity enforcement policy is documented in [docs/macos-identity-policy.md](macos-identity-policy.md).
- Windows DPAPI storage is shipped, while TPM-backed or CNG / KSP-backed non-exportable signing remains the future enterprise target.

## Reinstall Behavior

If the contributor removes and reinstalls the app, the device identity should be reused as long as the persistent MundusX data directory remains intact.

### Expected behavior

- app uninstall alone does not delete the identity
- reinstall reuses the existing encrypted identity record
- the node keeps the same contributor identity after reinstall

### When identity is lost

The device only becomes a new identity if the contributor also deletes the MundusX data directory or explicitly revokes the device.

That keeps the install path predictable:

- reinstall = same identity
- explicit reset = new identity
- app binary alone does not control contributor identity
