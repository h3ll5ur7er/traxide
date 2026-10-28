# KiCad integration

The three ways traxide meets KiCad, what each can and cannot do in KiCad 9, 10 and the coming 11, and which Board Access adapter implements it.

[TOC]

## Paths

| Path | How | Pros | Cons | Adapter |
| --- | --- | --- | --- | --- |
| Specctra DSN/SES | Export DSN from KiCad, route, import SES | Universal, what Freerouting does, how corpora compare | Lossy: custom rules not exported, quirky keepouts; KiCad 10 exports from the GUI (SWIG scripting goes away in KiCad 11) | `dsn` |
| Board files | Read `.kicad_pcb`, `.kicad_pro`, `.kicad_dru`; write tracks and vias back | Headless, CI, KiCad closed; complete rules | Must follow KiCad's file format versions | `kicad-file` |
| IPC API | Talk to the running KiCad | In-editor, undoable, live progress | Needs the GUI; off by default | `kicad-ipc` |

## The IPC API

What [[TRX-17]] verified against KiCad 10.0.7 (the `kicad/kicad:10.0` image) with the prototype in `spikes/kicad-ipc/`, and what it only read in the documentation. Sources: KiCad's developer documentation for add-on developers (the `kicad-dev-docs` repository), the protocol buffer definitions of KiCad 10.0.7 and of master read as an interface, the `plugin.json` schema `api/schemas/api.v1.schema.json`, `kicad-python`'s licence and changelog, `kicad-ipc-rs` 0.5.1 (MIT) and KiCad's observable behaviour. No KiCad source code was read.

### Transport

- **Protocol buffers over NNG's REQ/REP**, KiCad being the REP side. On Linux and macOS a Unix socket, on Windows a named pipe, created in the user's temporary directory as `kicad/api.sock`; a second KiCad appends its process id.
- A plugin launched by KiCad gets **`KICAD_API_SOCKET` as an NNG address, `ipc:///tmp/kicad/api.sock`, not a path**, and `KICAD_API_TOKEN`, a per-instance id to put in every request's header (a mismatch is refused, which tells a long-running client that KiCad restarted). A client started by hand has neither and falls back to the default path.
- **Framing** (the scalability protocols' IPC mapping, verified): both sides first send an 8-byte header `00 'S' 'P' 00`, protocol (`0x0030` REQ, `0x0031` REP), `00 00`; then each message is `0x01`, a big-endian `u64` length and the payload. REQ prefixes its payload with a 4-byte request id whose top bit is set, and REP echoes it.
- **Envelope**: `kiapi.common.ApiRequest { header { kicad_token, client_name }, message: Any }` and `ApiResponse { header, status { status, error_message }, message: Any }`. `Any.type_url` is `type.googleapis.com/` plus the full message name, such as `kiapi.board.commands.GetNets`.
- KiCad handles requests on its GUI thread: it answers **`AS_BUSY` while a modal dialog is open** (and `AS_NOT_READY` just after start), so a client retries. One connection per client; KiCad serialises requests anyway.
- **Speed**: the whole spike (about 12 requests, 247 pads, 370 tracks read twice, one commit, one undo) takes 0.6 s; reading items is not a bottleneck next to routing.

### What KiCad 10.0 serves for routing

| Need | Command (10.0) | Verified |
| --- | --- | --- |
| Board, project folder | `GetOpenDocuments` (board file name, project name and path); `KIPRJMOD` in a plugin's environment | Yes |
| Layers, stackup | `GetBoardEnabledLayers` (copper layer count), `GetBoardStackup` | Layer count |
| Nets | `GetNets` (by name; net codes are deprecated) | Yes (112) |
| Net classes with width, clearance, via stack, differential pairs | `GetNetClassForNets` | Yes |
| Pads | `GetItems` with `KOT_PCB_PAD`: id, number, net, position, padstack | Yes (247) |
| Exact pad copper | `GetPadShapeAsPolygon` per layer | Yes (straight segments) |
| Tracks, arcs, vias with locked state | `GetItems` with `KOT_PCB_TRACE`, `KOT_PCB_ARC`, `KOT_PCB_VIA` | Tracks, vias |
| Zones and keepouts | `GetItems` with `KOT_PCB_ZONE`; rule areas are `ZT_RULE_AREA` | No |
| Outline | `GetItems` with `KOT_PCB_SHAPE` on `Edge.Cuts` | No |
| The whole board as the text of its file | `SaveDocumentToString` | Yes ([[TRX-31]]) |
| Write routes | `BeginCommit`, `CreateItems`, `EndCommit` | Yes |
| Refill zones | `RefillZones` | Yes ([[TRX-31]]): inside an open commit it fills around the board as it was before the commit; after `EndCommit` it fills correctly but is an undo step of its own, which is how traxide uses it |
| Save the board | `SaveDocument` | Yes ([[TRX-31]]) |

- **Design rules are not served by 10.0**: its `BoardDesignRules` message is empty and has no command; `GetBoardDesignRules` and `GetCustomDesignRules` exist on master (KiCad 11). Net-class values do come over IPC, so on KiCad 10 the `kicad-ipc` adapter reads from the project files (`KIPRJMOD`) only what net classes do not carry: the board minimums of `.kicad_pro` (`min_clearance`, copper to edge, hole clearance) and the custom rules of `.kicad_dru`. Board Access hides this mix; nothing above it knows.
- **Writing**: items created between `BeginCommit` and `EndCommit` (`CMA_COMMIT`) appear together and form **one undo step named by the commit's message** (*Edit → Undo traxide: route*). Verified: a track and a through via in one commit, removed by one undo. `CMA_DROP` discards a commit. A via needs its padstack (drill, copper size on each layer), a track its width, layer and net by name.
- `RunAction` runs an editor action by name; KiCad calls it a prototyping aid, not an API. The spike uses `common.Interactive.undo` only to play the user's Ctrl+Z.

### The `kicad-ipc` adapter

What [[TRX-31]] built in Board Access (`kicad/ipc/`), verified against KiCad 10.0.7 by `cargo xtask e2e`. Clients name the board open in KiCad by the path `kicad://current` (`RUNNING_KICAD`), so a `RouteJob` needs no other field; `traxide route --kicad` is the same.

- **Reading**: KiCad hands over the open board as the text of its file (`SaveDocumentToString`, the editor's state, saved or not), and the board-file reader reads it, so the design is the one the board file gives: same shapes, same approximations, already checked against KiCad's DRC. The items one by one (`GetItems`, `GetPadShapeAsPolygon`) would need a second reader of every item and padstack kind, and KiCad's pad polygons may lie inside the true copper, where the file reader's shapes are never smaller.
- **Rules**: each net's class and the class's width, clearance and via come from `GetNetClassForNets`, as the editor holds them, over the project file's; the board's minimums (raising them) and the custom rules come from the project's `.kicad_pro` and `.kicad_dru` in `KIPRJMOD`, else in the project folder `GetOpenDocuments` names. A net of several classes gets an implicit class from KiCad; traxide takes its first constituent, as it does from the project file.
- **Storing**: Board Access reads the board again and refuses to store if it is no longer the design that was routed (`kicad://current changed since it was read; route it again`). Then one commit: `BeginCommit`, `CreateItems` with a track per straight piece and the vias, `EndCommit` with the message `traxide: route`; if KiCad refuses any item, the commit is dropped (`CMA_DROP`) and nothing changes. No routes, no commit.
- **Zones are refilled after the commit**, on boards with copper zones (rule areas do not count), so the board is clean in KiCad's DRC as it stands: `RefillZones` for every zone once `EndCommit` has pushed the routes. The fill is an undo step of its own, after the routes', so **on a board with copper zones the designer undoes twice** to take the routes back (the first undo takes back the fill); on a board without, once. The maintainer accepted this. Refilling inside the open commit would keep one undo but is wrong: measured on `matrix-3x4` with a zone of one net over the back, it filled the zone around the pads only, and KiCad's DRC found 49 clearance violations between the fill and the new tracks. If the refill fails, the routes stay and traxide says so.
- **Connecting**: `KICAD_API_SOCKET` (an `ipc://` address or a path), else `kicad/api.sock` in the temporary folder; `KICAD_API_TOKEN` in every request. `AS_BUSY` and `AS_NOT_READY` are retried for 10 s, then traxide asks to close KiCad's open dialog; a refused token means the KiCad that started traxide has closed. Without a socket, the error says how to enable the API.
- **Windows**: NNG serves `ipc://NAME` as the named pipe `\\.\pipe\NAME`; the client opens it as a file. Written, linted, untested; a pipe opened as a file cannot time out.
- **Tests without KiCad**: framing, the envelope and retries against an in-process REP peer; reading and storing against a fake KiCad that serves a fixture board, held to the board-file reader.

### Client: hand-written, not `kicad-ipc-rs`

| | Hand-written (`spikes/kicad-ipc`) | `kicad-ipc-rs` 0.5.1 |
| --- | --- | --- |
| Licence of what enters the binary | prost, prost-types: Apache-2.0 (protox and prost-build, MIT and Apache-2.0, only build) | MIT and Apache-2.0 throughout (kicad-ipc-rs, nng, nng-sys with the bundled C libnng, tokio, prost) |
| Build | Pure Rust, `protox` instead of `protoc` | Builds libnng (C) with CMake through `nng-sys` 1.4.0-rc.0, a release candidate; a C toolchain per target |
| Size | About 100 lines of transport plus the messages used; spike binary 0.8 MB | 21 crates with tokio; 0.7 MB for a bare connect |
| Coverage | What traxide needs, added per message | All of 10.0.1's commands, typed wrappers |
| KiCad's text in it | The `.proto` files it needs, unchanged, generated without their comments | The generated code carries the `.proto` files' comments (GPL-3.0-or-later headers) |
| Maintenance | Ours; the API is stable by protobuf rules (fields are added, never changed) | One maintainer, releases since February 2026, pinned to 10.0.1 |

Both worked against KiCad 10.0.7. The recommendation is the hand-written client: no C build in the cross-compiled release binaries ([[TRX-35]]), no async runtime, and the licence question below does not arise.

**Licence**: KiCad's `.proto` files carry GPL-3.0-or-later headers. The KiCad developers themselves publish `kicad-python`, generated from those files, under MIT (relicensed in its 0.2.0, February 2025), which suggests they do not treat the API definitions as making clients GPL; no explicit statement was found in the documentation. [[TRX-17]] first defined traxide's messages by hand from the field numbers to stay clear of the question; the maintainer then decided to accept the small risk, as many MIT and Apache projects do, and generate them from KiCad's files: `crates/access/board/proto/kicad/` holds the nine files traxide needs from KiCad 10.0.7, headers intact, and the build script compiles them with `protox` and `prost-build`, leaving their comments out. It is the one exception to the clean-room rules ([[Development#clean-room-rules]]).

## The plugin

KiCad's `plugin.json` (schema `https://go.kicad.org/api/schemas/v1`) allows `runtime.type` **`"exec"`**: the plugin is a native executable, so traxide's KiCad plugin is **the same binary, no Python**. Verified on Linux with KiCad 10.0.7:

```json
{
  "$schema": "https://go.kicad.org/api/schemas/v1",
  "identifier": "com.github.h3ll5ur7er.traxide",
  "name": "traxide",
  "description": "Fast autorouter",
  "runtime": { "type": "exec" },
  "actions": [
    {
      "identifier": "route",
      "name": "Route board",
      "description": "Route every unrouted connection",
      "show-button": true,
      "scopes": ["pcb"],
      "entrypoint": "traxide",
      "icons-light": ["icon.png"],
      "icons-dark": ["icon-dark.png"]
    }
  ]
}
```

- Required: `identifier`, `name`, `description`, `runtime`, `actions`; each action `identifier`, `name`, `description`, `entrypoint`. **There are no arguments and no per-platform entry point**: each action needs its own entry point, so several actions of one binary need a small launcher per action or one binary that tells actions apart by name (a hard link or copy per action, or `argv[0]`). One PCM package per platform carries the right binary; PCM's `metadata.json` lists `platforms` per version.
- KiCad starts the entry point **from the plugin's folder as working directory, with no arguments**, and with `KICAD_API_SOCKET`, `KICAD_API_TOKEN`, `KIPRJMOD` (the project folder) and KiCad's `KICAD10_*` paths in the environment. Since 10.0.1 KiCad shows what a plugin writes: verified on 10.0.7, **each line on stderr becomes a notification** under the warning sign at the right end of the status bar, prefixed with the action's name, with a line "exited with code N" when the code is not 0; **stdout is not shown**. KiCad 9.0 shows neither.
- KiCad 9 and 10 run IPC plugins only in the PCB editor.
- **Where plugins live**: KiCad loads `plugin.json` from subfolders of `${KICAD_DOCUMENTS_HOME}/10.0/plugins` (`~/.local/share/kicad`, `~/Documents/KiCad`, `C:\Users\<user>\Documents\KiCad`) and of PCM's `${KICAD_DOCUMENTS_HOME}/10.0/3rdparty/plugins`. PCM's *Install from File* unpacks the zip's `plugins/` into `3rdparty/plugins/<identifier with dots as underscores>` and `resources/` into `3rdparty/resources/<…>`; **the executable bit survives on Linux**, and the toolbar shows the action's icon, so the reported missing icons of PCM-installed IPC plugins did not reproduce there. Windows and macOS (including the quarantine attribute and notarisation) are untested.
- **What the user must do**: tick *Preferences → Preferences… → Plugins → Enable KiCad API* (stored as `api.enable_server` in `kicad_common.json`; off by default) and restart the PCB editor. Verified: with the API off there is no socket **and the plugin's buttons are not shown at all**, so the plugin cannot explain it itself; the PCM description, the README and a standalone `traxide` must. Ticking it starts the API at once, but the buttons appear only after the PCB editor restarts.
- KiCad 11 nightlies reportedly stop bundling Python, which makes Python plugins such as Freerouting's (which installs a JRE on demand) and OrthoRoute's (which builds a virtual environment) more fragile, an advantage for a native plugin. Konnect, a Rust KiCad plugin, already ships this way.

### traxide's plugin

What [[TRX-33]] built on these facts, and verified with KiCad 10.0.7 and 9.0 in Docker:

- **The `traxide` binary is the entry point.** Started with no arguments and `KICAD_API_SOCKET` in the environment, as KiCad starts a plugin, it routes the board open in KiCad, as `traxide route --kicad --quiet` does, but writes its summary to stderr, which KiCad 10 shows; started with no arguments elsewhere it prints its usage. A second action, *Route selected nets*, has a copy of the binary named `traxide-selected` as its entry point (`traxide-selected.exe` on Windows), which routes as `traxide route --selected --quiet` does: only the nets of the items `GetSelection` returns, a footprint's being those of its pads. A copy rather than a link, as zip files and Windows keep no links; the package grows by the binary's size, about 2 MB. Its icon is the first one in blue, by which the end-to-end test tells the two buttons apart on its screenshot.
- `kicad-plugin/` holds `plugin.json`, the PCM `metadata.json` and the icons (24 px for the toolbar, light and dark; 64 px for the package manager). `cargo xtask kicad-package PLATFORM TRAXIDE OUT.zip` lays the package out with the platform's binary (`traxide.exe` as the entry point on Windows), its version and `platforms`, and zips it with Info-ZIP, which keeps the executable bit.
- **KiCad 9 loads exec plugins from `3rdparty/plugins` too**, starts them from the plugin's folder with the same environment (`KICAD_API_SOCKET`, `KICAD_API_TOKEN`, `KIPRJMOD`) and shows their toolbar buttons, so the package asks for KiCad 9.0 or later. KiCad 9 shows nothing of what the plugin writes.
- `RunAction` did not know the plugin's action by any name tried (`api.plugin.<identifier>.<action>`, `<identifier>.<action>` and others), so the end-to-end test presses the button as a person does: it finds the icon on a screenshot by its fill colour and clicks it with `xdotool`.
- The plugin waits for `RefillZones` to answer, so once its process has exited the board is final: the end-to-end test waits for the board to change and the process to end.

## Testing without a person

KiCad 9 and 10 serve the API only from the GUI, but the GUI runs without a screen: `spikes/kicad-ipc/run-headless.sh BOARD.kicad_pcb` starts `pcbnew` from the `kicad/kicad:10.0` image on an `xvfb-run` display (the image has no X server; the host's is shared through `/tmp/.X11-unix`, with `-ac` because the container's host name does not match the cookie), shares `/tmp/kicad` with the host for the socket, and runs the spike against it: from nothing to a verified commit and undo in about 5 seconds.

- KiCad greets a new configuration with a **first-run wizard, a modal dialog that keeps the API answering `AS_BUSY`**. It stays away when all three global library tables exist (`fp-lib-table` and `sym-lib-table` ship in the image, `design-block-lib-table` does not) and `kicad_common.json` marks its questions as answered: `"do_not_show_again": { "data_collection_prompt": true, "update_check_prompt": true }`, next to `"api": { "enable_server": true }`.
- A board left open by a killed KiCad keeps its `~*.lck` files, and the next start asks whether to open it anyway, again a modal dialog; delete them first.
- Linux only: Docker Desktop on macOS and Windows does not pass a Unix socket through a bind mount. GitHub's Linux runners have `xvfb-run`, so `cargo xtask e2e` can drive the IPC path the same way.

## KiCad 11

From master's protocol definitions and the documentation: design rules over IPC (`GetBoardDesignRules`, `GetCustomDesignRules`, new `board_rules.proto`), a headless API server in `kicad-cli`, jobs for plotting and export, and IPC plugins in the schematic editor. The message layout stays compatible (fields are added, not changed), so a client built for 10.0 keeps working; on 11 the adapter can ask for the rules instead of reading files.

## Rules the DSN leaves out

KiCad's DSN export carries widths, clearances, net classes and vias, but not everything KiCad's DRC checks:

- **Copper to board edge.** KiCad applies its board setup, 0.5 mm by default. From a DSN, traxide keeps at least 0.5 mm on boards whose DSN names KiCad as `host_cad`; from a board file it reads `min_copper_edge_clearance` from `.kicad_pro`.
- **Custom rules** (`.kicad_dru`): not exported at all, and not read from board files yet ([[TRX-32]]).

## Reading board files

The `kicad` module of Board Access reads `.kicad_pcb` as KiCad 9 (nets by number) and 10 (nets by name) write it, with the rules of the `.kicad_pro` beside it, or KiCad's defaults without one ([[TRX-30]]). Tests check it against KiCad's own placement of every pad and against KiCad's DSN export of the same boards: pads, nets, layers, rules, outline and keepouts agree.

- **Coordinates** are KiCad's: millimetres to the nanometre, y pointing down; a pad's `at` angle includes its footprint's.
- **Rules**: each net class's width, clearance and via, raised to the board's minimums; a class leaves out what it takes from `Default`; nets join classes by `netclass_assignments`, then by the wildcards of `netclass_patterns`.
- **Holes without copper** keep everything out by the board's hole clearance (`min_hole_clearance`) all around, as KiCad's DSN export writes them; without it KiCad's DRC finds hole-clearance violations.
- **Shapes** cover KiCad's and are never smaller: rounded rectangles as octagons that touch the corners, trapezoids and chamfered rectangles as their bounding rectangles, custom pads as their anchor and primitives. Arcs become segments within 2 µm of the arc. Per-layer padstacks use the front shape.
- **Outline**: the drawings on Edge.Cuts, joined end to end; loops inside the outline are cutouts, kept out like keepouts. Copper zones are left out, as KiCad refills them; rule areas that forbid tracks or vias are keepouts.
- **Copper of no net** keeps other copper away, as keepouts.

Routes are written back as `segment` and `via` nodes before the board's closing parenthesis; every other byte stays. Board Access reads the board again first and refuses to write if it no longer gives the same design. By default `traxide route BOARD.kicad_pcb` writes `BOARD.routed.kicad_pcb` and copies the project and the custom rules beside it, so that KiCad opens the routed board with its rules.

### Custom rules

Board Access reads `BOARD.kicad_dru` beside the board into `DesignRules.custom` ([[TRX-32]]); the Rules Engine compiles them into the `ConstraintSet`. KiCad's documentation could not be reached while this was built, so what KiCad 10.0 does was observed: small boards with hand-written rules, judged by `kicad-cli pcb drc`.

What KiCad's DRC does, and so what traxide follows:

- **The last rule that applies wins**, and it overrides the net classes and even the board's minimums (Board Setup → Constraints): a rule may lower a clearance below both.
- **Two-item constraints try the condition both ways round**: `A.NetClass == 'HV'` gives the clearance between HV and anything, either way. For one-item constraints (widths, edge clearance) `A` is the item.
- **`==` matches wildcards (`*`, `?`) and ignores case**, for `NetClass` and `NetName` alike. Copper of no net has the class `Default` and the name `''`.
- **Limits**: clearances check only `min`; a matching rule without one still wins, but checks the class's value. Widths and via sizes check `min` and `max`; `opt` is never checked, but it is the width traxide routes with.
- **Lengths** need a unit: `mm`, `mil` or `in`, with or without a space, quoted or not; `"0.5mm + 0.5mm"` is evaluated. A length without a unit, `um`, `nm`, a missing `(version 1)` or a syntax error makes KiCad ignore the whole file, without a word in the DRC report. traxide warns instead and does its best, as the maintainer decided: a rule it cannot read (such a length, or an expression it does not evaluate) is left out, named with its line; a file without `(version 1)` is read all the same; a file that is not a list at all is left out whole. The route and check reports print the warnings, so the designer sees what KiCad silently drops.
- Lines starting with `#` are comments; `(severity ignore)` makes the DRC check nothing under that rule, so traxide leaves such a rule out (whatever applies instead keeps at least as much distance).
- `(layer outer)`, `(layer inner)` and `(layer "In1.Cu")` limit a rule to copper on those layers.

What traxide compiles:

| Constraint | Routing follows it as |
| --- | --- |
| `clearance` | the clearance between the two nets, per pair of *groups*: nets of one class that the rules' `NetName` tests treat alike, so a rule naming a net reaches only that net |
| `track_width` | the width of each net: the winning rule's `opt`, else the class's, within its `min` and `max` |
| `via_diameter`, `hole_size` | which of its class's vias a net may use; traxide makes no via of a new size, so a net whose vias a rule forbids routes on one layer or not at all |
| `hole_clearance` | other copper keeps the clearance from a via's copper, so from its hole by the via's ring as well; where `hole_clearance` exceeds clearance plus ring, the clearance between the nets grows. Holes without copper keep the project's hole clearance as keepouts |
| `edge_clearance` | one clearance for the whole board: the largest any net's rules ask |

**What traxide cannot tell never breaks KiCad's DRC.** A test it cannot evaluate (`A.Type == 'Pad'`, `A.inDiffPair('*')`, `A.intersectsArea(…)`, `A.Layer == …`, other properties and functions, an unparseable condition) is *unknown*, and so is a layer clause that covers only some of the board's copper layers, or names no copper layer of the board. The logic is three-valued: `&&`, `||` and `!` keep what can be told, so `A.inDiffPair('*') && A.NetClass == 'HV'` is false for every other class. For each pair of groups the Rules Engine walks the rules from last to first: the first that surely applies gives the value, and every rule after it that may apply can only tighten it (raise a minimum, lower a maximum, forbid a via). A rule that might lower a clearance is therefore ignored, and one that might raise it applies wherever it might.

Not followed: the other constraints (`disallow`, `physical_clearance`, `annular_width`, `diff_pair_gap`, lengths and skew, courtyard and silkscreen); per-layer values (a rule on some layers applies to all of them when it tightens); the clearance from plated pad holes; nets with several classes, whose first class traxide takes. Specctra DSN carries no custom rules, so boards that have them are routed from their own file: the end-to-end recipe `isolation` asks for 3 mm between an HV net and two signals that must cross it, a wider track for one net by name, a wider edge clearance for HV and a pad-to-pad rule traxide cannot evaluate. Routed from its `.kicad_pcb` it is clean in KiCad's DRC; routed with its rules file removed and judged with it, KiCad finds 13 violations.

## What is read

| Item | DSN | `.kicad_pcb` + project | IPC (KiCad 10) |
| --- | --- | --- | --- |
| Outline, stackup, layers | Yes | Yes | Yes |
| Pads with exact shapes | Approximated | Yes | As from the board file (KiCad hands over its text) |
| Nets, net classes, widths, clearances | Mostly | Yes (`.kicad_pro`) | Nets and net classes yes; board minimums from `.kicad_pro` |
| Custom rules (`.kicad_dru`) | No | Yes | From files |
| Keepouts, zones | Partly | Yes | Yes |
| Existing tracks and vias, locked state | Yes | Yes | Yes |

## Writing back

- DSN path: a SES file with wires and vias, imported by the user (or by Freerouting-style tools).
- File path: new `segment`, `arc` and `via` nodes in `.kicad_pcb`, every other byte kept; refuse to write while the board file changed since it was read.
- IPC path: one commit with all new items, then a refill of the copper zones as an undo step of its own: one undo removes the routing on a board without copper zones, two on a board with them.

## DRC reports

`traxide check BOARD [SESSION] --drc REPORT.json`, `traxide route … --drc REPORT.json` and Freerouting's `traxide freerouting … -drc REPORT.json` write the Verification Engine's verdict in the JSON format of `kicad-cli pcb drc --format json` (schema `https://schemas.kicad.org/drc.v1.json`), so tools that read KiCad's reports read traxide's ([[TRX-51]]). `check` reports the copper it checked; `route` the stored copper, the board's own included, as KiCad's DRC would see the routed board. Board Access writes it (`kicad/drc.rs`), as it writes KiCad's other formats.

| traxide | KiCad `type` | KiCad `description` |
| --- | --- | --- |
| Clearance | `clearance` | `Clearance violation (clearance 0.2000 mm)` |
| Edge | `copper_edge_clearance` | `Board edge clearance violation` |
| Keepout | `items_not_allowed` | `Items not allowed` |
| Unconnected | `unconnected_items`, in `unconnected_items` | `Missing connection between items` |

- Items are described as KiCad does (`Pad R1-2 [GND] on F.Cu`, `Track [GND] on F.Cu, length 2.5400 mm`, `Via [GND] on F.Cu - B.Cu`; `<no net>` for none) at their position in millimetres: a pad's centre, a track's first point, a via's centre. Every entry is an `error`.
- An unconnected net becomes one pair per pad apart from the net's first pad, which is what verification finds; KiCad pairs the nearest unconnected items, so the counts can differ when the pads apart are joined to each other.
- `kicad_version` names traxide and its version (`traxide 0.1.0`) rather than pretending to be KiCad, which the schema's pattern for KiCad versions does not allow; `date` is UTC in RFC 3339.
- Left out because a design does not carry it: the items' `uuid`, which the schema requires, and the actual distance KiCad adds to a clearance violation. Left out as optional: `included_severities` and `ignored_checks`.
- On the routed `crossing`, `matrix-3x4` and `keyboard-tkl` boards, KiCad 9 and 10 and traxide report the same: no violations, no unconnected items, the same `source`, `$schema`, `coordinate_units` and `schematic_parity`.
