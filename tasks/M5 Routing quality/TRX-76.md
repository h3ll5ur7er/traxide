---
id: TRX-76
title: Take out via hops by moving what they hop over
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing, optimization]
estimate: 3
layer: engine
---

A reviewer of TokenBoard, routed by Freerouting 2.5.0 and by traxide, would rather wait Freerouting's 25 minutes: its routing looks tidier, with fewer vias (199 new vias against traxide's 446) and parallel bundles where traxide's tracks wander.

Measured in October 2026 on TokenBoard's KiCad board (traxide on main, 34 s): the two use the same copper (10.07 m against 10.11 m), but traxide 2,728 segments against 1,915, and

| New vias | Freerouting | traxide |
| --- | --- | --- |
| all | 199 | 446 |
| in pairs that hop under 6 mm of the other layer, to pass another track | 44 | **250** |
| within 2 mm of another via of their net | 6 | 74 |
| on the column nets COL0–8 | 105 | 206 |
| on the row nets | 21 | 62 |

Most of the difference is hops: a connection runs on one layer, drops to the other for a few millimetres to pass a track there, and comes back. Tidying ([[TRX-72]]) searches each connection with vias again among the rest, crossing nothing, and keeps the route with fewer vias; a hop stays wherever the track it passes leaves no way on the first layer. Taking a hop out means moving that track.

Also measured, and not the way: preferred layer directions as a cost (a step across a layer's direction paying 25 %, 50 % or 100 % of its length extra, the estimate the lower of staying on the layer and a via plus the plain distance) gave 456, 517 and 624 vias instead of 446; a dearer bend (1.0 mm instead of 0.1 mm per 45°) 2,425 segments instead of 2,728 but 460 vias.

- [x] A failing test: two connections whose routes cross on one layer, one hopping over the other, end with the hop gone where moving the other connection makes room (`a_hop_under_a_track_that_could_run_on_the_other_layer_is_taken_out`)
- [x] After tidying, for each routed connection with more vias than it needs: search it again with vias as dear as tidying makes them, allowed to cross routed connections at a quarter of a via for each grid edge it overlaps them; take out what it crosses, route those again among the rest without crossing anything, and keep the change only if every one of them routes and the vias of all of them together fall (`Router::take_out_hops`). A second pass tries only the connections whose window meets copper that moved
- [ ] TokenBoard: fewer new vias without losing a net, within the time limit; the corpus gate passes

**Taking out hops, measured** (this machine, `--time-limit 90`; every board as complete as before, and no rule broken that main does not break):

| Board | Vias before | **After** | Time before | After |
| --- | --- | --- | --- | --- |
| TokenBoard's KiCad board | 446 | **363** | 34 s | 60 s |
| Sofle (Issue178) | 518 | **471** | 6.8 s | 19.6 s |
| Issue015-StackOverflow | 98 | **74** | 13.8 s | 19.2 s |
| Issue034-Green14SegLED | 150 | **134** | 33.6 s | 34.8 s |
| Issue110-RelayModule | 64 | **54** | 5.8 s | 7.1 s |
| Issue230-CNH | 92 | **88** | 6.0 s | 8.9 s |
| keyboard-tkl | 93 | **87** | 1.5 s | 3.2 s |

On TokenBoard the first pass took 14 s and 66 vias out, the second 10 s and 16 more, a third would take 6 s for 2. Of the first pass's 156 connections with more vias than they need, 59 found no way with fewer vias even crossing others, 42 crossed connections that then found no room, 37 gained less than the others lost, 18 succeeded. The price of a crossing decides much: a sixteenth of a via per overlapped edge left 405 vias, a quarter 379, half a via 428, a whole via 431. Limiting how many connections one may cross saved little time (57 s even at one), as the time goes into the searches that may cross.

What is left on TokenBoard's COL4 (26 vias, now 22; Freerouting 13): short hops beside each switch, where the column passes the switch's own row or diode copper, and a row run that steps around the switches' holes every key.
