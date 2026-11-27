---
id: TRX-56
title: Negotiate congestion until it settles
type: improvement
status: in-progress
priority: high
assignee: claude
labels: [detailed-routing]
estimate: 4
layer: engine
---

(Measured with negotiation ending after two rounds without gain; it now ends after four, which gives Mars-64 back 71 of 94 nets.) 131 connections on 9 corpus boards route alone but not together with the rest ([[Corpus-analysis#congestion]]); a wider window ([[TRX-55]]) settles 10 of them, the rest are lost in negotiation. Rip-up and reroute in `Router::route_plan` ([[crates/engines/detailed/src/lib.rs]]) stops ripping up once 2 × (number of connections) rip-ups have passed without more connections routed (`STALLED_ROUNDS`, from [[TRX-48]]), and then the connections still queued search without crossing anything and fail. On every congested board that rule ends negotiation: Issue039 after 472 rip-ups for 204 connections, Mars-64 after 412 for 106. No profile setting helps: `max_rip_ups` 100, `history_cost` 0 or 5, `heuristic_weight` 1.0, `via_cost` 0.5, `batch_size` 1 or a 0.05 mm grid route at most 9 more connections on Issue039 (155 → 164 of 204) and 3 more on Mars-64 (72 → 75 of 106).

Freerouting 2.5.0, run as a black box with 900 s, does much better: Mars-64 (Issue326) with all 94 nets against traxide's 62 (its only violations are 6 copper-to-edge ones), Issue039 with 64 of 74 nets against 32 (6 clearance violations), Issue283 with 55 of 58 against 42 (17 edge violations). Its log shows a fan-out stage that escapes the SMD pins first (236 of 242 on Issue039), then whole passes over all unrouted items with rising rip-up costs.

The routes traxide leaves on these boards wander and use many vias (Mars-64: 199 vias for 72 connections; Freerouting 155 for all of them), taking room that the last connections need.

- [x] Instrument first: per round, connections routed, rip-ups and the places of collisions; a picture of where Issue039's and Mars-64's connections take turns. `cargo run --release -p xtask -- negotiation BOARD` ([[Development#benchmarking]]); the router publishes each rip-up, with the place where the two routes meet. What it shows today (a round being as many commits as there are connections):
  - Issue039 (204): 146 routed after round 1, then 146, 134, 139 and 140 after rounds 2–5 with about 200 rip-ups each, and 157 in the last round, once rip-up has stopped and the queue routes without crossing anything. Past the first round, rip-up there only reshuffles. The rip-ups spread thinly over U1 and the resistor arrays RN1–RN4 and CA1–CA2 (at most 6 in any square millimetre); the pairs that trade places most are neighbouring pins (/VOUT4_DAC and /VOUT5_DAC at U1-21 and U1-20, five times).
  - Mars-64 (106): 57, 65, 67, 67, 66 and 63 routed after rounds 1–6 with about 100 rip-ups each, 83 in the last round. The pairs that trade places are again neighbouring pins of U101 and U102 (SPI_CLK and SPI_MOSI at pins 19 and 18).
  - keyboard-tkl (315): complete in two rounds, 47 rip-ups, all around the RP2350 (U1), with COL15 and COL16 at U1-12 and U1-13 among the pairs.
  - So the time goes to neighbouring fine-pitch pins taking each other's way out, over and over; the connections that only fit once nobody may rip up are the ones that come last. Both point at escaping the pins first.
- [ ] Negotiate in rounds over all connections, PathFinder's way: while negotiating, overlaps are allowed at a present-congestion price that rises each round, history costs accumulate, and only the final round must be legal; compare with the current first-come rip-up on the corpus
- [ ] A stopping rule from progress across rounds and the time budget, not a fixed count of rip-ups. Meanwhile, with the pins escaped, two rounds without gain end negotiation instead of four: as many boards complete, 4 connections more over the corpus, a tenth less time, half on the churning boards ([[Benchmarks#fine-pitch-pins-escaped-first-m5-trx-56]]). And `traxide route --time-limit SECONDS` ends negotiation at the caller's limit: what is still queued routes once more without ripping anything up, each in its own window only; with 90 s, the six boards that stop at 120 s give their routes after 93–114 s (Issue289-VGA after 200–220 s: its searches took 0.5–2 s each, all in working out the largest clearance on every copper query over its 242 net classes, since fixed: [[Benchmarks#the-largest-clearance-worked-out-once-m2-trx-48]]) ([[Benchmarks#a-time-limit-m5-trx-56]]). Still open: a rule from the progress across rounds itself
- [x] Try escaping fine-pitch SMD pins first (a short stub and a via, planned before the long connections, as [[TRX-24]] plans hemmed-in pads first) and measure it separately: kept as a stub without a via ([[Benchmarks#fine-pitch-pins-escaped-first-m5-trx-56]]); the keyboard routes in half the time, RelayModule and Issue297 complete, Mars-64 loses 2 connections
- [ ] Corpus, unrouted connections before → after: Issue039-bug-design (49), Issue102 and Issue326 Mars-64 (23 each, with [[TRX-54]]'s 11), Issue283-UnconnectedTracesUnderPads (22), Issue508-DAC2020_bm01 (3), Issue269-caniot-tiny-arm (1); the keyboard and the key matrices as before ([[Benchmarks]])

Tried in October 2026, on top of [[TRX-55]]: negotiation in rounds (every connection routed each round, crossing others at a present price for each connection crossed, from `rip_up_cost` up by half each round, history recorded where they overlap, only the overlapping ones routed again; ending when four rounds bring no fewer overlaps, then the most overlapping routes taken out until the rest is legal), in three ways of finishing (connections routed, measured on this machine):

| Board | Rip-up (today) | Rounds, then the leftovers routed strictly | Rounds, then rip-up | Rounds, then rip-up within a budget of the leftovers |
| --- | --- | --- | --- | --- |
| keyboard-tkl (315) | **315**, 1.4 s | 314, 3.2 s | **315**, 4.5 s | 314, 3.8 s |
| Issue034 (147) | **147**, 23 s | 146, 11 s | **147**, 24 s | 146, 11 s |
| Issue039 (204) | 157, 63 s | **180**, 153 s (53 of 74 nets instead of 37) | over 300 s | 158, 177 s |
| Issue102 Mars-64 (106) | 81, 90 s | **85**, 60 s, 200 vias instead of 284 | over 300 s | 80, 81 s |
| Issue110-RelayModule (88) | 77, 68 s | 86, 27 s | **88**, 64 s | 86, 36 s |
| Issue015 (61) | **61**, 18 s, 130 vias | 61, 33 s, 121 vias | 61, 36 s | 61, 35 s |
| Issue029 (91) | 87, 149 s | | over 300 s | 83, 116 s |

No variant beats rip-up without losing a complete board or taking far longer, so none was kept. The code of all of them is in [[docs/Research/experiments/trx-56-rounds.patch]] (applies to the commit that recorded it; `TRAXIDE_ROUNDS=1` switches the rounds on, `TRAXIDE_LOCKED=1` finishes as below). Rounds settle congested boards much better (Issue039, RelayModule, fewer vias everywhere), but the last few overlaps never resolve, and the routes taken out then no longer fit; rip-up afterwards finishes them on easy boards but churns without end on hard ones. Worth trying next: the overlaps of the last rounds by place rather than by count (they may sit at a few pins, where escaping fine-pitch pins first would help), and finishing with rip-up only for the connections that were taken out, the rest locked.

Tried next, after [[TRX-60]]: rounds, then rip-up and reroute for the leftovers only, every route the rounds settled locked (connections routed, time; same machine, 300 s limit):

| Board | Rip-up (today) | Rounds, then the leftovers ripping up only each other |
| --- | --- | --- |
| keyboard-tkl (315) | **315**, 1.5 s | 314, 4.0 s |
| matrix-6x17 (306) | **306**, 0.2 s | **306**, 0.2 s |
| Issue034 (147) | **147**, 28 s | 146, 12 s |
| Issue015 (61) | **61**, 19 s, 130 vias | **61**, 38 s, 121 vias |
| Issue110-RelayModule (88) | 84, 91 s | **87**, 24 s, 79 vias instead of 108 |
| Issue039 (204) | **157**, 72 s | over 300 s |
| Issue102 Mars-64 (106) | 83, 39 s | **87**, 110 s |
| Issue283 (200) | 182, 93 s, 396 vias | **188**, 84 s, 305 vias |
| Issue029 (91) | **87**, 167 s | 86, 203 s |

Rejected for the same reason as the others: the keyboard and Issue034 lose the one connection whose route the rounds took out last, which no longer fits between locked routes; on Issue039 the leftovers churn. Every rounds variant so far loses exactly that one connection on those two boards, and both times it ends at a fine-pitch pin: COL15 from SW16-1 to U1-12, a pin of the keyboard's RP2350 (QFN), and /M or /N from U4 to U5 on Issue034. Rounds and escaping fine-pitch pins first may well need each other: with the pins escaped before negotiation, nothing is left to take out at a pin.

Escaping fine-pitch pins first, measured in October 2026 ([[Benchmarks#fine-pitch-pins-escaped-first-m5-trx-56]]): the pins of parts with at least 8 pins that another net's copper hems in, and that connect to another part, get a stub straight away from their part before any connection is routed, held as fixed copper of their net; the connections leaving the part start at the stub's end (`traxide_planning::escape`, [[crates/engines/planning/src/escape.rs]]). Kept: the keyboard routes in 0.82 s instead of 1.64 s with 101 vias instead of 113, and over the whole corpus 49 boards complete instead of 47, with 18 connections more; only Mars-64 loses, 2 connections each, taking twice as long, for reasons not yet known. A via at each stub's end, as the checklist proposed, was rejected: it made the keyboard slower (2.1 s) and cost Mars-64 four connections, as 0.6 mm vias do not fit beside each other at 0.5 mm pitch.

Measured with `xtask negotiation` and with the rounds patch, before merging (escape alone, then with `TRAXIDE_ROUNDS=1`):

| Board | Without escape | With escape | Rounds alone | Rounds + escape |
| --- | --- | --- | --- | --- |
| keyboard-tkl (315) | 315, 1.5 s, 47 rip-ups | 315, 0.8 s, **18 rip-ups** | 314, 3.9 s | **315**, 1.5 s |
| Issue034 (147) | **147** | **147** | 146, 13 s | 146, 13 s |
| Issue039 (204) | 157, 72 s, 880 rip-ups | 164, 83 s, 941 rip-ups | 160, 153 s | **168**, 108 s |
| Mars-64 (106) | 83, 44 s, 618 rip-ups | 81, 88 s, 946 rip-ups | 82, 61 s | **85**, 95 s |
| RelayModule (88) | 84 | **88** | 85, 24 s | 87, 18 s |
| Issue029 (91) | 87 | | 83, 128 s | 83, 131 s |

- Escaping ends the churn at the keyboard's RP2350, and with it the rounds patch keeps the keyboard complete: COL15 at U1-12 escapes.
- On Issue039 the churn moves from U1's pins 20 and 21 to the GND, GNDA and resistor-network connections; it does not get smaller.
- On Mars-64 the stubs make the fight at U101's pins 18 and 19 worse (SPI_CLK and SPI_MOSI trade places 13 times instead of 5): the stubs are fixed copper there, which rip-up cannot move. That is the likely cause of its 2 lost connections.
- Rounds with the escape route the most yet on Issue039 and Mars-64, but still lose Issue034's /M from U4-3 to U5-3, where no pin is fine-pitch by the escape's rule. Merged: the escape alone ([[docs/Benchmarks.md]]); the rounds stay out until Issue034 keeps its last connection.

Tried after the merge, for Mars-64: no escape for a pin whose connections all lead to another escaped pin, as SPI_CLK and SPI_MOSI run between U101 and U102. Rejected: Mars-64 routes 80 instead of 81 (in 50 s instead of 85 s), Issue297 loses its completeness (160 of 161), caniot one connection, Issue039 gains one; the keyboard, RelayModule and bm01 route the same. Next to try for Mars-64: stubs that negotiation may move, held as the start of their connection's route rather than as fixed copper.

Measured again on 331073f (fine-pitch pins escaped, two rounds without gain; [[docs/Research/experiments/trx-56-rounds.patch]] applies cleanly), connections routed and time:

| Board | Rip-up (main) | Rounds | Rounds, settled routes locked |
| --- | --- | --- | --- |
| Mars-64 Issue102 (106) | 81, 26 s | 89, 76 s | **93**, 147 s |
| Issue039 (204) | 166, 36 s | 166, 67 s | **177**, 135 s |
| Issue508-bm01 (195) | 178, 56 s | 180, 158 s | **184**, 223 s |
| Issue283 (200) | 182, 84 s | 176, 40 s | **188**, 71 s |
| Issue269-caniot (89) | 71, 10 s | 73, 30 s | **75**, 43 s |
| Issue029 (91) | 85, 114 s | 84, 107 s | **86**, 182 s |
| Issue034 (147) | **147**, 23 s | 146, 11 s | 146, 12 s |
| Issue110-RelayModule (88) | **88**, 18 s | 87, 15 s | 86, 14 s |
| keyboard-tkl, matrix-6x17, Issue015 | complete | complete | complete (keyboard 1.6 s instead of 0.8 s) |

Locked rounds route the most on every congested board, but lose a connection on two boards rip-up completes and take two to four times as long. So they cannot replace rip-up; next to try: run them only where rip-up leaves connections unrouted, as a second attempt, and keep the result that routes more (a board rip-up completes never pays for it), within the time limit.

Done: `Router::route_plan` routes a board a second time where rip-up leaves connections unrouted, in rounds with the settled routes locked, and keeps the attempt that completes more nets (more connections where they tie); the time limit covers both ([[Benchmarks#a-second-attempt-in-rounds-where-rip-up-falls-short-m5-trx-56]]). Without a limit Mars-64 routes 93 instead of 81, Issue039 177 instead of 166, bm01 184 instead of 178, Issue283 188 instead of 182, caniot 75 instead of 71, Issue029 86 instead of 85, in three to seven times the time; with 90 s, Mars-64 85, caniot 75, Issue039 167, the rest as before. The boards rip-up completes route the same in the same time.

- [x] A second attempt in locked rounds where rip-up leaves connections unrouted, the better kept
- [x] The corpus routes every board with the 90 s negotiation limit that [[TRX-64]] gives KiCad projects, so that the second attempt cannot push a board past the 120 s timeout, and the baseline raised: 13 boards gain, TokenBoard completes all 241 nets; Issue283 completes 38 to 41 of 58 within the limit instead of 44 without one, as main does within it ([[Benchmarks#a-second-attempt-in-rounds-where-rip-up-falls-short-m5-trx-56]])
- [x] The gate notes rather than fails fewer nets on a board routed against the clock (taking the negotiation limit in the baseline or now): on CI, Mars-64, Issue326, Issue208 and Issue289-VGA routed 2 to 9 nets fewer than the local baseline within the limit, while Issue219 routed 36 more and Issue283 6 more, on the same commit

Tried in November 2026 on f480a8c: rip-up giving up after one round without a new most-routed (`STALLED_ROUNDS` 1) instead of two, to leave the second attempt more of the time limit. Not built: it is as good, not better.

- Over the corpus (120 s timeout, 90 s negotiation), 52 boards complete instead of 51 (Issue029) and 9 nets more out of 8877. The boards that complete within the limit route the same nets with the same vias, except Issue034 (59 vias instead of 69).
- Eight time-limited boards gain 22 nets, six lose 13; repeated twice each, the gains and losses do not hold:

  | Board (connections) | Stall after one round | Stall after two (today) |
  | --- | --- | --- |
  | Issue289-FHT-8086 (665) | 551, 552 | **555, 555** |
  | Issue208 (422) | 225, 224 | **227, 230** |
  | Issue103-Unrouted (702) | **596, 599** | 593, 595 |
  | Issue145-smoothieboard (582) | **577, 577** | 574, 574 |
  | Issue029 (91) | **91, 91**, 144 vias | 88, 88, 103 vias |

- 4083 against 4079 connections in all, within the run-to-run spread of a few connections on these boards. Issue029 completes, but with 41 more vias.
- So the number of rounds rip-up may stall is not what limits these boards; a stopping rule needs a better signal than a count.

**Depends on:** [[TRX-54]], [[TRX-55]]: both remove failures that otherwise look like congestion.
