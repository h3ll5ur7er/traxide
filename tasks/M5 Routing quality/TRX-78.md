---
id: TRX-78
title: Pour the zone of higher priority first where zones of two nets overlap
type: bug
status: in-progress
assignee: claude
priority: high
labels: [planning, verification, board-access]
estimate: 1
layer: engine
---

TokenBoard's In2.Cu holds a +3V3 zone of priority 2 inside a VLED zone of priority 0. KiCad pours +3V3 first and VLED around it, so the +3V3 vias the author locked all reach the +3V3 plane, and Freerouting's routing adds one via and two connections for the net. traxide read neither zone's priority and let no plane fill where one of another net overlaps it: planning saw none of the +3V3 copper on its plane and routed 27 connections for the net, with 9 vias, and `traxide check` counted Freerouting's routing 240 of 241 nets complete, 27 +3V3 pads unconnected.

- [x] Failing tests: a plane fills, and joins its net's copper, where a plane of another net of lower priority overlaps it (`a_plane_fills_where_a_plane_of_another_net_of_lower_priority_overlaps_it`, `a_plane_joins_its_pads_where_a_plane_of_another_net_of_lower_priority_overlaps_it`); of the same priority, still neither fills the overlap, as the board program chooses (`a_plane_joins_nothing_where_a_plane_of_another_net_overlaps_it`)
- [x] A KiCad zone's `(priority N)` is its planes' priority, 0 where it states none, and a DSN plane's 0 (`a_planes_priority_is_its_zones`); a plane gives way only to a plane of another net on its layer of at least its priority (`Plane::gives_way_to`), in planning and in the fill `traxide check` pours
- [x] TokenBoard: +3V3 needs 2 connections and 1 via; `traxide check` counts Freerouting's routing complete (241 of 241 nets, no pad unconnected)
- [ ] The corpus gate passes
