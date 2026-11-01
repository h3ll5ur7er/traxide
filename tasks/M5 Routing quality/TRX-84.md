---
id: TRX-84
title: Route in more rounds at other prices where time is left
type: improvement
status: review
assignee: claude
priority: high
labels: [detailed-routing, profiles]
estimate: 2
layer: engine
---

After [[TRX-80]] the default routes a board in two rounds, the second with vias twice as dear. Which price of running across a layer's direction suits a board differs as much as the price of a via did. Measured on this machine, `--time-limit 90`, every board as complete, the ten corpus boards together at a share of 0.35, 0.5 and 0.75: 959, 969 and 914 vias; the best of the three, board by board, 888 (Sofle 364, CNH 45, Issue034 94, RelayModule 43). TokenBoard without a limit: 267 vias at 0.5, 291 at 0.75, 276 at 1.0.

- [x] A profile's `rounds`: how many rounds of routings side by side are made while time is left, the best of all staying. The first at the profile's prices; then vias twice as dear, also without layer directions; then layer directions half as dear again and a third cheaper, at each price of a via (`LATER_ROUNDS`). Rounds after the second route only the ways with layer directions, as the way without depends on the price of a via alone
- [x] `default` keeps two rounds (six since [[TRX-86]]); `thorough` is `default` in six (`the_thorough_profile_is_the_default_routed_in_more_rounds`); a profile routes in one round at least (`a_board_is_routed_in_one_round_at_least`)
- [x] Test: more rounds route as much with no more vias (`more_rounds_route_as_much_with_no_more_vias`)
- [x] `traxide route --help` and the README name the built-in profiles
- [x] Measured (this machine, `--time-limit 900`, every board as complete): `thorough` routes the ten corpus boards with 888 vias instead of the default's 969, in about twice the time (Sofle 364 instead of 386 in 120 s instead of 56 s, CNH 45 instead of 64, Issue034 94 instead of 107, Issue022 70 instead of 79); TokenBoard without a limit 270 either way, in 427 s instead of 174 s. `default` routes as before (keyboard-tkl 61 vias, 8088sbc 69)
