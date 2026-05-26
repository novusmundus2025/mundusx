import { createServer } from "node:http";

const controlPlaneUrl = process.env.OPENGPU_CONTROL_PLANE_URL ?? "http://127.0.0.1:8787";
const port = Number(process.env.PORT ?? "3001");
const appUrl = `http://127.0.0.1:${port}`;
const installReleaseBaseUrl =
  process.env.OPENGPU_INSTALL_RELEASE_BASE_URL ?? "http://127.0.0.1:8788/releases/latest/download";
const installCommand = `RELEASE_BASE_URL=${installReleaseBaseUrl} bash install.sh`;

const sampleCompletedJobs = [
  {
    id: "job_8f21f3",
    model: "HuggingFaceTB/SmolLM2-135M-Instruct",
    prompt: "Summarize NovusX in one sentence.",
    status: "completed",
    credits: 0.5,
    duration: "11s",
    finished_at: "2m ago",
    node: "mac-mini-01",
    tokens: 126,
  },
  {
    id: "job_8f21be",
    model: "HuggingFaceTB/SmolLM2-135M-Instruct",
    prompt: "Write a friendly onboarding tip for first-time contributors.",
    status: "completed",
    credits: 0.75,
    duration: "18s",
    finished_at: "11m ago",
    node: "mac-mini-01",
    tokens: 180,
  },
  {
    id: "job_8f2184",
    model: "HuggingFaceTB/SmolLM2-135M-Instruct",
    prompt: "Draft a short reply explaining credit accrual.",
    status: "completed",
    credits: 0.62,
    duration: "14s",
    finished_at: "32m ago",
    node: "mac-mini-01",
    tokens: 148,
  },
  {
    id: "job_8f217c",
    model: "HuggingFaceTB/SmolLM2-135M-Instruct",
    prompt: "Translate our contributor portal into a friendlier sentence.",
    status: "completed",
    credits: 0.88,
    duration: "21s",
    finished_at: "48m ago",
    node: "mac-mini-01",
    tokens: 214,
  },
  {
    id: "job_8f2149",
    model: "HuggingFaceTB/SmolLM2-135M-Instruct",
    prompt: "Generate a brief status update for the operator dashboard.",
    status: "completed",
    credits: 0.7,
    duration: "15s",
    finished_at: "1h ago",
    node: "mac-mini-01",
    tokens: 160,
  },
  {
    id: "job_8f20f2",
    model: "HuggingFaceTB/SmolLM2-135M-Instruct",
    prompt: "Explain how a GPU owner checks credits.",
    status: "completed",
    credits: 0.65,
    duration: "13s",
    finished_at: "2h ago",
    node: "mac-mini-01",
    tokens: 152,
  },
];

const formatCount = (value) => new Intl.NumberFormat("en-US").format(Number(value ?? 0));
const formatCredits = (value) => {
  const normalized = Math.abs(Number(value ?? 0)) < 0.000001 ? 0 : Number(value ?? 0);
  return normalized.toFixed(2);
};

const escapeHtml = (input) =>
  String(input ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");

async function fetchJson(path) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 2500);

  try {
    const response = await fetch(new URL(path, controlPlaneUrl), {
      signal: controller.signal,
      headers: {
        Accept: "application/json",
      },
    });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    return await response.json();
  } finally {
    clearTimeout(timeout);
  }
}

function badge(label, tone = "neutral") {
  return `<span class="pill pill-${tone}">${escapeHtml(label)}</span>`;
}

function installManifest(installPath = "/install") {
  return {
    kind: "install-manifest",
    app: "opengpu",
    environment: "localhost-preview",
    release_base_url: installReleaseBaseUrl,
    install_command: installCommand,
    binary_name: "opengpu-aarch64-apple-darwin",
    checksum_name: "opengpu-aarch64-apple-darwin.sha256",
    landing_page: `${appUrl}${installPath}`,
    docs_page: `${appUrl}/docs/install`,
    onboarding_command: "opengpu onboarding",
    cap_command: "opengpu cap",
    start_command: "opengpu start",
    preview_note: "local preview only; public domain comes later",
  };
}

function renderCounts(snapshot = {}) {
  const cards = [
    ["Online", snapshot.online_count, "green"],
    ["Paused", snapshot.paused_count, "orange"],
    ["Policy blocked", snapshot.policy_blocked_count, "red"],
    ["Queued jobs", snapshot.queued_job_count, "blue"],
    ["Assigned jobs", snapshot.assigned_job_count, "amber"],
    ["Completed jobs", snapshot.completed_job_count, "green"],
    ["Failed jobs", snapshot.failed_job_count, "red"],
    ["Job events", snapshot.job_events, "neutral"],
  ];

  return cards
    .map(
      ([label, value, tone]) => `
        <div class="card">
          <div class="card-label">${escapeHtml(label)}</div>
          <div class="card-value">${formatCount(value)}</div>
          <div>${badge(tone === "neutral" ? "live" : tone, tone)}</div>
        </div>`,
    )
    .join("");
}

function renderNodes(nodes = []) {
  if (!nodes.length) {
    return `<div class="empty">No nodes are registered yet.</div>`;
  }

  return `
    <div class="table">
      <div class="thead">
        <div>Node</div>
        <div>Host</div>
        <div>Backend</div>
        <div>State</div>
        <div>Policy</div>
        <div>Power</div>
        <div>Updated</div>
      </div>
      ${nodes
        .map((node) => {
          const battery = node.battery_percent == null ? "unknown" : `${node.battery_percent}%`;
          const power = `${node.power_source ?? "unknown"} • ${node.on_battery ? "battery" : "AC"} • ${battery}`;
          const workerHealth = node.worker_health ?? null;
          const workerLine = workerHealth
            ? `<div class="meta">worker: ${escapeHtml(workerHealth.healthy ? "healthy" : "degraded")} • model ${escapeHtml(workerHealth.model_name ?? "none")} • ${escapeHtml(workerHealth.model_path ?? "missing")} • llama-cli ${workerHealth.llama_cli_available ? "yes" : "no"} • BLAS ${workerHealth.blas_device_available ? "yes" : "no"}</div><div class="meta">${escapeHtml((workerHealth.notes ?? []).length ? workerHealth.notes.join(" • ") : "no notes")}</div>`
            : `<div class="meta">worker: unknown</div>`;
          const policyTone = node.policy_allowed ? "green" : "red";
          const stateTone =
            node.state === "ready" ? "green" : node.state === "busy" ? "amber" : node.state === "paused" ? "orange" : "red";
          return `
            <div class="row">
              <div>
                <strong>${escapeHtml(node.node_id)}</strong>
                <div class="meta">fingerprint ${escapeHtml(node.public_key_fingerprint ?? "unknown")}</div>
                <div class="meta">cap ${escapeHtml(node.contribution_percent ?? 0)}%</div>
              </div>
              <div>
                <div>${escapeHtml(node.hostname ?? "unknown")}</div>
                <div class="meta">signed device</div>
              </div>
              <div>${escapeHtml(node.backend ?? "unknown")}</div>
              <div>${badge(node.state ?? "unknown", stateTone)}</div>
              <div>
                ${badge(node.policy_allowed ? "allowed" : "blocked", policyTone)}
                ${node.policy_reason ? `<div class="meta">${escapeHtml(node.policy_reason)}</div>` : ""}
              </div>
              <div>
                <div>${escapeHtml(power)}</div>
                <div class="meta">${escapeHtml(node.available_memory_mb ?? 0)} MB free • ${escapeHtml(node.available_gpu_percent ?? 0)}% GPU free</div>
                ${workerLine}
              </div>
              <div>${escapeHtml(node.updated_at ?? "unknown")}</div>
            </div>`;
        })
        .join("")}
    </div>`;
}

function renderEvents(events = []) {
  if (!events.length) {
    return `<div class="empty">No job events yet.</div>`;
  }

  return `
    <div class="events">
      ${events
        .slice()
        .reverse()
        .slice(0, 24)
        .map(
          (event) => `
            <div class="event">
              <div class="event-top">
                <strong>${escapeHtml(event.event_type)}</strong>
                <span class="meta">${escapeHtml(event.created_at ?? "unknown")}</span>
              </div>
              <div class="meta">node ${escapeHtml(event.node_id ?? "n/a")} • job ${escapeHtml(event.job_id ?? "n/a")}</div>
              <pre>${escapeHtml(JSON.stringify(event.payload ?? {}, null, 2))}</pre>
            </div>`,
        )
        .join("")}
    </div>`;
}

function renderCredits(credits = {}) {
  const ledger = Array.isArray(credits.ledger) ? credits.ledger : [];
  const byNode = credits.by_node ?? {};
  const total = Number(credits.total ?? 0);
  const balanceRows = Object.entries(byNode);

  const balances = balanceRows.length
    ? `
      <div class="balance-grid">
        ${balanceRows
          .map(
            ([nodeId, amount]) => `
              <div class="balance">
                <strong>${escapeHtml(nodeId)}</strong>
                <div class="meta">${formatCount(amount)} credits</div>
              </div>`,
          )
          .join("")}
      </div>`
    : `<div class="empty">No contributor balances yet.</div>`;

  const recentEntries = ledger.length
    ? `
      <div class="events">
        ${ledger
          .slice()
          .reverse()
          .slice(0, 12)
          .map(
            (entry) => `
              <div class="event">
            <div class="event-top">
                <strong>${escapeHtml(entry.entry_type ?? "credit")}</strong>
                <span class="meta">${escapeHtml(entry.created_at ?? "unknown")}</span>
              </div>
              <div class="meta">
                device ${escapeHtml(entry.device_id ?? "n/a")} • job ${escapeHtml(entry.job_id ?? "n/a")} •
                  ${formatCredits(entry.amount ?? 0)} ${escapeHtml(entry.currency ?? "credits")}
              </div>
                ${entry.metadata ? `<pre>${escapeHtml(JSON.stringify(entry.metadata, null, 2))}</pre>` : ""}
              </div>`,
          )
          .join("")}
      </div>`
    : `<div class="empty">No ledger entries recorded yet.</div>`;

  return `
    <div class="section">
      <div class="section-head">
        <h2 class="section-title">Credits</h2>
        <div class="meta">${formatCount(ledger.length)} ledger entries</div>
      </div>
      <div class="section-body">
        <div class="grid credits-grid">
          <div class="card">
            <div class="card-label">Total credits</div>
            <div class="card-value">${formatCredits(total)}</div>
            <div>${badge("ledger live", "green")}</div>
          </div>
          <div class="card">
            <div class="card-label">Contributor balances</div>
            <div class="card-value">${formatCount(balanceRows.length)}</div>
            <div>${badge(balanceRows.length ? "allocated" : "empty", balanceRows.length ? "blue" : "neutral")}</div>
          </div>
        </div>
        <h3 class="subhead">Balances by node</h3>
        ${balances}
        <h3 class="subhead">Recent awards</h3>
        ${recentEntries}
      </div>
    </div>`;
}

function renderContributorJobHistoryPage(requestUrl, basePath = "/portal") {
  const query = (requestUrl.searchParams.get("q") ?? "").trim();
  const normalizedQuery = query.toLowerCase();
  const requestedPage = Number.parseInt(requestUrl.searchParams.get("page") ?? "1", 10);
  const pageSize = 3;

  const filteredJobs = sampleCompletedJobs.filter((job) => {
    if (!normalizedQuery) return true;
    return [job.id, job.model, job.prompt, job.node, job.status]
      .join(" ")
      .toLowerCase()
      .includes(normalizedQuery);
  });

  const totalJobs = sampleCompletedJobs.length;
  const totalCredits = sampleCompletedJobs.reduce((sum, job) => sum + Number(job.credits ?? 0), 0);
  const averageDurationSeconds = sampleCompletedJobs.reduce((sum, job) => sum + Number(String(job.duration).replace(/[^0-9.]/g, "")), 0) /
    Math.max(sampleCompletedJobs.length, 1);
  const totalPages = Math.max(1, Math.ceil(filteredJobs.length / pageSize));
  const page = Math.min(
    Math.max(Number.isFinite(requestedPage) ? requestedPage : 1, 1),
    totalPages,
  );
  const pageJobs = filteredJobs.slice((page - 1) * pageSize, page * pageSize);

  const buildHref = (nextPage) => {
    const params = new URLSearchParams();
    if (query) params.set("q", query);
    if (nextPage > 1) params.set("page", String(nextPage));
    const qs = params.toString();
    return `${basePath}/jobs${qs ? `?${qs}` : ""}`;
  };

  const pageTitle = query ? `Job history for "${query}"` : "Completed jobs";

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>NovusX Contributor Job History</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --text: #0f172a;
        --muted: #5f6b85;
        --green: #0f9d58;
        --blue: #3452ff;
        --amber: #d97706;
        --shadow: 0 18px 60px rgba(15, 23, 42, 0.06);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 28%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
      }
      .wrap {
        max-width: 1240px;
        margin: 0 auto;
        padding: 22px 20px 48px;
      }
      .topbar {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        margin-bottom: 26px;
      }
      .brand {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 800;
      }
      .brand-mark {
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }
      .badge, .pill {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid var(--line);
        color: var(--muted);
        background: rgba(255, 255, 255, 0.8);
      }
      .hero {
        border: 1px solid var(--line);
        border-radius: 22px;
        background: rgba(255, 255, 255, 0.92);
        box-shadow: var(--shadow);
        padding: 24px;
      }
      .hero-grid {
        display: grid;
        grid-template-columns: minmax(0, 1.15fr) minmax(300px, 0.85fr);
        gap: 18px;
      }
      h1 {
        margin: 10px 0 0;
        font-size: clamp(36px, 4vw, 56px);
        line-height: 0.98;
        letter-spacing: -0.06em;
      }
      .sub {
        margin-top: 14px;
        color: var(--muted);
        line-height: 1.72;
        max-width: 68ch;
      }
      .statusline {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-top: 18px;
      }
      .pill-blue { background: rgba(52, 82, 255, 0.08); color: var(--blue); border-color: rgba(52, 82, 255, 0.16); }
      .pill-green { background: rgba(15, 157, 88, 0.08); color: var(--green); border-color: rgba(15, 157, 88, 0.16); }
      .pill-amber { background: rgba(217, 119, 6, 0.08); color: var(--amber); border-color: rgba(217, 119, 6, 0.16); }
      .summary-grid {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 12px;
        margin-top: 18px;
      }
      .summary-card {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 16px;
      }
      .summary-label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 11px;
      }
      .summary-value {
        margin-top: 8px;
        font-size: 28px;
        font-weight: 800;
        letter-spacing: -0.05em;
      }
      .toolbar {
        margin-top: 20px;
        display: flex;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
        align-items: center;
      }
      .search-form {
        display: flex;
        gap: 10px;
        flex: 1 1 420px;
      }
      .search-input {
        flex: 1 1 auto;
        min-width: 240px;
        min-height: 46px;
        border-radius: 14px;
        border: 1px solid var(--line);
        background: #fff;
        color: var(--text);
        padding: 0 14px;
        font: inherit;
      }
      .search-button, .nav-button {
        min-height: 46px;
        border-radius: 14px;
        border: 1px solid var(--line);
        background: var(--surface-2);
        color: var(--text);
        padding: 0 16px;
        font: inherit;
        font-weight: 600;
        text-decoration: none;
        display: inline-flex;
        align-items: center;
        justify-content: center;
      }
      .results {
        display: grid;
        gap: 14px;
        margin-top: 18px;
      }
      .job-card {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: rgba(255, 255, 255, 0.92);
        box-shadow: var(--shadow);
        padding: 18px;
      }
      .job-top {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
        align-items: start;
      }
      .job-id {
        font-weight: 800;
        letter-spacing: -0.02em;
      }
      .job-model {
        margin-top: 4px;
        color: var(--muted);
        font-size: 13px;
      }
      .job-prompt {
        margin: 14px 0 0;
        color: var(--text);
        line-height: 1.65;
        font-size: 15px;
      }
      .job-meta {
        display: grid;
        grid-template-columns: repeat(5, minmax(0, 1fr));
        gap: 12px;
        margin-top: 14px;
      }
      .meta-box {
        border: 1px solid var(--line);
        border-radius: 14px;
        background: var(--surface);
        padding: 12px 14px;
      }
      .meta-label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 11px;
      }
      .meta-value {
        margin-top: 6px;
        font-weight: 700;
        letter-spacing: -0.01em;
      }
      .pagination {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
        margin-top: 18px;
      }
      .muted {
        color: var(--muted);
        font-size: 13px;
      }
      .empty {
        border: 1px dashed var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 28px;
        color: var(--muted);
        text-align: center;
      }
      @media (max-width: 900px) {
        .hero-grid,
        .job-meta,
        .summary-grid {
          grid-template-columns: 1fr 1fr;
        }
      }
      @media (max-width: 720px) {
        .hero-grid,
        .job-meta,
        .summary-grid {
          grid-template-columns: 1fr;
        }
        .search-form {
          flex-direction: column;
        }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="topbar">
        <div class="brand"><span class="brand-mark"></span> NovusX Contributor Portal</div>
        <div class="badge">localhost preview • job history</div>
      </div>

      <div class="hero">
        <div class="hero-grid">
          <div>
            <div class="kicker">Detailed history</div>
            <h1>${escapeHtml(pageTitle)}</h1>
            <div class="sub">
              Completed jobs live on their own page so the list can scale with search and pagination.
              This view is contributor-first: every row shows the prompt, credits earned, duration,
              and the node that completed the work.
            </div>
            <div class="statusline">
              <span class="pill pill-green">${formatCount(totalJobs)} jobs total</span>
              <span class="pill pill-blue">${formatCredits(totalCredits)} credits earned</span>
              <span class="pill pill-amber">${formatCount(Math.round(averageDurationSeconds))}s avg duration</span>
            </div>
          </div>
          <div class="summary-grid">
            <div class="summary-card">
              <div class="summary-label">Completed jobs</div>
              <div class="summary-value">${formatCount(filteredJobs.length)}</div>
              <div class="muted">${query ? "matching the current search" : "visible in this history"}</div>
            </div>
            <div class="summary-card">
              <div class="summary-label">Credits earned</div>
              <div class="summary-value">${formatCredits(
                filteredJobs.reduce((sum, job) => sum + Number(job.credits ?? 0), 0),
              )}</div>
              <div class="muted">from the filtered set</div>
            </div>
            <div class="summary-card">
              <div class="summary-label">Page</div>
              <div class="summary-value">${formatCount(page)} / ${formatCount(totalPages)}</div>
              <div class="muted">3 jobs per page</div>
            </div>
          </div>
        </div>
      </div>

      <div class="toolbar">
        <form class="search-form" method="get" action="${escapeHtml(`${basePath}/jobs`)}">
          <input class="search-input" type="search" name="q" placeholder="Search by job ID, model, prompt, or node" value="${escapeHtml(query)}" />
          <button class="search-button" type="submit">Search</button>
        </form>
        <a class="nav-button" href="${escapeHtml(basePath)}">Back to portal</a>
      </div>

      <div class="results">
        ${
          pageJobs.length
            ? pageJobs
                .map(
                  (job) => `
                    <article class="job-card">
                      <div class="job-top">
                        <div>
                          <div class="job-id">${escapeHtml(job.id)}</div>
                          <div class="job-model">${escapeHtml(job.model)}</div>
                        </div>
                        <span class="pill pill-green">${escapeHtml(job.status)}</span>
                      </div>
                      <div class="job-prompt">${escapeHtml(job.prompt)}</div>
                      <div class="job-meta">
                        <div class="meta-box">
                          <div class="meta-label">Credits</div>
                          <div class="meta-value">${job.credits.toFixed(2)}</div>
                        </div>
                        <div class="meta-box">
                          <div class="meta-label">Duration</div>
                          <div class="meta-value">${escapeHtml(job.duration)}</div>
                        </div>
                        <div class="meta-box">
                          <div class="meta-label">Finished</div>
                          <div class="meta-value">${escapeHtml(job.finished_at)}</div>
                        </div>
                        <div class="meta-box">
                          <div class="meta-label">Node</div>
                          <div class="meta-value">${escapeHtml(job.node)}</div>
                        </div>
                        <div class="meta-box">
                          <div class="meta-label">Tokens</div>
                          <div class="meta-value">${formatCount(job.tokens)}</div>
                        </div>
                      </div>
                    </article>`,
                )
                .join("")
            : `<div class="empty">No completed jobs matched your search.</div>`
        }
      </div>

      <div class="pagination">
        <div class="muted">
          ${filteredJobs.length ? `Showing ${Math.min((page - 1) * pageSize + 1, filteredJobs.length)}-${Math.min(page * pageSize, filteredJobs.length)} of ${formatCount(filteredJobs.length)} jobs` : "No jobs to show"}
        </div>
        <div style="display: flex; gap: 10px; flex-wrap: wrap;">
          <a class="nav-button" href="${escapeHtml(buildHref(Math.max(page - 1, 1)))}" ${page <= 1 ? 'aria-disabled="true" style="pointer-events:none; opacity:0.5;"' : ""}>Previous</a>
          <a class="nav-button" href="${escapeHtml(buildHref(Math.min(page + 1, totalPages)))}" ${page >= totalPages ? 'aria-disabled="true" style="pointer-events:none; opacity:0.5;"' : ""}>Next</a>
        </div>
      </div>
    </div>
  </body>
</html>`;
}

function renderContributorPortal() {
  const sampleHealth = {
    healthy: true,
    power_source: "AC",
    on_battery: false,
    battery_percent: 100,
    policy_allowed: true,
    policy_reason: null,
    worker_health: {
      healthy: true,
      model_name: "HuggingFaceTB/SmolLM2-135M-Instruct",
      model_path: "/Users/DBATALL/.opengpu/models/...",
      llama_cli_available: true,
      blas_device_available: true,
      notes: ["ready for local jobs", "Mac-first preview"],
    },
  };

  const sampleStats = [
    ["Balance", "128.40 credits", "green", "earned this week"],
    ["Jobs completed", "84", "blue", "lifetime total"],
    ["Ready state", "Healthy", "green", "worker is online"],
    ["Policy", "Allowed", "blue", "cap and power are OK"],
  ];

  const sampleEvents = [
    {
      title: "job_completed",
      detail: "prompt: summarize NovusX in one sentence",
      time: "2m ago",
    },
    {
      title: "credit_awarded",
      detail: "0.50 credits added to contributor balance",
      time: "2m ago",
    },
    {
      title: "heartbeat",
      detail: "model healthy, AC power, 16 GB free",
      time: "just now",
    },
  ];

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>NovusX Contributor Portal</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --line-strong: rgba(15, 23, 42, 0.14);
        --text: #0f172a;
        --muted: #5f6b85;
        --green: #0f9d58;
        --blue: #3452ff;
        --amber: #d97706;
        --orange: #c47f1b;
        --red: #d14343;
        --shadow: 0 18px 60px rgba(15, 23, 42, 0.06);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 28%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
      }
      .wrap {
        max-width: 1380px;
        margin: 0 auto;
        padding: 22px 20px 48px;
      }
      .topbar {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        margin-bottom: 28px;
      }
      .brand {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 800;
        letter-spacing: 0.02em;
      }
      .brand-mark {
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }
      .badge {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid var(--line);
        color: var(--muted);
        background: rgba(255, 255, 255, 0.8);
      }
      .hero {
        border: 1px solid var(--line);
        border-radius: 22px;
        background: rgba(255, 255, 255, 0.92);
        box-shadow: var(--shadow);
        padding: 24px;
      }
      .hero-grid {
        display: grid;
        grid-template-columns: minmax(0, 1.2fr) minmax(320px, 0.8fr);
        gap: 20px;
      }
      h1 {
        margin: 0;
        font-size: clamp(42px, 5vw, 68px);
        line-height: 0.96;
        letter-spacing: -0.06em;
      }
      .sub {
        margin-top: 14px;
        color: var(--muted);
        line-height: 1.72;
        max-width: 68ch;
      }
      .statusline {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-top: 18px;
      }
      .pill {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid transparent;
      }
      .pill-green { background: rgba(15, 157, 88, 0.08); color: var(--green); border-color: rgba(15, 157, 88, 0.16); }
      .pill-blue { background: rgba(52, 82, 255, 0.08); color: var(--blue); border-color: rgba(52, 82, 255, 0.16); }
      .pill-orange { background: rgba(196, 127, 27, 0.08); color: var(--orange); border-color: rgba(196, 127, 27, 0.16); }
      .pill-red { background: rgba(209, 67, 67, 0.08); color: var(--red); border-color: rgba(209, 67, 67, 0.16); }
      .pill-neutral { background: rgba(95, 107, 133, 0.08); color: var(--muted); border-color: rgba(95, 107, 133, 0.16); }
      .sidebar {
        margin-top: 22px;
        border: 1px solid var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 18px;
      }
      .sidebar-head {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        margin-bottom: 16px;
      }
      .kicker {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .grid {
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: 14px;
        margin-top: 20px;
      }
      .card {
        border: 1px solid var(--line);
        background: var(--surface);
        border-radius: 18px;
        padding: 16px;
      }
      .card.card-action {
        width: 100%;
        text-align: left;
        font: inherit;
        cursor: pointer;
        transition: transform 140ms ease, border-color 140ms ease, box-shadow 140ms ease;
      }
      .card.card-action:hover,
      .card.card-action:focus-visible {
        transform: translateY(-1px);
        border-color: rgba(52, 82, 255, 0.26);
        box-shadow: 0 18px 48px rgba(52, 82, 255, 0.08);
        outline: none;
      }
      .card-link {
        display: block;
        color: inherit;
        text-decoration: none;
      }
      .card-label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .card-value {
        margin: 10px 0 8px;
        font-size: 28px;
        font-weight: 700;
      }
      .meta {
        color: var(--muted);
        font-size: 12px;
        line-height: 1.45;
      }
      .layout {
        display: grid;
        grid-template-columns: minmax(0, 1.25fr) minmax(340px, 0.75fr);
        gap: 18px;
        margin-top: 20px;
      }
      .section {
        border: 1px solid var(--line);
        border-radius: 22px;
        background: rgba(255, 255, 255, 0.92);
        box-shadow: var(--shadow);
      }
      .section-head {
        padding: 16px 20px;
        border-bottom: 1px solid var(--line);
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 16px;
        flex-wrap: wrap;
      }
      .section-title {
        margin: 0;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 14px;
      }
      .section-body {
        padding: 20px;
      }
      .panel-list {
        display: grid;
        gap: 12px;
      }
      .panel {
        border: 1px solid var(--line);
        border-radius: 16px;
        background: var(--surface);
        padding: 14px 16px;
      }
      .panel-top {
        display: flex;
        justify-content: space-between;
        gap: 12px;
        align-items: start;
        flex-wrap: wrap;
      }
      .panel strong {
        display: block;
        margin-bottom: 5px;
      }
      .panel p {
        margin: 0;
        color: var(--muted);
        line-height: 1.6;
      }
      .drawer-backdrop {
        position: fixed;
        inset: 0;
        background: rgba(15, 23, 42, 0.28);
        backdrop-filter: blur(8px);
        z-index: 30;
      }
      .drawer {
        position: fixed;
        top: 20px;
        right: 20px;
        width: min(760px, calc(100vw - 40px));
        max-height: calc(100vh - 40px);
        overflow: auto;
        border: 1px solid var(--line);
        border-radius: 24px;
        background: rgba(255, 255, 255, 0.98);
        box-shadow: 0 30px 80px rgba(15, 23, 42, 0.18);
        z-index: 40;
        transform: translateY(8px);
        opacity: 0;
        pointer-events: none;
        transition: opacity 160ms ease, transform 160ms ease;
      }
      .drawer.is-open {
        opacity: 1;
        pointer-events: auto;
        transform: translateY(0);
      }
      .drawer-head {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 16px;
        padding: 22px 22px 0;
      }
      .drawer-kicker {
        color: var(--blue);
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.14em;
        text-transform: uppercase;
      }
      .drawer-title {
        margin: 8px 0 0;
        font-size: 30px;
        line-height: 1;
        letter-spacing: -0.05em;
      }
      .drawer-close {
        border: 1px solid var(--line);
        background: var(--surface-2);
        color: var(--text);
        border-radius: 999px;
        min-height: 40px;
        padding: 0 14px;
        font: inherit;
        cursor: pointer;
      }
      .drawer-summary {
        display: grid;
        grid-template-columns: repeat(3, minmax(0, 1fr));
        gap: 12px;
        padding: 18px 22px 0;
      }
      .summary-card {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 16px;
      }
      .summary-label,
      .job-meta-label {
        color: var(--muted);
        font-size: 12px;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .summary-value {
        margin-top: 8px;
        font-size: 28px;
        font-weight: 800;
        letter-spacing: -0.05em;
      }
      .drawer-body {
        display: grid;
        gap: 12px;
        padding: 18px 22px 22px;
      }
      .job-row {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 16px;
      }
      .job-row-top {
        display: flex;
        align-items: flex-start;
        justify-content: space-between;
        gap: 12px;
      }
      .job-id {
        font-weight: 700;
        letter-spacing: -0.02em;
      }
      .job-model {
        margin-top: 4px;
        color: var(--muted);
        font-size: 13px;
      }
      .job-prompt {
        margin: 14px 0 0;
        color: var(--text);
        line-height: 1.65;
      }
      .job-meta-grid {
        display: grid;
        grid-template-columns: repeat(5, minmax(0, 1fr));
        gap: 12px;
        margin-top: 14px;
      }
      .job-meta-grid strong {
        display: block;
        margin-top: 6px;
        font-size: 15px;
        letter-spacing: -0.02em;
      }
      .device-box {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: linear-gradient(180deg, #ffffff, #f9fbff);
        padding: 16px;
        margin-top: 16px;
      }
      .device-grid {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 12px;
        margin-top: 14px;
      }
      .device-field {
        border: 1px solid var(--line);
        border-radius: 14px;
        background: var(--surface);
        padding: 12px 14px;
      }
      .device-field .label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 11px;
        margin-bottom: 6px;
      }
      .device-field .value {
        font-size: 14px;
        line-height: 1.5;
      }
      .links {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
      }
      a {
        color: var(--blue);
        text-decoration: none;
      }
      a:hover { text-decoration: underline; }
      .footer {
        margin-top: 18px;
        color: var(--muted);
        font-size: 12px;
      }
      @media (max-width: 1100px) {
        .hero-grid,
        .layout { grid-template-columns: 1fr; }
        .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
        .drawer-summary,
        .job-meta-grid { grid-template-columns: 1fr 1fr; }
      }
      @media (max-width: 760px) {
        .grid,
        .device-grid { grid-template-columns: 1fr; }
        .drawer {
          top: 10px;
          right: 10px;
          left: 10px;
          width: auto;
          max-height: calc(100vh - 20px);
        }
        .drawer-summary,
        .job-meta-grid { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="topbar">
        <div class="brand"><span class="brand-mark"></span> NovusX Contributor Portal</div>
        <div class="badge">localhost preview • contributor view</div>
      </div>

      <div class="hero">
        <div class="hero-grid">
          <div>
            <div class="kicker">Owner-facing portal</div>
            <h1>See what your GPU is doing, and what it earned.</h1>
            <div class="sub">
              This is the contributor view: earnings, health, policy, cap, and job history in one
              place. The worker still runs locally on the machine, while the portal shows the
              company-side summary that the contributor cares about most.
            </div>
            <div class="statusline">
              <span class="pill pill-green">earning preview</span>
              <span class="pill pill-blue">health visible</span>
              <span class="pill pill-neutral">trust path shown</span>
              <span class="pill pill-orange">local preview only</span>
            </div>
          </div>
          <div class="sidebar">
            <div class="sidebar-head">
              <div>
                <div class="kicker">Machine summary</div>
                <strong>Mac contributor node</strong>
              </div>
              <span class="pill pill-green">healthy</span>
            </div>
            <div class="device-box">
              <div class="device-grid">
                <div class="device-field">
                  <div class="label">Balance</div>
                  <div class="value">128.40 credits</div>
                </div>
                <div class="device-field">
                  <div class="label">Policy</div>
                  <div class="value">Allowed</div>
                </div>
                <div class="device-field">
                  <div class="label">Cap</div>
                  <div class="value">20%</div>
                </div>
                <div class="device-field">
                  <div class="label">Power</div>
                  <div class="value">AC power</div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="grid">
        ${sampleStats
          .map(
            ([label, value, tone, detail]) => `
              ${
                label === "Jobs completed"
                  ? `<a class="card card-action card-link" href="/portal/jobs" aria-label="Open completed jobs history">`
                  : `<div class="card">`
              }
                <div class="card-label">${escapeHtml(label)}</div>
                <div class="card-value">${escapeHtml(value)}</div>
                <div class="meta">${badge(tone === "green" ? "live" : tone, tone)}</div>
                <div class="meta" style="margin-top: 8px;">${escapeHtml(detail)}</div>
              ${label === "Jobs completed" ? "</a>" : "</div>"}`,
          )
          .join("")}
      </div>

      <div class="layout">
        <div class="section">
          <div class="section-head">
            <h2 class="section-title">Recent activity</h2>
            <div class="meta">latest jobs and awards</div>
          </div>
          <div class="section-body">
            <div class="panel-list">
              ${sampleEvents
                .map(
                  (event) => `
                    <div class="panel">
                      <div class="panel-top">
                        <strong>${escapeHtml(event.title)}</strong>
                        <span class="meta">${escapeHtml(event.time)}</span>
                      </div>
                      <p>${escapeHtml(event.detail)}</p>
                    </div>`,
                )
                .join("")}
            </div>
          </div>
        </div>

        <div class="section">
          <div class="section-head">
            <h2 class="section-title">Machine health</h2>
            <div class="meta">${sampleHealth.healthy ? "healthy" : "degraded"}</div>
          </div>
          <div class="section-body">
            <div class="panel-list">
              <div class="panel">
                <div class="panel-top">
                  <strong>Worker</strong>
                  <span class="pill pill-green">ready</span>
                </div>
                <p>
                  Model ${escapeHtml(sampleHealth.worker_health.model_name)} is loaded and the
                  local worker is available for jobs.
                </p>
              </div>
              <div class="panel">
                <div class="panel-top">
                  <strong>Trust path</strong>
                  <span class="pill pill-blue">signed</span>
                </div>
                <p>
                  The node identity is sign-only and survives reinstall through the local
                  encrypted fallback.
                </p>
              </div>
              <div class="panel">
                <div class="panel-top">
                  <strong>Next actions</strong>
                  <span class="pill pill-neutral">simple</span>
                </div>
                <p>
                  Start, pause, adjust your cap, or review earnings history from the portal.
                </p>
              </div>
            </div>

            <div class="footer" style="margin-top: 16px;">
              This portal reads from the company control plane and its durable state, not directly
              from the worker.
            </div>
          </div>
        </div>
      </div>

      <div class="footer" style="margin-top: 20px;">
        Contributor portal preview only. The live public portal would sit on top of the company
        control plane and show the same data.
      </div>
    </div>
  </body>
</html>`;
}

function renderInstallPage(installPath = "/install") {
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>NovusX Install</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --text: #0f172a;
        --muted: #5f6b85;
        --blue: #3452ff;
        --blue-2: #1f3fe6;
        --shadow: 0 18px 60px rgba(15, 23, 42, 0.08);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 28%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
      }
      .wrap {
        max-width: 1100px;
        margin: 0 auto;
        padding: 22px 20px 48px;
      }
      .topbar {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        margin-bottom: 28px;
      }
      .brand {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 700;
        letter-spacing: 0.02em;
      }
      .brand-mark {
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }
      .chip {
        color: var(--muted);
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      .hero {
        display: grid;
        grid-template-columns: minmax(0, 1.2fr) minmax(330px, 0.8fr);
        gap: 28px;
        align-items: start;
      }
      .eyebrow {
        color: var(--blue);
        font-size: 12px;
        font-weight: 700;
        letter-spacing: 0.14em;
        text-transform: uppercase;
        margin-bottom: 18px;
      }
      h1 {
        margin: 0;
        max-width: 9ch;
        font-size: clamp(54px, 7vw, 88px);
        line-height: 0.92;
        letter-spacing: -0.08em;
      }
      .sub {
        margin-top: 18px;
        max-width: 52ch;
        color: var(--muted);
        font-size: 18px;
        line-height: 1.72;
      }
      .actions {
        display: flex;
        gap: 12px;
        flex-wrap: wrap;
        margin-top: 22px;
      }
      .button {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-height: 46px;
        padding: 0 18px;
        border-radius: 12px;
        text-decoration: none;
        font-weight: 600;
      }
      .button-primary {
        background: linear-gradient(180deg, var(--blue), var(--blue-2));
        color: white;
        box-shadow: var(--shadow);
      }
      .button-secondary {
        background: var(--surface-2);
        color: var(--text);
        border: 1px solid var(--line);
      }
      .tags {
        display: flex;
        gap: 8px;
        flex-wrap: wrap;
        margin-top: 22px;
      }
      .tag {
        padding: 6px 10px;
        border-radius: 999px;
        background: var(--surface-2);
        border: 1px solid var(--line);
        color: var(--muted);
        font-size: 12px;
      }
      .install-card {
        position: sticky;
        top: 20px;
        padding: 22px;
        border-radius: 24px;
        border: 1px solid var(--line);
        background: rgba(255, 255, 255, 0.9);
        box-shadow: var(--shadow);
      }
      .label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 11px;
        margin-bottom: 10px;
      }
      .command {
        display: flex;
        align-items: center;
        gap: 12px;
        justify-content: space-between;
        padding: 16px 16px;
        border-radius: 16px;
        border: 1px solid var(--line);
        background: #fff;
        overflow-x: auto;
      }
      .command-shell {
        min-height: 54px;
        display: flex;
        align-items: center;
      }
      code {
        font-family: "SFMono-Regular", Menlo, Monaco, Consolas, monospace;
        font-size: 14px;
        white-space: nowrap;
      }
      .copy-btn {
        border: 1px solid var(--line);
        background: var(--surface-2);
        color: var(--text);
        border-radius: 10px;
        padding: 9px 12px;
        cursor: pointer;
        font: inherit;
      }
      .copy-btn:hover { background: #eef2ff; }
      .install-list {
        display: grid;
        gap: 10px;
        margin-top: 18px;
      }
      .install-step {
        display: flex;
        gap: 12px;
        padding: 12px 0;
        border-top: 1px solid var(--line);
      }
      .install-step:first-child {
        border-top: 0;
        padding-top: 0;
      }
      .num {
        width: 26px;
        height: 26px;
        border-radius: 999px;
        display: inline-flex;
        align-items: center;
        justify-content: center;
        flex: 0 0 auto;
        background: #eef2ff;
        color: var(--blue);
        font-weight: 700;
      }
      .install-step strong {
        display: block;
        margin-bottom: 4px;
      }
      .install-step p {
        margin: 0;
        color: var(--muted);
        line-height: 1.6;
        font-size: 14px;
      }
      .footer {
        margin-top: 28px;
        color: var(--muted);
        font-size: 12px;
      }
      .footer code {
        font-size: 12px;
      }
      .manifest-note {
        margin-top: 12px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.5;
      }
      .manifest-note strong {
        color: var(--text);
      }
      .manifest-status {
        margin-top: 12px;
        color: var(--blue);
        font-size: 12px;
        letter-spacing: 0.06em;
        text-transform: uppercase;
      }
      @media (max-width: 900px) {
        .hero {
          grid-template-columns: 1fr;
        }
        .install-card {
          position: static;
        }
        h1 {
          max-width: none;
        }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="topbar">
        <div class="brand"><span class="brand-mark"></span> NovusX Install</div>
        <div class="chip">localhost preview • Mac-first</div>
      </div>

      <div class="hero">
        <div>
          <div class="eyebrow">Local-first install flow</div>
          <h1>Install NovusX on your Mac</h1>
          <div class="sub">
            A simple, Mac-first install page for Apple Silicon. Copy one command, verify the
            signed release binary when available, then move straight into onboarding, cap
            selection, and start.
          </div>

          <div class="actions">
            <a class="button button-primary" href="#command">Copy install command</a>
            <a class="button button-secondary" href="/docs">Open docs preview</a>
          </div>

          <div class="tags">
            <span class="tag">Apple Silicon</span>
            <span class="tag">signed binary</span>
            <span class="tag">checksum verified</span>
            <span class="tag">localhost preview</span>
          </div>
        </div>

          <div class="install-card" id="command">
          <div class="label">Install command</div>
          <div class="command">
            <div class="command-shell"><code id="install-command">Loading install manifest...</code></div>
            <button class="copy-btn" id="copy-button" type="button" disabled>Copy</button>
          </div>
          <div class="label" style="margin-top: 18px;">Install flow</div>
          <div class="install-list">
            <div class="install-step">
              <div class="num">1</div>
              <div>
                <strong>Download</strong>
                <p>Fetch the Mac release binary from the release channel.</p>
              </div>
            </div>
            <div class="install-step">
              <div class="num">2</div>
              <div>
                <strong>Verify</strong>
                <p>Checksum verification happens when the release artifact publishes one.</p>
              </div>
            </div>
            <div class="install-step">
              <div class="num">3</div>
              <div>
                <strong>Start</strong>
                <p>Review onboarding, set your cap, and then run <code>opengpu start</code>.</p>
              </div>
            </div>
          </div>
          <div class="manifest-status" id="manifest-status">Fetching ./install.json…</div>
          <div class="manifest-note">
            The install page is now a shell that reads the command and release metadata from
            <strong>./install.json</strong> so the HTML, installer, and release preview stay in
            sync.
          </div>
          <div class="footer" id="copy-status">
            Local preview only. Public domain comes later.
          </div>
        </div>
      </div>

      <div class="footer" style="margin-top: 22px;">
        Local preview URL: <code>${escapeHtml(appUrl)}${escapeHtml(installPath)}</code> • Docs preview:
        <code>${escapeHtml(appUrl)}/docs</code>
      </div>
    </div>
    <script>
      (async () => {
        const statusEl = document.getElementById("manifest-status");
        const commandEl = document.getElementById("install-command");
        const copyButton = document.getElementById("copy-button");
        const copyStatus = document.getElementById("copy-status");
        const manifestUrl = new URL("./install.json", window.location.href);

        try {
          const response = await fetch(manifestUrl, { headers: { Accept: "application/json" } });
          if (!response.ok) {
            throw new Error("HTTP " + response.status);
          }

          const manifest = await response.json();
          const installCommand = String(manifest.install_command ?? "");
          const docsPage = String(manifest.docs_page ?? "/docs/install");
          const releaseBaseUrl = String(manifest.release_base_url ?? "");
          const onboardingCommand = String(manifest.onboarding_command ?? "opengpu onboarding");
          const capCommand = String(manifest.cap_command ?? "opengpu cap");
          const startCommand = String(manifest.start_command ?? "opengpu start");

          if (commandEl) {
            commandEl.textContent = installCommand;
          }
          if (copyButton) {
            copyButton.disabled = false;
            copyButton.addEventListener("click", async () => {
              try {
                await navigator.clipboard.writeText(installCommand);
                if (copyStatus) {
                  copyStatus.textContent = "Copied to clipboard";
                }
              } catch (_) {
                if (copyStatus) {
                  copyStatus.textContent = "Copy failed; select and copy the command manually";
                }
              }
            });
          }
          if (statusEl) {
            statusEl.textContent = "Manifest loaded from ./install.json";
          }

          const footer = document.querySelector(".manifest-note");
          if (footer) {
            footer.innerHTML =
              "The install page is now a shell that reads the command and release metadata from " +
              "<strong>./install.json</strong> so the HTML, installer, and release preview stay in sync. " +
              "Follow up with <code>" +
              onboardingCommand +
              "</code>, <code>" +
              capCommand +
              "</code>, then <code>" +
              startCommand +
              "</code>. Docs preview: <code>" +
              docsPage +
              "</code>. Release source: <code>" +
              releaseBaseUrl +
              "</code>.";
          }
        } catch (error) {
          if (statusEl) {
            statusEl.textContent = "Manifest load failed";
          }
          if (commandEl) {
            commandEl.textContent = "Unable to load install manifest";
          }
        }
      })();
    </script>
  </body>
</html>`;
}

function page({ health, status, events, credits, error }) {
  const snapshot = status ?? health?.snapshot ?? {};
  const storageSource = health?.storage_source ?? snapshot.storage_source ?? "unknown";
  const supabase = health?.supabase ?? "unknown";
  const isHealthy = health?.status === "ok";
  const title = "NovusX Dashboard";

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta http-equiv="refresh" content="15" />
    <title>${title}</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --line-strong: rgba(15, 23, 42, 0.14);
        --text: #0f172a;
        --muted: #5f6b85;
        --green: #0f9d58;
        --orange: #c47f1b;
        --amber: #d97706;
        --red: #d14343;
        --blue: #3452ff;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 30%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
      }
      .wrap {
        max-width: 1380px;
        margin: 0 auto;
        padding: 22px 20px 48px;
      }
      .hero {
        border: 1px solid var(--line);
        background: rgba(255, 255, 255, 0.92);
        border-radius: 22px;
        padding: 24px;
        box-shadow: 0 18px 60px rgba(15, 23, 42, 0.06);
      }
      .topline {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 20px;
        flex-wrap: wrap;
      }
      h1 {
        margin: 0;
        font-size: clamp(40px, 5vw, 64px);
        line-height: 0.96;
        letter-spacing: -0.06em;
      }
      .sub {
        margin-top: 12px;
        color: var(--muted);
        line-height: 1.7;
      }
      .statusline {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
        margin-top: 18px;
      }
      .pill {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid transparent;
      }
      .pill-green { background: rgba(15, 157, 88, 0.08); color: var(--green); border-color: rgba(15, 157, 88, 0.16); }
      .pill-orange { background: rgba(196, 127, 27, 0.08); color: var(--orange); border-color: rgba(196, 127, 27, 0.16); }
      .pill-amber { background: rgba(217, 119, 6, 0.08); color: var(--amber); border-color: rgba(217, 119, 6, 0.16); }
      .pill-red { background: rgba(209, 67, 67, 0.08); color: var(--red); border-color: rgba(209, 67, 67, 0.16); }
      .pill-blue { background: rgba(52, 82, 255, 0.08); color: var(--blue); border-color: rgba(52, 82, 255, 0.16); }
      .pill-neutral { background: rgba(95, 107, 133, 0.08); color: var(--muted); border-color: rgba(95, 107, 133, 0.16); }
      .grid {
        display: grid;
        grid-template-columns: repeat(4, minmax(0, 1fr));
        gap: 14px;
        margin: 18px 0 24px;
      }
      .card {
        border: 1px solid var(--line);
        background: var(--surface);
        border-radius: 18px;
        padding: 16px;
      }
      .card-label {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .card-value {
        margin: 10px 0 12px;
        font-size: 30px;
        font-weight: 700;
      }
      .subhead {
        margin: 0 0 12px;
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
      }
      .section {
        margin-top: 24px;
        border: 1px solid var(--line);
        background: rgba(255, 255, 255, 0.92);
        border-radius: 22px;
        overflow: hidden;
        box-shadow: 0 18px 60px rgba(15, 23, 42, 0.05);
      }
      .section-head {
        padding: 16px 20px;
        border-bottom: 1px solid var(--line);
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 16px;
        flex-wrap: wrap;
      }
      .section-title {
        margin: 0;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 14px;
      }
      .section-body {
        padding: 20px;
      }
      .table .thead,
      .table .row {
        display: grid;
        grid-template-columns: 1.3fr 1fr 0.7fr 0.7fr 1fr 1.1fr 0.7fr;
        gap: 14px;
        align-items: start;
      }
      .table .thead {
        color: var(--muted);
        text-transform: uppercase;
        letter-spacing: 0.08em;
        font-size: 12px;
        padding-bottom: 12px;
        margin-bottom: 12px;
        border-bottom: 1px solid var(--line);
      }
      .table .row {
        padding: 14px 0;
        border-bottom: 1px solid rgba(15, 23, 42, 0.06);
      }
      .table .row:last-child { border-bottom: 0; }
      .meta {
        color: var(--muted);
        font-size: 12px;
        line-height: 1.4;
      }
      .empty {
        color: var(--muted);
        padding: 28px 0;
      }
      .events {
        display: grid;
        gap: 12px;
      }
      .balance-grid {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
        gap: 12px;
        margin-bottom: 18px;
      }
      .balance {
        border: 1px solid var(--line);
        border-radius: 14px;
        background: var(--surface);
        padding: 14px 16px;
      }
      .balance strong {
        display: block;
        margin-bottom: 6px;
        font-size: 14px;
        color: var(--text);
        overflow-wrap: anywhere;
      }
      .event {
        border: 1px solid var(--line);
        border-radius: 14px;
        background: var(--surface);
        padding: 14px 16px;
      }
      .event-top {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 12px;
        flex-wrap: wrap;
      }
      pre {
        overflow: auto;
        margin: 12px 0 0;
        color: #31415f;
        font-size: 12px;
        line-height: 1.5;
        white-space: pre-wrap;
        word-break: break-word;
      }
      .error {
        margin-top: 18px;
        border: 1px solid rgba(209, 67, 67, 0.22);
        background: rgba(209, 67, 67, 0.06);
        color: var(--red);
        padding: 14px 16px;
        border-radius: 14px;
      }
      .links {
        display: flex;
        gap: 10px;
        flex-wrap: wrap;
      }
      a {
        color: var(--blue);
        text-decoration: none;
      }
      a:hover { text-decoration: underline; }
      .brand {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 800;
        letter-spacing: 0.02em;
      }
      .brand-mark {
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }
      .badge {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid var(--line);
        color: var(--muted);
        background: rgba(255, 255, 255, 0.8);
      }
      @media (max-width: 1200px) {
        .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
        .table .thead,
        .table .row { grid-template-columns: 1.1fr 0.9fr 0.7fr 0.7fr 1fr 1fr 0.7fr; }
      }
      @media (max-width: 820px) {
        .grid { grid-template-columns: 1fr; }
        .table .thead { display: none; }
        .table .row {
          grid-template-columns: 1fr;
          gap: 10px;
          padding: 16px 0;
        }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="hero">
        <div class="topline">
          <div>
            <div class="brand"><span class="brand-mark"></span> NovusX Dashboard</div>
            <div class="sub">Live operator view for nodes, jobs, storage source, and audit trail.</div>
            <div class="statusline">
              ${badge(isHealthy ? "healthy" : "degraded", isHealthy ? "green" : "red")}
              ${badge(`storage: ${storageSource}`, storageSource === "supabase" ? "green" : "amber")}
              ${badge(`supabase: ${supabase}`, supabase.startsWith("enabled") ? "green" : "red")}
            </div>
          </div>
          <div class="links">
            <a href="${escapeHtml(appUrl)}/docs" target="_blank" rel="noreferrer">docs</a>
            <a href="${escapeHtml(appUrl)}/install" target="_blank" rel="noreferrer">install</a>
            <a href="${escapeHtml(controlPlaneUrl)}" target="_blank" rel="noreferrer">control plane</a>
            <a href="${escapeHtml(controlPlaneUrl)}/v1/status" target="_blank" rel="noreferrer">status json</a>
            <a href="${escapeHtml(controlPlaneUrl)}/v1/job-events" target="_blank" rel="noreferrer">job events</a>
            <a href="${escapeHtml(controlPlaneUrl)}/health" target="_blank" rel="noreferrer">health</a>
          </div>
        </div>

        <div class="grid">
          ${renderCounts(snapshot)}
        </div>

        ${error ? `<div class="error">${escapeHtml(error)}</div>` : ""}
      </div>

      <div class="section">
        <div class="section-head">
          <h2 class="section-title">Nodes</h2>
          <div class="meta">${formatCount(snapshot.nodes?.length ?? 0)} registered</div>
        </div>
        <div class="section-body">${renderNodes(snapshot.nodes ?? [])}</div>
      </div>

      <div class="section">
        <div class="section-head">
          <h2 class="section-title">Job Events</h2>
          <div class="meta">${formatCount(events.length)} events captured</div>
        </div>
        <div class="section-body">${renderEvents(events)}</div>
      </div>

      ${renderCredits(credits ?? {})}
    </div>
  </body>
</html>`;
}

function docsRoute(basePath, path = "") {
  const normalized = basePath.endsWith("/") ? basePath.slice(0, -1) : basePath;
  return `${normalized}${path}`;
}

function docsShell({ title, subtitle, active, body, basePath = "/docs" }) {
  const sections = [
    ["Overview", docsRoute(basePath), "overview"],
    ["Install", docsRoute(basePath, "/install"), "install"],
    ["Device identity", docsRoute(basePath, "/identity"), "identity"],
    ["Onboarding", docsRoute(basePath, "/onboarding"), "onboarding"],
    ["Credits", docsRoute(basePath, "/credits"), "credits"],
    ["Releases", docsRoute(basePath, "/releases"), "releases"],
  ];

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>${escapeHtml(title)}</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #ffffff;
        --surface: #fbfcff;
        --surface-2: #f5f7fb;
        --line: rgba(15, 23, 42, 0.09);
        --line-strong: rgba(15, 23, 42, 0.14);
        --text: #0f172a;
        --muted: #5f6b85;
        --blue: #3452ff;
        --green: #0f9d58;
        --amber: #c47f1b;
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        min-height: 100vh;
        background:
          radial-gradient(circle at top left, rgba(52, 82, 255, 0.06), transparent 28%),
          linear-gradient(180deg, var(--bg) 0%, var(--surface) 100%);
        color: var(--text);
        font-family: Inter, "SF Pro Text", "Segoe UI", sans-serif;
      }
      .wrap {
        max-width: 1240px;
        margin: 0 auto;
        padding: 22px 20px 48px;
      }
      .topbar {
        display: flex;
        justify-content: space-between;
        align-items: center;
        gap: 12px;
        flex-wrap: wrap;
        margin-bottom: 28px;
      }
      .brand {
        display: inline-flex;
        align-items: center;
        gap: 10px;
        font-weight: 800;
        letter-spacing: 0.02em;
      }
      .brand-mark {
        width: 14px;
        height: 14px;
        border-radius: 4px;
        background: linear-gradient(135deg, var(--blue), #5a79ff);
      }
      .badge {
        display: inline-flex;
        align-items: center;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        letter-spacing: 0.04em;
        text-transform: uppercase;
        border: 1px solid var(--line);
        color: var(--muted);
        background: rgba(255, 255, 255, 0.8);
      }
      .layout {
        display: grid;
        grid-template-columns: 280px minmax(0, 1fr);
        gap: 18px;
      }
      .sidebar,
      .content {
        border: 1px solid var(--line);
        border-radius: 22px;
        background: rgba(255, 255, 255, 0.92);
        box-shadow: 0 18px 60px rgba(15, 23, 42, 0.06);
      }
      .sidebar {
        padding: 18px;
        position: sticky;
        top: 20px;
        height: fit-content;
      }
      .content {
        padding: 24px;
      }
      .nav {
        display: grid;
        gap: 8px;
        margin-top: 16px;
      }
      .nav a {
        display: block;
        padding: 12px 14px;
        border-radius: 12px;
        color: var(--text);
        text-decoration: none;
        border: 1px solid transparent;
        background: var(--surface);
      }
      .nav a:hover {
        border-color: var(--line-strong);
      }
      .nav a.active {
        border-color: rgba(52, 82, 255, 0.18);
        background: rgba(52, 82, 255, 0.06);
      }
      h1 {
        margin: 0;
        max-width: 12ch;
        font-size: clamp(42px, 5vw, 68px);
        line-height: 0.96;
        letter-spacing: -0.06em;
      }
      .subtitle {
        margin-top: 14px;
        color: var(--muted);
        line-height: 1.7;
        max-width: 70ch;
      }
      .cards {
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 16px;
        margin-top: 22px;
      }
      .card {
        border: 1px solid var(--line);
        border-radius: 18px;
        background: var(--surface);
        padding: 18px;
      }
      .card h2,
      .card h3 {
        margin: 0 0 10px;
        font-size: 14px;
        text-transform: uppercase;
        letter-spacing: 0.08em;
      }
      .card p,
      .card li,
      .card ol {
        color: var(--muted);
        line-height: 1.7;
      }
      .card ul,
      .card ol {
        margin: 0;
        padding-left: 18px;
      }
      .mono {
        color: var(--text);
        white-space: nowrap;
      }
      a {
        color: var(--blue);
        text-decoration: none;
      }
      a:hover { text-decoration: underline; }
      code {
        color: var(--text);
        white-space: nowrap;
        background: rgba(52, 82, 255, 0.06);
        padding: 0 4px;
        border-radius: 4px;
      }
      .footer {
        margin-top: 18px;
        color: var(--muted);
        font-size: 12px;
        line-height: 1.6;
      }
      @media (max-width: 900px) {
        .layout { grid-template-columns: 1fr; }
        .sidebar { position: static; }
        .cards { grid-template-columns: 1fr; }
      }
    </style>
  </head>
  <body>
    <div class="wrap">
      <div class="topbar">
        <div class="brand"><span class="brand-mark"></span> NovusX Docs</div>
        <div class="badge">localhost preview • local layout</div>
      </div>
      <div class="layout">
        <aside class="sidebar">
          <div class="mono">Mac-first docs</div>
          <div class="nav">
            ${sections
              .map(
                ([label, href, key]) => `
                  <a class="${active === key ? "active" : ""}" href="${escapeHtml(href)}">${escapeHtml(label)}</a>`,
              )
              .join("")}
          </div>
          <div class="footer">
            This local site mirrors the release docs shape while the flow stays localhost-only.
          </div>
        </aside>
        <main class="content">
          <h1>${escapeHtml(title)}</h1>
          <div class="subtitle">${subtitle}</div>
          ${body}
          <div class="footer">
            Local preview URL: <code>${escapeHtml(appUrl)}</code>
          </div>
        </main>
      </div>
    </div>
  </body>
</html>`;
}

function renderDocsHome(basePath = "/docs") {
  return docsShell({
    title: "NovusX Docs",
    subtitle:
      "A Mac-first public docs surface for install, identity, onboarding, credits, and release flow. This preview is local, but the copy is written as the public source of truth.",
    active: "overview",
    basePath,
    body: `
      <div class="cards">
        <div class="card">
          <h2>Install</h2>
          <p>
            One-line install command, checksum verification, and what the contributor sees
            after download.
          </p>
          <p><a href="${escapeHtml(docsRoute(basePath, "/install"))}">Open install page</a></p>
        </div>
        <div class="card">
          <h2>Device identity</h2>
          <p>
            Explain how the Mac identity survives reinstall, why the private key is not
            exportable, and what metadata is signed.
          </p>
          <p><a href="${escapeHtml(docsRoute(basePath, "/identity"))}">Open identity page</a></p>
        </div>
        <div class="card">
          <h2>Onboarding</h2>
          <p>
            The first-run checklist for a contributor machine: identity, cap, model, and
            start flow.
          </p>
          <p><a href="${escapeHtml(docsRoute(basePath, "/onboarding"))}">Open onboarding page</a></p>
        </div>
        <div class="card">
          <h2>Credits</h2>
          <p>
            Append-only ledger rules for contributor balances and how job awards are tracked.
          </p>
          <p><a href="${escapeHtml(docsRoute(basePath, "/credits"))}">Open credits page</a></p>
        </div>
        <div class="card">
          <h2>Releases</h2>
          <p>
            Mac-first localhost install flow, signed artifacts, and how the release page maps to
            the installer.
          </p>
          <p><a href="${escapeHtml(docsRoute(basePath, "/releases"))}">Open releases page</a></p>
        </div>
        <div class="card">
          <h2>Dashboard</h2>
          <p>
            Operator view, live status, and the install preview are still available in the
            dashboard app.
          </p>
          <p><a href="/">Open dashboard</a></p>
        </div>
      </div>
    `,
  });
}

function renderDocsInstall(basePath = "/docs") {
  return docsShell({
    title: "Install NovusX",
    subtitle:
      "The install page is the first touch for contributors. For now it stays localhost-only, keeps the command identical everywhere, and points to onboarding and cap selection immediately after install.",
    active: "install",
    basePath,
    body: `
      <div class="cards">
        <div class="card">
          <h2>Canonical command</h2>
            <p><code>${escapeHtml(installCommand)}</code></p>
            <p>That command should match the installer script, the docs, and the localhost preview.</p>
        </div>
        <div class="card">
          <h2>Expected flow</h2>
          <ol>
            <li>Download the Mac-first release binary.</li>
            <li>Verify checksum when available.</li>
            <li>Run <code>opengpu onboarding</code>.</li>
            <li>Choose a contribution cap with <code>opengpu cap</code>.</li>
            <li>Start with <code>opengpu start</code>.</li>
          </ol>
        </div>
      </div>
    `,
  });
}

function renderDocsIdentity(basePath = "/docs") {
  return docsShell({
    title: "Device Identity",
    subtitle:
      "The Mac identity is a sign-only encrypted-at-rest fallback today. The app never reads raw private-key bytes, and reinstall should reuse identity as long as the NovusX data directory remains intact.",
    active: "identity",
    basePath,
    body: `
      <div class="cards">
        <div class="card">
          <h2>What survives reinstall</h2>
          <ul>
            <li>identity record</li>
            <li>public key</li>
            <li>fingerprint</li>
            <li>hostname metadata</li>
          </ul>
        </div>
        <div class="card">
          <h2>What is not exposed</h2>
          <ul>
            <li>raw private key bytes</li>
            <li>exportable app-visible secret</li>
            <li>hostname as identity proof</li>
          </ul>
        </div>
      </div>
    `,
  });
}

function renderDocsOnboarding(basePath = "/docs") {
  return docsShell({
    title: "Onboarding",
    subtitle:
      "The first-run checklist keeps the Mac-first path understandable: review identity, choose a cap, confirm the model, and only then go live.",
    active: "onboarding",
    basePath,
    body: `
      <div class="cards">
        <div class="card">
          <h2>Checklist</h2>
          <ol>
            <li>Review device identity.</li>
            <li>Choose the contribution cap.</li>
            <li>Confirm the active model.</li>
            <li>Run <code>opengpu start</code>.</li>
          </ol>
        </div>
        <div class="card">
          <h2>Policy note</h2>
          <p>The node should remain paused until the cap is set and the policy allows work.</p>
        </div>
      </div>
    `,
  });
}

function renderDocsCredits(basePath = "/docs") {
  return docsShell({
    title: "Credits",
    subtitle:
      "Credits are tracked as an append-only ledger on the control plane. They belong to the contributor account, not the hostname or the device key itself.",
    active: "credits",
    basePath,
    body: `
      <div class="cards">
        <div class="card">
          <h2>Ledger rules</h2>
          <ul>
            <li>job completion writes a ledger entry</li>
            <li>balances roll up by contributor account</li>
            <li>device and hostname stay attached for audit</li>
          </ul>
        </div>
        <div class="card">
          <h2>What users should expect</h2>
          <p>NovusX should show earned credits clearly and make the contributor balance easy to inspect in the dashboard.</p>
        </div>
      </div>
    `,
  });
}

function renderDocsReleases(basePath = "/docs") {
  return docsShell({
    title: "Releases",
    subtitle:
      "The release surface stays Mac-first and localhost-only for now. The install page, installer script, manifest endpoint, and signed binary artifacts should always agree on the same release source.",
    active: "releases",
    basePath,
    body: `
      <div class="cards">
        <div class="card">
          <h2>Source of truth</h2>
          <p>The install command, checksum, manifest, and release asset must point at the same Mac-first localhost build.</p>
        </div>
        <div class="card">
          <h2>Review rule</h2>
          <p>Any release-page copy change should be checked against the installer script and release docs.</p>
        </div>
      </div>
    `,
  });
}

async function collectData() {
  const [health, status, events, credits] = await Promise.all([
    fetchJson("/health"),
    fetchJson("/v1/status"),
    fetchJson("/v1/job-events"),
    fetchJson("/v1/credits"),
  ]);

  return { health, status, events, credits, error: null };
}

createServer(async (req, res) => {
  const requestUrl = new URL(req.url ?? "/", appUrl);
  if (requestUrl.pathname === "/install") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderInstallPage("/install"));
    return;
  }

  if (requestUrl.pathname === "/install.json") {
    res.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
    res.end(JSON.stringify(installManifest("/install"), null, 2));
    return;
  }

  if (requestUrl.pathname === "/public/install") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderInstallPage("/public/install"));
    return;
  }

  if (requestUrl.pathname === "/public/install.json") {
    res.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
    res.end(JSON.stringify(installManifest("/public/install"), null, 2));
    return;
  }

  if (requestUrl.pathname === "/docs" || requestUrl.pathname === "/docs/") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsHome());
    return;
  }

  if (requestUrl.pathname === "/docs/install") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsInstall());
    return;
  }

  if (requestUrl.pathname === "/docs/identity") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsIdentity());
    return;
  }

  if (requestUrl.pathname === "/docs/onboarding") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsOnboarding());
    return;
  }

  if (requestUrl.pathname === "/docs/credits") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsCredits());
    return;
  }

  if (requestUrl.pathname === "/docs/releases") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsReleases());
    return;
  }

  if (requestUrl.pathname === "/public/docs" || requestUrl.pathname === "/public/docs/") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsHome("/public/docs"));
    return;
  }

  if (requestUrl.pathname === "/public/docs/install") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsInstall("/public/docs"));
    return;
  }

  if (requestUrl.pathname === "/public/docs/identity") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsIdentity("/public/docs"));
    return;
  }

  if (requestUrl.pathname === "/public/docs/onboarding") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsOnboarding("/public/docs"));
    return;
  }

  if (requestUrl.pathname === "/public/docs/credits") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsCredits("/public/docs"));
    return;
  }

  if (requestUrl.pathname === "/public/docs/releases") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderDocsReleases("/public/docs"));
    return;
  }

  if (requestUrl.pathname === "/portal") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderContributorPortal());
    return;
  }

  if (requestUrl.pathname === "/portal/jobs") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderContributorJobHistoryPage(requestUrl, "/portal"));
    return;
  }

  if (requestUrl.pathname === "/public/portal") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderContributorPortal());
    return;
  }

  if (requestUrl.pathname === "/public/portal/jobs") {
    res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    res.end(renderContributorJobHistoryPage(requestUrl, "/public/portal"));
    return;
  }

  let data;
  try {
    data = await collectData();
  } catch (error) {
    data = {
      health: null,
      status: null,
      events: [],
      credits: null,
      error: error instanceof Error ? error.message : String(error),
    };
  }

  res.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
  res.end(page(data));
}).listen(port, "127.0.0.1", () => {
  process.stdout.write(
    `NovusX dashboard listening on http://127.0.0.1:${port} (proxying ${controlPlaneUrl})\n`,
  );
});
