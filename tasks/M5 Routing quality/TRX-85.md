---
id: TRX-85
title: What a hop's way crosses finds room further off
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

Taking out hops ([[TRX-76]]) searches a connection again allowed to cross routed connections, takes out what it crosses and routes those again among the rest; the change stays only if all of them route. Measured in [[TRX-76]], of the first pass's 156 connections on TokenBoard, 42 crossed connections that then found no room. They searched only their own window, the box around their pads and 5 mm beyond, though room for them may lie further off.

- [x] A failing test: B, crossed by A's way without a hop, finds a way only round a wall's end 7 mm off its pads (`a_hop_is_taken_out_where_what_it_passes_finds_room_only_beyond_its_window`)
- [x] A crossed connection searches a window six times as far beyond its pads as the profile's detour margin (`CROSSED_ROOM`). Widening only where its own window holds no way, four times at a time as negotiation does, found room as often but not the room with fewer vias
- [ ] Measured on TokenBoard and the corpus boards
