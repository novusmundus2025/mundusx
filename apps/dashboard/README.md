# Dashboard

Web interface for node health, live routing, job history, and credits.

This package now runs a local operator dashboard that proxies the live control plane and renders:

- storage source and Supabase status
- node health and policy state
- job counts
- append-only job event history
- a local install page preview at `/install`

Run it with:

```bash
cd apps/dashboard
OPENGPU_CONTROL_PLANE_URL=http://127.0.0.1:8787 PORT=3001 npm run dev
```

For install page review, open `http://127.0.0.1:3001/install` (or another local port if you override `PORT`).

See [docs/repo-standards.md](/Users/DBATALL/Documents/aigrid/docs/repo-standards.md) for the canonical folder structure and review rules.
