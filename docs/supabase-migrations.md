# Supabase Migrations

The control plane now includes a small migration runner so the schema is no longer “paste SQL by hand and hope”.

## What It Does

- Reads the checked-in SQL files from `supabase/`
- Applies them in version order
- Records applied versions in `public.schema_migrations`
- Keeps the schema reviewable in Git

## Current Migration Files

0. `supabase/schema.sql`
   - base schema for `users`, `devices`, `heartbeats`, `jobs`, `job_events`, `policy_rules`, and `credits_ledger`

1. `supabase/migrations/0001_rls.sql`
   - enables RLS and revokes direct access from `anon` and `authenticated`

2. `supabase/migrations/0002_job_execution_payload.sql`
   - adds execution profile fields to jobs

3. `supabase/migrations/0003_identity_trust_path.sql`
   - stores the node identity trust path on devices and heartbeats

## How To Run

From the repo root:

```bash
cargo run --manifest-path apps/control-plane/Cargo.toml -- migrate
```

The command uses `DATABASE_URL` from `/.env`.

## What Still Needs Verification

- the live Supabase project must have the schema applied
- the live Supabase project must have the RLS rollout applied
- the live Supabase project must have the execution payload and identity trust path migrations applied
- the control plane must restart and report `storage_source: supabase`
- the browser dashboard must show restored state from Supabase
