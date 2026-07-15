# Contributor App Parity

The CLI can stay available for power users and automation, but ordinary
contributors should not need PowerShell to use MundusX. Any contributor-facing
capability added to `opengpu` must also have an app surface, or the PR must
document why the capability is intentionally script-only.

## Rule

Every contributor workflow should be available in both places:

- `opengpu` CLI for scripting, diagnostics, and recovery.
- Windows tray / setup / onboarding app for normal contributor use.

The app can call the CLI internally, but it must expose the workflow with clear
labels, status, errors, and next steps.

## Current Parity Matrix

| Capability | CLI Surface | App Surface Required |
| --- | --- | --- |
| Install binaries and runtime | `install.ps1`, release assets | Clickable setup EXE |
| Choose control plane URL | `opengpu install` | Setup/onboarding screen |
| Choose contribution cap | `opengpu install`, `opengpu cap` | Setup/onboarding screen plus later settings screen |
| Detect backend | `opengpu status`, `opengpu doctor`, `opengpu start` | Read-only status panel with detected backend and reason |
| Choose backend preference | `opengpu start --backend`, config | Settings screen |
| Choose/download official model | `opengpu model use`, `opengpu model add` | Model picker with fit/reason labels |
| Import local GGUF model | `opengpu model import` | Advanced model import screen |
| Show active model/cache | `opengpu model list` | Model status screen |
| Start contribution foreground | `opengpu start` | Start button with live state |
| Start background contribution | `opengpu start --background` | Start in background toggle/button |
| Pause contribution | `opengpu pause` | Tray menu and main app button |
| Resume contribution | `opengpu resume` | Tray menu and main app button |
| Disconnect / cool GPU | `opengpu exit`, `opengpu disconnect` | Stop/disconnect button that confirms GPU runtime is cooled |
| Show readiness | `opengpu status` | Home/status card with ready/not-ready reason |
| Show policy rejection | `opengpu status`, control-plane response | Status card with red/yellow reason text |
| Show logs | `opengpu logs` | Logs page or "Open logs" action |
| Run doctor | `opengpu doctor` | Diagnostics page |
| Onboarding checklist | `opengpu onboarding` | First-run onboarding wizard |
| Complete/reset onboarding | `opengpu onboarding --complete/reset` | Onboarding review/redo controls |
| Update/reinstall | `opengpu update`, setup EXE | Update screen or setup relaunch |

## App Behavior Requirements

- The app must never hide a control-plane rejection behind a generic success
  message.
- The app must show the same readiness fields the CLI prints: backend, active
  model, contribution cap, policy state, and ready-for-jobs state.
- The app must show whether the GPU runtime is warm or cooled.
- The app must use the same local config files as the CLI instead of maintaining
  a separate source of truth.
- The app must keep actions idempotent. Clicking Start, Pause, Resume, or Stop
  repeatedly should not corrupt local state.

## Implementation Direction

Prefer a shared local command layer:

1. Keep CLI commands as the stable automation API.
2. Let the tray/app invoke CLI subcommands for now.
3. Move repeated logic into shared Rust modules when app flows become richer.
4. Add app parity checks whenever a new contributor CLI command is introduced.

This keeps the CLI honest without forcing users to learn it.
