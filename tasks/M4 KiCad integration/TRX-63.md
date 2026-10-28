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
- [ ] *Optimise* (UC4): needs the Optimization Engine
- [ ] Progress window from events, with cancel: KiCad 9 and 10's API offers no dialog, so the window is the plugin's own and needs a GUI toolkit; weigh its size and licences against KiCad 10's notifications, which show the summary the plugin writes to stderr once it is done
