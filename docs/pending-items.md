# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

## Next Up

1. **Verify the Supabase-backed control plane restore path**
   - keep nodes, heartbeats, jobs, and policy state durable in Supabase
   - restore control-plane state from Supabase on startup
   - confirm `/health` reports `storage_source: supabase`
   - confirm a restart restores the same state

2. **Make Supabase the primary source of truth**
   - keep local JSON only as a dev fallback cache
   - keep service-role access server-side only
   - stop relying on local JSON for production persistence

3. **Port secure device identity to all platforms**
   - macOS secure storage is in place
   - keep the private key non-exportable on Windows and Linux too
   - preserve the current file-backed prototype only as a dev fallback

4. **Define the federated governance model**
   - document the top-level standards / clearing-house org
   - document how operator companies join and certify
   - define settlement, revocation, and protocol versioning rules
   - keep the company control plane separate from the governance layer

## Why These Are Pending

- The local control-plane prototype works, but the durable company-side source of truth still needs the Supabase startup restore path verified live.
- We need the Supabase backend to be the primary source of truth before public rollout.
- Supabase is the chosen path for that durable backend.
- The federated governance layer is still a design target, not a shipped subsystem.
- The job event audit trail is implemented now; it should be tracked as complete in the main missing-items list.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.
