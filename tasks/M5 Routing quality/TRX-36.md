---
id: TRX-36
title: Optimise routes in the Optimization Engine
type: feature
status: in-progress
assignee: claude
priority: medium
labels: [optimization]
estimate: 6
layer: engine
---

[[Architecture/Routing-pipeline#optimise]]; UC4 *optimise an existing routing* in the Routing Manager.

- [x] Pull-tight: `traxide-optimization`'s `pull_tight`, run by the Routing Manager as the stage *Optimising* after routing and the plane joins. Each new track's runs of corners are replaced by the shortest octilinear way between their ends (straight, or a diagonal and a straight stretch) where it fits in the layout, the longest run first, with no corner sharper than 90°; the track's ends and the vias stay, so nothing joined comes apart. Passes repeat while any track changes, at most four
- [ ] 45° mitering of the right angles routing leaves: few remain after pull-tight (2 of 658 corners on ReadPlaceScope)
- [ ] Stub removal: escape stubs are already trimmed to the copper that uses them ([[TRX-56]])
- [ ] Via-pair removal and layer reassignment
- [ ] Region-parallel on copy-on-write snapshots; a move is kept only if it improves and adds no violation

**Depends on:** [[TRX-27]], [[TRX-19]]

**Pull-tight, measured** (October 2026, this machine; the same connections, vias and verdicts as before):

| Board | Segments | Corners | Track |
| --- | --- | --- | --- |
| Sofle (Issue178) | 3,759 → 2,893 | 2,821 → 1,955 | 10,015 → 9,951 mm |
| CNH functional tester (Issue230) | 1,040 → 733 | 804 → 497 | 5,624 → 5,576 mm |
| ReadPlaceScope (Issue035) | 1,514 → 1,016 | 1,156 → 658 | 5,375 → 5,339 mm |
| wavefolder (Issue153) | 353 → 318 | 220 → 185 | 1,044 → 1,040 mm |

Its time is lost in the noise of routing's. The routes were already nearly as short as they can be on the grid; what pull-tight removes is the corners: the staircases of short jogs routing leaves where it stepped round other copper on the grid. One pass did most of it, a second some 3 % more, a third and fourth little.

**Via pairs** (down to another layer and back) on ReadPlaceScope: 37 of its 86 new vias, the hop between them some 9 mm, under other copper. Tidying ([[TRX-72]]) already searched each connection again with dearer vias; a straight way back on the first layer would cross that copper, so removing pairs needs moving the copper they hop over.
