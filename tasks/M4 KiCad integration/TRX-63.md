---
id: TRX-63
title: More of the KiCad plugin: route selected nets, optimise, a progress window
type: feature
status: in-progress
priority: medium
labels: [kicad, client]
layer: client
---

What the first release of the native plugin ([[TRX-33]]) left out, each waiting on something that does not exist yet.

- [x] Routing restricted to some nets, the rest kept as it is: `traxide route --net NAME`, once per net (`RouteJob::nets`). Plane joins and the joins after routing ([[TRX-67]]) are made only for those nets: a net left out, whether by `--net` or a net class left unrouted, gets no copper. TokenBoard's USB_D_P and USB_D_N alone: 10 connections in 1.6 s
- [x] *Route selected nets* (UC3 of [[Architecture/Use-cases]]): the selection read over IPC (`GetSelection`), the nets of its copper and of its footprints' pads routed as above (`Nets::SelectedInKicad`, `traxide route --selected`); a selection of no net is an error that says what to select. The second button's entry point is a copy of the binary named `traxide-selected`, told apart by `argv[0]` ([[Research/KiCad-integration#the-plugin]]); `cargo xtask e2e` selects a footprint in the running KiCad, presses the button and requires copper of its nets only
- [ ] *Optimise* (UC4): the board's own unlocked wiring made shorter and simpler, the rest kept, verified, and written back in place
  - [x] Optimization Engine: segments meeting end to end where nothing else joins their net chained into tracks and pulled tight; never at a via, a pad of the net or a branch, so nothing comes apart; arcs left as they are (`traxide_optimization::optimise`; `segments_meeting_end_to_end_are_pulled_tight_as_one_track`, `segments_meeting_on_a_pad_of_their_net_stay_on_it`, `a_branch_stays_joined_where_it_meets_the_segments`)
  - [ ] Board Access: replacing wiring, in a board file and in the running KiCad, where the replaced segments are deleted by their ids and the new ones created in one commit, one undo
  - [ ] Routing Manager, `traxide optimise BOARD`, and the plugin's *Optimise* button
- [ ] Progress window from events, with cancel: KiCad 9 and 10's API offers no dialog, so the window is the plugin's own and needs a GUI toolkit; weigh its size and licences against KiCad 10's notifications, which show the summary the plugin writes to stderr once it is done
