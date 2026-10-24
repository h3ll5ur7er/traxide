---
id: TRX-72
title: Change layers less often
type: improvement
status: in-progress
assignee: claude
priority: medium
labels: [detailed-routing]
layer: engine
---

From the TokenBoard report ([[Research/TokenBoard-report]]): traxide placed 547 new vias where Freerouting 1.9.0 placed 194, and 4,431 segments against about 2,700. Many vias come in pairs, down to the other layer and straight back up a few tenths of a millimetre on. Fewer layer changes leave more room around a dense QFN and need fewer drill hits.

- [ ] Measure where the pairs come from: rip-up costs paid on one layer, the via cost against the bend cost, the escapes
- [ ] Fewer vias on the keyboard and TokenBoard without losing connections or time

[[TRX-71]] lets a search change layers through a via of its net the board came with: TokenBoard's KiCad board takes 523 new vias instead of 553. Vias another connection routed are not used so, as rip-up may take them away; a path through one would need that connection kept or the via handed over.
