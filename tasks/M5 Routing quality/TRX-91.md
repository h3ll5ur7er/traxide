---
id: TRX-91
title: The corpus counts each router's vias and track
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [benchmarks]
estimate: 1
layer: tooling
---

The corpus table says, for traxide and Freerouting, how long each took, how many nets it completed and how many rules it broke, but not how many vias it used or how much track: on the real boards the routers compared on completion only, and vias were measured by hand on TokenBoard and ten boards. The maintainer asked for both.

- [x] Failing tests: a router is credited only with the vias and track it added to the board's own (`a_router_is_credited_only_with_the_vias_and_track_it_added`); on the boards both complete cleanly, their vias and track are summed side by side (`boards_both_routers_complete_cleanly_compare_their_vias_and_track`)
- [x] Each cell of the table names the vias and the length of track added, from `traxide check --json`; below the table, the line comparing both routers on the boards both route complete without breaking rules ([[Development]])
- [ ] CI's corpus run shows them
