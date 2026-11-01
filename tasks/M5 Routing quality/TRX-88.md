---
id: TRX-88
title: Dearer vias and a cheaper estimate while tidying
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

Tidying, taking out hops and growing trees search with vias 4 times as dear as negotiation and a heuristic weight of 2 ([[TRX-72]]). Since what a hop's way crosses finds room further off ([[TRX-85]], [[TRX-87]]), the price was measured again. With `--profile fast`, TokenBoard and ten corpus boards together (vias, TokenBoard, time):

| Vias dearer | Weight | Vias | TokenBoard | Time |
| --- | --- | --- | --- | --- |
| 4 | 2 | 1,293 | 318 | 341 s |
| 3 | 2 | 1,407 | 340 | 358 s |
| 6 | 2 | 1,221 | 309 | 362 s |
| 8 | 2 | 1,201 | 278 | 398 s |
| 10 | 2 | 1,162 | 269 | 410 s |
| 4 | 1.5 | 1,240 | 306 | 388 s |
| 6 | 1.5 | 1,177 | 285 | 419 s |
| 8 | 1.5 | 1,144 | 286 | 453 s |
| **10** | **1.5** | **1,090** | **259** | **478 s** |
| 12 | 1.5 | 1,120 | 275 | 516 s |

- [x] Vias 10 times dearer and a weight of 1.5 while tidying (`TIDYING_VIA_COST`, `TIDYING_HEURISTIC_WEIGHT`): 16 % fewer vias in 40 % more time; Issue022 53 instead of 78, Sofle 347 instead of 390, Issue015 47 instead of 49
- [ ] TokenBoard with the default profile, without a limit; the corpus gate passes
