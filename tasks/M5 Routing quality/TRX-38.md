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
- [ ] Coupled regions in the plan
- [ ] Pose-based coupled A* (position and heading); split into two tracks
- [ ] Stitching vias next to layer changes

**Depends on:** [[TRX-27]], [[TRX-32]]
