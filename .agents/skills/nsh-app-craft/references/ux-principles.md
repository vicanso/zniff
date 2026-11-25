# UX Principles & Guardrails ("Complexity Stays in the System")

Audit every flow and surface against these 8 rules before shipping:

## 1. System Infers When Possible
- Never push detectable state onto the user (e.g., detected external/existing certificates, running simulators, read-only snapshot modes, already-satisfied prerequisites).
- Demote redundant setup CTAs to secondary style or hide them once satisfied; elevate the actual remaining step.

## 2. Hide What Is Not Needed Right Now
- Use progressive disclosure for secondary/advanced options (e.g., collapse auxiliary setup strips unless an option inside them is active).
- When a container displays an empty state with no actionable items, disable or omit destructive bulk buttons rather than letting users click no-ops.

## 3. Act Automatically Instead of Nagging
- Complete obvious prerequisite transitions automatically (e.g., enabling "route system proxy" while capture is stopped starts capture in the same user gesture).
- Provide in-context recovery actions (e.g., single-host TLS bypass, jump to isolated launcher, repair invalid input) right on error banners rather than forcing navigation hunts.

## 4. Same Action, Same Surface Everywhere
- If a capability exists in a primary button, row context menu, and detail menu (e.g., inline mocking), route all entry points to the same rich workflow—never send context-menu users to a degraded bare form while buttons open a rich editor.
- Context menus on list rows must track a temporary `picked` row ID without mutating active workspace selection, clearing unsaved compose drafts, or overwriting detail panes unless an action explicitly selects and opens the flow.

## 5. Truthful, Immediate Feedback & Reversibility
- **Truthful status**: Never report partial setup (e.g., installed but untrusted CA) as "Ready/Done"; never leave a "Default / None" preset highlighted when the underlying configuration is actually customized.
- **Tactile & local confirmation**: Copying a URL, header, or snippet gives in-place visual feedback (e.g., a 1.4s `Check` icon swap on the clicked button) without shifting sibling layout width.
- **Buffered edits & Undo/Cancel**: Multi-step popovers, host allow-lists, auth credentials, and inline notes edit inside a local `draft` signal with explicit `Apply` / `Save` / `Cancel` / `Reset` controls—so typing or cancelling never mutates live state prematurely.

## 6. Never Block the UI Thread
- Move blocking OS probes (GPU info, hardware/cert/subprocess lookups) onto background threads that update reactive signals (`"…"` until settled), keeping modal opens instant.
- Sensitive fields (proxy passwords, tokens) default `.masked(...)` with an explicit Show/Hide toggle.

## 7. Frequent Actions Effortless, Destructive Actions Guarded
- Every destructive or data-clearing action (clear unstarred flows, remove rule, discard draft, delete marker/watch rule, reset access list) requires either:
  - Undo toast window (`5s` reversible), OR
  - Explicit **two-step inline confirmation** (`Confirm?` / `"再次点击确认..."`) with an auto-reset timer (`~3500ms`) and auto-cancel when closing the popover/menu or editing the draft.

## 8. Self-Explanatory Copy & Terminology Hygiene
- **One concept, one term**: Never mix near-synonyms across surfaces (e.g., keep one consistent verb for "filter" everywhere; distinguish primary vs secondary filter only when a primary filter is actually active).
- **Locale punctuation & tone**: In Chinese copy, use full-width `（）`, `：`, and Unicode ellipsis `…` (never ASCII `...`); keep error reasons actionable rather than dumping raw stack internals.
- Wrap live countdowns/TTLs (e.g., breakpoint timeout `59s`) with `.tabular_nums()` so digits never jitter horizontally.
