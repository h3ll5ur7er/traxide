---
id: TRX-92
title: Taking out hops prices a crossing as a whole, however wide the tracks
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

CI's corpus run ([[TRX-91]]) showed CNH (Issue230) routed with 48 vias where Freerouting takes 5; on the 20 boards both route complete without breaking rules, traxide 210 vias, Freerouting 132. Taking out hops ([[TRX-76]], [[TRX-87]]) lets a connection's new way cross routed connections at an eighth of a via for each grid edge it overlaps them: crossing TokenBoard's 0.2 mm tracks and clearances overlaps some 8 edges, about a via, but CNH's 0.25 to 0.64 mm tracks with up to 0.64 mm clearance 10 to 21 edges, two vias and more, so a hop there never paid for moving what it passes under.

- [x] A failing test: a hop under a wide track is taken out as under a narrow one (`a_hop_under_a_wide_track_is_taken_out_as_under_a_narrow_one`)
- [x] Crossing a connection costs a share of a via, spread over the edges the crossing overlaps it, from both tracks' widths and the clearance between their nets (`search::Present::PerCrossing`, `CROSSING_EIGHTHS_OF_A_VIA`)
- [x] The share, measured with `--profile fast` on TokenBoard and 28 corpus boards: three eighths of a via, 1,618 vias instead of 1,686 per edge, in 53 % more time; half a via 1,640, five eighths 1,650. CNH 19 instead of 52, 8088sbc 51 instead of 71, Pajalnaja 26 instead of 42, bm07 20 instead of 33; Sofle 362 instead of 343, Issue034 97 instead of 82 ([[Benchmarks]])
- [x] CNH with the default profile, at half a via: 13 vias without a limit instead of 37, 19 within 90 s instead of 48
- [ ] TokenBoard with the default profile
- [ ] The corpus gate passes
