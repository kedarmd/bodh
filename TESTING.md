# Validation and GUI evaluation

The current runner targets `spikes/native-sequence` and the headless `spikes/pr-capabilities` behavioral harness. Live read-only GitHub compatibility acquisition is opt-in; see [the capability reproduction command](spikes/pr-capabilities/README.md). Add future harnesses explicitly as they are implemented. Run it from any directory using its path; it resolves the repository root itself.

| Command from the repository root | Purpose |
| --- | --- |
| `./scripts/validate` | Runner regression tests, Rust formatting, headless typecheck, Clippy and behavior tests |
| `./scripts/validate desktop` | Core checks plus native GPUI typecheck, Clippy and full tests; opens no GUI |
| `./scripts/validate gui` | Build and run the interactive demonstration in release mode |
| `./scripts/validate profile` | Build and run the release native CPU redraw diagnostic |

Install Bash, Git, Python 3 and rustup. `rust-toolchain.toml` pins Rust and its rustfmt/Clippy components; rustup installs them on first use. Cargo uses the committed lockfile. Desktop builds additionally need Linux C/C++ build tools, pkg-config, fontconfig/freetype, xkbcommon and Wayland/X11 development libraries. The CI workflow records the Ubuntu package recipe. GUI/profile modes additionally need a working graphical session and Vulkan driver; a successful desktop build alone verifies neither.

The runner prints revision, dirty state, system, Rust/Cargo versions and target directory. Preserve its output with evidence. It stops at the first failed command and propagates the failing status. GUI/profile modes use `cargo run --release --locked`, which builds before launching; a failed build ends the run instead of falling through to an older executable. Both modes are opt-in and are excluded from CI.

## CI

`.github/workflows/validate.yml` runs on pushes, pull requests and manual dispatch. Core and Ubuntu desktop jobs call the same runner used locally. Checkout has read-only repository permissions and does not persist credentials. Both jobs install the pinned toolchain; the desktop job installs native development libraries. Neither job launches the app, manipulates the review checkout, or measures GPU/input performance. Hosted-runner success must be observed in Actions; a local pass is not evidence that the Ubuntu job passed.

The runner tests replace external tool invocations to check failure propagation, argument handling and launch policy. Real Cargo checks/tests verify the Rust code separately. These checks run automatically through the workflow once this branch is pushed; branch-protection requirements are a separate repository setting.

## Native GUI trial

1. State the trial's purpose and visual fidelity before opening it. The current spike evaluates rendering and immutable-source interaction. Its outlined controls and monospace chrome are unapproved diagnostic UI, not a product design proposal.
2. Run `./scripts/validate desktop`, then `./scripts/validate gui`. The first release build can take several minutes. During long builds, report the active stage, completed checks and remaining work at least once per minute; distinguish compilation from a running demo.
3. Try selection and pane dragging on the ordinary, 120-step and 1,000-step fixtures. Verify the selected source excerpt, line numbers and explanation against the fixture. Include returns, self-calls, notes and both conditional branches.
4. Scroll horizontally and vertically, select partially visible rows, switch flows and return. Check source identity, retained selection/scroll state, window shrinking, pane resizing, themes, keyboard navigation and visible focus. A hit on empty space must not select unrelated source.
5. Record user feedback separately from test results. A process staying alive is a smoke observation; responsiveness and source-selection acceptance require interaction evidence. Record any failed trial before subsequent successful feedback.

For GUI automation, use a dedicated graphical session when available. Validate the target PID/window and fresh geometry before input or screenshots. Coordinate automation with the user so it does not compete with their pointer or keyboard. Capture only the target window. Keep PID, build/revision, logs, exit status/signal, session/backend and driver versions with crash evidence. An unexplained termination remains unexplained even if later trials succeed.

## Performance evidence

Use `./scripts/validate profile` in the recorded Linux graphical environment. The diagnostic reports its viewport, fixture names, participant/interaction counts, warmup/sample counts and per-flow selection/resize timings. Its frame-budget result is diagnostic output, not a CI assertion or a frozen product/data-size limit; exceeding it currently does not change the process exit code.

Distinguish three measurements:

- Geometry microbenchmarks measure the layout algorithm, not UI responsiveness.
- The native CPU diagnostic measures state updates, layout, text/path painting and per-frame arena cleanup. GPU/compositor presentation and physical input latency are excluded.
- A live trial observes the user experience. Preserve that feedback alongside the narrower measurements.

For comparisons, build both revisions in the same mode with the same toolchain, lockfile, viewport, fixtures and machine/backend. Stop concurrent compilation and other substantial workloads before measuring. Include warmup/sample counts and timing scope. Preserve raw results and the source revisions; label dirty-tree results explicitly.

Give baseline packages distinct Cargo identities or separate `CARGO_TARGET_DIR` directories. Reusing one target directory for two copies of an identically named package caused a stale-library/binary comparison during the spike. Run a benchmark only after its build succeeds, through Cargo. For repeatable runs, finish the build first, then invoke the runner; retain the version header with the output.

When modifying the diagnostic, follow GPUI's normal frame lifecycle: return the `ArenaClearNeeded` token from the window update and clear it before the next draw. Include cleanup in the timer. Review evidence for that lifecycle and the exact measured layer; a fast number from an incomplete frame is insufficient.

The accepted spike's measurements, failed trials, successful user feedback and limitations are preserved in [EVIDENCE.md](spikes/native-sequence/EVIDENCE.md). Product visual design still needs its own proposal and approval.
