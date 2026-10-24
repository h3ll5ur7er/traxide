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

- [x] Measure where the pairs come from: rip-up costs paid on one layer, the via cost against the bend cost, the escapes
- [ ] Fewer vias on the keyboard and TokenBoard without losing connections or time

[[TRX-71]] lets a search change layers through a via of its net the board came with: TokenBoard's KiCad board takes 523 new vias instead of 553. Vias another connection routed are not used so, as rip-up may take them away; a path through one would need that connection kept or the via handed over.

**Measured** on the report's DSN (607 new vias for 392 connections, Freerouting 194): since [[TRX-71]]'s hole-to-hole distance no via pair is closer than 1 mm; 120 of the 441 wires from via to via are 1–3 mm, a hop under something in the way.

- A dearer via throughout: 4 mm, 546 vias but 390 of 392 connections and 151 s; 8 mm, 512 vias with connections lost. On TokenBoard's KiCad board 234 and 233 of 241 nets instead of 241. Negotiating at that price costs connections and time.
- Preferred layer directions (horizontal on one routable layer, vertical on the next; against it half the length extra, diagonally a quarter): the DSN 449 vias but 210 s instead of 94 s, TokenBoard 469 vias and 240 nets. Not without tuning; the estimate no longer sees the cost.
- **Kept:** once everything is routed, each connection with vias is routed again, crossing nothing, with vias four times dearer, and the route with fewer vias stays. Batched as negotiation is, so it is the same on any number of threads, and left out past the deadline. The DSN: 521 vias instead of 607, 91 s instead of 78 s, all 392 connections; TokenBoard: 435 vias instead of 531, 71 s instead of 57 s, 241 of 241 nets, the same track length. Eight times dearer gave 514 and 429, sixteen times 510 and 429.
