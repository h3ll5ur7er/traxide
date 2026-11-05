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
- [ ] Completion gain measured on the corpus
- [ ] Deeper shoves: what moves aside may push others in turn, to a bounded depth
- [ ] Walkaround of octilinear segments: what is crossed moves beside the new way, keeping its course, instead of being searched again

**Depends on:** [[TRX-27]]
