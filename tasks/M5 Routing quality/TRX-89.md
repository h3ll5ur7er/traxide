---
id: TRX-89
title: Routing past its deadline stops searching after a tenth of the time given
type: bug
status: in-progress
assignee: claude
priority: high
labels: [detailed-routing]
estimate: 1
layer: engine
---

The corpus run of [[TRX-88]] stopped LogicBoard (Issue219) after 120 s, its time limit being 90 s. Past the deadline, negotiation routes what is still queued once more without rip-up ([[TRX-56]]), "so routing ends a little later"; on LogicBoard each of the three ways of running its layers ([[TRX-79]]) drained its queue on one thread, 14, 31 and 43 s past the deadline (on this machine, 130 s in all), and the slowest decides. Not [[TRX-88]]'s doing: the stages after negotiation only start before the deadline.

- [x] A failing test: long past its deadline, routing searches nothing more (`long_past_its_deadline_routing_searches_nothing_more`)
- [x] Past its deadline, routing searches for a tenth of the time given more, a second at least, and leaves what is still queued unrouted (`SEARCHING_PAST_THE_DEADLINE`); the help of `--time-limit` says so
- [x] LogicBoard within 90 s: 100 s instead of 130 s, 316 of 343 nets complete instead of 328: the searches past the deadline still routed, but a limit overrun by half is no limit
- [ ] The corpus gate passes
