---
id: TRX-24
title: Plan connections and batches in the Planning Engine
type: feature
status: in-progress
assignee: claude
priority: high
labels: [planning]
estimate: 4
layer: engine
---

[[Architecture/Routing-pipeline#plan]].

- [ ] RSMT (FLUTE-style) per net, beyond the MST of [[TRX-46]]; ordered chains for key-matrix rows and columns
- [ ] Recognise switch–diode stubs and buses
- [ ] Escape patterns for QFN pads (RP2350: QFN-60 and QFN-80, 0.4 mm pitch)
- [x] Ordering: connections from hemmed-in pads first, then shortest first; ties by names, so the order a board lists its parts in changes nothing ([[Benchmarks#hemmed-in-pads-first-m2-trx-24]]). Batches come from detailed routing's territories ([[TRX-27]])

**Depends on:** [[TRX-13]]

Measured on TokenBoard in November 2026, routed within 90 s, against the board's author:

- **Where the vias are.** Of traxide's 223 new vias, 110 lie within 50 mm of the MCU (U1), against the author's 73 of 199; farther out, 113 against 126. Within 50 mm the row nets take 31 against 15, the columns 26 against 21, the MCU's other signals the rest.
- **Why the rows.** Each row net is a line of switch pins across the board and one MCU pin. The spanning tree joins the MCU pin to the row's nearest pad, straight above or below the MCU, so each row's way to the MCU runs across the other rows and changes layers: about three vias a row. The author brings the rows to the MCU from the left as one bundle of parallel tracks on the front copper, each dropping to the MCU through a via close to it: one and a half a row. Which point of a row the MCU's connection joins, and so the bundle, is the planning's choice, not the search's.
- **Packing is not the difference.** The share of the rows' and columns' track running beside another net's at the 0.4 mm pitch, track and clearance: traxide 11 %, the author 10 %; within 0.4 to 0.7 mm, 22 % and 29 %.
- **Measured and not built: hugging.** A step running beside another net's routed copper, within a grid step of the clearance, paying 15 % or 30 % less: 13 % packed instead of 11 %, but 19 and 8 vias more.


Measured again on TokenBoard in November 2026, routed within 90 s (223 new vias), tracing each matrix net's copper from its MCU pin to the first pad it reaches:

- **The joins are mostly the same.** The author joins the MCU pin to the pad nearest to it, the spanning tree's choice, on 12 of the 19 row and column nets. The other seven (ROW1, ROW3, ROW5, ROW9, COL2, COL7, COL8) join a pad off to the side of the MCU rather than straight above or below it. The ways from the MCU to those first pads take 37 vias in traxide's routing and 32 in the author's. The rest of the gap is in the rows' and columns' own chains, 110 vias against 94, and in the MCU's other signals, two vias more on many of them.
- **Measured and not built: the author's joins.** Planning the seven joins as the author made them: 231 vias instead of 223. The rows take 29 instead of 33, the rest more.
- **Measured and not built: another order.** Connections shortest first, hemmed-in pads not first: 232 vias. Hemmed-in pads first only for connections shorter than 20 mm: 227.
- **More time is not the lever either.** Within 300 s instead of 90: the same 223 vias, as the second of the six rounds routed no better than the first.
- **Nor the prices.** Vias at 3 mm instead of 2: 224 vias. Steps across a layer's direction at 0.33 or 0.75 of their length instead of 0.5: 238 and 239.
- **Measured and not built: fewest crossings.** Counting how many other nets' planned connections each candidate join crosses, as straight lines, predicts none of the author's joins: for ROW5 the author's pad crosses 19, the nearest 7.

So which pad the MCU joins is not the lever on this board; the gap is spread over the congested area around the MCU. The small nets around the MCU take about 26 vias more than the author's, two each where the author routes on one layer: QSPI_SD1 runs 16 mm straight from the MCU to the flash on one layer, and traxide takes two vias round the row and column ways running down beside the MCU; LED_DIN54 hops 1.6 mm under ROW6's way on the back copper. What the author has and traxide lacks is the even rows' ways running together on the front copper, a bundle between the rows, where traxide's run singly on the back copper beside the odd rows' diodes.
