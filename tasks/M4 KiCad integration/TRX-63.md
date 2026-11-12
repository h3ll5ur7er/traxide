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
- [x] *Optimise* (UC4): the board's own unlocked wiring made shorter and simpler, the rest kept, verified, and written back in place
  - [x] Optimization Engine: segments meeting end to end where nothing else joins their net chained into tracks and pulled tight; never at a via, a pad of the net or a branch, so nothing comes apart; arcs left as they are (`traxide_optimization::optimise`; `segments_meeting_end_to_end_are_pulled_tight_as_one_track`, `segments_meeting_on_a_pad_of_their_net_stay_on_it`, `a_branch_stays_joined_where_it_meets_the_segments`)
  - [x] Board Access: wiring stored in place of the board's unlocked segments and vias, arcs and locked copper kept (`store_in_place_of_wiring`): into a copy of a board file, and into the running KiCad as one commit, *traxide: optimise*, which deletes the old ones by their ids and creates the new, one undo; refused on a board changed since it was read
  - [x] Routing Manager, `traxide optimise BOARD` and `traxide optimise --kicad`, and the plugin's *Optimise* button, a copy of the binary named `traxide-optimise` with an amber icon (`traxide_routing::optimise`; `a_boards_own_detour_is_pulled_tight_into_a_copy_beside_it`, `the_tracks_of_a_differential_pair_are_left_as_they_were`, `the_optimise_button_starts_traxide_by_a_third_name`). The tracks of differential pairs stay as they are, as pulling each tight would pull a pair apart. Stored only where it changes something and breaks no rule the board did not break; the summary says which
  - [x] TokenBoard as its author routed it: 1,828 segments instead of 1,915, 10,059 mm of track instead of 10,070 mm, in 1.4 s; every net complete, no rule broken. As traxide routed it: as tight as it fits already
  - [x] Pressing the button in a running KiCad: `cargo xtask e2e` has a recipe, *detour*, whose board comes with wiring round a detour; after routing it presses *Optimise* and requires less track (24.1 mm became 20.0), no violation in KiCad's DRC, and one undo to take it back; passes in KiCad 10.0 and 9.0. The test's KiCad window is now as large as its virtual screen: at KiCad's default 1280 by 720 the toolbar hid all but one of the three buttons
- [ ] Progress window from events, with cancel: KiCad 9 and 10's API offers no dialog, so the window is the plugin's own and needs a GUI toolkit; weigh its size and licences against KiCad 10's notifications, which show the summary the plugin writes to stderr once it is done
