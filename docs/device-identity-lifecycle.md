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

Use non-exportable storage or an encrypted-at-rest sign-only fallback instead of a plain readable file:

- macOS: current implementation uses a file-encrypted, sign-only fallback so the app never reads raw private-key bytes
- Windows: TPM-backed or CNG / KSP-backed key storage
- Linux: TPM / PKCS#11 / system keyring when available

The current macOS implementation keeps the private key encrypted-at-rest inside the local identity record and only exposes sign operations to the CLI and agent. The Rust CLI and agent persist public metadata plus the encrypted key blob and nonce, but they never persist raw private-key bytes. The machine-derived secret is only used locally to decrypt for signing.

The old plain file-backed prototype remains only for non-macOS development paths.

## Lifecycle

### First enrollment

1. The CLI or agent creates a device key locally and immediately stores it in encrypted-at-rest form.
2. The app records the public key, fingerprint, hostname, and a stable machine label.
3. The control plane stores the public key, fingerprint, hostname, and device metadata.
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

- macOS: file-encrypted sign-only fallback is implemented now, and the app-visible identity record omits raw private-key bytes.
- Other platforms: still use the file-backed prototype for development convenience.
- The long-term design is still non-exportable OS-backed storage everywhere.

## Reinstall Behavior

If the contributor removes and reinstalls the app, the device identity should be reused as long as the persistent OpenGPU data directory remains intact.

### Expected behavior

- app uninstall alone does not delete the identity
- reinstall reuses the existing encrypted identity record
- the node keeps the same contributor identity after reinstall

### When identity is lost

The device only becomes a new identity if the contributor also deletes the OpenGPU data directory or explicitly revokes the device.

That keeps the install path predictable:

- reinstall = same identity
- explicit reset = new identity
- app binary alone does not control contributor identity
