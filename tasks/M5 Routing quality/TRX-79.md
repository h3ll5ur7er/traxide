---
id: TRX-79
title: Route the ways layers may run side by side
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing, performance]
estimate: 2
layer: engine
---

Since [[TRX-77]] the board is routed, layers are oriented as that routing leant, and the board is routed again with each layer running one way. That doubles the time: TokenBoard reaches the 90 s limit, and once the first routing reaches it, the second never starts. Routing uses one core of four ([[Benchmarks]]), so routings side by side cost little wall time; and which orientation suits a board only routing tells.

- [x] The board is routed three ways side by side, each in a copy of the layout and on its share of the threads: without directions, which publishes progress, and with the layers crosswise in either orientation; the first of the routings that complete most with fewest vias stays (`Router::route_plan`, `Router::ways_to_run_layers`). The orientation guessed from the first routing goes (`directions_of`)
- [x] Tests: layers alternate through the stack of routable layers either way, and run no way with one layer to route on or no price (`layers_run_crosswise_through_the_stack_either_way`, `layers_run_no_way_on_a_board_with_one_layer_to_route_on_or_without_a_price`); the routes stay the same on any number of threads with directions (`the_routes_do_not_depend_on_the_number_of_threads`)
- [x] TokenBoard and the corpus boards: as complete, faster, and fewer vias on Sofle, Issue034 and the keyboard (see [[Benchmarks]])
- [ ] The corpus gate passes
