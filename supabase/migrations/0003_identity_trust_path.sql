-- OpenGPU migration 0003: track device identity trust path on Supabase rows.

alter table if exists public.devices
  add column if not exists identity_trust_path text not null default 'unknown';

alter table if exists public.heartbeats
  add column if not exists identity_trust_path text not null default 'unknown';
