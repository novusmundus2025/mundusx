# Credits Model

The control plane now keeps an append-only credits ledger for completed jobs.

## Purpose

- track how much contribution each device earned
- keep an audit trail for every reward
- make the contributor balance visible in the operator dashboard

## Ledger Shape

Each ledger row records:

- `id`
- `user_id` when the contributor account is known
- `device_id`
- `job_id`
- `entry_type`
- `amount`
- `currency`
- `metadata`
- `created_at`

## Reward Rule

The current prototype awards credits when a job completes successfully.

The formula is intentionally simple:

- compute a work unit count from prompt/output length
- apply a contribution multiplier from the device cap
- round the result to two decimal places

## Visibility

- `/v1/credits` returns the full ledger snapshot
- the dashboard shows total credits, balances by node, and recent awards
- the control-plane home page shows the ledger counts and total credits

## Notes

- This is still a prototype accounting model.
- User-level payout ownership comes later.
- The current implementation is enough to prove the ledger path and dashboard surface.
