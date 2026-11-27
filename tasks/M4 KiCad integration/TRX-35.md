---
id: TRX-35
title: Release static binaries for six targets
type: chore
status: review
assignee: claude
priority: high
labels: [release, ci]
estimate: 2
layer: infrastructure
---

[[Research/Tech-stack#distribution]].

- [x] Binaries workflow on every push, as artifacts: Linux x86-64 and ARM64 musl (static), Windows x86-64 and ARM64 with `crt-static`, macOS universal; each routes a board as a smoke test (no GPU yet, so musl serves Linux)
- [x] Release workflow (`.github/workflows/release.yml`), hand-written rather than cargo-dist: on a tag `vX.Y.Z` it checks that the tag names the version in `Cargo.toml`, runs the Binaries workflow, and drafts a GitHub release with the five archives (six targets: Linux and Windows on x86-64 and ARM64, macOS universal), the five KiCad packages and `SHA256SUMS`. Draft, so that the maintainer reads it through and publishes it
- [ ] Linux glibc 2.28 builds with the GPU backend: once [[TRX-28]] exists; until then the static musl builds serve Linux
- [ ] macOS signing and notarisation; optional Windows signing. The universal binary is signed ad hoc (`codesign --sign -`), which Apple silicon requires. Open, as it needs the maintainer's accounts: an Apple Developer ID certificate and an app-specific password as repository secrets, then `codesign --options runtime --timestamp --sign "Developer ID Application: …"` and `xcrun notarytool submit --wait` on a zip of the binary (a bare binary cannot be stapled; Gatekeeper checks it online). Windows: a code-signing certificate and `signtool`, optional
- [x] Job that assembles the KiCad PCM packages: the native plugin of [[TRX-33]] with each platform's traxide as its entry point (`cargo xtask kicad-package`)
- [x] `cargo deny` licence check in the release: only permissive licences in the binary (the Release workflow's `licences` job, before anything is built)

Decisions:

- No cargo-dist: the Binaries workflow already builds every target, and a single binary needs no installer; cargo-dist would replace a working workflow and generate one of its own to keep up to date. Reconsider when the GPU builds bring more targets.
- Archives are named `traxide-VERSION-TARGET`, the KiCad packages `traxide-kicad-VERSION-TARGET.zip`; Windows archives are zip files, the others `.tar.gz` with the executable bit.
- Not testable here: the workflows themselves (they run on GitHub), Windows and macOS. They are valid YAML and pass `actionlint` (which only does not know the `windows-11-arm` label yet); the archive step was run locally on a mock of the artifacts, and the Linux KiCad package through `cargo xtask e2e`.

Open, for the maintainer: Developer ID signing and notarisation (credentials), and the glibc GPU builds once [[TRX-28]] exists. To release: set the version in `Cargo.toml`, tag `vX.Y.Z`, push the tag, then read through and publish the draft release.

**Depends on:** [[TRX-3]], [[TRX-14]]

**Waiting for the maintainer** since the merge of PR #6: signing and notarisation need the maintainer's Apple Developer ID and, optionally, a Windows certificate; the glibc build waits for [[TRX-28]].
