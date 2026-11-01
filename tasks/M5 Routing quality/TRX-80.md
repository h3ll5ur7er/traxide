---
id: TRX-80
title: Route the board again with dearer vias where time is left
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

After [[TRX-79]], TokenBoard has 314 new vias where Freerouting has 199. The routing moves by 10 to 20 vias on small changes (main routes it with 304 vias at a via cost of 1.99 instead of 2.0), and the price of a via moves it most. Measured with the via cost of the default profile raised for all three routings (this machine, `--time-limit 90`, every board as complete unless said):

| Board | 2 mm | 3 mm | 4 mm |
| --- | --- | --- | --- |
| TokenBoard's KiCad board | 314 | 287 | 275 |
| Sofle (Issue178) | 401 | 378 | 409 |
| Issue230-CNH | 84 | 75 | 66 |
| Issue015-StackOverflow | 74 | 72 | 63 |
| Issue217-8088sbc | 93 | 74 | 73 |
| Issue022-AutoRouter_interrupted | 113 | 95 | 97 |
| keyboard-tkl, DAC2020_bm04 | 65, 44 | 67, 47 | 62, 41 |
| Issue034, RelayModule, ReadPlaceScope | 110, 49, 48 | 116, 52, 55 | 110, 49, 50 |

On boards that reach the limit, 4 mm completed more nets on Mars-64 (78 of 94 instead of 73; revE 79 instead of 76), VGA (108 of 242 instead of 103) and smoothieboard (238 instead of 236), and fewer on LogicBoard (315 of 343 instead of 333) and Issue066 (224 of 266 instead of 231), as [[TRX-72]] found before. Dearer vias for the directed routings only left LogicBoard at 315 too: its best routing is a directed one at 2 mm. Neither price suits every board; the best of both does.

- [x] Where time is left after the three routings of [[TRX-79]], the board is routed the three ways again with vias twice as dear (`DEARER_VIAS`), and the routing that completes more, or as much with fewer vias, stays; a board that reaches the limit in the first round routes as before (`Router::route_plan`, `Router::route_side_by_side`)
- [x] Test: the routing kept completes as much as routing once, with no more vias (`the_routing_kept_completes_as_much_as_routing_once_with_no_more_vias`)
- [x] TokenBoard and the corpus boards: fewer vias, no board less complete (TokenBoard 275 without a limit, CNH 66 instead of 84, 8088sbc 73 instead of 93; see [[Benchmarks]]); boards that finish take two to three times as long
- [ ] The corpus gate passes
