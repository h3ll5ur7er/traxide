---
id: TRX-48
title: Search free space instead of a fine grid
type: improvement
status: in-progress
priority: high
assignee: claude
labels: [detailed-routing, performance]
estimate: 8
layer: engine
---

The detailed search walks a 0.1 mm grid: millions of states on a large board, most of them in open space where nothing needs deciding. Searching a graph of the places where something does change — corners of obstacles inflated by their clearance, the corridor's crossing points, a coarse grid in between — needs far fewer states for the same paths ([[Research/Algorithms#classical-baselines]]).

- [x] Profile the boards that stop at the corpus's limit (gdb stack samples of a release build with line tables; no perf in the container): Issue289's two boards spent their time scanning the 242 × 242 clearance table on every copper query, now worked out once, so both finish ([[Benchmarks#the-largest-clearance-worked-out-once-m2-trx-48]]). Issue066 has no such hot spot: its samples spread over the open list's heap, copper queries and history lookups, so it simply expands too many states, which only a coarser graph, below, can change. Issue103-Board-Unrouted and Issue113, sampled in October 2026 with the second attempt of [[TRX-56]] and `--time-limit 90` (20 samples each): no hot spot either; the busy threads' innermost frames spread over the search, the state table (`get_or_insert_with`), copper queries (`intersects`, `of_track`, the bucket candidates) and geometry, and most threads wait, as few connections can be searched side by side. The time goes into how many states the search expands, as on Issue066
- [ ] Implicit octilinear graph from corners of inflated obstacles plus a fallback grid
- [ ] Same results as the grid search on the end-to-end boards, measured in [[Benchmarks]]
- [ ] Decide after [[TRX-26]]: corridors may shrink the grid search enough

Measured in [[TRX-27]]: the keyboard's time is in its few long connections, which overlap everything and so cannot run side by side. The worst, from the MCU across the board, popped 4.4 million of its window's 17.7 million grid states and took 2 s, mostly expanding states again ([[TRX-49]] fixed that: 0.77 s); most searches touch under 2 % of their window.

Measured on real boards ([[TRX-15]]'s corpus), October 2026: on Issue029-hw48na, the long searches cost 1.1–4 times their straight-line estimate and flood up to 1.8 million states each. A coarse distance map of the fixed copper (0.5 mm cells, Dijkstra from the target) as the estimate barely raised it (24 → 34 mm where 98 mm were paid) and made nothing faster: what makes those paths dear is negotiation, crossing routed copper, not detours around fixed copper. The outline index, this task's first step, made each expansion three times cheaper on boards with rounded corners.

Searching free space first, and with rip-ups only where it finds no way, measured October 2026: Issue110-RelayModule routed 85 of 88 connections in 11 s instead of 81 in 40 s, but Issue034-Green14SegLED lost its completeness (144 of 147 instead of 147) and the keyboard took 2.3 s instead of 1.9 s. Free space first while paying history costs did worse (142 of 147, Issue015 no faster). Negotiation's flooding stays: where the only ways cross routed copper, the search pays rip-up costs its straight-line estimate does not know.

A heavier estimate, measured October 2026 against today's 1.2: at 1.5 the keyboard routed in 0.64 s instead of 1.45 s but with 128 vias instead of 113, Issue015 in 30 s instead of 35 s with 10 % more vias, and Issue110 took twice as long; at 2.0 Issue034 lost its completeness (143 of 147). The default stays 1.2; a heavier one would suit only a profile that trades vias for time.

What the flooding searches pay, measured on Issue015 in October 2026: the largest estimated 72–88 mm and paid 105–250 mm, of which the length was 84–103 mm, 3–11 vias at 2 mm each (the estimate allows at most one), and on some searches 17–143 mm of rip-up and history costs. A coarse cost-to-target map that knows on which layer routed copper lies, and what a via costs, could capture most of that; the map of fixed copper alone could not.

Tried in October 2026: such a coarse map (1 mm cells, entering a cell with routed copper of other connections costing their rip-up price, vias at their cost, Dijkstra from the target) as a floor under the straight-line estimate. It made things worse: the keyboard took 170 vias instead of 113, Issue015 expanded 20 million states instead of 9, Issue034 lost 10 connections; only Mars-64 gained (74 of 106 instead of 72, half the expansions). Charging every cell that holds routed copper overestimates, as a way can pass beside the copper inside the same cell; a map like this would have to count crossings, not cells.

Tried in October 2026, as the cheapest form of a coarser graph: in open space (`is_open_around`), a search going straight leaps 4 or 8 nodes ahead instead of one, and its single straight step is left out (connections routed, vias, time, nets complete; `--time-limit 90`):

| Board | Steps (today) | Leaps of 4 | Leaps of 8 |
| --- | --- | --- | --- |
| keyboard-tkl | 315, 101 vias, 0.96 s | 315, 127 vias, 1.88 s | 315, 113 vias, 1.16 s |
| Issue015 | 61, 130 vias, 23 s | 61, 133 vias, 28 s | 61, 108 vias, 20 s |
| Issue034 | 147, 216 vias, 32 s | 147, 243 vias, 67 s | 147, 170 vias, 18 s |
| Issue110-RelayModule | 88, 84 vias, 25 s | 88, 81 vias, 12 s | 88, 79 vias, 30 s |
| Mars-64 (Issue102) | 72 nets, 168 vias | 74 nets, 215 vias | 73 nets, 256 vias |
| Issue066 | 221 of 266 nets | 208 nets | 213 nets |
| Issue103-Board-Unrouted | 160 of 282 nets | 154 nets | 154 nets |

Rejected: the boards that take the whole limit, the reason for this task, complete fewer nets, and elsewhere it is as often slower as faster. Bends only every 4 or 8 nodes in open space change which routes negotiation finds more than the leaps save. A graph of corners would have to keep a bend possible wherever one matters, not only where the grid of leaps allows it.

Ending negotiation after two rounds without gain left the Mars-64 boards at 62 of 94 nets instead of 71; four rounds route them as completely as before ([[Benchmarks#negotiation-that-goes-round-in-circles-m2-trx-48]]).

Profiled again in October 2026 (Mars-64 on one thread, gdb stack samples of a release build with line tables): a fifth of the time went into `is_near_a_via_of_its_path`, which walked each path back from every node a via could go, and half into copper queries. Two fixes, the routes the same to the byte on every board measured ([[Benchmarks#ripped-up-copper-out-of-the-index-m2-trx-48]]):

- Each search state keeps how far its path has run since its last via; the walk back starts only where a via of the path can still be within reach.
- Removing a route took its copper out of the layout by a flag, leaving it in the spatial index, where every later query walked past it: after thousands of rip-ups, mostly dead entries around the busy pins. Removing it now takes it out of the index. Issue034 routes in 39–41 s instead of 55–61 s, caniot in 49 s instead of 73 s, RelayModule in 7 s instead of 9–11 s; against the 90 s limit the boards route about as much as before.

Next, the spatial index listed each obstacle in every cell of its bounding box, where a diagonal track or a keepout reaches only a few: it now lists each only in the cells it reaches. Sofle routes in 7.5 s instead of 30.5 s (803 keepouts), the keyboard in 1.25 s instead of 1.48 s, most boards 10–20 % faster, the sessions the same ([[Benchmarks#copper-listed-only-where-it-reaches-m2-trx-48]]). Searching the next connections side by side and committing them in order while nothing before them changed what they explored was simulated first and not built: at most 1.35× on Issue034, nothing on Issue015, as the long searches explore what every commit changes.

Profiled after that on Issue015: the copper queries are down to a fifth of the time, the open list's heap and the state arrays a third. A heap with four children per node instead of the standard binary heap, the same order of states, was no faster (0–8 % slower on five boards) and was not kept.

What is left of the copper queries is their geometry (`intersects`, `is_closer_than`), half the time on Mars-64.

Done so far: negotiation that ends when it goes round in circles ([[Benchmarks#negotiation-that-goes-round-in-circles-m2-trx-48]]), far polygon edges passed over at a glance (Issue110 in 40 s instead of 132 s), an indexed outline (three times cheaper expansions on boards with rounded corners) and one search state per node and layer instead of nine (up to 2.5 times faster), [[Benchmarks#one-state-per-node-m2-trx-48]].

**Depends on:** [[TRX-23]]
