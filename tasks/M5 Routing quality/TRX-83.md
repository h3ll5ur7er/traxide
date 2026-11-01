---
id: TRX-83
title: Growing trees leaves the lanes it was continuing dangling
type: bug
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

A pad with a lane of the board's wiring is reached at the lane's free end, and `continue_lanes` routes a connection again from that end where its route joined the pad elsewhere ([[TRX-73]]). `grow_trees` comes after it and may search the connection again from the pad to copper of its net nearby: shorter, so taken, and the lane's free end is left dangling. Seen on TokenBoard's GP0_UART_TX (a 0.6 mm stub of the author's wiring at the MCU) and LED_EN, in routings with other prices that happened to grow those trees.

- [x] A failing test: A1's lane leads away from A2; A3's route to A2 makes a tree from A1 straight to A2's side shorter than going on from the lane's end (`a_route_that_continues_a_fixed_lane_keeps_doing_so_when_the_net_grows_as_a_tree`)
- [x] A connection whose route goes on from a free end of the wiring of one of its pads stays where it is when the trees grow (`continues_a_lane`)
