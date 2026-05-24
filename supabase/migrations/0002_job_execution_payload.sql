-- OpenGPU Supabase migration 0002
-- Add richer execution payload columns to jobs.

alter table public.jobs
  add column if not exists system_prompt text,
  add column if not exists max_tokens integer,
  add column if not exists temperature numeric,
  add column if not exists top_p numeric,
  add column if not exists seed bigint;
