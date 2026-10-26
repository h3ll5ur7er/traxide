---
id: TRX-72
title: Change layers less often
type: improvement
status: review
assignee: claude
priority: medium
labels: [detailed-routing]
layer: engine
---

From the TokenBoard report ([[Research/TokenBoard-report]]): traxide placed 547 new vias where Freerouting 1.9.0 placed 194, and 4,431 segments against about 2,700. Many vias come in pairs, down to the other layer and straight back up a few tenths of a millimetre on. Fewer layer changes leave more room around a dense QFN and need fewer drill hits.

- [x] Measure where the pairs come from: rip-up costs paid on one layer, the via cost against the bend cost, the escapes
- [ ] Fewer vias on the keyboard and TokenBoard without losing connections or time (fewer vias and no connections lost; 10–30 % more time)

[[TRX-71]] lets a search change layers through a via of its net the board came with: TokenBoard's KiCad board takes 523 new vias instead of 553. Vias another connection routed are not used so, as rip-up may take them away; a path through one would need that connection kept or the via handed over.

**Measured** on the report's DSN (607 new vias for 392 connections, Freerouting 194): since [[TRX-71]]'s hole-to-hole distance no via pair is closer than 1 mm; 120 of the 441 wires from via to via are 1–3 mm, a hop under something in the way.

- A dearer via throughout: 4 mm, 546 vias but 390 of 392 connections and 151 s; 8 mm, 512 vias with connections lost. On TokenBoard's KiCad board 234 and 233 of 241 nets instead of 241. Negotiating at that price costs connections and time.
- Preferred layer directions (horizontal on one routable layer, vertical on the next; against it half the length extra, diagonally a quarter): the DSN 449 vias but 210 s instead of 94 s, TokenBoard 469 vias and 240 nets. Not without tuning; the estimate no longer sees the cost.
- **Kept:** once everything is routed, each connection with vias is routed again, crossing nothing, with vias four times dearer, and the route with fewer vias stays. Batched as negotiation is, so it is the same on any number of threads, and left out past the deadline. The DSN: 521 vias instead of 607, 91 s instead of 78 s, all 392 connections; TokenBoard: 435 vias instead of 531, 71 s instead of 57 s, 241 of 241 nets, the same track length. Eight times dearer gave 514 and 429, sixteen times 510 and 429.

**Tidied greedily:** with the profile's heuristic weight, tidying took as long as routing on small boards (Issue230 23 s instead of 11 s, Issue178 40 s instead of 24 s). Its searches now head for the target with weight 2, as any way with fewer vias will do, and connections that already have the fewest vias they can are left out: none between pads that share a layer tracks run on, else one. Weight 3 saved less (Issue230 103 vias), vias twice dearer at the profile's weight about the same in more time.

| | before | tidied |
| --- | --- | --- |
| The report's DSN, 392 of 392 | 607 vias, 78 s | 539 vias, 88–95 s |
| TokenBoard's KiCad board, 241 of 241 nets | 531 vias, 57 s | 445 vias, 63–73 s |
| Issue230, complete | 112 vias, 11 s | 92 vias, 14 s |
| Issue178, complete | 564 vias, 24 s | 518 vias, 34 s |

The whole corpus passes the gate (0 regressions, 8 improvements, 50 complete). Freerouting's 194 vias on the report's board stay out of reach this way: they would need the layout negotiation settles on to change, such as preferred directions per layer, which want tuning first.

**Fixed after the merge:** tidying in batches broke a rule on Issue283 (CI, PR #18): a clearance between tracks of Net-(U1-Pad24) and Net-(J13-Pad1). Batches keep apart what connections may search, not what they routed, and a route found in a wider window ([[TRX-55]]) reaches beyond that: taken out together with a neighbour whose window was apart, the neighbour's tidier route ran where the wide route lay, and the wide route came back. Tidying is now one connection at a time; batched, it had saved some 3 %. Plane joins after routing ([[TRX-67]]) also route in a layout of their own now, holding the routes as the board's copper: sharing the routing's layout, their plan's ids named the routing's connections, and tidying them took those out.
