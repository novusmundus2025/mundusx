# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

## Next Up

1. **Finish the Supabase-backed control plane rollout**
   - keep nodes, heartbeats, jobs, and policy state durable in Supabase
   - apply the checked-in schema in the Supabase SQL editor
   - make Supabase the primary source of truth once RLS is in place

2. **Add Supabase RLS**
   - lock down row access rules
   - keep service-role access server-side only

3. **Add migrations and RLS**
   - create the Supabase schema from the sketch
   - keep the database schema reviewable and versioned

4. **Add job event auditing**
   - capture state transitions in `job_events`
   - preserve a durable history of claims, completions, and failures

5. **Port secure device identity to all platforms**
   - macOS secure storage is in place
   - keep the private key non-exportable on Windows and Linux too
   - preserve the current file-backed prototype only as a dev fallback

6. **Define the federated governance model**
   - document the top-level standards / clearing-house org
   - document how operator companies join and certify
   - define settlement, revocation, and protocol versioning rules
   - keep the company control plane separate from the governance layer

## Why These Are Pending

- The local control-plane prototype works, but the durable company-side source of truth is still not fully locked down with RLS and migrations.
- We need the Supabase backend to be the primary source of truth before public rollout.
- Supabase is the chosen path for that durable backend.
- The federated governance layer is still a design target, not a shipped subsystem.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.
