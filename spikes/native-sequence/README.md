# Bodh native workspace and sequence renderer

This GPUI executable now has a product-mode home, repository/PR browser, and prepared review workspace for [issue #16](https://github.com/kedarmd/bodh/issues/16). The original rendering feasibility demo for [issue #2](https://github.com/kedarmd/bodh/issues/2) remains available with `--diagnostic`; it is not the product UI.

From the repository root, in a Linux graphical session:

```sh
./scripts/validate gui
```

Use the optimized build for interaction evaluation. The first release build downloads/compiles GPUI and its platform dependencies and can take several minutes. Install a C/C++ toolchain, pkg-config, fontconfig/freetype, xkbcommon, Wayland/X11 development libraries and a working Vulkan driver. The tested environment and limitations are in [EVIDENCE.md](EVIDENCE.md). The lockfile is required: upgrading dependencies has already exposed a concrete compatibility failure.

The default launch opens the native home screen. Add an existing checkout, then search its pull requests by title, number or author. Selecting a pull request uses Bodh's shared pinned-preparation service. Home also lists saved repositories and recent preparation IDs. To open directly into a review, pass `--open --repo <checkout> --pr <number>` to the executable. The app runs GitHub CLI work off the UI thread and stores navigation preferences separately from review content.

The product styling follows the user-approved D2 proposal at `/home/kedarmd/dev/bodh-design-proposal/index.html`; the approval record is `/home/kedarmd/.local/state/bodh-implementation/design/APPROVAL-D2.md`. This does not accept the native screen. The prepared review currently has no generated analysis: Overview therefore shows the manual/no-analysis state, as D2 specifies. The diagram/source pair appears only when a later analysis ticket supplies a real diagram. Changes presents the pinned file/source reader; Findings states that no result is attached. Native screen acceptance is pending separately.

The GPUI dependency graph includes a small local compatibility patch for transitive `xattr 0.2.3`. GPUI 0.2.2 depends on `zed-async-tar 0.5.0-zed`, whose `xattr ^0.2` Linux source refers to `libc::ENOATTR`; the Bodh graph pins libc 0.2.191, where Linux exposes this error as `ENODATA`. `vendor/xattr/src/sys/mod.rs` maps the Linux/Android alias to `ENODATA` and retains upstream `ENOATTR` on macOS and BSD. Crates.io `xattr 1.7.0` is outside the transitive `^0.2` range, so a lockfile update cannot select it. The local patch is confined to this native crate and preserves the rest of upstream xattr 0.2.3.

The old demo is available explicitly:

```sh
cargo run --release --locked --manifest-path spikes/native-sequence/Cargo.toml -- --diagnostic
```

The native CPU redraw benchmark remains available with `./scripts/validate profile`.

Click a numbered interaction to reuse the source inspector. Arrow keys select/reveal interactions, Tab cycles flows, T toggles light/dark, and `[` / `]` resize the inspector. Drag the divider for continuous resizing. Scroll the diagram in both directions and the source independently. Flow switching retains each flow's selection and both scroll positions; pane width and theme are shared. State lasts for the process only.

The ordinary flow includes messages, dashed returns, a self-call, a note, a two-branch top-level alt and an opt. Stress flows contain 12 participants/120 interactions and 24 participants/1,000 interactions with long labels. These are samples, not product caps. Participant and interaction order comes from the handcrafted data. Conditional types cannot nest. Fixtures are embedded using `include_str!`; changing files after launch cannot alter displayed source. Highlighting is derived from line intersection with the embedded equal-length base/head fixture pair, not agent-provided style.

Renderer coordinates, spacing, typography, accent and hit regions are fixed application rules, never semantic input fields. All output is GPUI elements and native paths; there is no WebView, Mermaid, SVG or agent-defined rendering.

Core behavior checks without a display:

```sh
./scripts/validate core
cargo run --locked --manifest-path spikes/native-sequence/Cargo.toml --no-default-features --release --example measure
```

Desktop checks:

```sh
./scripts/validate desktop
```

The user accepted interaction performance and completed the remaining native scrolling QA after viewport optimization and a release launch. Earlier failed trials and the original unexplained termination remain in the evidence record. For future changes, use the [GUI trial checklist](../../TESTING.md#native-gui-trial).

The diagnostic demo retains its **unapproved spike styling**. The default product mode uses the approved D2 visual baseline; its native screen still needs separate acceptance.

Native CPU redraw benchmark (requires the same graphical environment):

```sh
./scripts/validate profile
```

It times selection and pane-width changes through GPUI state update, layout, text/path painting and per-frame arena cleanup. It excludes GPU/compositor presentation and physical input latency. Its 16.7 ms diagnostic frame budget is a measurement reference, not a product data-size cap. The benchmark creates a hidden native window, writes timings, then exits. See [testing guidance](../../TESTING.md#performance-evidence) for comparable runs and interpretation.
