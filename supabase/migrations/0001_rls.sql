-- OpenGPU Supabase RLS
-- Apply after supabase/schema.sql.

revoke all on table public.users from anon, authenticated;
revoke all on table public.devices from anon, authenticated;
revoke all on table public.heartbeats from anon, authenticated;
revoke all on table public.jobs from anon, authenticated;
revoke all on table public.job_events from anon, authenticated;
revoke all on table public.policy_rules from anon, authenticated;
revoke all on table public.credits_ledger from anon, authenticated;

alter table public.users enable row level security;
alter table public.devices enable row level security;
alter table public.heartbeats enable row level security;
alter table public.jobs enable row level security;
alter table public.job_events enable row level security;
alter table public.policy_rules enable row level security;
alter table public.credits_ledger enable row level security;

-- Intentionally no client-facing policies yet.
-- The OpenGPU control plane uses the service role server-side only.
-- This keeps contributor, job, and accounting data inaccessible from direct browser access.
