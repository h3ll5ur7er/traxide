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
- [x] Dangling copper: a track end whose round end touches no other copper of its net on its layer, and lies in no pour of its net there, is a `Violation::Dangling`; KiCad's DRC report names it `track_dangling`, a warning, as KiCad does. Vias are not judged yet. Its first catch, on Mars-64 (Issue102): a route joined a fine-pitch pin's escape stub ([[TRX-56]]) in its middle, as the pad at the stub's end is all of the stub, and left 1.5 mm of it leading nowhere; each stub now ends where the route farthest from its pin joins it
- [ ] Metrics: completion, total length, vias, per-net length
- [ ] Parallel by tile
- [x] Tested on boards with seeded violations

**Depends on:** [[TRX-10]], [[TRX-11]]
