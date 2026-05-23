# Pending Items

This document tracks the next concrete implementation steps after the current Mac-first prototype.

## Next Up

1. **Wire the control plane to Supabase**
   - move nodes, heartbeats, jobs, and policy state off local JSON
   - keep the current API shape but persist it in Postgres

2. **Add auth for control-plane users and devices**
   - admin / operator login
   - device identity verification
   - server-side service role handling

3. **Add migrations and RLS**
   - create the Supabase schema from the sketch
   - lock down row access rules

4. **Add job event auditing**
   - capture state transitions in `job_events`
   - preserve a durable history of claims, completions, and failures

## Why These Are Pending

- The local control-plane prototype works, but it still resets on restart.
- We need a durable company-side source of truth before public rollout.
- Supabase is the chosen path for that durable backend.

## How To Use This Doc

When one of these items is complete, move it into the main missing-items tracker and update the relevant architecture docs.

