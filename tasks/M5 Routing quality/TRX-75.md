---
id: TRX-75
title: Move only the unlocked wiring that blocks a connection
type: improvement
status: backlog
priority: low
labels: [detailed-routing]
layer: engine
---

The finer form of [[TRX-59]]. `traxide route --reroute-existing` routes all of a board's unlocked wiring again. That frees Issue157's last two nets, but on caniot it loses three nets, as its author's routing of the one routing layer beats traxide's.

Moving only the wiring that blocks a connection would keep the rest: route as today, and where the board's own unlocked copper hems a connection in, take out only the wiring a search through it would cross, and route those connections again.

- [ ] A failing test: a connection hemmed in by a net's unlocked wiring routes, while the rest of that wiring stays where it is
- [ ] Issue157 completes, and caniot keeps what it routes today

**Depends on:** [[TRX-59]]
