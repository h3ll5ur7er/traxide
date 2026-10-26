---
id: TRX-59
title: Move the board's unlocked wiring when it blocks a connection
type: feature
status: done
assignee: claude
priority: low
labels: [detailed-routing, board-access]
estimate: 3
layer: engine
---

traxide keeps all copper a board comes with as it is. Specctra marks wiring the router must keep as `(type protect)` or `(type fix)`; `(type route)` wiring is ordinary routing that a router may rip up and route again, and KiCad exports unlocked tracks so. Ten corpus boards carry such wiring (Issue069-KiCad6, 113, 153, 157, 190, 214, 269-caniot, 269-z10, 367 ×2). On two of them it blocks connections that cannot be routed otherwise ([[Corpus-analysis]]):

- Issue269-caniot-tiny-arm: routing has one layer (B.Cu is a power layer, [[TRX-57]]), and the board's 335 tracks enclose J8-9 and the QFP's pins 51–53, so that no way exists at any grid: 5 connections.
- Issue157-TeamAdapt-LinePCB: a track of Net-(D49-Pad1) runs across the pads Q1-3 and Q2-3 of other nets, a rule the board already breaks, and leaves them no way out: 2 connections.

Moving a designer's tracks is a decision for the designer, so it is opt-in.

- [x] Board Access keeps the wire type: protected (`protect`, `fix`) or not (`route` and others)
- [x] With an option (`traxide route --reroute-existing`, also in [[TRX-34]]'s Freerouting mode if it has an equivalent), unprotected wiring joins rip-up and reroute as routed copper of connections planned like any other
- [x] Tests: an unprotected track that blocks the only way is moved; a protected one is not
- [x] Issue269-caniot-tiny-arm and Issue157-TeamAdapt-LinePCB before → after

**Done, in the simplest form:** with `--reroute-existing` the design keeps only its protected wiring (DSN `fix` or `protect`, KiCad's locked tracks, arcs and vias, which the KiCad reader now records in `Design::locked`), and everything else is planned and routed afresh. The session then carries no unprotected wiring; a KiCad board file is stored without its unlocked tracks (Board Access's `without_unlocked_wiring`, moved from the corpus's stripping). A running KiCad refuses the option: its tracks would have to be deleted through the API.

| Board (`--time-limit 90`) | Wiring kept | `--reroute-existing` |
| --- | --- | --- |
| Issue157-TeamAdapt-LinePCB | 63 of 65 nets | **65 of 65**, 1.6 s |
| Issue269-caniot-tiny-arm | 82 of 94 | 79 of 94 |
| Issue113-Protein | 153 of 203 (against the clock) | 151 of 203 |
| Issue214-freerouting | complete, 215 vias, 4462 mm | complete, 128 vias, 4117 mm |
| Issue367-UltraFlactyl | complete | complete |

On caniot its author's routing of the one routing layer beats traxide's, so routing all of it again loses three nets. Moving only the wiring that blocks a connection would keep the rest: [[TRX-75]].
