---
id: TRX-19
title: Check designs independently in the Verification Engine
type: feature
status: in-progress
priority: high
labels: [verification, drc]
estimate: 5
layer: engine
---

The referee ([[Architecture/Volatilities]] V9): never shares clearance code with the router.

- [x] Clearance between all copper of different nets per layer, outline and keepouts (sweep along x, independent of Layout Access)
- [ ] Widths and via rules
- [ ] Clearances of a type of object in DSN, such as `(clearance 6 (type wire_via))`: read only the general clearance, the judge is stricter than such a board and counts some of Freerouting's legal copper as broken rules (Issue143, Issue270 of the corpus)
- [x] Unconnected pins
- [x] Dangling copper: a track end whose round end touches no other copper of its net on its layer, and lies in no pour of its net there, is a `Violation::Dangling`; KiCad's DRC report names it `track_dangling`, a warning, as KiCad does. Vias are not judged yet. Its first catch, on Mars-64 (Issue102): a route joined a fine-pitch pin's escape stub ([[TRX-56]]) in its middle, as the pad at the stub's end is all of the stub, and left 1.5 mm of it leading nowhere; each stub now ends at the point of a route, or of the board's own wiring, on it farthest from its pin (Issue367-Charger's own wiring bends off a stub). It also found an older bug on Issue289-8086: a route that the search finished at a pin off the grid, reached along the pin's axis by a stub with a jog, ran into the pin and back out to the jog, leaving an antenna and, in the test that reproduces it, a clearance broken to the next pin; the finishing stub's points are now in the right order
- [ ] Metrics: completion, total length, vias, per-net length
- [ ] Parallel by tile
- [x] Tested on boards with seeded violations

**Depends on:** [[TRX-10]], [[TRX-11]]
