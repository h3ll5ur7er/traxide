---
id: TRX-90
title: Connections that meet at a pad they share do not hold each other in place
type: bug
status: review
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

TokenBoard routed with the default profile has two vias on every key column where one would do: the column runs down the front copper, drops through a via to reach the back copper's switch pad, and a second via 6 mm on lifts it back to the front. One via at the pad, the front copper running past, would join both.

Growing trees ([[TRX-76]]) leaves a connection where it is once another ends on its copper, so nothing comes apart. It took a route that started at its own pad, where the connection before it also ends, for one ending on that connection: the connection before stayed where it was, its via with it. The pad joins the two without either's copper.

- [x] A failing test: a route that meets another at its own pad does not end on it (`a_route_that_meets_another_at_its_own_pad_does_not_end_on_it`)
- [x] `ends_on` leaves out the route's own pads
- [x] TokenBoard and the corpus boards: fewer vias, no net less complete. With the default profile, TokenBoard without a limit routes with 221 vias instead of 229, 48 of them in hops instead of 70, with 12,962 mm of track; with `--profile fast`, TokenBoard and the ten corpus boards 1,078 instead of 1,090 (TokenBoard 252 instead of 259, Sofle 343 instead of 347), none with more
- [x] Measured and not built: growing trees twice, 1,075 vias instead of 1,078 in 3 % more time
- [x] The corpus gate passes (Benchmark run 165)
