---
id: TRX-77
title: Run each layer one way, as the board's first routing mostly ran it
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 2
layer: engine
---

Follows [[TRX-76]]: TokenBoard still has 335 new vias where Freerouting has 199, and Freerouting's tracks run in parallel bundles, one layer mostly horizontal and the other mostly vertical. Where both layers run every way, a connection running along a track of the other direction has to hop under it.

Preferred layer directions were measured in [[TRX-76]] before hops were taken out, with the front copper horizontal, and cost vias (456, 517 and 624 instead of 446). Measured again after hops and trees, the orientation decides it: with the back copper horizontal, as traxide's own routing of TokenBoard mostly runs it (64 % of the back's track length horizontal, 56 % of the front's), a step across paying half its length extra left 292 vias instead of 335; with the front horizontal, 400. The orientation that suits a board is the one its routing leans to, which only routing tells.

- [x] A failing test: with a layer's direction set, a track between two through-hole pads runs on the layer whose direction it takes, also where a straight track fits on either (`a_track_runs_on_the_layer_whose_direction_it_takes`)
- [x] The search: a step across its layer's direction costs the profile's `against_layer_direction` share of its length more, a diagonal step half that; the estimate is the lower of staying on the layer, paying its direction, and a via plus the plain distance (`Search::with_directions`)
- [x] The orientation: alternating through the stack of routable layers, the one under which more of the first routing's track length runs along its layer's direction; none where the two differ by less than 2 % of that length, or the board has one layer to route on (`directions_of`; `each_layer_runs_the_way_the_first_routing_mostly_ran_it`, `layers_run_no_way_where_the_first_routing_leant_neither`)
- [x] Routing: the board is routed as before, then once more against those directions, and the second routing stays if it completes more, or as much with fewer vias (`Router::route_plan`, `Attempt::is_better_than`; `a_board_routed_again_with_layer_directions_stays_complete_and_legal`)
- [x] Found on keyboard-tkl: a route that left a fine-pitch pin's escape stub through a via at the stub's end, while another of the net joined the pin itself, lost the stub, which was trimmed to the farthest track on it and ignored vias; the via dangled and the net fell apart. Stubs now reach the vias of their net on them too (`a_stub_reaches_a_via_of_its_net_at_its_end`)
- [x] TokenBoard, the keyboard and the corpus boards: fewer vias without losing a net (TokenBoard 335 → 297 within 90 s, 292 without a limit; ReadPlaceScope 77 → 48; see [[Benchmarks]]); routing takes up to twice as long
- [ ] The corpus gate passes
