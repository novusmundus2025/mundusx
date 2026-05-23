# Supabase Schema

This page sketches the company-side database schema for the OpenGPU control plane.

## Ownership Split

- Contributor machine: local config, identity, caches, and heartbeat logs only
- Control plane: Supabase Postgres, Auth, and server-side policy / billing / audit data

Supabase is a good fit here because it gives us managed Postgres plus Auth, and its docs recommend using Row Level Security for database access control. The service role key must stay server-side only.

## Local Development Env

Put the connection string in a repo-root `.env` file so the control plane can load it locally:

```bash
DATABASE_URL=postgresql://postgres.yjlvhhouncxhjkghnwyj:[YOUR-PASSWORD]@aws-1-eu-central-1.pooler.supabase.com:6543/postgres
```

The control plane reads `DATABASE_URL` at startup and reports whether it is configured, so you can verify the env is loaded before we wire the actual Supabase client.

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

- `id` UUID primary key
- `user_id` UUID references `users.id`
- `node_id` text unique
- `public_key_fingerprint` text unique
- `backend` text
- `contribution_percent` integer
- `power_source` text
- `on_battery` boolean
- `battery_percent` integer nullable
- `policy_allowed` boolean
- `policy_reason` text nullable
- `agent_version` text
- `state` text
- `last_seen_at` timestamptz
- `created_at` timestamptz
- `updated_at` timestamptz

### `heartbeats`

Append-only log of agent updates.

Suggested columns:

- `id` UUID primary key
- `device_id` UUID references `devices.id`
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
- `updated_at` timestamptz
- `created_at` timestamptz

### `jobs`

One row per submitted request.

Suggested columns:

- `id` UUID primary key
- `request_id` text unique
- `user_id` UUID references `users.id`
- `prompt` text
- `preferred_backend` text
- `model` text nullable
- `status` text
- `assigned_device_id` UUID nullable references `devices.id`
- `worker_id` text nullable
- `backend` text nullable
- `output` text nullable
- `error` text nullable
- `submitted_at` timestamptz
- `assigned_at` timestamptz nullable
- `completed_at` timestamptz nullable
- `created_at` timestamptz
- `updated_at` timestamptz

### `job_events`

Append-only audit trail for job lifecycle changes.

Suggested columns:

- `id` UUID primary key
- `job_id` UUID references `jobs.id`
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
- `device_id` UUID nullable references `devices.id`
- `job_id` UUID nullable references `jobs.id`
- `entry_type` text
- `amount` numeric
- `currency` text
- `metadata` jsonb
- `created_at` timestamptz

## RLS Guidance

- Enable Row Level Security on exposed tables.
- Use authenticated-user policies for user-owned tables.
- Keep service-role access server-side only.
- Use the service role for scheduler / agent / billing automation.

## Suggested MVP Order

1. `users`
2. `devices`
3. `jobs`
4. `heartbeats`
5. `job_events`
6. `policy_rules`
7. `credits_ledger`

## What This Is For

This schema keeps contributor machines lightweight while giving the company a durable source of truth for:

- node registry
- job queue
- policy state
- audit trail
- credits and billing
