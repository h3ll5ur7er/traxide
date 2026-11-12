# Use cases

The iDesign Method validates an architecture by showing that the core use cases are carried out by the same components, interacting differently, rather than by components built for each use case. These are traxide's core use cases and their call chains. Solid arrows are synchronous calls, dashed arrows events or queued calls.

[TOC]

## Core use cases

| # | Use case | Client | Manager |
| --- | --- | --- | --- |
| UC1 | Route a board from a Specctra DSN file to a SES file | CLI, Freerouting-compatible CLI | Routing |
| UC2 | Route the board open in KiCad and write the tracks back, undoable | KiCad plugin | Routing |
| UC3 | Route only selected or unrouted nets, keeping locked copper | CLI, KiCad plugin | Routing |
| UC4 | Optimise an existing routing | CLI, KiCad plugin | Routing |
| UC5 | Check a board: DRC and quality metrics | CLI | Inspection |
| UC6 | Run the benchmark corpus and compare with baselines | CLI (`bench`), CI | Inspection, which queues jobs for Routing |
| UC7 | Watch a routing job live | Viewer | Routing |

## UC1: Route a board from DSN to SES

The use case every other one varies.

```mermaid
sequenceDiagram
  participant C as CLI
  participant RM as Routing Manager
  participant PA as Profile Access
  participant BA as Board Access
  participant R as Rules Engine
  participant LA as Layout Access
  participant P as Planning Engine
  participant G as Global Routing Engine
  participant CA as Compute Access
  participant D as Detailed Routing Engine
  participant O as Optimization Engine
  participant V as Verification Engine
  participant RA as Run Access

  C->>RM: route(job: board.dsn → board.ses, profile)
  RM->>PA: load_profile(name, overrides)
  RM->>BA: load_design(board.dsn)
  RM->>R: compile(design.rules)
  R-->>RM: constraint set
  RM->>LA: build_layout(design, constraints)
  RM->>P: plan(design, constraints, layout)
  P-->>RM: connections, order, batches
  RM->>G: route(plan, layout)
  G->>CA: update_congestion / expand_wavefronts (iterations)
  G-->>RM: corridors, layer assignment
  loop batches, in parallel inside the engine
    RM->>D: route(batch, corridors)
    D->>LA: begin, obstacles_near, commit
  end
  RM->>O: optimize(routes, layout)
  RM->>V: verify(design, constraints, routes)
  V-->>RM: violations, metrics
  RM->>BA: store_routes(board.ses, routes)
  RM->>RA: record_run(report)
  RM--)C: progress events, finished(report)
```

The manager decides the stages and their order from the profile: a *fast* profile skips optimisation and stops detailed routing after a time budget; a *quality* profile adds more rip-up iterations and every optimisation pass.

## UC2: Route the board open in KiCad

Identical to UC1 except for the client and for what Board Access talks to: the IPC API of the running KiCad instead of files. The job names the board `kicad://current`. Board Access reads the board and the net classes from KiCad (on KiCad 10 the board's minimums and custom rules from the project's files), and writes all tracks and vias in one commit, so a single undo removes them ([[Research/KiCad-integration#the-kicad-ipc-adapter]]).

```mermaid
sequenceDiagram
  participant K as KiCad plugin
  participant RM as Routing Manager
  participant BA as Board Access
  participant KC as KiCad (IPC API)
  K->>RM: route(job: kicad://current, profile)
  RM->>BA: load_design(kicad://current)
  BA->>KC: get board, nets, footprints, rules
  Note over RM: same stages as UC1
  RM->>BA: store_routes(kicad://current, routes)
  BA->>KC: begin commit, create tracks and vias, push commit
  RM--)K: progress events, finished(report)
```

Nothing above Board Access knows KiCad is involved. Supporting another EDA tool is a new adapter inside Board Access.

## UC3: Route selected or unrouted nets

The job names the nets to route and marks the rest as locked. Board Access loads existing copper as locked routes; Layout Access treats them as obstacles; Planning plans only the requested nets. The same engines run; the manager's workflow differs only in its inputs.

## UC4: Optimise an existing routing

The Routing Manager skips planning, global and detailed routing: load, compile rules, build the layout from the existing routes, optimise, verify, store. Verification guards the optimiser: if a pass would add a violation, the manager keeps the previous routes.

Built ([[TRX-63]]): `traxide optimise BOARD.kicad_pcb`, `traxide optimise --kicad`, and the plugin's *Optimise* button. The board's unlocked segments, chained where nothing else joins their net (no via, pad or branch), are pulled tight; arcs, locked copper and the tracks of differential pairs stay as they are. The result is stored only where it changes something and breaks no rule the board did not break: into a copy `BOARD.optimised.kicad_pcb`, or into KiCad as one commit, *traxide: optimise*, that deletes the old segments and vias by their ids and creates the new.

## UC5: Check a board

```mermaid
sequenceDiagram
  participant C as CLI
  participant IM as Inspection Manager
  participant BA as Board Access
  participant R as Rules Engine
  participant LA as Layout Access
  participant V as Verification Engine
  participant RA as Run Access
  C->>IM: check(board.dsn + board.ses)
  IM->>BA: load_design, load routes
  IM->>R: compile(rules)
  IM->>LA: build_layout(design, routes)
  IM->>V: verify(...)
  V-->>IM: violations, metrics
  IM->>RA: record_run(report)
  IM-->>C: report
```

Running the same Verification Engine on Freerouting's output and on ours is what makes the benchmark fair.

`traxide check BOARD [SESSION]` prints the report, `--json` as JSON; `--drc REPORT.json` has the manager store the verdict through Board Access as a DRC report in KiCad's JSON format ([DRC reports](../Research/KiCad-integration.md#drc-reports)). `traxide route --drc` and Freerouting's `-drc` do the same for the stored routes, in the Routing Manager.

## UC6: Run the benchmark corpus

```mermaid
sequenceDiagram
  participant C as CLI bench
  participant IM as Inspection Manager
  participant RA as Run Access
  participant Q as Job queue
  participant RM as Routing Manager
  participant V as Verification Engine
  C->>IM: benchmark(corpus, profile)
  IM->>RA: load_corpus
  loop each board
    IM-)Q: route job
    Q-)RM: route(job)
    RM-)Q: finished(routes, timings)
    Q-)IM: finished
    IM->>V: verify
    IM->>RA: baseline_of(board)
  end
  IM->>RA: record_run(comparison)
  IM-->>C: table: time, completion, vias, length, violations vs baseline
```

The managers never call each other directly: the queue (from the Events utility) decouples them, which also lets the benchmark run boards concurrently.

## UC7: Watch a routing job live

The viewer starts a route job like the CLI does and subscribes to its events. The Routing Manager publishes stage changes, congestion maps after each global iteration and every committed batch. The viewer draws them; it never reads the layout itself. A cancel button cancels the job's token; the manager stops at the next batch boundary, stores a checkpoint through Run Access, and the job can be resumed (UC1 with `resume`).
