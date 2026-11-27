---
id: TRX-33
title: Route inside KiCad with a native plugin
type: feature
status: review
assignee: claude
priority: high
labels: [kicad, client]
estimate: 3
layer: client
---

UC2 of [[Architecture/Use-cases#uc2-route-the-board-open-in-kicad]]: an `exec` plugin, no Python.

Facts from [[TRX-17]]: [[Research/KiCad-integration#the-plugin]].

- [x] `plugin.json` with `runtime.type: "exec"` and the action *Route board*, whose entry point is the `traxide` binary itself: started with no arguments and `KICAD_API_SOCKET` set, it routes the board open in KiCad. *Route selected nets* and *Optimise* need routing features that do not exist yet: moved to [[TRX-63]]
- [x] The plugin finds the board through the API and the project through `KIPRJMOD` (Board Access does both for `kicad://current`, [[TRX-31]]); verified by pressing the button in KiCad 10.0.7 and 9.0
- [x] On a board with copper zones, tell the designer that traxide refilled them and that taking the routes back takes two undos, the refill being a step of its own ([[TRX-31]])
- [x] Progress window from events; cancel: moved to [[TRX-63]]. KiCad's API offers no dialog, so a window needs a GUI toolkit; for the first release the plugin writes its summary to stderr, which KiCad 10 shows line by line as notifications once the plugin ends (KiCad 9 shows nothing)
- [x] PCM package per platform: `plugins/` with `plugin.json`, binary and light and dark icons, `resources/icon.png`, `metadata.json` with `platforms` and `kicad_version` 9.0 (KiCad 9 runs the plugin too). `cargo xtask kicad-package` builds it
- [x] Say in the PCM description and the README that the KiCad API must be enabled first: without it KiCad shows no traxide button at all
- [ ] Install from a local PCM repository on Linux, Windows and macOS: executable bit (kept on Linux), toolbar icons, Windows named pipe, macOS notarisation and quarantine. Linux done: `cargo xtask e2e` installs the package as *Install from File* does into KiCad 10.0.7 and 9.0, presses its button and judges the board with KiCad's DRC. Windows and macOS need a person with those machines: install the package from the Binaries workflow with *Install from File*, enable the API, restart the PCB editor, press *Route board* on a board, and note whether the button shows its icon, whether it routes, and on macOS whether Gatekeeper blocks the binary
- [x] Retire the Python action plugin of [[TRX-53]] once this covers KiCad 10: removed, as the native plugin covers KiCad 9 too

Decisions:

- One binary, no launcher: KiCad passes no arguments, so a bare `traxide` started with KiCad's API socket in its environment is the button. In a terminal a bare `traxide` still prints its usage.
- The plugin writes only its summary (as `--quiet`), and to stderr: KiCad 10.0.7 shows each stderr line as a notification once the plugin ends and ignores stdout, as pressing the button showed; a progress line rewritten in place would be noise there.
- The package asks for KiCad 9.0: KiCad 9 loads and runs the exec plugin as KiCad 10 does, and the IPC path is tested on both.

Open, for a person with Windows and macOS: the install check above. Everything else is done and in `cargo xtask e2e`.

**Depends on:** [[TRX-31]], [[TRX-13]]

**Waiting for the maintainer** since the merge of PR #6: installing the package on Windows and macOS needs those machines; Linux is tested by `cargo xtask e2e`.
