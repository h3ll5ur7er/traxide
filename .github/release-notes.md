traxide, a fast PCB autorouter for KiCad and every tool that speaks Specctra DSN.

## In KiCad 9 and 10

1. **Enable KiCad's API first**: *Preferences → Preferences… → Plugins → Enable KiCad API*. Without it KiCad shows no traxide button at all.
2. Download the KiCad package for your computer and install it with *Plugin and Content Manager → Install from File…*:
   - Linux: `traxide-kicad-…-x86_64-unknown-linux-musl.zip`, or `…-aarch64-unknown-linux-musl.zip` on ARM
   - Windows: `traxide-kicad-…-x86_64-pc-windows-msvc.zip`, or `…-aarch64-pc-windows-msvc.zip` on ARM
   - macOS: `traxide-kicad-…-macos-universal.zip`
3. Restart the PCB editor and press **Route board** in its top toolbar, or select some footprints or tracks and press **Route selected nets** beside it to route only their nets. *Edit → Undo* takes the routes back; on a board with copper zones, which traxide refills around the routes, it takes two undos.

## On the command line

The archives hold the `traxide` binary, which needs nothing else installed: static on Linux, with the C runtime linked in on Windows, universal on macOS. `traxide route board.kicad_pcb` routes a KiCad board, `traxide route board.dsn` a Specctra design; `traxide --help` says more.

`SHA256SUMS` lists the checksums of every file. traxide is licensed MIT OR Apache-2.0.
