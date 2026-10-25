---
id: TRX-74
title: Keep the edge clearance from arcs, not only from their chords
type: bug
status: done
assignee: claude
priority: medium
labels: [kicad, rules, verification]
layer: access
---

KiCad's DRC of TokenBoard routed with [[TRX-73]]'s first try found two `copper_edge_clearance` errors at LED177's cutout: rule `LED cutout` 0.2000 mm, actual 0.1992 mm, a GP3_I2C_SCL track. traxide's check found none. The KiCad reader reads an arc as chords no more than `ARC_TOLERANCE` (2 µm) from it ([[crates/access/board/src/kicad/board.rs#L47-L48]]); a chord of a hole's arc lies inside the true hole, so copper the chords keep 0.2 mm off may come up to 2 µm closer to the arc itself.

- [x] A failing test: a track the chords keep the clearance from but the arc does not is too close
- [x] Copper keeps the clearance from the arc: the outline's edges read with their tolerance added to the clearance from them, or chords that run outside a hole's arc
- [x] TokenBoard: no `copper_edge_clearance` in KiCad's DRC

The design carries `outline_tolerance`, how far its outline may lie inside the board's true edges: the KiCad reader's `ARC_TOLERANCE`, 2 µm, and none for a DSN, whose outline is polygons. The rules add it to every edge and cutout clearance, in routing and verification alike. KiCad's DRC of TokenBoard routed so finds no `copper_edge_clearance`.
