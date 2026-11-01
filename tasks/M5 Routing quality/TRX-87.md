---
id: TRX-87
title: Cheaper crossings and a third pass for taking out hops
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

Taking out hops ([[TRX-76]]) prices crossing a routed connection at a quarter of a via per grid edge, and makes at most two passes: cheaper crossings cost vias then, as more tracks had to move and fewer found room. Since [[TRX-85]] what is crossed searches six times as far for room, and the trade-off moved.

Measured with `--profile fast` on TokenBoard and ten corpus boards together (vias, time):

| Crossing per edge | Passes | Vias | Time |
| --- | --- | --- | --- |
| 1/4 via | 2 | 1,492 | 215 s |
| 1/2 via | 2 | 1,659 | 189 s |
| 1/8 via | 2 | 1,355 | 265 s |
| 1/12 via | 2 | 1,362 | 301 s |
| 1/16 via | 2 | 1,390 | 326 s |
| 1/4 via | 3 | 1,467 | 227 s |
| **1/8 via** | **3** | **1,311** | **303 s** |

- [x] Crossing an eighth of a via per edge (`CROSSING_EDGES_PER_VIA`), and three passes (`MOST_HOP_PASSES`): 12 % fewer vias in 41 % more time; TokenBoard 332 instead of 335, Issue034 98 instead of 117, Issue015 49 instead of 70, RelayModule 42 instead of 53, CNH 67 instead of 87
- [x] TokenBoard with the default profile, without a limit: 256 vias instead of 261, in 579 s instead of 476 s, no rule broken
- [ ] The corpus gate passes
