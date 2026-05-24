# Supabase Schema

This page sketches the company-side database schema for the OpenGPU control plane.

## Ownership Split

- Contributor machine: local config, identity, caches, and heartbeat logs only
- Control plane: Supabase Postgres, Auth, and server-side policy / billing / audit data

Supabase is a good fit here because it gives us managed Postgres plus Auth, and its docs recommend using Row Level Security for database access control. The service role key must stay server-side only.
The control plane now restores its in-memory registry from Supabase first when the Supabase env is configured, then mirrors registration, heartbeat, job, claim, and completion events into Supabase over HTTP. The boot-time health/status endpoints report whether the restore came from Supabase or from the local fallback cache, and the control plane now keeps an append-only local `job_events` log as well.
The repo also includes a migration runner at [docs/supabase-migrations.md](/Users/DBATALL/Documents/aigrid/docs/supabase-migrations.md) so the schema and RLS can be applied in a versioned way instead of copy/pasting SQL by hand. The live Supabase project has now been updated with that runner.

## Local Development Env

Put the connection details in a repo-root `.env` file so the control plane can load them locally:

```bash
DATABASE_URL=postgresql://postgres.yjlvhhouncxhjkghnwyj:[YOUR-PASSWORD]@aws-1-eu-central-1.pooler.supabase.com:6543/postgres
SUPABASE_SERVICE_ROLE_KEY=[YOUR-SERVICE-ROLE-KEY]
```

The control plane derives `SUPABASE_URL` from `DATABASE_URL` when needed, or you can set `SUPABASE_URL` directly.
`DATABASE_URL` is only used as a local helper here. We are not relying on `psql`.
If Supabase is not configured, the control plane keeps using local JSON as a fallback cache and logs the sync skip.
The executable schema lives in [supabase/schema.sql](/Users/DBATALL/Documents/aigrid/supabase/schema.sql).
The RLS rollout lives in [supabase/migrations/0001_rls.sql](/Users/DBATALL/Documents/aigrid/supabase/migrations/0001_rls.sql).

## Core Tables

### `users`

One row per human operator or account owner.

Suggested columns:

- `id` UUID primary key
- `email` text unique
- `display_name` text nullable
- `role` text
- `created_at` timestamptz
- `updated_at` timestamptz

### `devices`

One row per contributor machine.

Suggested columns:

- `node_id` text primary key
- `user_id` UUID references `users.id`
- `public_key_fingerprint` text unique
- `public_key_hex` text unique
- `identity_trust_path` text
- `backend` text
- `contribution_percent` integer
- `identity_trust_path` text
- `power_source` text
- `on_battery` boolean
- `battery_percent` integer nullable
- `policy_allowed` boolean
- `policy_reason` text nullable
- `agent_version` text
- `state` text
- `available_memory_mb` integer
- `available_gpu_percent` integer
- `last_seen_at_epoch` bigint
- `created_at` timestamptz
- `updated_at` timestamptz

The contributor node signs device requests with its local private key. The control plane verifies the signature using the stored `public_key_hex` before accepting register, heartbeat, claim, or completion requests. The signed device record also carries the machine hostname and `identity_trust_path` so contributor identity is easier to audit and review.

### `heartbeats`

Append-only log of agent updates.

Suggested columns:

- `id` bigint identity primary key
- `node_id` text references `devices.node_id`
- `backend` text
- `agent_state` text
- `available_memory_mb` integer
- `available_gpu_percent` integer
- `contribution_percent` integer
- `power_source` text
- `on_battery` boolean
- `battery_percent` integer nullable
- `policy_allowed` boolean
- `policy_reason` text nullable
- `observed_at_epoch` bigint
- `created_at` timestamptz

### `jobs`

One row per submitted request.

Suggested columns:

- `job_id` text primary key
- `request_id` text unique
- `user_id` UUID references `users.id`
- `prompt` text
- `preferred_backend` text
- `model` text nullable
- `system_prompt` text nullable
- `max_tokens` integer nullable
- `temperature` numeric nullable
- `top_p` numeric nullable
- `seed` bigint nullable
- `status` text
- `assigned_node_id` text nullable references `devices.node_id`
- `worker_id` text nullable
- `backend` text nullable
- `output` text nullable
- `error` text nullable
- `submitted_at_epoch` bigint
- `assigned_at_epoch` bigint nullable
- `completed_at_epoch` bigint nullable
- `created_at` timestamptz
- `updated_at` timestamptz

### `job_events`

Append-only audit trail for node and job lifecycle changes.

Suggested columns:

- `id` bigint identity primary key
- `node_id` text nullable references `devices.node_id`
- `job_id` text nullable references `jobs.job_id`
- `event_type` text
- `payload` jsonb
- `created_at` timestamptz

### `policy_rules`

Control-plane policy and safety knobs.

Suggested columns:

- `id` UUID primary key
- `name` text unique
- `enabled` boolean
- `rule_type` text
- `rule_config` jsonb
- `description` text nullable
- `created_at` timestamptz
- `updated_at` timestamptz

### `credits_ledger`

Contributor accounting and later payouts.

Suggested columns:

- `id` UUID primary key
- `user_id` UUID references `users.id`
- `device_id` text nullable references `devices.node_id`
- `job_id` text nullable references `jobs.job_id`
- `entry_type` text
- `amount` numeric
- `currency` text
- `metadata` jsonb
- `created_at` timestamptz

## RLS Guidance

- Enable Row Level Security on all exposed tables.
- Keep service-role access server-side only.
- Do not expose the service role to the browser or contributor machines.
- Add authenticated-user policies only when the dashboard and billing paths need direct browser access.

## Suggested MVP Order

1. `devices`
2. `heartbeats`
3. `jobs`
4. `job_events`
5. `policy_rules`
6. `users`
7. `credits_ledger`

## What This Is For

This schema keeps contributor machines lightweight while giving the company a durable source of truth for:

- node registry
- job queue
- policy state
- audit trail
- credits and billing
