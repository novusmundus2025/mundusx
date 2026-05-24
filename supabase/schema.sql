-- OpenGPU Supabase schema
-- Apply this in the Supabase SQL editor.

create extension if not exists pgcrypto;

create or replace function public.set_updated_at()
returns trigger
language plpgsql
as $$
begin
  new.updated_at = now();
  return new;
end;
$$;

create table if not exists public.users (
  id uuid primary key default gen_random_uuid(),
  email text unique not null,
  display_name text,
  role text not null default 'operator',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.devices (
  node_id text primary key,
  user_id uuid references public.users(id),
  public_key_fingerprint text not null unique,
  public_key_hex text not null unique,
  hostname text not null,
  identity_trust_path text not null default 'unknown',
  backend text not null,
  contribution_percent integer not null,
  agent_version text not null,
  state text not null,
  available_memory_mb integer not null default 0,
  available_gpu_percent integer not null default 0,
  power_source text not null default 'unknown',
  on_battery boolean not null default false,
  battery_percent integer,
  policy_allowed boolean not null default false,
  policy_reason text,
  last_seen_at_epoch bigint,
  updated_at_epoch bigint,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.heartbeats (
  id bigint generated always as identity primary key,
  node_id text not null references public.devices(node_id) on delete cascade,
  backend text not null,
  agent_state text not null,
  available_memory_mb integer not null,
  available_gpu_percent integer not null,
  contribution_percent integer not null,
  hostname text not null,
  identity_trust_path text not null default 'unknown',
  power_source text not null,
  on_battery boolean not null,
  battery_percent integer,
  policy_allowed boolean not null,
  policy_reason text,
  observed_at_epoch bigint not null,
  created_at timestamptz not null default now()
);

create table if not exists public.jobs (
  job_id text primary key,
  request_id text not null unique,
  prompt text not null,
  preferred_backend text not null,
  model text,
  system_prompt text,
  max_tokens integer,
  temperature numeric,
  top_p numeric,
  seed bigint,
  status text not null,
  assigned_node_id text references public.devices(node_id),
  worker_id text,
  backend text,
  output text,
  error text,
  submitted_at_epoch bigint,
  assigned_at_epoch bigint,
  completed_at_epoch bigint,
  updated_at_epoch bigint,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.job_events (
  id bigint generated always as identity primary key,
  node_id text references public.devices(node_id) on delete set null,
  job_id text references public.jobs(job_id) on delete cascade,
  event_type text not null,
  payload jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create table if not exists public.policy_rules (
  id uuid primary key default gen_random_uuid(),
  name text not null unique,
  enabled boolean not null default true,
  rule_type text not null,
  rule_config jsonb not null default '{}'::jsonb,
  description text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.credits_ledger (
  id uuid primary key default gen_random_uuid(),
  user_id uuid references public.users(id),
  device_id text references public.devices(node_id),
  job_id text references public.jobs(job_id),
  entry_type text not null,
  amount numeric not null default 0,
  currency text not null default 'credits',
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

drop trigger if exists devices_set_updated_at on public.devices;
create trigger devices_set_updated_at
before update on public.devices
for each row execute function public.set_updated_at();

drop trigger if exists jobs_set_updated_at on public.jobs;
create trigger jobs_set_updated_at
before update on public.jobs
for each row execute function public.set_updated_at();

drop trigger if exists policy_rules_set_updated_at on public.policy_rules;
create trigger policy_rules_set_updated_at
before update on public.policy_rules
for each row execute function public.set_updated_at();

create index if not exists heartbeats_node_id_idx on public.heartbeats(node_id);
create index if not exists jobs_status_idx on public.jobs(status);
create index if not exists jobs_assigned_node_id_idx on public.jobs(assigned_node_id);
create index if not exists job_events_node_id_idx on public.job_events(node_id);
create index if not exists job_events_job_id_idx on public.job_events(job_id);
