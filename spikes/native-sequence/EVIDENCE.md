# Evidence and decision

## Decision

The small custom deterministic Rust sequence renderer is the candidate for this spike; it is not an accepted product renderer. The initial live trial failed the interaction-performance gate. No inspected dependency qualifies as a direct replacement for the exact semantics. This does not establish a production performance budget or freeze numeric contract caps.

## Dependency comparison

Inspected the published crate sources using `cargo info gpui@0.2.2`, `cargo info layout-rs@0.1.3`, and `cargo info nereid@0.9.0`; candidates were not added to the application dependencies.

| Required behavior | Custom renderer | layout-rs 0.1.3 | Nereid 0.9.0 |
| --- | --- | --- | --- |
| Declared participant order | Vector order | General graph placement, not lifeline columns | `layout_sequence` assigns lexical ObjectId order, not declaration order |
| Declared step order / repeated interactions | Flattened ordered interactions, including branches | Graph edges are not an ordered event timeline | Message order is `(order_key, message_id)` |
| Message / return / self-call | Native solid/dashed/loopback paths | General edge/self-edge routing; no sequence semantics | Sequence messages and loop spacing exist |
| Note / top-level alt / opt | Fixed native note and enclosing frame/branch rules | No sequence note/conditional model | Sequence AST/block support, terminal grid output; would still need a native adapter |
| Long labels / native hit regions | Shared deterministic row bounds; native text wrapping | Requires a new sequence constraint layer and native backend | Spacing budgets exist; not native GPUI hit regions |
| Reused pinned inspector / themes / changed cues | Application-owned | Application-owned | Application-owned |
| Suitable dependency | Chosen, bounded implementation | MIT, but incompatible layout abstraction | License excludes incorporation/product use without separate written permission |

Primary sources:

- [layout-rs API](https://docs.rs/layout-rs/0.1.3/layout/): GraphViz graph construction/layout, `VisualGraph`, nodes and edges; its [source](https://github.com/nadavrot/layout) implements topological placement rather than sequence lifelines. Self-edges alone do not supply ordered sequence semantics. Adding event nodes/constraints and frames would recreate the custom renderer around this engine.
- [Nereid sequence layout source](https://docs.rs/crate/nereid/0.9.0/source/src/layout/sequence.rs): `layout_sequence` explicitly documents lexical participant columns and ordered messages. [License](https://docs.rs/crate/nereid/0.9.0/source/LICENSE) prohibits incorporation into another project and product use without permission. No code was copied or adapted.
- [GPUI 0.2.2](https://docs.rs/gpui/0.2.2/gpui/): application elements, scrolling and native path painting. It supplies UI mechanics, not sequence layout.

No candidate was installed as a runtime dependency merely to claim a comparison. The mismatches above are source-inspection evidence, not claimed execution results.

## Recorded environment

2026-11-03 session (local timezone Asia/Kolkata): CachyOS rolling, Linux 7.2.7-1-cachyos x86_64; Intel Core i5-1035G1, 8 logical CPUs, 7.3 GiB RAM. Rust 1.99.0 (`b940084d7`), Cargo 1.99.0 (`5f94df478`). Hyprland 0.56.2, Wayland session, 1920×1080 output at scale 1. `pkg-config`: xkbcommon 1.13.2, wayland-client 1.26.0, Vulkan 1.4.357. GPUI pinned to 0.2.2; blade-graphics 0.7.1, blade-util 0.3.0 (see Cargo.lock for complete dependency versions). X11 is compiled by GPUI's defaults; it is not thereby verified.

Concrete compatibility problem: initial resolution selected libc 0.2.191; xattr 0.2.3 (transitive through GPUI's archive support) failed to compile because `libc::ENOATTR` was removed. Locking libc to 0.2.186 allowed `cargo check` to pass. Do not run an unreviewed `cargo update` as a reproduction step. Upstream proc-macro-error2 2.0.1 also emits a future-incompatibility notice. Neither problem contradicts the approved native-rendering baseline.

## Measurements and checks

Core tests exercise the public renderer/hit boundary with real immutable fixtures: blank space cannot select source; all 1,000 stress interaction rectangles resolve in order; branch interactions share one enclosing alt frame; a last-column self-call stays inside scrollable bounds. Red/green cycles caught missing API, separate alt frames and a clipped rightmost self-call before fixes.

Release layout/hit timings are measured by `examples/measure.rs`; they exclude GPU rendering, text shaping, allocation of GPUI elements, input latency and source inspection. Exact current run results and native observations follow below. Linear hit lookup is quadratic when querying every row; the GUI uses GPUI's element hit dispatch for numbered row rectangles, not this exhaustive benchmark loop.

## Limits and remaining work

- The demo trusts handcrafted data. It is not a schema validator: empty/invalid references, alt branch cardinality, arbitrary malformed inputs and real Git diff/source mapping belong to later shared services.
- Source is an embedded fixture pair with equal line counts. The changed-line comparison is intentionally not a rename/insertion/deletion mapper. Head/base fixture identity is explicit; there is no fabricated Git SHA.
- Layout reserves a conservative monospace character budget rather than measuring shaped glyphs. International text, unbroken strings, font substitution and accessibility scaling need further measurement before a production text algorithm is selected.
- The initial renderer constructed all rows/elements on each render and tessellated full-height dashed lifelines. User feedback exposed severe lag. The follow-up caches immutable geometry and builds labels/hit targets only for the actual viewport during native prepaint, with paths clipped before tessellation. Larger-flow limits still require measurement.
- At small window widths the minimum 260-pixel source pane leaves little diagram space; horizontal scrolling preserves geometry and hit targets. This is an observed design tradeoff, not a hidden participant cap.
- No persistence across restarts, syntax highlighting, remote PR acquisition or production source validation is claimed by this bounded spike.
- User reported a termination during clicking; successful interaction feedback is still pending. See the reliability record below. Do not mark the issue complete.

## Release measurement run

Single release run on the environment above, while the first desktop build was compiling (not a statistically controlled benchmark):

| Participants / steps | Scene logical pixels | Layout | Exhaustive row hit checks |
| --- | --- | --- | --- |
| 3 / 7 long-label steps | 840 × 1,516 | 2.679 μs | 465 ns |
| 12 / 120 | 3,000 × 25,122 | 6.468 μs | 13.741 μs |
| 24 / 1,000 | 5,880 × 213,366 | 65.323 μs | 729.214 μs |
| 48 / 10,000 | 11,640 × 2,155,994 | 895.199 μs | 80.081 ms |

The 10,000-step result illustrates that fast geometry allocation does not imply fast exhaustive querying or practical GUI rendering. It is a core measurement only; that size is not offered as a GUI fixture or asserted as a supported limit.

## Code review

Reviewed against starting commit `7f19442` using separate Standards and Spec review agents.

### Standards

No documented-standard violations. One initial Data Clumps heuristic (parallel per-flow vectors) was fixed by grouping selection and scroll handles with their flow. Follow-up: zero remaining findings.

### Spec

Two initial findings: retained inspector width could exceed a resized viewport; resetting inspector scroll could hide selected lines. Fixed by clamping effective width on every render and placing the selected source excerpt before explanation. Follow-up: zero remaining code findings. Native visual verification and human interaction feedback are separate acceptance work.

## Native observations and reliability feedback

Live GPUI/Wayland rendering was inspected at 1280×800, the compositor's tiled 947×1016, and a floating 640×480. Captures: [dark selected](evidence/dark-selected.png), [light selected](evidence/light-selected.png), [120-step stress](evidence/stress-120.png), [640×480 resize](evidence/resize-640.png). These are real native window crops, not generated mockups. Long labels wrap above their arrows; neutral conditional frames remain independent of changed/selected interactions. The smallest capture deliberately shows horizontal clipping of the scrollable diagram and source, while keeping the selected source range visible. Pane resizing retains its width preference but clamps its effective width to the current viewport.

Vulkan reports Intel UHD Graphics (ICL GT1), Mesa 26.2.3-arch3.1, Vulkan API 1.4.354. The direct native flow-switch harness delivered two Tab events (logged flow 0 then flow 1) and kept the 1,000-step flow alive for ten seconds at about 96 MiB peak process RSS. Thirty native mouse clicks on the ordinary flow selected different interaction blocks and left the app alive with the expected source excerpt. These are smoke observations, not latency or comprehensive navigation tests. A separate attempted smoke run did not deliver both focus events and was rejected as inconclusive rather than counted as passing.

**User feedback received:** “The app just crashed, couldn't verify it”; last action: “I was clicking on some blocks of the sequence diagram.” The tool-owned launch exited with SIGKILL (status 137), without a Rust panic or recorded core dump. Kernel logs are not readable in this environment. The original failure has not been deterministically reproduced; neither the 1,000-step switch harness nor the thirty-click replay reproduced termination. A crash fix is therefore **not claimed**.

The initial debug app was relaunched as an independent transient user service to separate application lifetime from the tool's background session:

```sh
systemd-run --user --unit=bodh-sequence-spike --collect \
  --working-directory="$PWD/spikes/native-sequence" \
  "$PWD/spikes/native-sequence/target/debug/bodh-sequence-spike"
systemctl --user status bodh-sequence-spike
journalctl --user -u bodh-sequence-spike --no-pager
# To stop the demonstration:
systemctl --user stop bodh-sequence-spike
```

This service remained active through subsequent clicking, flow switching, light/dark changes and window resizing. Observed service cgroup memory was approximately 244 MiB current and 1.04 GiB peak (includes GPU/driver allocations and is distinct from process RSS). Moving the launch outside the tool session is a diagnostic isolation step, not proof that the app caused or fixed the prior SIGKILL.

Final checks: `cargo check --locked --all-targets` passed; full desktop-enabled `cargo test --locked` passed (four behavior tests, no failures); formatting and `git diff --check` passed. No production caps, reliable large-flow input-latency guarantee or broad Linux support are inferred from these results.

**Acceptance remains open:** the user could not complete interaction verification because of the reported termination. Preserve this runnable branch and reliability evidence; obtain successful interaction feedback and investigate any reproducible recurrence before treating the spike gate as satisfied.


## Performance and visual feedback follow-up

The user reported that the 1,000-step option slowed the app, and that selection and pane resizing were also laggy in the other flows. They also found the UI dated and questioned whether it represented the intended product direction. This rejects the initial interaction experience; a surviving process is not an adequate performance test.

The outlined controls, monospace application chrome and technical status text are an **unapproved diagnostic harness**, not the product design system or a proposal for Bodh's finished UI. No product visual direction was accepted through this spike. A native GPUI renderer does not require this styling.

Diagnosis used the new `--profile` command against the actual GPUI redraw path. The initial probe omitted per-frame arena cleanup; review caught that and both baseline/fixed runs were repeated with the same corrected harness. Preliminary figures from that first probe are superseded by the corrected results below. The corrected timer includes view state updates, native layout, text/path painting, and element arena cleanup after each draw. It does **not** measure physical input dispatch, GPU submission, compositor presentation or perceived latency. Ten samples per operation follow two warmup draws; p95 is nearest-rank (the maximum for ten samples). The 16.7 ms diagnostic reference is not a schema limit or frozen product performance promise.

Confirmed causes/fixes:

- A clipping mask did not prevent tessellation of offscreen dashed lifelines. Paths now cover only visible lifeline spans, retaining dash phase while scrolling; offscreen interaction paths are skipped.
- The original view rebuilt and laid out all labels/hit regions and cloned the whole fixture flow. Immutable flow/geometry is now cached; a custom native element creates only viewport-intersecting children during prepaint. Scrolling re-evaluates the actual content mask, so it cannot keep an obsolete row-to-source map.
- Selection/pane drag avoid invalidating the whole view for unchanged selections or clamped/subpixel width changes.
- Reproduction now uses `cargo run --release`; debug text/layout costs are not used as a proxy for optimized interaction performance.

The new public renderer-boundary regression verifies partial-row viewport intersections and original source identity at the last row. The full suite passes with five behavior tests. The Standards and Spec follow-up reviews each have zero remaining code findings after the profiling lifecycle correction.

### Corrected redraw measurements

Same machine, 1280×800 native CPU viewport, no concurrent compilation for the release comparison. Baseline is `8de16ba` with the corrected profiling module transplanted into a temporary package with a distinct Cargo package name; fixed is the viewport renderer. Both use the same locked dependencies and release optimization. Diagnostic windows are hidden to avoid disturbing the user's desktop; GPUI still executes the native layout/text/path painting pipeline. This measures CPU redraw preparation, not display presentation.

| Flow | Release selection p50 before → after | Release pane-resize p50 before → after | Fixed selection / resize p95 |
| --- | --- | --- | --- |
| Ordinary 7 steps | 1.438 → 1.389 ms | 0.658 → 0.610 ms | 2.856 / 1.585 ms |
| 120 steps | 10.238 → 1.456 ms | 9.582 → 0.515 ms | 2.279 / 0.576 ms |
| 1,000 steps | 93.785 → 1.458 ms | 91.774 → 0.595 ms | 2.110 / 0.821 ms |

Raw release runs: [before](evidence/redraw-before-release.txt), [after](evidence/redraw-after-release.txt). The same optimized build still exceeded the diagnostic budget for the original 1,000-step renderer, so release optimization alone was insufficient; viewport-bounded work is necessary. Conversely, the small flow's source selection was already fast in release: the initial debug launch contributed substantially to its perceived lag.

Corrected debug runs ([before](evidence/redraw-before-debug.txt), [after](evidence/redraw-after-debug.txt)) were taken during concurrent release compilation and are not controlled latency comparisons. They show the scaling problem but should not be mixed with the final optimized timings. Earlier commentary with preliminary figures is superseded by the corrected release comparison above.

The optimized viewport renderer was relaunched as `bodh-sequence-spike.service` using `target/release/bodh-sequence-spike`; a fresh [native render capture](evidence/viewport-renderer.png) confirms the ordinary diagram/source layout remains intact. The service's fresh observed memory was about 154 MiB current / 158 MiB peak, a point observation rather than a long-run memory guarantee. The user has been asked to retry selection, pane dragging and the 1,000-step flow. That live acceptance, native scrolling/hit-testing after virtualization, and any recurrence of the earlier termination remain open. The dated visual treatment is recorded as rejected/unapproved and has not been promoted to a product design direction.


### Successful user trial of the optimized build

The user subsequently reported: “its much much better. The selection and resizing felt smooth, did not experience any lag or crashes.” This supplies successful live feedback for selection/resizing responsiveness and records no crash during that trial. It does not establish the root cause of the original SIGKILL or a general crash-free guarantee. The earlier failed trials remain in the record.

The interaction-performance feedback gate is satisfied for the optimized build. Explicit native scrolling/hit-testing verification after virtualization remains a separate QA item; product visual design remains unapproved and outside this diagnostic UI's acceptance.
