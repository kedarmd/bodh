# Bodh

Linux-native PR review workspace. The build specification is [issue #1](https://github.com/kedarmd/bodh/issues/1); the [headless contract/CLI](contracts/README.md) provides complete fixture-context validation for the frozen v1 result contract. Production preparation and persistence remain a later slice. A [disposable native renderer spike](spikes/native-sequence/README.md) remains separate from the production application.

Run `./scripts/validate` for core checks or `./scripts/validate desktop` for the native build and tests. See [TESTING.md](TESTING.md) for prerequisites, CI coverage and GUI evaluation.
