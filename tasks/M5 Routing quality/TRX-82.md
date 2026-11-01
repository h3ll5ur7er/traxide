---
id: TRX-82
title: A diagonal step pays its layer's direction in full
type: improvement
status: review
assignee: claude
priority: medium
labels: [detailed-routing]
estimate: 1
layer: engine
---

With layer directions ([[TRX-77]]) a straight step across a layer's direction pays half its length more, a diagonal step a quarter. Long 45° runs across the keys of TokenBoard stayed cheap on the layer meant to run the other way, where Freerouting's tracks run straight in bundles.

- [x] A failing test: a diagonal step pays as much as a straight step across (`a_diagonal_step_pays_its_layers_direction_like_a_step_across`); the estimate prices the diagonal stretch of a way on the layer alike
- [x] Measured (this machine, `--time-limit 90`, every board as complete, no rule broken that the board did not break): ten corpus boards together 969 vias instead of 1,010 (Sofle 386 instead of 401, Issue022 79 instead of 97, 8088sbc 69 instead of 73, Issue015 60 instead of 63, CNH 64 instead of 66, Issue034 107 instead of 110, keyboard-tkl 61 instead of 62; RelayModule 52 instead of 49, ReadPlaceScope 49 instead of 48, bm04 42 instead of 41); TokenBoard without a limit 267 instead of 275
- [x] The boards that reach the limit as complete as such boards vary: Mars-64 77 and 78 of 94 nets instead of 75 and 76, Issue066 238 of 266 instead of 228, LogicBoard 328 of 343 instead of 333, VGA 103 of 242 instead of 107, smoothieboard 234 of 245 instead of 239
- [ ] The corpus gate passes
