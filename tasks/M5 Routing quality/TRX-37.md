---
id: TRX-37
title: Walk around and shove in the Detailed Routing Engine
type: feature
status: in-progress
assignee: claude
priority: medium
labels: [detailed-routing]
estimate: 8
layer: engine
---

Negotiation rips up what is in a connection's way and leaves it unrouted until it routes again, and gives up after so many rip-ups; on the corpus, connections are left unrouted among routed copper (DAC2020 bm05: "the tracks of other nets took every way out of U27-8", bm01: thirteen "no way found"). Shoving moves what is in the way only where it routes elsewhere, so nothing comes off the board.

- [x] A failing test: without rip-up, a connection shoves one in its only way to another (`without_rip_up_a_connection_shoves_one_in_its_only_way_to_another`)
- [x] Repair after negotiation, one level deep: each connection left unrouted searches once more crossing routed connections at a price for each crossing, and what its way crosses routes again around it; it routes if all of them do (`shove_aside_for_unrouted`, `move_aside`, shared with taking out hops). Two tests whose blocked connection could now be shoved make what blocks it unable to move
- [ ] Completion gain measured on the corpus. Within 90 s it seldom runs: the congested boards reach the limit while negotiating (the 20 boards CI left incomplete: 2,397 nets complete instead of 2,399, noise). Given the time, with `--profile fast` and no limit, two deep: bm01 91 of 99 nets instead of 87 (8 pads unconnected instead of 12) but 254 vias instead of 176; caniot-tiny-arm 86 of 94 instead of 82 (8 pads unconnected instead of 45), 90 vias instead of 81
- [x] A failing test: a shoved connection shoves another in turn (`a_shoved_connection_shoves_another_in_turn`)
- [x] Deeper shoves: what moves aside may cross others in turn, which move aside so too, to `SHOVE_DEPTH` (2); what the move has placed already is held and never crossed (`search::Present::PerCrossing::held`), and a failed move is taken back whole (`undo`). Taking out hops stays one deep, its results the same
- [ ] On bm05 (90 s) neither depth routes more: of its four connections left unrouted, the ways found cross what is in them, which then finds no way round the held new copper, or none out of U27's fine-pitch pads; one, two deep 50 of 54 nets as before. Pushing a track sideways, keeping its course, is what these need (walkaround, below)
- [ ] Walkaround of octilinear segments: what is crossed moves beside the new way, keeping its course, instead of being searched again

**Depends on:** [[TRX-27]]
