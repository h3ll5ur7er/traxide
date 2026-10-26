---
id: TRX-55
title: Widen the search window when no way is found
type: improvement
status: in-progress
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

A search only covers the box around its two pads grown by the profile's `detour_margin`, 5 mm (`window` in [[crates/engines/detailed/src/search.rs]]). On real boards the way round a connector, a row of headers or the board's own wiring is often longer: [[Corpus-analysis]] found 23 connections on 10 boards that route alone only with a wider window, and on several boards the connections that collided with them went too. With `detour_margin = 100` (the whole board), measured on the corpus in the 4-core container, every session still clean:

| Board | Default (5 mm) | 100 mm |
| --- | ---: | ---: |
| Issue029-hw48na | 83 of 91 | **91 of 91** |
| Issue093-interf_u | 2 of 3 | **3 of 3** |
| Issue110-RelayModule | 81 of 88 | **88 of 88** |
| Issue110-Pajalnaja_stancija | 104 of 109 | 108 of 109 (the last pad lies outside the board) |
| Issue178-KeebMaker_Sofle_Choc | 425 of 426 | **426 of 426** |
| Issue217-8088sbc | 234 of 235 | **235 of 235** |
| Issue230-CNH_Functional_Tester (both) | 192 of 193 | **193 of 193** |
| Issue269-z10_module | 63 of 68 | 65 of 68 |

A wide window for every search would make the long searches slower; only the searches that fail need it.

- [x] Failing test first: a connection whose only way runs round a wall longer than the margin routes
- [x] When a search finds no way in its window, search again in a window grown step by step (×4) up to the board's outline, before reporting no way found
- [x] Batches stay apart without growing their territories: the wider searches run one at a time after their batch has committed ([[TRX-27]])
- [x] The boards above complete as in the table, but for Issue269-z10 (65 of 68); the keyboard routes the same in the same time ([[Benchmarks#a-wider-window-where-no-way-was-found-m5-trx-55]])
- [ ] Issue029 routes 87 of 91 instead of 83 but takes 149 s instead of 40 s (in CI it sits at the corpus's 120 s limit: 108 s at 5dd40cc, stopped at 63b004d): make the wider searches cheaper, or the negotiation they start ([[TRX-56]]). Not the cause, measured: widening a connection only until it fails once, or starting its wider search from the margin that worked before (147.5 s)

**Cut-off connections searched once** (October 2026): a connection that the board's own copper, keepouts or edge cut off can never route, yet every failure searched it again in ever wider windows, in each round of both negotiation attempts. On Issue269-z10_module two GND connections are cut off: 51 searches of up to 2.5 s each took 28 of its 29 s. Now, when a search finds no way, whether the connection is cut off is checked first (one search through routed copper, as before only after widening), and remembered: a cut-off connection is not widened, and later batches do not search it at all. z10_module routes the same in 6.2 s instead of 31.7 s on this machine (CI: 14.8 s before). Every other board routes exactly as before; the corpus takes 1954 s instead of 1982 s.
