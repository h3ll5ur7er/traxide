"""The KiCad half of traxide's end-to-end test, run with KiCad's own Python.

  python3 roundtrip.py export RECIPE FOLDER
      builds the recipe's board, saves FOLDER/RECIPE.kicad_pcb with its custom
      rules in FOLDER/RECIPE.kicad_dru, if it has any, and exports
      FOLDER/RECIPE.dsn, as a user would with File → Export → Specctra DSN.
      Its zones stay unfilled.

  python3 roundtrip.py dsn BOARD.kicad_pcb
      exports a saved board, with its project's rules, as BOARD.dsn.

  python3 roundtrip.py import BOARD.kicad_pcb SESSION.ses ROUTED.kicad_pcb
      imports the session into the board, as File → Import → Specctra Session
      does, saves the result as ROUTED.kicad_pcb and prints, as JSON, how many
      tracks and vias it holds, how long the tracks are, and how many copper
      zones it has and how many of them are filled.

  python3 roundtrip.py stats ROUTED.kicad_pcb
      loads a board routed without KiCad, as KiCad opens it, and prints the
      same statistics.

  python3 roundtrip.py refill ROUTED.kicad_pcb
      fills the board's unfilled copper zones and saves it, as a designer
      does before checking a routed board: an unfilled zone joins nothing.

  python3 roundtrip.py refill-all ROUTED.kicad_pcb
      fills all of the board's copper zones again and saves it: a board file
      keeps the fills its zones came with, poured round the copper before
      routing.

KiCad's DRC (kicad-cli pcb drc) then judges the routed board.
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path

import pcbnew

from boards import RECIPES


def build(recipe: str, folder: Path) -> None:
    folder.mkdir(parents=True, exist_ok=True)
    pcbnew.SaveBoard(str(folder / f"{recipe}.kicad_pcb"), RECIPES[recipe].build())


def export(recipe: str, folder: Path) -> None:
    # A board built in memory has no project, so saving it writes no net
    # classes. Loaded by a fresh process it has one, and saving it then writes
    # RECIPE.kicad_pro, where KiCad's DRC reads the rules.
    subprocess.run([sys.executable, __file__, "build", recipe, str(folder)], check=True)
    board_file = folder / f"{recipe}.kicad_pcb"
    board = pcbnew.LoadBoard(str(board_file))
    RECIPES[recipe].apply_rules(board)
    pcbnew.SaveBoard(str(board_file), board)
    custom_rules = board_file.with_suffix(".kicad_dru")
    custom_rules.unlink(missing_ok=True)
    if RECIPES[recipe].custom_rules is not None:
        custom_rules.write_text(RECIPES[recipe].custom_rules)
    if not pcbnew.ExportSpecctraDSN(board, str(folder / f"{recipe}.dsn")):
        raise SystemExit(f"KiCad could not export {recipe} as DSN")


def export_board(board_file: Path) -> None:
    board = pcbnew.LoadBoard(str(board_file))
    if not pcbnew.ExportSpecctraDSN(board, str(board_file.with_suffix(".dsn"))):
        raise SystemExit(f"KiCad could not export {board_file} as DSN")


def import_session(board_file: Path, session: Path, routed: Path) -> None:
    board = pcbnew.LoadBoard(str(board_file))
    if not pcbnew.ImportSpecctraSES(board, str(session)):
        raise SystemExit(f"KiCad could not import {session}")
    pcbnew.SaveBoard(str(routed), board)
    # Saving wrote a project with KiCad's default rules; the board's own are in
    # its project and its custom rules.
    shutil.copy(board_file.with_suffix(".kicad_pro"), routed.with_suffix(".kicad_pro"))
    if board_file.with_suffix(".kicad_dru").is_file():
        shutil.copy(board_file.with_suffix(".kicad_dru"), routed.with_suffix(".kicad_dru"))
    print_stats(board)


def refill(board_file: Path) -> None:
    board = pcbnew.LoadBoard(str(board_file))
    # A board traxide routed in a running KiCad comes refilled, and is
    # judged as it came.
    unfilled = [zone for zone in board.Zones() if not zone.GetIsRuleArea() and not zone.IsFilled()]
    if unfilled:
        pcbnew.ZONE_FILLER(board).Fill(unfilled)
        pcbnew.SaveBoard(str(board_file), board)


def refill_all(board_file: Path) -> None:
    board = pcbnew.LoadBoard(str(board_file))
    # A board file keeps the fills its zones came with, poured round the
    # copper before routing; a designer refills them before checking.
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea()]
    if zones:
        pcbnew.ZONE_FILLER(board).Fill(zones)
        pcbnew.SaveBoard(str(board_file), board)


def print_stats(board) -> None:
    items = list(board.GetTracks())
    vias = [item for item in items if item.GetClass() == "PCB_VIA"]
    length = sum(item.GetLength() for item in items if item.GetClass() != "PCB_VIA")
    zones = [zone for zone in board.Zones() if not zone.GetIsRuleArea()]
    print(json.dumps({
        "tracks": len(items) - len(vias),
        "vias": len(vias),
        "length_mm": pcbnew.ToMM(length),
        "zones": len(zones),
        "filled_zones": sum(1 for zone in zones if zone.IsFilled()),
    }))


if __name__ == "__main__":
    match sys.argv[1:]:
        case ["build", recipe, folder]:
            build(recipe, Path(folder))
        case ["export", recipe, folder]:
            export(recipe, Path(folder))
        case ["dsn", board_file]:
            export_board(Path(board_file))
        case ["import", board_file, session, routed]:
            import_session(Path(board_file), Path(session), Path(routed))
        case ["stats", routed]:
            print_stats(pcbnew.LoadBoard(routed))
        case ["refill", routed]:
            refill(Path(routed))
        case ["refill-all", routed]:
            refill_all(Path(routed))
        case _:
            raise SystemExit(__doc__)
