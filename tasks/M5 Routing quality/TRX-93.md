---
id: TRX-93
title: Growing trees leaves no pad and no via without its copper
type: bug
status: review
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

CI's corpus gate for [[TRX-92]] failed on two boards whose every connection was routed: Issue034 within 90 s left VCC in two parts with a dangling via, and DAC2020 bm04 left a GND pad, U51-2, unconnected. Both come from growing trees ([[TRX-76]]), which leaves a connection where it is once another ends on its copper; [[TRX-92]] only made them show.

- **A route of a via alone.** Growing trees may join two copper of the net that cross on different layers with a via and no track. `ends_on` looked at the ends of a route's tracks only, so the connection whose copper the via stood on was not held, and moved away later; on Issue034 the via was left dangling.
- **A pad left alone.** A connection's sides are the connections joined to each of its pads by the plan, but a route grown onto copper beside its pad no longer reaches the pad. On bm04, U51-2 to U47-2 grew to start on copper joined to U51-2 only through U50-2 to U51-2; that one then grew to end on the first, and nothing reached U51-2.

- [x] Failing tests: a route of a via alone ends on the copper it stands on (`a_route_of_a_via_alone_ends_on_the_copper_it_stands_on`); a route grown off its pad holds the routes still reaching the pad (`a_route_grown_off_its_pad_holds_the_routes_still_reaching_the_pad`)
- [x] `ends_on` counts a route's vias; a route grown off a pad holds the connections of that side still reaching it, and is not taken where none does (`holding_pads_left`)
- [x] Issue034 and bm04 within 90 s: every net complete, no rule broken
- [x] With `--profile fast`, TokenBoard and 28 corpus boards: 1,621 vias instead of 1,618 (Issue034, Sofle and keyboard-tkl one more each)
- [x] The corpus gate passes (Benchmark run 169)
