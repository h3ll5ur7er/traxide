---
id: TRX-24
title: Plan connections and batches in the Planning Engine
type: feature
status: todo
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

