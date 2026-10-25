# Development

How the repository is laid out, how the layer rules are enforced, and how to build, test and benchmark. The choices behind the stack are in [Tech stack](Research/Tech-stack.md).

[TOC]

## Repository layout

One Cargo workspace; one crate per component, in folders named after the iDesign layers:

```text
crates/
  contracts/                 data and service contracts
  utilities/
    geometry/                integer geometry, predicates, offsetting
    diagnostics/             tracing setup, counters, timings
    events/                  progress events, cancellation, job queue
    pool/                    work pool, deterministic reductions
  access/
    board/                   DSN/SES, .kicad_pcb, KiCad IPC API adapters
    layout/                  the routing state: spatial index, transactions
    compute/                 CPU (rayon, SIMD) and GPU (wgpu) backends
    profile/                 profiles, traxide.toml, overrides
    run/                     reports, checkpoints, corpus, baselines
  engines/
    rules/  planning/  global/  detailed/  optimization/  verification/
  managers/
    routing/  inspection/
  clients/
    cli/                     the traxide binary, including the Freerouting-compatible mode
    viewer/                  optional live viewer
    wasm/                    the web demo
xtask/                       repository tasks: layer check, end-to-end test, later corpus and release
spikes/                      throwaway prototypes of research tasks, outside the workspace and never released
kicad-plugin/                the KiCad plugin: its plugin.json, PCM metadata and icons; the traxide binary is its entry point
e2e/kicad/                   the KiCad half of the end-to-end test: board recipes, import, export
fixtures/boards/             small DSN boards shared by the tests of several crates
corpus/                      benchmark boards and baselines (Git LFS), with [[TRX-15]]
docs/                        this wiki
tasks/                       the task board
```

Crates are named `traxide-<component>` (`traxide-detailed`, `traxide-board-access`). The binary is `traxide`.

## Layer rules

The call rules of the [Architecture](Architecture.md#rules-between-components) are checked by `cargo xtask layers`, which reads `cargo metadata` and fails when a crate depends on one its layer may not use:

- clients → managers, contracts, utilities
- managers → engines, access, contracts, utilities
- engines → contracts, utilities. An engine uses Layout Access and Compute Access only through their **traits** in `traxide-contracts`; the manager hands it the implementation (dependency inversion), so an engine crate never depends on an access crate
- access → contracts, utilities, third-party crates for their resource (never another access crate)
- utilities → other utilities at most; contracts → geometry only

Crates appear when the task that needs them starts, not before (YAGNI): the layout above is the destination, not a scaffold.

CI runs the check on every pull request, next to `cargo fmt --check`, `cargo clippy -- -D warnings` and the tests.

CI lints on Windows too, where some types are larger (a path takes 32 bytes, not 24) and size lints can fire that Linux never shows. Linting for Windows needs no Windows: `rustup target add x86_64-pc-windows-msvc`, then `cargo clippy --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings`.

## Building

```bash
cargo build --release                                  # the traxide binary in target/release
cargo run --release -p traxide-cli -- route board.dsn  # writes board.ses next to it
cargo test --all-features                              # unit and end-to-end tests
cargo xtask layers                                     # the layer rules
cargo xtask e2e                                        # the KiCad round trip, see below
cargo xtask kicad-package linux TRAXIDE traxide-kicad-linux.zip  # the KiCad plugin's package with that binary
```

Release builds use `lto = "fat"`, `codegen-units = 1` and `panic = "abort"`. Once the GPU backend exists ([[TRX-28]]) it will be a default feature, and `--no-default-features` will build a CPU-only binary.

`traxide route` exits with 0 when every connection is routed, 2 when some are not, 3 when the independent check finds a broken rule (a bug: please report it with the board) and 1 on errors.

## Craftsmanship

How code is written here, in order of importance:

1. **Test first.** Red, green, refactor. A behaviour is specified by a failing test before it is built, and a bug by a failing test before it is fixed. Tests are named after the behaviour they pin down (`routes_around_a_pad_of_another_net`), not after the function they call.
2. **Names instead of comments.** A well-chosen name for a file, type, function or variable replaces a paragraph. Extract a function or a variable when a name would explain what a comment otherwise would.
3. **Comments say why, never what.** The code says what it does. A comment earns its place by explaining a reason the code cannot show: a constraint, a trade-off, a quirk of KiCad, a reference to a paper. Doc comments on public items state the contract (units, invariants, errors), not the implementation.
4. **KISS and YAGNI.** The simplest thing that passes the tests and fits the architecture. No speculative generics, options or crates; extension points exist where [[Architecture/Volatilities]] says change is expected, and nowhere else.
5. **DRY for knowledge, WET for code until the third time.** Every rule, unit conversion and format quirk has one home. Similar-looking code is merged on the third occurrence, once the shared concept is clear. Tests may repeat themselves when that keeps each test readable on its own.
6. **SOLID, the Rust way.** One responsibility per module and type; traits only at component boundaries and volatilities (open/closed); small traits a client can implement fully (interface segregation); engines depend on access traits, not implementations (dependency inversion).
7. **Make illegal states unrepresentable.** Newtypes for ids and units (`Nm`, `NetId`), enums instead of flags, constructors that validate. A value that exists is valid.
8. **Errors are values.** Libraries return `Result` with `thiserror` types that say what went wrong and where (file, line, item); the client reports them with `anyhow`. `unwrap` and `expect` only where an invariant guarantees success, and `expect` says which.
9. **Deterministic always.** No iteration over hash maps in results, no time or address dependence; ties break on ids.

## Testing

Three levels, all run by `cargo test` except where noted:

- **Unit tests** next to the code. Engines are pure and take contracts, so most tests build a small `Design` in code.
- **End-to-end tests** in `tests/` of the CLI crate: run the `traxide` binary on DSN files and check the SES it writes — every connection routed, no clearance violation.
- **Real-world tests** through KiCad: `cargo xtask e2e` builds the board recipes of `e2e/kicad/boards.py` with KiCad's Python API, exports them as DSN, routes them with `traxide`, imports the sessions back into KiCad and requires KiCad's own DRC (`kicad-cli pcb drc`, on the board with its copper zones refilled first, as a designer does before checking a routed board) to report no violations and no unconnected items. KiCad, not our code, is the judge. Results land in `target/e2e/<recipe>/`.
  - By default KiCad runs in the official Docker image `kicad/kicad:10.0`; `TRAXIDE_KICAD_IMAGE=kicad/kicad:9.0` tests KiCad 9. CI runs both.
  - In Docker, each recipe is also routed in a running KiCad ([[TRX-31]]): the PCB editor opens the board on a virtual display (`xvfb-run`, Linux only) with its API enabled; its API must give the same design as the board file; traxide's KiCad plugin, packaged as `cargo xtask kicad-package` packages it and installed as KiCad's *Install from File* installs it, routes it when the test presses its toolbar button (found on a screenshot by the colour of its icon, pressed with `xdotool`; both it and ImageMagick's `import` must be installed); KiCad saves it and its DRC must find it clean, with its copper zones refilled by traxide (`ground-pour`); one undo must take the routes back, two on a board with copper zones, whose refill is an undo step of its own. Results land in `target/e2e/<recipe>/in-kicad/`.
  - `cargo xtask e2e --local` uses an installed KiCad instead: `TRAXIDE_KICAD_PYTHON` names a Python that can `import pcbnew` (on macOS the one inside `KiCad.app`), `TRAXIDE_KICAD_CLI` the `kicad-cli`.
  - Name recipes to run only those: `cargo xtask e2e crossing matrix-6x17`. The default set stays fast; bigger boards are benchmarks ([[Benchmarks]]).
  - A recipe with `custom_rules` saves them as `RECIPE.kicad_dru` and is routed only from its `.kicad_pcb`: DSN cannot carry custom rules. Building the static traxide for the plugin needs `rustup target add x86_64-unknown-linux-musl`.
  - KiCad 7 and 8 are not supported: they are not what people route with, and KiCad 7 cannot import sessions from a script.
- **Ground truth** from KiCad: `e2e/kicad/fixtures.py` exports boards together with KiCad's own placement of every pad; `crates/access/board/tests/kicad_ground_truth.rs` holds the DSN reader to it.
- **Test designs** in code: `traxide-contracts` with the `testing` feature offers `DesignBuilder`, so engine tests build boards in a few lines.
- **Property tests** (`proptest`) for geometry and for Layout Access: random shapes, random concurrent transactions.
- **Golden tests** for Board Access: parse, write, parse again, compare; and fixed DSN files with expected SES output.
- **Determinism tests**: route the same board with 1, 4 and all threads, CPU and GPU, and require identical routes.
- **Verification**: every routing test ends with a clearance check (the Verification Engine once it exists); a violation fails the test.

## Benchmarking

`cargo xtask bench [RECIPE…]` routes boards with traxide and, given `--freerouting "java -jar freerouting.jar"`, with Freerouting too, and has KiCad judge both the same way: time, unconnected items and violations from KiCad's DRC, tracks, vias and track length. `--timeout SECONDS` stops a router that runs too long. The default boards are `matrix-6x17` and `keyboard-tkl`; results land in `target/bench/`. The Benchmark workflow runs it in CI against Freerouting 2.5.0 and shows the table in the job summary; [[Benchmarks]] keeps the history.

`cargo run --release -p xtask -- negotiation BOARD` routes one board and shows what negotiation did ([[TRX-56]]): per round of as many commits as the board has connections, the connections routed at its end and the rip-ups it took; the pairs of connections that take each other's place most often; and the 1 mm squares with the most rip-ups, drawn over the board in `target/negotiation/<board>/rip-ups.svg` with the connections ripped up as lines. Built in release, as routing in a debug build takes ten times as long.

`cargo xtask corpus` routes the real boards of [[corpus/freerouting-fixtures.txt]] and writes traxide's results to `target/corpus/results-K-of-N.json`; `cargo xtask gate RESULTS.json…` compares them with the baseline in `corpus/baseline.json` and fails on a regression: a board the baseline completes that is no longer complete, any rule broken, or clearly fewer nets routed ([[TRX-22]] has the tolerances). A board that takes traxide's negotiation limit, in the baseline or now, is routed against the clock: how many nets it routes then depends on the runner's speed, by up to a tenth on the same commit, so fewer nets there is only noted ([[TRX-56]]). It lists the improvements; `--update-baseline` writes the results into the baseline, which is how it is raised, on purpose and in its own commit. `--partial` compares only the boards the results hold, as after one shard:

```bash
cargo xtask corpus --shard 1/6 && cargo xtask gate --partial target/corpus/results-1-of-6.json
```

**Adding a KiCad project to the corpus** ([[TRX-64]]): add a line to [[corpus/kicad-projects.txt]] naming the GitHub repository, the commit to pin and the board's path in it:

```text
https://github.com/h3ll5ur7er/tokenboard 478b4f9 kb/kb.kicad_pcb
```

The corpus downloads the board with the `.kicad_pro` and `.kicad_dru` of the same name into `target/corpus/projects/<repository>-<board>-<commit>/` and never stores them, so add only boards whose licence or author allows them to be used so. A board committed routed is stripped of its unlocked tracks, arcs and vias first; its zones and the copper its author locked stay, and the board as committed is judged too, in the table's column "Routed by its author". traxide routes the stripped `.kicad_pcb` itself, custom rules included. On every board of the corpus traxide routes with `--time-limit` at three quarters of `--timeout`, as real boards, and congested ones that it routes a second time ([[TRX-56]]), take longer than the corpus's limit, and a stopped board tells the gate nothing. Freerouting routes the DSN that KiCad, in its Docker image as for `cargo xtask e2e` (`--local` for an installed KiCad), exports of the stripped board, and KiCad imports its session; `traxide check` judges all three boards alike. KiCad's DRC judges them too, in the column "KiCad's DRC": its errors by kind and its missing connections, with every zone refilled first (`roundtrip.py refill-all`), as a board file keeps the fills its zones came with, poured round the copper before routing, and a designer refills before checking; without KiCad the column says why it judged nothing. A new board enters the gate when its result is written into `corpus/baseline.json` (`cargo xtask gate --partial --update-baseline` on its results).

The Benchmark workflow runs the gate after its six corpus shards; its `corpus-results` artifact, copied over `corpus/baseline.json`, raises the baseline to that run. Later, with [[TRX-21]], CI keeps the history across commits and comments regressions on the pull request that caused them.

## Clean-room rules

traxide is licensed `MIT OR Apache-2.0` ([LICENSE-MIT](../LICENSE-MIT), [LICENSE-APACHE](../LICENSE-APACHE)), so that anyone, including companies that ban copyleft, can use it. That only holds if no GPL code gets in:

- **Don't read the source of GPL routers while writing traxide's code**: Freerouting, KiCad's router (PNS), the GPL Rust port of Freerouting. Learn from papers, documentation, their observable behaviour and the summaries in [[Research/Prior-art]] — ideas are free, code is not.
- **Formats and interfaces are fair to implement from their specifications**: Specctra DSN/SES, KiCad's file formats, Freerouting's command line flags and API.
- **One exception, decided by the maintainer: KiCad's IPC API definitions.** The `.proto` files traxide needs are copied unchanged, GPL-3.0-or-later headers included, into `crates/access/board/proto/kicad/` (its README names their source and version), and the build generates the messages from them ([[TRX-17]]). They describe an interface, as do those of the many permissively licensed clients generated from them, KiCad's own `kicad-python` (MIT) among them; the maintainer accepts the small remaining risk. Nothing else of KiCad's comes in: its router and the rest of its source, like Freerouting, stay off limits.
- **Dependencies must be permissive** (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 for unmodified files). `cargo deny check licenses` enforces this in CI. Code from MIT projects such as Topola, KiCadRoutingTools or AutoRoute may be reused with its copyright notice.
- **Corpus boards** are test data, not part of the binary; each keeps its own licence in `corpus/`.
- Contributions are accepted under both licences (inbound = outbound), stated in the contribution guide.

## Commits and branches

Name the task in commit messages and pull requests (`TRX-12: Route connections with multi-layer A*`). Tasks and wiki pages are committed with the work they describe.

## Working in parallel

Several agent sessions may work at once, each in an area of its own, so that their branches merge without conflicts ([[TRX-56]] showed that experiments in one area, such as negotiation, interfere and are better kept in one hand). Each session:

- owns its tasks: their `assignee` names it (`claude` is the main session, which also reviews and merges the others; `claude-release`, `claude-planes`, `claude-gate` and so on are the others), and it keeps them true as it goes: `in-progress` when it starts, checklist items ticked as each lands, decisions and blockers noted, `review` when its branch waits to be merged, `done` only once it is merged and verified;
- works on a branch of its own, `claude/<task>-<topic>`, and leaves merging to the main session, which reviews the branch, runs every check and the KiCad round trip again, and merges it into the branch of the open pull request;
- records what it measured in [[Benchmarks]] and the task, rejected attempts included, with their numbers.
- takes a new task's ID only after fetching: the next number above the highest used on `main` and on every `origin/claude/*` branch, as two sessions creating tasks at once would otherwise give both the same ID.

The board in `tasks/` is how the maintainer follows the work: a task nobody works on is `todo`, however much of its checklist is ticked.
