---
id: TRX-62
title: Check custom rules against KiCad's DRC on real boards
type: improvement
status: in-progress
assignee: claude
priority: medium
labels: [rules, kicad]
layer: engine
---

Custom rules (`.kicad_dru`) are compiled as KiCad 10's DRC was observed to apply them on small hand-written boards ([[TRX-32]], [[Research/KiCad-integration#custom-rules]]). Real boards may use what those did not.

- [ ] Compare with KiCad's DRC on corpus boards with their own files: no rule traxide routes against may disagree (needs [[TRX-30]]'s corpus from board files)
- [x] `memberOfFootprint`: TokenBoard ([[TRX-64]]) lowers the edge clearance to 0.2 mm at the cutouts of its LED footprints (`A.memberOfFootprint('LED*') || B.memberOfFootprint('LED*')`). traxide cannot tell, keeps the board's 0.3 mm there, and `traxide check` reports 4 edge violations on the author's routing that KiCad accepts ([[TRX-65]]). Cutouts would need to know their footprint, and the edge clearance to vary by cutout Done: the KiCad reader keeps which footprint draws each cutout and resolves `memberOfFootprint` against the board's footprint references (a library name, `Lib:Name`, may hold or not); at the edge, routed copper belongs to no footprint and a cutout to the one that draws it, tried both ways round, so each cutout has its own clearance in layout and verification. TokenBoard as its author routed it: `traxide check` finds no rule broken (4 edge violations before). Routed by traxide within 90 s: all 241 nets in 47 s, KiCad's DRC finds no violation, but one missing VLED connection, a plane join walled off by other nets' vias ([[TRX-67]])
- [x] Judge the corpus's KiCad projects ([[TRX-64]]) with KiCad's DRC as well, zones refilled, beside `traxide check`: it finds traxide's TokenBoard clean, but `traxide check` counts 4 edge clearance violations on the author's board that KiCad does not, until `memberOfFootprint` is read
- [ ] If boards need them: `disallow`, `physical_clearance`, `annular_width`, per-layer values, vias of the sizes rules ask

**KiCad's DRC in the corpus:** each routing of a KiCad project, traxide's, Freerouting's and the author's, is judged by KiCad 10's DRC (errors only), every zone refilled, in the column "KiCad's DRC". TokenBoard: traxide clean, author clean. Refilling only the zones left unfilled, as the end-to-end test does for a board routed in a running KiCad, judged traxide's routing by the fills the author's routing left: 698 errors (499 clearance, 199 hole clearance) where the GND pours of F.Cu and B.Cu overlap the new tracks. A designer meets the same after routing a board file, so traxide now says to refill the zones (Edit → Fill All Zones) when the board it wrote has copper zones. KiCad 9's `kicad-cli` has no `--refill-zones`, so the round-trip script refills (`refill-all`).
