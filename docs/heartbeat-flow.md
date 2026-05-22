# Heartbeat Flow

This document sketches how nodes should announce presence to the control plane once that layer exists.

## Goal

Keep one source of truth for online nodes by having each active machine send a small heartbeat to the control plane.

## Recommended Timing

- `connected` and `not paused`: send a heartbeat every `5` seconds
- `paused` but still installed: send a heartbeat every `30` seconds
- `exit` or `disconnect`: stop sending heartbeats immediately

## What the Heartbeat Includes

- device ID
- public key fingerprint
- backend detected on the machine
- contribution percent
- connected / paused state
- optional machine hints like CPU cores and memory

## Control-Plane Interpretation

- heartbeat received within the expected window: node is `online`
- heartbeat delayed but not yet expired: node is `degraded`
- several missed heartbeats: node is `offline`

Suggested thresholds:

- `2` missed heartbeats: mark `degraded`
- `3` missed heartbeats: mark `offline`

## Why Heartbeats Instead of Broadcasts

- one source of truth
- less network noise
- easier counting
- easier to show a trustworthy online total in the dashboard and CLI

## Future Flow

1. The node agent starts.
2. It sends the first heartbeat immediately.
3. It continues on the interval above while active.
4. The control plane updates the registry and online count.
5. The CLI and dashboard read the count from the control plane.

