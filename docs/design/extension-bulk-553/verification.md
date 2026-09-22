# Bulk extension action verification — #553 / PR #669

Verified on macOS, 2026-09-22, after integrating #550 from dev. The browser fixture used real ExtensionResults, ExtensionBulkActions, HostConfirmation and app styles. Only core host wrappers were stubbed. Twelve Kustomizations were selected, three suspended; the accepted writes were held until explicitly released. One write returned a transport error with a reason longer than 1,000 characters.

## UI evidence

Chrome driven through Playwright, desktop 1280×900 and narrow 390×844. Screenshots were inspected; light, paper, dark, midnight and high-contrast themes were checked.

- Keyboard Enter on Reconcile moved focus into the review. Escape closed it and restored Reconcile. Confirmation moved focus to Cancel remaining, and completion moved it to Done. Cancel dismissal is also covered by a keyboard regression test.
- The real predicate path displayed `applies to 9 of 12`, listed nine resources and confirmed nine. No injected availability callback was used.
- Four operations started, with five pending. Cancel remaining prevented the five queued writes; the four started operations completed. The result said `Partial: 3 accepted, 1 failed, 5 not requested.` The transport failure appeared under Failed; its reason was escaped and bounded to 80 characters.
- Switching cluster during a held run allowed exactly the four started old-cluster operations to finish. No queued old-cluster writes started. Selecting resources in the new cluster showed neither the previous progress nor its result.
- At 390px every theme had no horizontal document overflow. The failure reason wrapped inside the panel; focus remained on Done. Resource names stayed on one line.

[Running operation](running.png) · [Dark result](result-dark.png) · [High-contrast result](result-contrast.png)

This checks the browser layout and interaction, not a native Tauri window or real-cluster admission behavior.

## Automated evidence

TDD regressions were observed failing before fixes:

- 9 failures reproduced the old scope queue, missing review focus, rejection wording, empty-message success and unbounded reasons.
- 2 failures reproduced missing real availability integration and unavailable retry after a resource read failure.
- 1 independent-review regression reproduced obsolete inspection queues continuing after selection changed.

Final checks:

- Extension tests: 9 files, 148 tests passed.
- Full frontend coverage: 409 files, 6,792 tests passed; 91.27% lines, 84.49% branches, 85.72% functions. Existing jsdom/xterm canvas warnings remain.
- `pnpm typecheck`: all four projects passed.
- `pnpm build`: passed; existing large-chunk warning remains.
- `cargo test --workspace`: passed, including the TUI suite; live-cluster ignored tests were not run.
- Independent review identified the obsolete inspection queue and found no other important regressions; its regression passed after cancellation was added.
- GitNexus refreshed before edits and before final change analysis. Its call graph supports impact analysis; final refresh reported the full-text search index unavailable.
