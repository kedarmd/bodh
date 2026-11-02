# Evidence and decision

## Decision

Use the small custom deterministic Rust sequence renderer for this spike. No inspected dependency qualifies as a direct replacement for the exact semantics. This does not establish a production performance budget or freeze numeric contract caps.

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
- All rows/elements are constructed on each render. The 1,000-row sample is not evidence that much larger flows remain interactive. Virtualization and hit-indexing remain engineering follow-ups if measurements justify them.
- At small window widths the minimum 260-pixel source pane leaves little diagram space; horizontal scrolling preserves geometry and hit targets. This is an observed design tradeoff, not a hidden participant cap.
- No persistence across restarts, syntax highlighting, remote PR acquisition or production source validation is claimed by this bounded spike.
- User feedback on the demonstrated interactions is pending. Do not mark the issue complete until it is recorded.
