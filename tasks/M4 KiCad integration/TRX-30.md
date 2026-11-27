---
id: TRX-30
title: Read and write KiCad board files in Board Access
type: feature
status: in-progress
assignee: claude
priority: high
labels: [board-access, kicad]
estimate: 5
layer: access
---

The `kicad` module of Board Access ([[Research/KiCad-integration#reading-board-files]]).

- [x] S-expression parser for `.kicad_pcb` (KiCad 9 and 10 formats); footprints, pads, zones, keepouts, tracks, vias. Owned strings, not zero-copy: a 320 kB board parses in milliseconds
- [x] Net classes and board setup from `.kicad_pro`: widths, clearances, vias, edge and hole clearances, assignments and patterns
- [x] Custom rules from `.kicad_dru` ([[TRX-32]])
- [x] Write back new `segment` and `via` nodes, keeping every other byte; refuse if the file changed since loading
- [x] `traxide route BOARD.kicad_pcb` writes `BOARD.routed.kicad_pcb` with its project; `cargo xtask e2e` judges it with KiCad's DRC, clean on every recipe
- [ ] Run the corpus from the boards' own files where they have them

**Depends on:** [[TRX-7]]
