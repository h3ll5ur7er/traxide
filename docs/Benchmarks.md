# Benchmarks

Measurements of traxide on the end-to-end boards, newest first; the comparison with Freerouting runs in CI, the rest by hand; the gate of [[TRX-22]] fails the Benchmark workflow when a corpus board regresses against `corpus/baseline.json`. Every board traxide routed below was imported into KiCad 10.0.7 and passed its DRC with no violations.

[TOC]

## How to measure

```bash
cargo xtask corpus --timeout 60   # 76 real boards from Freerouting's bug reports and the KiCad projects, downloaded at run time
cargo xtask corpus --list corpus/kicad-projects.txt   # only the KiCad projects, such as TokenBoard
cargo xtask corpus --freerouting "java -jar freerouting.jar" --shard 1/6   # the same, against Freerouting; CI runs 6 shards
cargo xtask gate target/corpus/results-*.json   # against corpus/baseline.json: fails on a regression, lists improvements
cargo xtask e2e placements crossing matrix-3x4 matrix-6x17   # builds, routes and checks the recipes
time target/release/traxide route -q target/e2e/matrix-6x17/matrix-6x17.dsn
```

The machine: a 4-core cloud container (x86-64), one thread used.

## Hops taken out by moving what they pass (M5, TRX-76)

Routed by Freerouting, TokenBoard has 199 new vias; routed by traxide, 446, with the same length of track. 250 of traxide's are in pairs that hop under 6 mm of the other layer to pass another track (Freerouting: 44). After tidying, each connection with more vias than it needs is searched again with vias four times dearer, allowed to cross routed connections at a quarter of a via per grid edge it overlaps them; what it crosses routes again among the rest, crossing nothing, and the change stays only if all of them route and their vias together fall. A second pass tries only connections near what moved. This machine, `--time-limit 90`, every board as complete as before and no rule broken that main does not break:

| Board | Vias before | **After** | Time before | After |
| --- | --- | --- | --- | --- |
| TokenBoard's KiCad board | 446 | **363** | 34 s | 60 s |
| Sofle (Issue178) | 518 | **471** | 6.8 s | 19.6 s |
| Issue015-StackOverflow | 98 | **74** | 13.8 s | 19.2 s |
| Issue034-Green14SegLED | 150 | **134** | 33.6 s | 34.8 s |
| Issue110-RelayModule | 64 | **54** | 5.8 s | 7.1 s |
| Issue230-CNH | 92 | **88** | 6.0 s | 8.9 s |
| keyboard-tkl | 93 | **87** | 1.5 s | 3.2 s |

Then nets grow as trees: each routed connection, shortest first, is searched again from any copper of its net already joined to its first pad to any joined to its second, and takes the new way if it has fewer vias, or as many and is shorter; TokenBoard 335 vias, Sofle 449, Issue034 125, RelayModule 53, ReadPlaceScope 77 instead of 86, and TokenBoard's track 10,061 mm in 2,494 segments (Freerouting 10,070 mm in 1,915) ([[TRX-76]]).

Preferred layer directions, with the front copper horizontal, cost vias before the hops were taken out: a step across a layer's direction paying 25 %, 50 % or 100 % of its length extra gave 456, 517 and 624 vias instead of 446 ([[TRX-76]]).

## Each layer one way, as the first routing ran it (M5, TRX-77)

After hops and trees, the orientation decides whether layer directions pay: on TokenBoard, whose routing runs 64 % of the back copper's track horizontally and 56 % of the front's, a step across paying half its length left 292 vias with the back horizontal and 400 with the front. So the board is routed as before, the layers are oriented crosswise through the stack as that routing leant (where it leant by 2 % of its track length or more), the board is routed again against those directions, and the routing that completes as much with fewer vias stays. This machine, `--time-limit 90`, every board as complete as on main, with the same rules broken (the boards' own):

| Board | Vias on main | **Now** | Time on main | Now |
| --- | --- | --- | --- | --- |
| TokenBoard's KiCad board | 335 | **297** (292 without a limit) | 60 s | 92 s |
| Sofle (Issue178) | 449 | **439** | 21.7 s | 35.7 s |
| Issue034-Green14SegLED | 125 | **121** | 33.5 s | 71.8 s |
| Issue110-RelayModule | 53 | **49** | 7.4 s | 25.5 s |
| Issue230-CNH | 87 | **84** | 10.1 s | 26.4 s |
| Issue035-ReadPlaceScope | 77 | **48** | 4.1 s | 6.6 s |
| keyboard-tkl | 77 | **72** | 7.1 s | 22.1 s |
| Issue015-StackOverflow (leans neither way) | 74 | 74 | 18.0 s | 19.2 s |

TokenBoard's new copper: 2,900 track segments instead of 3,128 (with the board's own), 12,323 mm of track instead of 12,372. On TokenBoard the second routing reaches the limit, which leaves it 5 vias short of what it finds without one.

## Copper listed only where it reaches (M2, TRX-48)

The layout's spatial index, 1 mm cells per layer, listed each obstacle in every cell of its bounding box. A diagonal track's box is a square, and a keepout's box can be far larger than the keepout: in those cells every query found the obstacle, passed the bounding-box test and checked it exactly. Each obstacle is now listed only in the cells it reaches (the cell's square against the shape, with the keepout's clearance; the outermost cells stand for everything beyond the board, as before). Same sessions to the byte; this machine, 4 cores:

| Board | Before | **After** |
| --- | --- | --- |
| Sofle (Issue178), 803 keepouts | 30.5 s | **7.5 s** |
| Issue015-StackOverflow | 17.0 s | **14.3 s** |
| Issue027-zMRETestFixture | 21.3 s | **17.9 s** |
| Issue022-AutoRouter_interrupted | 31.9 s | **27.7 s** |
| Issue110-RelayModule | 6.7 s | **5.7 s** |
| Issue230, Issue217, Issue034, caniot | 9.0, 7.1, 34.1, 44.0 s | 7.4, 6.2, 32.4, 42.3 s |
| keyboard-tkl, median of five | 1.48 s | **1.25 s** |

**Measured and not built: searching ahead in parallel.** Routing uses one core of four on these boards (user time equals wall time): batches put connections side by side only where their whole windows are apart, and the long ones overlap everything. Searches touch far less than their window, so the next k connections could be searched side by side against the same layout and committed in queue order while nothing committed before them changed what they explored, which keeps the routes those of one connection at a time. Simulated from a log of every search (its time and the box it expanded) and every commit (the boxes of its route and of what it ripped up), with 1 mm of reach around both: with k = 16 and 8 workers Issue034 would take 37 s instead of 50 s of searching, Issue015 and RelayModule no less at all. Under two connections commit per batch on average: what each commit changes lies in what the next searches explored, around the same busy pins. Issue034's time is churn (1,297 searches for 147 connections, 38 ms each), Issue015's a few long searches across the board (15 searches over 0.5 s take 12.8 of its 16.2 s).



Profiled on Mars-64 (one thread, gdb stack samples of a release build with line tables), half the time went into copper queries and a fifth into `is_near_a_via_of_its_path`. Removing a route flagged its copper as removed but left it in the layout's spatial buckets, where every later query walked past it; after thousands of rip-ups the cells around the busy pins held mostly dead entries. Removing a route now takes its copper and holes out of the buckets. Besides: each search state keeps how far its path has run since its last via, so the walk back for a via of the path happens only where one can be within reach, and a node is found open around by asking whether anything is there rather than for every obstruction.

The sessions are the same to the byte on every board measured; this machine, 4 cores, without a time limit:

| Board | Before | **After** |
| --- | --- | --- |
| Issue269-caniot-tiny-arm | 73 s | **49 s** |
| Issue034-Green14SegLED | 55–61 s | **39–41 s** |
| Issue110-RelayModule | 9–11 s | **7 s** |
| Issue015-StackOverflow | 20–24 s | 20 s |
| Issue217-8088sbc | 8.3–8.5 s | 7.3–7.7 s |
| Issue022, Issue027, Issue230 | 38, 25, 10 s | 35, 23, 9 s |
| Sofle (Issue178) | 34 s | 33–34 s |
| keyboard-tkl, median of five | 1.69 s | 1.54 s |

Against the 90 s limit, where the time is spent anyway, the boards route about as much as before: Mars-64 93 connections instead of 87, Issue039 171 instead of 169, Issue113 254 instead of 258, smoothieboard 566 instead of 568; one run each, and routing against the clock varies by a few connections from run to run.

## Edge clearance by the footprint that draws a cutout (M4, TRX-62)

TokenBoard's custom rule lowers the edge clearance from 0.3 to 0.2 mm at the cutouts its LED footprints draw (`A.memberOfFootprint('LED*') || B.memberOfFootprint('LED*')`). traxide now reads it, so each cutout keeps its own clearance ([[TRX-62]]). On this machine, with `--time-limit 90` as in the corpus:

| | `traxide check` | KiCad 10's DRC, zones refilled |
| --- | --- | --- |
| The author's routing, as committed | 240 of 241 nets, **no rule broken** (4 edge violations before) | no violation |
| traxide before (main) | 241 of 241 nets, no rule broken | no violation, nothing unconnected |
| traxide now: 386 of 386 connections, 559 vias, 47 s | 241 of 241 nets, no rule broken | no violation, **1 missing VLED connection** |

The missing connection is a plane join of [[TRX-58]] walled off into an island of the VLED plane on In2.Cu: a +3V3 zone of higher priority 0.5 mm south of it, the LED148 cutout north of it, and vias of ROW1 and SWCLK east. The routing before happened not to close that pocket. Since [[TRX-67]], `traxide check` pours each plane around other nets' copper and zones as KiCad does and counts the pad unconnected too (240 of 241 nets), and the corpus's baseline for TokenBoard says so.

## A real KiCad project: TokenBoard (TRX-64)

October 2026, on this machine (4 cores), `cargo xtask corpus --list corpus/kicad-projects.txt --freerouting …`: the maintainer's keyboard [TokenBoard](https://github.com/h3ll5ur7er/tokenboard) at commit 478b4f9 (`kb/kb.kicad_pcb`, KiCad 10, 4 layers with In1.Cu and In2.Cu as power planes, 241 nets, 7 zones), committed as Freerouting routed it. The corpus strips its 2114 unlocked tracks and vias; the 634 tracks and 439 vias its author locked stay. `traxide check` judges all boards; it counts pads joined only through zones and planes as unconnected, so the author's power nets show 438 of them while their nets count as complete.

| Router | Time | Nets complete | Pads unconnected | Rules broken | Tracks | Vias | Track length |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Its author (Freerouting, as committed) | | 238 of 241 | 438 | 0 | 2549 | 638 | 12381 mm |
| traxide, no limit | 189 s | 236 of 241 | 6 | 0 | 8515 | 1567 | 17983 mm |
| traxide, `--time-limit 90` (as in the corpus) | 92 s | 236 of 241 | 13 | 0 | 7770 | 1487 | 17160 mm |
| traxide with [[TRX-65]], `--time-limit 90` (the baseline) | 93 s | 232 of 241 | 18 | 0 | 7690 | 1479 | 17196 mm |
| traxide on KiCad's DSN, session imported by KiCad | 189 s | 239 of 241 | 3 | 38 clearance | 9238 | 1992 | 19994 mm |
| Freerouting 2.5.0, defaults | no session after 28 min | | | | | | |

Freerouting 2.5.0 (Java 25) wrote no session within 28 minutes, nor, limited to two passes (`-mp 2`), within 40: its fanout stage alone ran five passes of about 7.5 minutes each over 872 SMD pins, and it counted 243 violations in the board as exported that it called unfixable. In the corpus's 120 s it stops. Another corpus run with traxide's Freerouting mode in its place (`--freerouting "traxide freerouting"`) tried the whole way Freerouting's session takes, from KiCad's DSN export to KiCad's import: it completes 239 nets, but breaks 38 clearance rules, 0.2 mm between VLED or GND and other nets at the LEDs' and capacitors' pads, that routing the `.kicad_pcb` itself keeps: the DSN loses them. With the time limit, traxide's result moves by a few nets between runs (239 of 241 in a second one), within the gate's tolerance of 2 %.

traxide routes the power nets with tracks, as it cannot join pads through zones yet ([[TRX-58]]): hence its tracks and vias. The 5 nets it leaves incomplete are around the RP2350 (U1) and its flash (U2): `/MCU/VREG_LX`, `/MCU/DVDD`, `/MCU/QSPI_SCLK`, `/MCU/QSPI_SD1`, `ROW9`.

**KiCad's DRC disagreed with `traxide check`.** With the zones refilled (`kicad-cli pcb drc --refill-zones`), KiCad 10 found 199 `copper_edge_clearance` violations on traxide's board: GND tracks 0.136 mm from the cutouts the LED footprints draw on Edge.Cuts, where the board's custom rule asks for 0.2 mm. [[TRX-65]] makes such loops holes in the board, which routing and checking keep the edge clearance from: traxide then routes 232 of 241 nets, and KiCad's DRC with the zones refilled finds no violation, only the 9 incomplete nets. `traxide check` now counts 4 edge clearance violations on the author's board, where KiCad finds none: tracks of COL1, ROW2, COL5 and COL8. Without refilling, KiCad also counts the old zone fills against the new tracks.

These numbers predate the planes of [[TRX-58]], which joins pads through zones and planes: with it, traxide routes 236 of 241 nets with 5 pads unconnected in 154 s, and `traxide check` counts the author's board at 240 of 241 nets with 27 pads unconnected.

## traxide against Freerouting 2.5.0

October 2026, `cargo xtask bench` in the Benchmark workflow on a GitHub Actions runner (ubuntu-latest, 4 vCPUs): both routers get the same DSN exported by KiCad 10, KiCad 10 imports each session, and KiCad's own DRC judges both. Freerouting ran with its defaults (`-de board.dsn -do board.ses --gui.enabled=false`); traxide with its default profile. Two runs on different days' runners:

| Board | Router | Time | Unconnected | Violations | Tracks | Vias | Track length |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| matrix-6x17 | **traxide** | **1.4 s / 1.0 s** | 0 | **0** | 547 | **23** | **5947 mm** |
| matrix-6x17 | Freerouting | 54.4 s / 90.0 s | 0 | 30 / 18 | 852 / 831 | 40 / 36 | 6605 / 6662 mm |
| keyboard-tkl | **traxide** | **9.8 s / 7.2 s** | 0 | **0** | 1552 / 1550 | **108** | **8304 mm** |
| keyboard-tkl | Freerouting | 266.2 s / 480.9 s | 0 | 199 / 199 | 1645 / 1577 | 225 / 230 | 8548 / 8574 mm |

**After the speed work of M2** ([[TRX-49]], [[TRX-24]], [[TRX-50]]), the same benchmark on a CI runner:

| Board | Router | Time | Unconnected | Violations | Tracks | Vias | Track length |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| matrix-6x17 | **traxide** | **0.5 s** | 0 | **0** | 561 | **19** | **5942 mm** |
| matrix-6x17 | Freerouting | 29.2 s | 0 | 7 | 831 | 47 | 6672 mm |
| keyboard-tkl | **traxide** | **1.5 s** | **0** | **0** | 1616 | **107** | **8279 mm** |
| keyboard-tkl | Freerouting | 105.3 s | 1 | 199 | 1647 | 244 | 8345 mm |

Freerouting's times vary a lot between runners (29–90 s and 105–481 s over three runs); traxide's routes never change. In the first two runs, traxide was 39–90 times faster on the matrix and 27–67 times on the keyboard, with about half the vias and less track, and its boards are the ones KiCad finds clean. traxide's routes are the same in every run; Freerouting's differ from run to run.

**What Freerouting's violations are.** In the second run KiCad listed them by kind: all 18 on the matrix and all 199 on the keyboard are `track_width`, with no clearance violation and nothing unconnected. They are tracks narrower than 0.2 mm, the minimum track width of KiCad 10's default board setup, which the recipes keep, and the width the DSN asks for. So they come from Freerouting's own choice of narrower tracks, not from a rule the DSN leaves out; the KiCad edge clearance that tripped traxide early on ([[Research/KiCad-integration#rules-the-dsn-leaves-out]]) did not trip Freerouting here. The Benchmark workflow now keeps the sessions, DRC reports and routed boards as an artifact, to show where those tracks are.

## A second attempt in rounds where rip-up falls short (M5, TRX-56)

Where rip-up and reroute leaves connections unrouted, the board is routed a second time: in rounds first, PathFinder's way, then rip-up and reroute for the leftovers only, every route the rounds settled locked. The attempt that completes more nets is kept, the one that routes more connections where they tie, and a board rip-up completes never pays for the second ([[TRX-56]]). Measured on this machine, connections routed, vias and time, without a limit and with `--time-limit 90` (the limit covers both attempts):

| Board | Rip-up | **With the second attempt** | Rip-up, 90 s | **With the second attempt, 90 s** |
| --- | --- | --- | --- | --- |
| Issue102 Mars-64 (106) | 81, 234 vias, 27 s | **93**, 261 vias, 189 s | 81, 26 s | **85**, 178 vias, 91 s |
| Issue039 (204) | 166, 327 vias, 37 s | **177**, 337 vias, 192 s | 166, 36 s | **167**, 217 vias, 91 s |
| Issue508-bm01 (195) | 178, 445 vias, 54 s | **184**, 459 vias, 326 s | 178, 58 s | 178, 92 s |
| Issue283 (200) | 182, 396 vias, 78 s | **188**, 305 vias, 191 s | 182, 80 s | 182, 91 s |
| Issue269-caniot (89) | 71, 122 vias, 9 s | **75**, 144 vias, 56 s | 71, 10 s | **75**, 57 s |
| Issue029 (91) | 85, 175 vias, 110 s | **86**, 184 vias, 310 s | 84, 96 s | 84, 94 s |
| keyboard-tkl, Issue034, Issue110-RelayModule, Issue015 | complete | the same routes, the same time | | |

On 796 small random boards (one layer, no vias, 8 to 14 nets on a 10 × 10 grid of pads at 1 mm), the second attempt routed more on 138 and fewer on none. On the congested corpus boards it takes three to seven times as long without a limit, as the rounds take two to four times as long as rip-up and come after it. Within 90 s it still gains on three of six boards and never loses, as the better attempt is kept whenever the limit cuts the second short. Rejected before: the rounds in place of rip-up, as they lose a connection on Issue034 and RelayModule, which rip-up completes ([[TRX-56]]).

The whole corpus, on this machine, with every board now negotiating for 90 s of the corpus's 120 s (KiCad projects did already), against the baseline of [[TRX-22]] (no limit but on projects): no board breaks a rule, the boards complete before stay complete, and 13 boards gain. The eight that stopped at 120 s now finish within it (Issue029, 066, 103-Unrouted, 113, 145, 208, 219 and 289-VGA); Mars-64 (Issue102) completes 75 of 94 nets instead of 70, Issue326's Mars-64 72 instead of 70, caniot 80 instead of 79, z10 82 instead of 81, and TokenBoard all 241 nets instead of 232, in 50 s instead of 93 (with the planes of [[TRX-58]]). The first choice, by connections routed, lost Issue039 five complete nets (39 of 74 instead of 44, with 168 connections routed instead of 166): the attempts now stand by nets completed first. Issue283 completes fewer nets, 38 to 41 of 58 instead of 44, as rip-up alone takes about 94 s there: main does the same within 90 s (41), and without a limit traxide now completes 47 (188 connections, 243 s). The baseline is now this run, every board within the same limit.

## Pads joined through their net's planes (M5, TRX-58)

traxide now reads planes: DSN's `(plane NET (polygon LAYER …))` and a KiCad board's copper zones. On a layer that takes no tracks (a power layer that a plane fills), the plane joins the through-hole pads and vias of its net on it, and every other pad of the net gets a short track to a via onto the plane beside it, before anything is routed: the nearest place within 2 mm of its edge, on a 0.05 mm grid, where both fit and the via leaves room for a track beside every pad of another net. Pours on layers that take tracks are routed as before: routes of other nets may cut them apart, and only KiCad's refill tells where ([[TRX-58]]). Measured on this machine against commit 8511670, the same `traxide check` judging both (connections the plan holds after the joins, nets complete, time):

| Board | Before | **After** |
| --- | --- | --- |
| Issue269-z10_module (68) | 3 unrouted, 81 of 83 nets, 24 s | 14 to route, 2 unrouted, **82 of 83** nets, **4.7 s** |
| Issue230-CNH_Functional_Tester (both, 193) | complete, 345 tracks, 144 vias, 8 s | 144 to route, complete, **264 tracks, 112 vias**, 7.7 s |
| Issue269-min_fr_test (both, 2) | complete, 28 mm, no via | 1 to route, complete, 20 mm, 2 vias |
| Issue145-smoothieboard (792), no limit | 10 unrouted, 236 of 245 nets, 126 s | 582 to route, 7 unrouted, **238** of 245 nets, 197 s |
| Issue145-smoothieboard, `--time-limit 90` | 228 of 245 nets, 194 pads unconnected | **234** of 245 nets, 23 pads unconnected |
| keyboard-tkl, matrix-6x17 (medians of 15) | 858 ms, 238 ms | the same routes, 852 ms, 240 ms |

The other boards with planes (Issue015, 022, 027, 029, 054, 093, 113, 153, 163, 208, 214, 219, 367) have them only on layers that take tracks, and route exactly as before. No session breaks a rule the board did not already break. The whole corpus, 120 s per board: 49 complete, 16 incomplete, 8 stopped, 3 failed, as before, 1559 s in all (1548 s before). KiCad 10's and 9's DRC find every e2e recipe clean, among them `inner-plane`, a ground plane on a power-type inner layer, now with the zones refilled before judging.

Rejected, on Issue145 (no limit) and z10:

| Variant | Issue145 | z10 unrouted |
| --- | --- | --- |
| A via beside every pad, searched on eight straight lines | — | 2 (IC101-28 has room only off them) |
| A via beside every pad, on the grid | 15 unrouted, 236 nets, 66 pads unconnected, 176 s: the vias beside the QFP's ground pins took the ways out of the 3.3V pins between them | 1 |
| No via for pins hemmed in by their neighbours | 12 unrouted, 234 nets, 185 s | 3 |
| **A via leaves a track's room beside other nets' pads (kept)** | **7 unrouted, 238 nets**, 197 s | 2 |

On Issue145 the vias around the QFP make negotiation go on longer: with no limit it ends later, but further; at any fixed time it is ahead. On z10, IC101-28's only place for a via lies closer than a track's room to another net's pad, and IC101-42 has none within reach.

## Fine-pitch pins escaped first (M5, TRX-56)

Freerouting's log shows a fan-out stage that escapes most SMD pins before anything else (236 of 242 on Issue039). Planning now does the same for fine-pitch pins: each SMD pin of a part with at least 8 pins that another net's copper hems in, and that connects to another part, gets a stub straight away from its part, or at 45° to either side, to the nearest point off the pin where a track may turn any way. The stubs are planned before any connection is routed, one after the other along each row, and held as fixed copper of their nets, so other connections cannot take a pin's only way out; the connections that leave the part start at the stub's end ([[TRX-56]]). Measured alone on this machine, connections routed and time (the keyboard and matrix: median of nine runs; every session as clean as before):

| Board | Before | A stub and a via (rejected) | **A stub (kept)** |
| --- | --- | --- | --- |
| keyboard-tkl (315) | 315, 1.64 s, 113 vias | 315, 2.1 s, 136 vias | **315, 0.82 s, 101 vias** |
| matrix-6x17 (306) | 306, 0.21 s | the same routes | the same routes, 0.22 s |
| Issue039-bug-design (204) | 157, 72 s | 156, 91 s | **164**, 83 s |
| Issue102, Issue326 Mars-64 (106 each) | 83, 41 s | 79, 42 s | 81, 86 s |
| Issue283-UnconnectedTracesUnderPads (200) | 182, 91 s | 182, 91 s | 182, 88 s |
| Issue508-DAC2020_bm01 (195) | 169, 80 s | 172, 99 s | **173**, 105 s |
| Issue269-caniot-tiny-arm (89) | 72, 19 s | 69, 21 s | **74**, 17 s |
| Issue034-Green14SegLED (147) | 147, 26 s | 147, 25 s | 147, 25 s |
| Issue110-RelayModule (88) | 84, 86 s | **88**, 43 s | **88, 20 s** |
| Issue015-StackOverflow (61) | 61, 19 s | 61, 20 s | 61, 20 s |

On the keyboard 36 pins escape. A via at the end of each stub made the keyboard slower and cost Mars-64 four connections: at 0.5 mm pitch, 0.6 mm vias do not fit beside each other, so only half the pins of each TQFP got one, and those vias stood in the way of the rest. With `cargo xtask bench`, KiCad 10's DRC finds the keyboard (1.0 s, 101 vias) and the matrix clean, with nothing unconnected.

The whole corpus, 120 s per board: 49 boards complete instead of 47 (Issue110-RelayModule and Issue297-myboard), none less complete except Mars-64, 18 connections more in all (Issue508-bm05 99 → 103 of 107 in 2.4 s instead of 5.1 s), and no session breaks a rule the board did not already break. Mars-64 loses 2 connections of 106 and takes twice as long: its stubs at U101's pins 18 and 19, fixed copper, sharpen the fight between SPI_CLK and SPI_MOSI there ([[TRX-56]]).

In CI (commit 7768025, 120 s per board, the same setup as the corpus runs above):

| Router | Complete | Incomplete | Breaking rules | Stopped at 120 s | Failed | Time in all |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **traxide** | **50** | 17 | **0** | 6 | 3 | 1415 s |
| Freerouting 2.5.0 | 18 | 7 | 19 | 30 | 2 | 4627 s |

Against 264cb19: Issue110-RelayModule (17 s) and Issue297 complete; Issue145-smoothieboard finishes in 78 s instead of stopping, with 236 of 245 nets; bm05 routes 50 of 54 nets instead of 47, Issue039 41 of 74 instead of 37, caniot 80 of 94 instead of 78, bm01 79 of 99 instead of 78; both Mars-64 boards 69 of 94 instead of 72. KiCad's DRC finds the keyboard, routed in 0.5 s, and the matrix clean; Freerouting leaves the keyboard with one unconnected item and 199 track widths broken after 148 s.

With the pins escaped, negotiation no longer needs four rounds without gain before it ends: two route as much. All 76 boards, 120 s each, on this machine (connections routed, time; every other board routes the same in about the same time):

| Board | Four rounds | **Two rounds** |
| --- | --- | --- |
| Issue039-bug-design (204) | 164, 81 s | **166, 42 s** |
| Issue102, Issue326 Mars-64 (106 each) | 81, 86 s | 81, **30 s** |
| Issue508-DAC2020_bm01 (195) | 173, 117 s | **178, 63 s** |
| Issue269-caniot-tiny-arm (89) | **74**, 19 s | 71, 13 s |
| In all | 49 complete, 6314 connections, 2145 s | 49 complete, **6318** connections, **1934 s** |

Issue029 stops at 120 s here either way (with no limit, 87 of 91 in 177 s with four, 85 in 132 s with two); three rounds route fewer than either. The keyboard routes the same in 0.72 s with four and 0.78 s with two (medians of seven).

In CI (commit 0640dce): traxide completes 50 boards, 17 incomplete, 6 stopped, 3 failed, 1379 s in all instead of 1415 s, and still breaks no rule; Freerouting 2.5.0 completes 18, breaks rules on 18, stops on 31, in 4867 s. Against 7768025: both Mars-64 boards route 70 of 94 nets instead of 69, in 24 s and 18 s instead of 69 s and 43 s; Issue039 44 of 74 instead of 41, in 33 s instead of 51 s; bm01 83 of 99 instead of 79, in 36 s instead of 55 s; caniot 79 of 94 instead of 80; Issue029 157 of 163 instead of 159. KiCad's DRC finds the keyboard (0.7 s) and the matrix (0.2 s) clean.

## The largest clearance worked out once (M2, TRX-48)

Every copper query of the layout looked up the largest clearance between any two net classes, to know how far around a track or via to look, by scanning the whole class-by-class table. Issue289's boards have 242 net classes, so each query scanned 58,564 entries; sampled with gdb while Issue289-VGA routed, 38 of 40 stacks were in that scan, and single searches took up to 2 s. The table is fixed once the rules are compiled, so its largest entry is now kept beside it. The same routes everywhere; on this machine, 300 s at most:

| Board | Before | **After** |
| --- | --- | --- |
| Issue289-Autorouter_PCB_FHT-8086 (665) | stopped at 300 s | **540, 81 s** |
| Issue289-Autorouter_PCB_FHT-VGA (747) | stopped at 300 s | **567, 175 s** |
| keyboard-tkl (315), median of five | 0.79 s | 0.75 s |
| Issue039, Mars-64, bm01, Issue219, Issue208, Issue113 | | the same within a few per cent |

Issue066 and Issue103-Board-Unrouted still take more than 300 s.

In CI (commit 91b6981, 120 s per board): traxide stops on 3 boards instead of 6, as Issue208 (224 of 415 nets, 93 s), Issue289-VGA (100 of 242, 95 s) and Issue289-8086 (69 of 142, 69 s) now finish; 50 complete, 20 incomplete, 3 failed, 1267 s in all instead of 1379 s, and still no rule broken. Freerouting 2.5.0: 18 complete, 17 breaking rules, 32 stopped, 4784 s. KiCad's DRC finds the keyboard (0.7 s) and the matrix (0.2 s) clean.

## A time limit (M5, TRX-56)

`traxide route --time-limit SECONDS` ends negotiation once the limit has passed, as when it stalls: what is still queued routes once more without ripping anything up, each connection in its own window only, without the wider windows of [[TRX-55]] or the check whether the board cuts it off. With 90 s, on this machine, the boards that stop at the corpus's 120 s and those near it (connections routed, time):

| Board | No limit | **`--time-limit 90`** |
| --- | --- | --- |
| Issue208-freerouting (422) | stopped at 120 s | 190, 93 s |
| Issue289-Autorouter_PCB_FHT-8086 (665) | stopped | 549, 103 s |
| Issue289-Autorouter_PCB_FHT-VGA (747) | stopped | 577, 220 s |
| Issue103-Board-Unrouted (702) | stopped | 556, 102 s |
| Issue066-Project_GP8B (703) | stopped | 649, 113 s |
| Issue113-Protein (326) | stopped | 264, 94 s |
| Issue029-hw48na (91) | 85, 132 s | 80, 99 s |
| Issue145-smoothieboard (792) | 782, 127 s | 776, 91 s |
| Issue219-LogicBoard_smt (991) | 991, 124 s | 945, 103 s |

Before the last pass kept to each window, it took minutes where many connections were still queued (Issue219 290 s, Issue208 216 s, both Issue289 boards over 300 s). Issue289-VGA's last pass alone took 100–130 s: 365 of its connections were committed within 20 s, but only 34 more by 100 s, as single searches there took 0.5–2 s each. Sampled with gdb, the time went to working out the largest clearance on every copper query, below. The corpus ran without a limit then, so that its numbers stayed comparable with the earlier ones; since [[TRX-56]]'s second attempt, it routes every board with `--time-limit` at three quarters of its timeout.

## Each edge checked once, and copper queries no wider than copper clearances (M2, TRX-48)

Two changes with byte for byte the same routes: an edge of the grid is checked once for both directions instead of once from each end, and a query for copper near a track or via looks as far as the largest clearance between copper, no longer as far as the board's edge clearance, which KiCad sets to 0.5 mm and the outline check covers on its own.

| Board | Before | **After** |
| --- | ---: | ---: |
| keyboard-tkl, complete | 1.84 s | **1.45 s** |
| Issue034-Green14SegLED, complete | 40 s | **25 s** |
| Issue015-StackOverflow, complete | 40 s | **35 s** |
| Issue110-RelayModule, 81 of 88 | 40 s | **37 s** |

## Far edges skipped at a glance (M2, TRX-48)

Stack samples of Issue110-RelayModule showed nearly all of its time in checking a track's clearance to pads drawn as polygons of dozens of edges, each edge with the exact test, at 33 µs per node expanded. Two segments whose bounding boxes lie the distance apart along an axis are now passed over at once, a polygon keeps its bounding box, and a shape far from a polygon skips its edges altogether. The routes are byte for byte the same:

| Board | Before | **After** |
| --- | ---: | ---: |
| Issue110-RelayModule, 81 of 88 | 132 s | **40 s** |
| Issue015-StackOverflow, complete | 46 s | **40 s** |
| keyboard-tkl, complete | 1.8 s | **1.9 s** |

## Off-grid pins left along their axis (M5, TRX-54)

A pin of a fine-pitch chip whose centre lies off the 0.1 mm grid could not be left: between its neighbours a track fits only on the pin's axis, and every grid node in or next to the pin lies off it ([[Corpus-analysis#off-grid-fine-pitch-pins]]). A search now also leaves every pad along its eight axes, off the grid, to where the stub first crosses a grid line beyond the copper around it, and joins the grid there by a short jog square to that line. Measured on this machine with `traxide route` and `traxide check`, unrouted connections and those reported hemmed in; every session breaks no rule the board did not already break:

| Board | Before | **After** | Hemmed in | Time |
| --- | --- | --- | --- | --- |
| Issue508-DAC2020_bm01 | 153 of 195 | **169 of 195** | 42 → **0** | 8.8 s → 94 s |
| Issue508-DAC2020_bm05 | 85 of 107 | **99 of 107** | 22 → 2 | 0.05 s → 4.8 s |
| Issue508-DAC2020_bm06 | 94 of 98 | **98 of 98** | 4 → **0** | 0.1 s |
| Issue102, Issue326 Mars-64 (each) | 81 of 106 | **83 of 106** | 11 → **0** | 126 s → **49 s** |
| Issue269-caniot-tiny-arm | 3 of 89 | 6 of 89 | 33 → 6 | 17 s → 35 s |
| Issue297-myboard | 157 of 161 | **160 of 161** | 4 → 1 | 2.6 s → 5.1 s |
| Issue157-TeamAdapt-LinePCB | 211 of 214 | 211 of 214 | 3 → 3 | 1.3 s |
| keyboard-tkl, matrix-3x4 | | the same routes | | keyboard 2.05 s → 2.10 s |

bm06 is complete, as Freerouting completes it. The time goes to the connections that now leave their pins: on bm01 and bm05 they negotiate, and those that still find no way search the whole window before they fail, where before they failed at once; a variant that walked the axes only for pads with no other way out routed the same and took as long. On caniot the other unrouted connections end at pads on its power layers ([[TRX-57]]) and now fail with "no way found" on its one routable layer. Issue157's three end at Q1-3 or Q2-3, across which the board's own unlocked track runs ([[TRX-59]]). The keyboard's time is the median of nine runs each (1.95–2.20 s before, 1.98–2.24 s after); the routes are byte for byte the same. The KiCad round trip (`cargo xtask e2e`) could not run: the container had no Docker daemon.

The whole corpus, before and after, each board stopped after 300 s: no board routes fewer connections, none breaks a rule it did not already break, and the times of the others stay within the noise. One more board gains: Issue110-RelayModule routes 84 of 88 connections instead of 77 (one pad was hemmed in), in 106 s instead of 94 s.

## A wider window where no way was found (M5, TRX-55)

A search covers the box around its pads grown by 5 mm; the corpus analysis ([[Corpus-analysis#search-window-too-small]]) found ways that run further round. A connection that finds no way now searches again, alone after its batch, in windows four times wider each, up to the whole board. Measured on this machine, every session as clean as before (the violations counted are the boards' own):

| Board | Before | **After** |
| --- | --- | --- |
| Issue093-interf_u | 172 of 173 nets | **173 of 173**, 0.1 s |
| Issue110-Pajalnaja_stancija | 59 of 64 nets | **63 of 64** (the last pad lies outside the board), 2.1 s |
| Issue178-KeebMaker_Sofle_Choc | 175 of 176 nets | **176 of 176**, 14 s |
| Issue217-8088sbc | 103 of 104 nets, 188 vias | **104 of 104, 159 vias**, 5.9 s instead of 7.6 s |
| Issue230-CNH_Functional_Tester | 763 of 764 nets | **764 of 764**, 6.6 s |
| Issue269-z10_module | 63 of 68 connections, 0.3 s | 65 of 68, 27 s |
| Issue029-hw48na | 83 of 91 connections, 40 s | 87 of 91, **149 s** |
| keyboard-tkl, Issue015, Issue034 | | the same routes in the same time |

Issue029 pays for its four connections: 97 searches in wider windows found ways (57 s), 4 found none (9 s), and the connections they routed gave negotiation more to settle. Widening each connection only until it fails once changed nothing.

## Negotiation that goes round in circles (M2, TRX-48)

On congested corpus boards the routed count rose, then swung back and forth: Issue039 between 135 and 151 of 204 connections for as long as it was let run, as connections took each other's ways in turn, each up to 20 times. Negotiation now ends once two rounds of rip-ups, as many as there are connections each, have routed no more connections than the best so far; the connections left search without ripping up others. Measured on this machine:

| Board | Before | One round | **Two rounds** | Three rounds |
| --- | --- | --- | --- | --- |
| Issue039-bug-design | > 150 s | 28 s, 158 of 204 | **54 s, 155 of 204** | 84 s, 153 of 204 |
| Issue110-RelayModule | > 150 s | 97 s, 77 of 88 | **134 s, 81 of 88** | |
| Issue034-Green14SegLED | 40 s, 147 of 147 | 28 s, 144 of 147 | **41 s, 147 of 147** | 42 s, 147 of 147 |
| Issue029-hw48na | 66 s, 83 of 91 | 45 s, 82 of 91 | **68 s, 83 of 91** | |
| keyboard-tkl | 1.9 s, complete | 1.9 s, complete | **1.9 s, complete** | |

One round cost Issue034 three connections; two keep every board as complete as before and end the circling.

Two turned out too few for the Mars-64 boards of the corpus (62 of 94 nets instead of 71). Measured again with four:

| Board | Two rounds | **Four rounds** |
| --- | --- | --- |
| Issue102-Mars-64 | 22 s, 72 of 106 | **83 s, 78 of 106** (as many as without an end) |
| Issue039-bug-design | 54 s, 155 of 204 | **65 s, 160 of 204** |
| Issue110-RelayModule | 37 s, 81 of 88 | **74 s, 77 of 88** |
| Issue029-hw48na | 46 s, 83 of 91 | **46 s, 83 of 91** |

Four route the most in all (398 connections against 391) and stay within the corpus's 120 s, so negotiation then ended after four rounds without gain; since fine-pitch pins escape first, it ends after two ([[#fine-pitch-pins-escaped-first-m5-trx-56]]). The counts move unevenly between settings: where negotiation churns, which connection wins a contested place decides much. The boards stay slow for another reason: single searches of up to half a second each where the board is crowded ([[TRX-48]]).

## Real boards: the corpus against Freerouting (M2, TRX-15)

October 2026, `cargo xtask corpus --freerouting … --timeout 120` in the Benchmark workflow, six shards on GitHub Actions runners (4 vCPUs each), on commit 7534cf5. The 76 boards are the DSN files of Freerouting's own bug reports. Both routers get the same DSN and 120 s; `traxide check` judges both sessions alike, counting only the rules a session breaks beyond those the board already broke.

| Router | Complete | Incomplete | Breaking rules | Stopped at 120 s | Failed | Time in all |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **traxide** | **40** | 23 | **0** | **10** | 3 | **1806 s** |
| Freerouting 2.5.0 | 15 | 5 | 19 | 32 | 5 | 4969 s |

- **traxide broke no rule on any board.** Freerouting's sessions break rules on 19 boards, up to 520 violations on Issue632-MiniAutoPilot.
- **Where both finished, traxide was 10–100 times faster**: the three Z80 boards (529 nets) in 0.2 s against 21.6 s or more than 120 s; Issue230 (764 nets) in 6 s against 62 s; Issue199 in 0.2 s with all 28 nets against 109 s with 1.
- **Where Freerouting connected more**, seven boards, it broke rules to do so on six: Issue632 (211 of 211 nets with 520 violations, against traxide's 124 of 211, clean), Issue508-bm05, Issue110-Pajalnaja, Issue217 (104 of 104 with 6 violations, against 103 of 104) and both Issue069 boards. Issue508-bm06 (38 of 38 against 35 of 38, both clean) is the exception.
- **traxide's gaps**: 10 boards stop at 120 s (large or congested ones: Issue039, 066, 103-Unrouted, 113, 145, 208, 219, 283, 289 ×2; [[TRX-48]]), and some finish incomplete where a pad is hemmed in or no way was found (Issue269-caniot 47 of 94, Issue102/326-Mars-64 71 of 94). Why, connection by connection, and which of them can be routed legally at all: [[Corpus-analysis]].
- **Failures**: Issue006 is a schematic, not a design; Issue143-rpi_splitter_mod names a component that does not exist; Issue420 names a pin its component lacks. Three of Freerouting's five failures are the judge's: its sessions define their own via padstacks, which `traxide check` did not read ([[TRX-15]]). It reads them now: Freerouting renames the padstacks of the vias it places (`Round1$13.779528` becomes `Round1$13`, `Via[0-1]_1016:485.7_um` becomes `Via[0-1]_1016:485_um`), so the judge takes each for the design's via with the same copper, within the session's resolution. Judged so, Freerouting 2.5.0 completes Issue229 (30 of 30 nets, clean), Issue143-rpi_splitter and Issue270 (5 of 5 nets, 2 clearances broken each, see below).

The judge is traxide's own Verification Engine, which reads the rules from the DSN as the router does; KiCad's DRC is the independent judge only for the KiCad-built boards above. It reads only a board's general clearance, not the clearances of a type such as `(clearance 6 (type wire_via))`, so it may be stricter than such a board: the clearances Freerouting breaks on Issue143 and Issue270 all involve its vias (without them none is left), and those boards allow 6 mil between a via and a wire, pin or via, where their general clearance is 12 mil; on Issue143 a track passes 6.0 mil from a via.

**After the speed work of TRX-48** (commit efc52c4, same setup):

| Router | Complete | Incomplete | Breaking rules | Stopped at 120 s | Failed | Time in all |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **traxide** | 40 | 27 | **0** | **6** | 3 | **1323 s** |
| Freerouting 2.5.0 | 17 | 7 | 16 | 31 | 5 | 4863 s |

Four boards that stopped now finish (Issue039 in 19 s, Issue113, Issue145 and Issue283 short of complete); Issue022 routes in 45 s instead of 76 s, Issue178 in 12 s instead of 69 s. One regression: the two Mars-64 boards finish in 18 s instead of 111 s, with 62 of 94 nets instead of 71: negotiation ends too early there ([[TRX-48]]).

**After the wider window and KiCad's power layers** ([[TRX-55]], [[TRX-57]]; commit e41c332, same setup):

| Router | Complete | Incomplete | Breaking rules | Stopped at 120 s | Failed | Time in all |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **traxide** | **45** | 20 | **0** | 8 | 3 | 1516 s |
| Freerouting 2.5.0 | 18 | 7 | 18 | 31 | 2 | 4842 s |

Issue093, Issue178, Issue217 and both Issue230 boards are now complete (Issue217 is one Freerouting completed only by breaking 6 rules), Issue110-Pajalnaja is short only of the pad outside the board, Issue269-caniot routes 61 of 94 nets instead of 47, Issue145 228 of 245 instead of 220 in less time. Issue029 now stops at 120 s: it routes more connections but takes longer ([[TRX-55]]).

**After pads left along their axes** ([[TRX-54]]; commit 5dd40cc, same setup):

| Router | Complete | Incomplete | Breaking rules | Stopped at 120 s | Failed | Time in all |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| **traxide** | **47** | 19 | **0** | 7 | 3 | 1561 s |
| Freerouting 2.5.0 | 18 | 7 | 18 | 31 | 2 | 4914 s |

Two more boards are complete, Issue508-DAC2020_bm06 among them (38 of 38 nets, as Freerouting routes it), and Issue029 finishes again within the limit (159 of 163 nets in 108 s). Where traxide is short of complete, the gaps that remain are Issue632-MiniAutoPilot (124 of 211 nets: most of its parts lie outside the outline, [[TRX-60]]), Issue039 (37 of 74), both Mars-64 boards (72 of 94) and bm01 (78 of 99). Seven boards stop at 120 s for both routers. On the KiCad-judged bench of the same run, traxide routes matrix-6x17 in 0.2 s and keyboard-tkl in 1.0 s, both complete and clean; Freerouting takes 33 s and 592 s and breaks 14 and 199 track widths.

Three boards sit at the 120 s limit and finish or stop from one CI run to the next with the same routes: Issue029 (108 s at 5dd40cc, stopped at 63b004d, 85 s at 264cb19), Issue145 (120 s, stopped) and Issue219 (stopped, then complete in 106 s). So at 264cb19, after [[TRX-60]], traxide completes 48 boards, 18 incomplete, 7 stopped, 3 failed, 1503 s in all, and still breaks no rule; the corpus totals move by a board or two with the runner's speed alone.

## One state per node (M2, TRX-48)

October 2026: on the real boards of the corpus, the long searches expanded about 8 states per node and layer: the heading a node was entered with was part of the search state, and as a bend costs only 0.1 mm, all nine ways into a node cost nearly the same and were each expanded. A state is now a node on a layer; its heading is the direction of the cheapest way in found, which bend costs and the rule against acute turns follow. Every board stays clean in KiCad 10's DRC; track lengths are within 0.2 % of before.

| Board | Before | One state per node | And one check around open nodes |
| --- | ---: | ---: | ---: |
| matrix-6x17 | 0.9 s | 0.35 s | **0.25 s** |
| keyboard-tkl | 3.3 s | 2.6 s | **2.2 s** |
| Issue015-StackOverflow (corpus) | 99 s | 72 s | **50 s** |
| Issue029-hw48na (corpus) | > 120 s | 103 s, 83 of 91 | **72 s**, 83 of 91 |

Then most nodes of a search lie in open space: one check of a disk around a node that covers its eight edges replaces eight checks when nothing is in reach, with the same routes.

## History costs (M2, TRX-50)

October 2026: after the new ordering, the keyboard's MCU connections still ripped each other up 96 times, nearly all within a few millimetres of the QFN's pins. Rip-up prices only protect the connection ripped up; negotiated congestion also remembers *where* routes collide. Every segment or via of a committed route that displaced others now makes its 0.5 mm cell dearer by `history_cost` for every later search ([[Profiles]]).

| `history_cost` | keyboard-tkl | Vias |
| ---: | ---: | ---: |
| 0 (before) | 7.4 s | 117 |
| 0.5 mm | 6.2 s | 135 |
| 1 mm | 4.2 s | 134 |
| 1.5 mm | 3.7 s | 119 |
| **2 mm (the default, a via)** | **3.4 s** | **107** |
| 2.5 mm | 3.4 s | 107 |
| 3 mm | 4.2 s | 117 |
| 5 mm | 8.0 s | 139 |

One run each, all cores of the 4-core container. The key matrices route as before. With the default, `cargo xtask bench` in the container: keyboard-tkl in 3.4 s, 315 of 315 connections, 107 vias, 8279 mm of track, clean in KiCad 10's DRC with nothing unconnected; the routes are the same on 1 and 4 threads.

## Hemmed-in pads first (M2, TRX-24)

October 2026: after [[TRX-49]], 83 rip-ups remained on the keyboard, all among the connections from the MCU's matrix pins, routed last because they are long: each pin's way out of the 0.4 mm pitch QFN was blocked by the copper of its neighbours. Planning now routes connections from **hemmed-in pads** first, pads with another net's copper too close for a track to pass between them, then the rest shortest first. Measured on the same DSN, three runs each, all cores of the 4-core container:

| keyboard-tkl | Time (median) | Tracks | Vias | KiCad 10 DRC |
| --- | ---: | ---: | ---: | --- |
| Shortest first | 12.6 s | 423 | 109 | clean |
| Hemmed-in pads first | **6.6 s** | 432 | 117 | **clean, nothing unconnected** |

The key matrices have no hemmed-in pads and route as before.

**Same board, same routes.** KiCad exports the parts of a board in a different order each time, and planning broke ties by that order: two exports of the keyboard routed differently, and one of them left a connection unrouted after its 10 rip-ups. Planning now breaks ties by names, so both exports give the same routes, and `max_rip_ups` is 20 ([[Profiles]]). The rip-up price made no difference: 5 and 10 mm gave identical routes.

## Each state expanded once (M2, TRX-49)

October 2026: a weighted search reaches many states first by a worse way round, and expanded them again for every better way: the keyboard's longest search expanded 3.7 million times, but only 590 000 distinct states. Now each state is expanded once, which keeps the path within the weight of the cheapest. Measured in the 4-core container, every board clean in KiCad 10's DRC with nothing unconnected:

| Board | Time before | Time after | Vias | Track length |
| --- | ---: | ---: | ---: | ---: |
| matrix-3x4 | 0.08 s | 0.08 s | 4 → 4 | 637 mm |
| matrix-6x17 | 1.65 s | **0.78 s** | 23 → **19** | 5947 → 5943 mm |
| keyboard-tkl | 13.7 s | **12.6 s** | 108 → 109 | 8304 → 8334 mm |

Each search of the keyboard's longest connection is now three times faster (0.77 s instead of 2.2 s), but the long connections out of the MCU now take turns ripping each other up, 5 to 8 searches each, for the QFN's escape room: planning their escape ([[TRX-24]]) comes next.

## Parallel batches (M2, TRX-27)

October 2026: detailed routing searches batches of connections side by side, connections whose territories (search window plus copper reach plus clearance) are apart, and commits them in queue order ([[Architecture/Parallelism-and-GPU#batches]]). Measured in the 4-core cloud container, three runs each, median, default profile (`batch_size = 16`):

| Threads | keyboard-tkl | matrix-6x17 |
| ---: | ---: | ---: |
| 1 | 13.7 s | 1.65 s |
| 2 | 13.6 s | 1.64 s |
| 4 | 13.4 s | 1.61 s |

The sessions were byte-identical in all nine runs of each board, and identical to routing one connection at a time. That correctness is the gain: **no speed-up** on these boards, for two reasons found by tracing every batch:

- **The time is in the long connections.** On the keyboard, 89 of 125 batches held a single connection, and those took 10.8 s of the 12.2 s of searching: connections from the MCU or the header across the board, whose windows overlap everything. The connections that did run side by side took under a second together. One connection alone, searched twice after rip-ups, costs 4 s: it pops 4.4 million of its window's 17.7 million states.
- **Memory.** The search arrays covered each window densely, over a gigabyte for the longest searches, and every thread needed its own. Filling fresh memory for each thread made four threads 15–30 % *slower* than one. The arrays are now tiled, and a search maps only the tiles it writes: peak memory fell from 1.15 GB to 293 MB on one thread (2.2 GB to 365 MB on four), and extra threads cost nothing. On one thread, the tiles cost a few per cent of search speed.

Batches will matter on boards with many short connections spread out; on keyboards, cheaper long searches come first ([[TRX-48]]). A first attempt that searched the next 8–32 connections speculatively, regardless of overlap, and searched colliding ones again, was 2–4 times *slower*: the long connections all converge on the MCU and kept colliding.

## Global routing corridors: measured, and off (M2, TRX-26)

October 2026: the Global Routing Engine plans a corridor for every connection on a grid of cells four track pitches wide (1.4 mm on the keyboard), with negotiated congestion; detailed routing then keeps inside the corridor, or searches the whole window when the corridor turns out to be blocked.

| Detailed routing | keyboard-tkl | matrix-6x17 |
| --- | --- | --- |
| Without corridors (the default) | **17.7 s**, 315 of 315 | **2.7 s**, 306 of 306 |
| Inside corridors | 153 s, 310 of 315 | 4.5 s, 306 of 306 |
| Corridors as guidance (steps outside cost double) | 182 s, 315 of 315 | 5.5 s, 306 of 306 |

Global routing itself takes 0.1–0.6 s. The time it costs goes into the detailed searches: the slow connections are not slow for want of a coarse plan but in their last millimetres into a crowded pin field, which a 1.4 mm grid cannot see; inside a narrow corridor such a search can only rip up instead of detouring. Corridors stay available (`global_routing = true` in a profile, see [[Profiles]]) and may pay off on larger boards; the next attempts are a lane model from the real copper instead of bounding boxes, and corridors that only guide the order of searches.

## The keyboard: a 4-layer RP2350 ISO TKL (M1)

The board traxide was started for, as `e2e/kicad/boards.py` builds it (`keyboard-tkl`): 88 MX switches in the ISO tenkeyless layout with a diode each (NKRO, 6 × 17 matrix), an RP2350-like MCU (QFN-60, 0.4 mm pitch) with USB-C, series resistors, QSPI flash, crystal, regulator and decoupling, on four signal layers; tracks 0.2 mm, clearance 0.15 mm.

| Board | Connections | Routed | Tracks | Vias | Time | KiCad 10 DRC |
| --- | --- | --- | --- | --- | --- | --- |
| keyboard-tkl | 315 | **315** | 423 | 109 | **17.7 s** | **clean, nothing unconnected** |

![keyboard-tkl routed by traxide, imported into KiCad 10: top copper red, inner layers orange and green, bottom copper blue](images/keyboard-tkl.png)

![The RP2350's fan-out: 0.4 mm pitch pins escaping on all four layers](images/keyboard-tkl-mcu.png)

What it took, each found by routing this board:

- **Search from the tighter end.** The first attempt never finished: searches into the MCU's 0.4 mm pitch pins flooded the board, because the way into a hemmed-in pin is far longer than the straight distance. Searching from the end with fewer ways in or out (the pin, 9 access states, against 1936 for a switch pad) floods only the pocket around the pin.
- **Keepouts of footprints.** KiCad exports a footprint's non-plated holes, a switch's centre post and pegs, as keepouts of its image; traxide now reads them, placed and mirrored with their footprint.

The time is spent in a handful of long searches, as on the matrices: global routing corridors ([[TRX-26]]) and parallel batches ([[TRX-27]]) come next. The comparison with Freerouting 2.5 on this board runs in CI ([[TRX-15]]), where Freerouting can be downloaded.

## Octilinear estimate and weighted search (M1)

October 2026: the search estimates the remaining cost by octilinear instead of straight distance, and ranks states by that estimate weighted with the profile's `heuristic_weight` ([[Profiles]]).

Of matrix-6x17's 307 searches, 288 took under a millisecond; all the time went to 19 long column-to-header connections of 50–145 mm, each searching up to 15 million states. Measured on that board:

| Search | Time | Track length | Vias |
| --- | --- | --- | --- |
| Straight-distance estimate (before) | 33 s | 5942.6 mm | 35 |
| Octilinear estimate | 16.3 s | 5944.0 mm | 35 |
| Octilinear, weight 1.2 (the default) | **2.2 s** | 5962.6 mm (+0.3 %) | **19** |
| Octilinear, weight 1.5 | 0.74 s | 5977.6 mm (+0.6 %) | 52 |

| Board | Connections | Routed | Vias | Time | KiCad DRC |
| --- | --- | --- | --- | --- | --- |
| placements | 7 | 7 | 0 | < 0.01 s | clean |
| crossing | 2 | 2 | 2 | 0.07 s | clean |
| matrix-3x4 | 36 | 36 | 4 | 0.35 s | clean |
| matrix-6x17 | 306 | **306** | 24 | **2.7 s** | clean |

KiCad's DRC also caught a rule the DSN does not carry: KiCad keeps copper 0.5 mm from the board edge by default. Boards from KiCad now keep at least that distance (see [[Research/KiCad-integration]]).

## Rip-up and reroute (M1, TRX-23)

October 2026, the same search, plus negotiated rip-up and reroute and octilinear shortcuts ([[TRX-47]]).

| Board | Connections | Routed | Vias | Time | KiCad DRC |
| --- | --- | --- | --- | --- | --- |
| placements | 7 | 7 | 0 | < 0.01 s | clean |
| crossing | 2 | 2 | 2 | 0.07 s | clean |
| matrix-3x4 | 36 | 36 | 4 | 0.7 s | clean |
| matrix-6x17 | 306 | **306** | 35 | 33 s | **clean, nothing unconnected** |

Complete, and still slow: the next work is speed ([[TRX-24]] to [[TRX-27]], [[TRX-48]]).

## Walking skeleton (M0)

October 2026, sequential A* on a 0.1 mm octilinear grid, no rip-up, no global routing.

| Board | Connections | Routed | Vias | Time | KiCad DRC |
| --- | --- | --- | --- | --- | --- |
| placements (parts on both sides at 0°, 30°, 45°, 90°) | 7 | 7 | 0 | < 0.01 s | clean |
| crossing (two nets that must cross) | 2 | 2 | 2 | 0.15 s | clean |
| matrix-3x4 (12 keys: MX switches, diodes on the back, header) | 36 | 36 | 4 | 0.8 s | clean |
| matrix-6x17 (102 keys, a tenkeyless-sized matrix) | 306 | 304 | 30 | 34 s | 0 violations, the 2 unrouted connections |

![matrix-3x4 as routed by the walking skeleton and imported into KiCad 10: top copper red, bottom copper blue](images/matrix-3x4-m0.png)

The picture shows matrix-3x4 after the round trip: top copper red, bottom blue, vias as small rings. Columns run on top, rows on the bottom, and the header's column pins reach their columns through vias. The straight diagonals from the header and the stubs from switches to diodes run at any angle, which [[TRX-47]] changes to 45° steps.

### What the numbers say (M0)

- **Correct before fast.** KiCad agrees with traxide's own verification on every board: what traxide stores breaks no rule.
- **Most connections are trivial.** In matrix-3x4, 32 of 36 connections are straight lines; the rest cost the time. Four searches popped up to 470 000 states each, because the search state includes the heading (9 per node and layer) and a row of switch pads forces long detours that the straight-line estimate does not foresee.
- **Long connections are the problem.** The two failures in matrix-6x17 run from the outer columns to the header across the whole board, after everything else blocked their way. Rip-up and reroute ([[TRX-23]]) and global routing corridors ([[TRX-26]]) address exactly this; batches on all cores ([[TRX-27]]) multiply the rest.
- Already done in M0: dense search arrays reused between connections (3.5 s → 0.9 s on matrix-3x4), a fast board-edge test for rectangular outlines.
