---
name: frontend-runtime-acceptance
description: Repair and verify frontend runtime behavior, including React/Vue/Svelte/Angular rendering, modal scrolling, responsive layout, missing imports, console exceptions, and failed browser interactions.
---

# Frontend Runtime Acceptance

Use this skill whenever a frontend is created or changed, or when the user reports a visual, interaction, scrolling, responsive, import, or browser runtime problem.

Treat the user's reported flow as the primary acceptance criterion. Reproduce that exact flow before editing when practical, then inspect the owning frontend package, its manifest, and the directly relevant source. Keep focused defects on one worker when the same component, entry point, or dependency boundary owns both the cause and the fix.

Build from the directory containing the frontend `package.json`, or use an explicit package prefix. Resolve missing imports and exports against that manifest. A successful backend command does not count as a frontend build.

After the final source change:

1. Create or update a project-owned regression test for the reported behavior and run it successfully.
2. Run the frontend production build successfully.
3. Start the development or preview server in the background on `127.0.0.1` and poll the exact URL until it returns HTTP success.
4. Open that URL in browser automation and wait for the page to load.
5. Exercise the user's exact interaction, including opening the relevant modal, panel, menu, form, or route.
6. Check meaningful visible content, the browser console, failed API requests, and DOM/layout state.
7. Repeat at a desktop viewport around 1440 pixels wide and a narrow viewport around 850 pixels wide. Confirm the changed flow is usable and there is no accidental horizontal overflow.
8. If any check fails, use the concrete browser or console result to repair the source, then repeat the test, build, and browser flow. Do not stop after merely explaining the failure.
9. Stop every server or test process started for acceptance.

When the browser capability is exposed through `execute_code`, drive the browser and perform all checks in that tool call. After the checks pass, emit this exact structured object as tool output:

```json
{
  "MUNDUSX_BROWSER_ACCEPTANCE_V1": true,
  "rendered": true,
  "flow_exercised": true,
  "console_errors": [],
  "desktop_checked": true,
  "narrow_checked": true
}
```

Emit the object only from the browser execution result after observing every field. Never write it in prose or use it to conceal a failed check.
