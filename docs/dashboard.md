# Dashboard

The dashboard is the local operator UI for the company-side control plane.

## What It Shows

- storage source used at boot
- Supabase enabled/disabled status
- node health and policy state
- job counts
- append-only job event history

## How It Works

- It runs as a small local Node server in `apps/dashboard`
- It proxies the live control plane endpoints server-side
- It renders the live data into a readable operator view

## Run

```bash
cd apps/dashboard
OPENGPU_CONTROL_PLANE_URL=http://127.0.0.1:8787 PORT=3001 npm run dev
```

## Notes

- This is the first real dashboard pass.
- Credits and onboarding are still product work for later.

