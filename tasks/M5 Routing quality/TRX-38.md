---
id: TRX-38
title: Route differential pairs
type: feature
status: in-progress
assignee: claude
priority: medium
labels: [rules, detailed-routing]
estimate: 6
layer: engine
---

For USB D+/D− first.

- [x] A class's pair width and gap read from the KiCad project (`diff_pair_width`, `diff_pair_gap`; KiCad's defaults 0.2 and 0.25 mm)
- [x] Pairs in the constraint set: the nets of a class with pair geometry whose names differ only in a closing `+` and `-`, or `P` and `N` (`nets_named_alike_but_for_a_closing_plus_and_minus_or_p_and_n_are_a_pair`)
- [x] Coupled regions in the plan: a pair's connections that start beside each other and end beside each other, within six pair pitches, not between pins of one part (`traxide_planning::coupled`)
- [x] Coupled routing: one centre line as wide as both tracks and the gap, from launch points in front of the pads, split into two tracks; fan-ins straight to the pad's edge and at 45°; two vias side by side at a change of layers, farther apart where the line turns; held as the board's own wiring (`traxide_detailed::route_pairs`)
- [x] Where one end leaves no room to fan in, the tracks stop at its launch point and the router joins the pads there
- [x] TokenBoard within 90 s: the USB pair 79 % coupled instead of 0 %, 238 vias instead of 241, every net complete, no rule broken ([[Benchmarks]])
- [x] The corpus gate passes (Benchmark run 177)
- [ ] Pose-based coupled A* (position and heading), so that a pair need not leave its pads square to them
- [ ] Stitching vias next to layer changes

**Depends on:** [[TRX-27]], [[TRX-32]]
