# Native sequence/source spike — issue #2

Disposable feasibility evidence for [issue #2](https://github.com/kedarmd/bodh/issues/2), on branch `spike/2-native-sequence`. This is not the application or a frozen contract.

From the repository root, in a Linux graphical session:

```sh
CARGO_BUILD_JOBS=4 cargo run --release --locked --manifest-path spikes/native-sequence/Cargo.toml
```

Use the optimized build for interaction evaluation. The first release build downloads/compiles GPUI and its platform dependencies and can take several minutes. Install a C/C++ toolchain, pkg-config, fontconfig/freetype, xkbcommon, Wayland/X11 development libraries and a working Vulkan driver. The tested environment and limitations are in [EVIDENCE.md](EVIDENCE.md). The lockfile is required: upgrading dependencies has already exposed a concrete compatibility failure.

Click a numbered interaction to reuse the source inspector. Arrow keys select/reveal interactions, Tab cycles flows, T toggles light/dark, and `[` / `]` resize the inspector. Drag the divider for continuous resizing. Scroll the diagram in both directions and the source independently. Flow switching retains each flow's selection and both scroll positions; pane width and theme are shared. State lasts for the process only.

The ordinary flow includes messages, dashed returns, a self-call, a note, a two-branch top-level alt and an opt. Stress flows contain 12 participants/120 interactions and 24 participants/1,000 interactions with long labels. These are samples, not product caps. Participant and interaction order comes from the handcrafted data. Conditional types cannot nest. Fixtures are embedded using `include_str!`; changing files after launch cannot alter displayed source. Highlighting is derived from line intersection with the embedded equal-length base/head fixture pair, not agent-provided style.

Renderer coordinates, spacing, typography, accent and hit regions are fixed application rules, never semantic input fields. All output is GPUI elements and native paths; there is no WebView, Mermaid, SVG or agent-defined rendering.

Core behavior checks without a display:

```sh
cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml --no-default-features
cargo run --locked --manifest-path spikes/native-sequence/Cargo.toml --no-default-features --release --example measure
```

Desktop checks:

```sh
cargo check --locked --manifest-path spikes/native-sequence/Cargo.toml --all-targets
cargo test --locked --manifest-path spikes/native-sequence/Cargo.toml
```

Feedback requested on the running demonstration: can you distinguish changed, selected and supporting interactions; does selecting a return/self-call show the expected source; does switching flows/resizing preserve your place? Record observed problems against these interactions rather than reopening the approved product scope. The first user trial reported a termination during clicking; successful interaction feedback and reliability acceptance remain open. See the evidence report for diagnostic results and the independent service launch.

The current typography, outlined controls and diagnostic status text are **unapproved spike UI**, not the proposed Bodh visual design. This experiment evaluates native rendering/source interaction; it does not establish the product design system.

Native CPU redraw benchmark (requires the same graphical environment):

```sh
cargo run --release --locked --manifest-path spikes/native-sequence/Cargo.toml -- --profile
```

It times selection and pane-width changes through GPUI state update, layout, text/path painting and per-frame arena cleanup. It excludes GPU/compositor presentation and physical input latency. Its 16.7 ms diagnostic frame budget is a measurement reference, not a product data-size cap. The benchmark opens a diagnostic window, writes timings, then exits.
