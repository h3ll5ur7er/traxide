---
id: TRX-86
title: The default routes in six rounds
type: improvement
status: in-progress
assignee: claude
priority: high
labels: [profiles]
estimate: 1
layer: access
---

The maintainer's call: quality first. `thorough` of [[TRX-84]] routed the ten corpus boards with 888 vias instead of 969 in two rounds, in about twice the time, so the default routes in its six rounds, and `thorough` goes. `fast` stays, routing once.

- [x] A failing test: the default profile routes in six rounds (`the_default_profile_routes_in_six_rounds`)
- [x] `default` takes `rounds = 6`; `thorough` goes from the built-in profiles, the help, the README and [[Profiles]]
- [x] Each round is announced (`Event::RoundStarted`, `each_round_is_announced_before_its_routing`), and `traxide route` shows it: "Routing, round 3 of 6: 63/86", where it counted the connections from 0 again with no word why
- [ ] Measured: the ten corpus boards within the corpus's 90 s, TokenBoard without a limit
