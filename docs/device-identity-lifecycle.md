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

Use OS-managed secure storage instead of a plain file:

- macOS: Keychain, and Secure Enclave when available
- Windows: TPM-backed or CNG / KSP-backed key storage
- Linux: TPM / PKCS#11 / system keyring when available

If secure storage is unavailable, the prototype can fall back to file-backed identity for development only.

## Lifecycle

### First enrollment

1. The CLI or agent asks the OS to create a non-exportable device key.
2. The OS returns a public key and a signing handle.
3. The control plane stores the public key, fingerprint, hostname, and device metadata.
4. The device uses the same signing handle for future signed requests.

### Normal start

1. `opengpu start` or the agent looks for the existing secure-store key.
2. If the key is present, it is reused.
3. The control plane sees the same contributor identity.

### App reinstall

1. The app is removed and installed again.
2. The OS key remains in secure storage.
3. The CLI and agent reuse the same key on next launch.
4. The device keeps the same identity.

### OS update

1. The OS updates.
2. The secure-store key remains available.
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

The current repo prototype still uses a file-backed identity for development convenience.
That is acceptable for the prototype, but the production design should move to non-exportable OS-backed storage.
