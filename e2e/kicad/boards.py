"""Test boards for traxide, built with KiCad's own Python API (KiCad 9 and 10)."""

from dataclasses import dataclass

import pcbnew

MM = pcbnew.FromMM


@dataclass(frozen=True)
class Smd:
    number: str
    x: float
    y: float
    width: float
    height: float
    net: str | None = None
    rotation: float = 0.0


@dataclass(frozen=True)
class Npth:
    """A non-plated hole, such as a switch's centre post: no copper, no net."""
    number: str
    x: float
    y: float
    drill: float
    net: None = None


@dataclass(frozen=True)
class Tht:
    number: str
    x: float
    y: float
    diameter: float
    drill: float
    net: str | None = None


@dataclass(frozen=True)
class NetClass:
    """A net class for the nets whose names match `pattern`."""
    name: str
    pattern: str
    track: float
    clearance: float


@dataclass(frozen=True)
class Recipe:
    """How to build a board, the rules of its net classes and its custom rules
    (the text of a .kicad_dru). KiCad keeps net classes in the project, not in
    the board, so they are applied when the board is saved with its project
    (roundtrip.py)."""
    build: object
    track: float = 0.2
    clearance: float = 0.2
    classes: tuple[NetClass, ...] = ()
    custom_rules: str | None = None

    def apply_rules(self, board) -> None:
        settings = board.GetDesignSettings().m_NetSettings
        default_class = settings.GetDefaultNetclass()
        default_class.SetTrackWidth(MM(self.track))
        default_class.SetClearance(MM(self.clearance))
        for spec in self.classes:
            net_class = pcbnew.NETCLASS(spec.name)
            net_class.SetTrackWidth(MM(spec.track))
            net_class.SetClearance(MM(spec.clearance))
            settings.SetNetclass(spec.name, net_class)
            settings.SetNetclassPatternAssignment(spec.pattern, spec.name)


class BoardBuilder:
    def __init__(self, width: float, height: float, copper_layers: int = 2):
        self.board = pcbnew.BOARD()
        self.board.SetCopperLayerCount(copper_layers)
        self._nets: dict[str, pcbnew.NETINFO_ITEM] = {}
        self._outline(width, height)

    def _outline(self, width: float, height: float) -> None:
        corners = [(0, 0), (width, 0), (width, height), (0, height)]
        for start, end in zip(corners, corners[1:] + corners[:1]):
            edge = pcbnew.PCB_SHAPE(self.board)
            edge.SetShape(pcbnew.SHAPE_T_SEGMENT)
            edge.SetStart(pcbnew.VECTOR2I(MM(start[0]), MM(start[1])))
            edge.SetEnd(pcbnew.VECTOR2I(MM(end[0]), MM(end[1])))
            edge.SetLayer(pcbnew.Edge_Cuts)
            edge.SetWidth(MM(0.05))
            self.board.Add(edge)

    def _net(self, name: str | None):
        if name is None:
            return None
        if name not in self._nets:
            net = pcbnew.NETINFO_ITEM(self.board, name)
            self.board.Add(net)
            self._nets[name] = net
        return self._nets[name]

    def footprint(self, reference: str, x: float, y: float, pads, rotation: float = 0.0, back: bool = False):
        footprint = pcbnew.FOOTPRINT(self.board)
        footprint.SetReference(reference)
        footprint.SetFPID(pcbnew.LIB_ID("traxide", reference.rstrip("0123456789")))
        footprint.Reference().SetVisible(False)
        footprint.Value().SetVisible(False)
        self.board.Add(footprint)
        for spec in pads:
            footprint.Add(self._pad(footprint, spec))
        footprint.SetPosition(pcbnew.VECTOR2I(MM(x), MM(y)))
        footprint.SetOrientationDegrees(rotation)
        if back:
            footprint.Flip(footprint.GetPosition(), pcbnew.FLIP_DIRECTION_LEFT_RIGHT)
        return footprint

    def track(self, net: str, layer, points, width: float = 0.25) -> None:
        """Wiring of `net` along `points`, a segment for each step, as a designer draws it."""
        for start, end in zip(points, points[1:]):
            segment = pcbnew.PCB_TRACK(self.board)
            segment.SetStart(pcbnew.VECTOR2I(MM(start[0]), MM(start[1])))
            segment.SetEnd(pcbnew.VECTOR2I(MM(end[0]), MM(end[1])))
            segment.SetWidth(MM(width))
            segment.SetLayer(layer)
            segment.SetNet(self._net(net))
            self.board.Add(segment)

    def zone(self, net: str, layer, corners) -> None:
        """A copper zone of `net`, left unfilled, as a board fresh from a script is."""
        zone = pcbnew.ZONE(self.board)
        zone.SetLayer(layer)
        zone.SetNet(self._net(net))
        outline = zone.Outline()
        outline.NewOutline()
        for x, y in corners:
            outline.Append(MM(x), MM(y))
        self.board.Add(zone)

    def _pad(self, footprint, spec):
        pad = pcbnew.PAD(footprint)
        pad.SetNumber(spec.number)
        if isinstance(spec, Npth):
            pad.SetAttribute(pcbnew.PAD_ATTRIB_NPTH)
            pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE)
            pad.SetSize(pcbnew.VECTOR2I(MM(spec.drill), MM(spec.drill)))
            pad.SetDrillSize(pcbnew.VECTOR2I(MM(spec.drill), MM(spec.drill)))
            pad.SetLayerSet(pad.UnplatedHoleMask())
        elif isinstance(spec, Smd):
            pad.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
            pad.SetShape(pcbnew.PAD_SHAPE_RECTANGLE)
            pad.SetSize(pcbnew.VECTOR2I(MM(spec.width), MM(spec.height)))
            pad.SetLayerSet(pad.SMDMask())
            pad.SetOrientationDegrees(spec.rotation)
        else:
            pad.SetAttribute(pcbnew.PAD_ATTRIB_PTH)
            pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE)
            pad.SetSize(pcbnew.VECTOR2I(MM(spec.diameter), MM(spec.diameter)))
            pad.SetDrillSize(pcbnew.VECTOR2I(MM(spec.drill), MM(spec.drill)))
            pad.SetLayerSet(pad.PTHMask())
        pad.SetFPRelativePosition(pcbnew.VECTOR2I(MM(spec.x), MM(spec.y)))
        net = self._net(spec.net)
        if net is not None:
            pad.SetNet(net)
        return pad


def placements() -> pcbnew.BOARD:
    """Footprints on both sides at awkward angles, to pin down how DSN places pads."""
    builder = BoardBuilder(60, 40)
    asymmetric = lambda net: [
        Smd("1", -1.5, 0, 1.0, 0.6, net),
        Smd("2", 1.0, 0.8, 0.8, 0.5, net, rotation=30),
    ]
    builder.footprint("U1", 10, 10, asymmetric("A"))
    builder.footprint("U2", 25, 10, asymmetric("B"), rotation=30)
    builder.footprint("U3", 40, 10, asymmetric("C"), back=True)
    builder.footprint("U4", 10, 25, asymmetric("D"), rotation=90, back=True)
    builder.footprint("U5", 25, 25, asymmetric("E"), rotation=30, back=True)
    builder.footprint("J1", 45, 28, [Tht("1", -1.27, 0, 1.7, 1.0, "A"), Tht("2", 1.27, 0, 1.7, 1.0, "B")], rotation=45)
    return builder.board


def crossing_pads() -> BoardBuilder:
    builder = BoardBuilder(30, 30)
    pad = lambda net: [Smd("1", 0, 0, 1.2, 1.2, net)]
    builder.footprint("TP1", 5, 15, pad("ACROSS"))
    builder.footprint("TP2", 25, 15, pad("ACROSS"))
    builder.footprint("TP3", 15, 5, pad("DOWN"))
    builder.footprint("TP4", 15, 25, pad("DOWN"))
    return builder


def crossing() -> pcbnew.BOARD:
    """Two nets whose straight lines cross: one must change layers."""
    return crossing_pads().board


def detour() -> pcbnew.BOARD:
    """The crossing board with DOWN wired by hand round a detour, which the
    Optimise button pulls straight, and ACROSS left for routing."""
    builder = crossing_pads()
    builder.track("DOWN", pcbnew.F_Cu, [(15, 5), (20, 10), (20, 20), (15, 25)])
    return builder.board


def ground_pour() -> pcbnew.BOARD:
    """Two crossing signals over a ground zone that covers the back: the zone
    must be refilled around the routes, the via included."""
    builder = BoardBuilder(30, 30)
    pad = lambda net: [Smd("1", 0, 0, 1.2, 1.2, net)]
    builder.footprint("TP1", 5, 15, pad("ACROSS"))
    builder.footprint("TP2", 25, 15, pad("ACROSS"))
    builder.footprint("TP3", 15, 5, pad("DOWN"))
    builder.footprint("TP4", 15, 25, pad("DOWN"))
    ground = lambda: [Tht("1", 0, 0, 1.7, 1.0, "GND")]
    builder.footprint("J1", 5, 5, ground())
    builder.footprint("J2", 25, 25, ground())
    builder.zone("GND", pcbnew.B_Cu, [(0, 0), (30, 0), (30, 30), (0, 30)])
    return builder.board


def inner_plane() -> pcbnew.BOARD:
    """A 4-layer board whose first inner layer, of type power, is a ground
    plane: the ground pads of its capacitors, on both sides, join it by vias
    beside them, the header's ground pin through its hole; the supply and a
    signal are routed as tracks."""
    builder = BoardBuilder(40, 30, copper_layers=4)
    builder.board.SetLayerType(pcbnew.In1_Cu, pcbnew.LT_POWER)
    cap = lambda a, b: [Smd("1", -0.8, 0, 0.9, 1.0, a), Smd("2", 0.8, 0, 0.9, 1.0, b)]
    builder.footprint("J1", 20, 4, [Tht("1", -1.27, 0, 1.7, 1.0, "GND"),
                                    Tht("2", 1.27, 0, 1.7, 1.0, "VCC")])
    builder.footprint("C1", 8, 8, cap("VCC", "GND"))
    builder.footprint("C2", 32, 8, cap("VCC", "GND"), rotation=90)
    builder.footprint("C3", 8, 22, cap("VCC", "GND"), back=True)
    builder.footprint("C4", 32, 22, cap("SIG", "GND"), back=True)
    builder.footprint("TP1", 20, 26, [Smd("1", 0, 0, 1.2, 1.2, "SIG")])
    builder.zone("GND", pcbnew.In1_Cu, [(0.5, 0.5), (39.5, 0.5), (39.5, 29.5), (0.5, 29.5)])
    return builder.board


def isolation() -> pcbnew.BOARD:
    """A high-voltage net that two signals must cross, at the distance the
    board's custom rules ask (ISOLATION_RULES)."""
    builder = BoardBuilder(40, 30)
    pad = lambda net: [Smd("1", 0, 0, 1.2, 1.2, net)]
    builder.footprint("TP1", 5, 8, pad("HV"))
    builder.footprint("TP2", 35, 22, pad("HV"))
    builder.footprint("TP3", 5, 22, pad("SIG1"))
    builder.footprint("TP4", 35, 8, pad("SIG1"))
    builder.footprint("TP5", 20, 3, pad("SIG2"))
    builder.footprint("TP6", 20, 27, pad("SIG2"))
    return builder.board


ISOLATION_RULES = """(version 1)
# Signals keep 3 mm from the high-voltage net, on every layer.
(rule "HV isolation"
    (constraint clearance (min 3mm))
    (condition "A.NetClass == 'HV' && B.NetClass != 'HV'"))
(rule "SIG2 is wide"
    (constraint track_width (min 0.4mm))
    (condition "A.NetName == 'SIG2'"))
(rule "HV away from the edge"
    (constraint edge_clearance (min 2mm))
    (condition "A.NetClass == 'HV'"))
# traxide cannot tell a pad from a track, so this may only keep copper apart.
(rule "pads close"
    (constraint clearance (min 0.1mm))
    (condition "A.Type == 'Pad' && B.Type == 'Pad'"))
"""


KEY_PITCH = 19.05


def key_matrix(rows: int, columns: int, copper_layers: int = 2) -> pcbnew.BOARD:
    """A keyboard's switch matrix: an MX switch and a diode per key, rows and
    columns wired to a header along the top edge, as on a hand-wired NKRO board."""
    header_pins = rows + columns
    width = max(columns * KEY_PITCH, header_pins * 2.54 + 10)
    builder = BoardBuilder(width, rows * KEY_PITCH + 12, copper_layers)
    header = [Tht(str(i + 1), i * 2.54, 0, 1.7, 1.0, net) for i, net in
              enumerate([f"ROW{r}" for r in range(rows)] + [f"COL{c}" for c in range(columns)])]
    builder.footprint("J1", (width - (header_pins - 1) * 2.54) / 2, 4, header)
    for r in range(rows):
        for c in range(columns):
            x, y = KEY_PITCH * (c + 0.5), 12 + KEY_PITCH * (r + 0.5)
            key = f"K{r}_{c}"
            builder.footprint(f"SW{r * columns + c + 1}", x, y, [
                Tht("1", -3.81, -2.54, 2.2, 1.5, f"COL{c}"),
                Tht("2", 2.54, -5.08, 2.2, 1.5, key),
            ])
            builder.footprint(f"D{r * columns + c + 1}", x + 4.5, y + 3, [
                Smd("1", -1.65, 0, 0.9, 1.2, f"ROW{r}"),
                Smd("2", 1.65, 0, 0.9, 1.2, key),
            ], rotation=90, back=True)
    return builder.board


# The physical layout of an ISO tenkeyless keyboard (88 keys), in key units of
# 19.05 mm: (x, y, width, height) of each key, row by row as the matrix wires them.
ISO_TKL = [
    [(0, 0, 1, 1)] + [(x, 0, 1, 1) for x in (2, 3, 4, 5, 6.5, 7.5, 8.5, 9.5, 11, 12, 13, 14, 15.25, 16.25, 17.25)],
    [(x, 1.25, 1, 1) for x in range(13)] + [(13, 1.25, 2, 1)] + [(x, 1.25, 1, 1) for x in (15.25, 16.25, 17.25)],
    [(0, 2.25, 1.5, 1)] + [(1.5 + i, 2.25, 1, 1) for i in range(12)] + [(13.75, 2.25, 1.25, 2)]
    + [(x, 2.25, 1, 1) for x in (15.25, 16.25, 17.25)],
    [(0, 3.25, 1.75, 1)] + [(1.75 + i, 3.25, 1, 1) for i in range(12)],
    [(0, 4.25, 1.25, 1), (1.25, 4.25, 1, 1)] + [(2.25 + i, 4.25, 1, 1) for i in range(10)]
    + [(12.25, 4.25, 2.75, 1), (16.25, 4.25, 1, 1)],
    [(0, 5.25, 1.25, 1), (1.25, 5.25, 1.25, 1), (2.5, 5.25, 1.25, 1), (3.75, 5.25, 6.25, 1)]
    + [(x, 5.25, 1.25, 1) for x in (10, 11.25, 12.5, 13.75)] + [(x, 5.25, 1, 1) for x in (15.25, 16.25, 17.25)],
]


def mx_switch(column: str, key: str) -> list:
    return [
        Tht("1", -3.81, -2.54, 2.2, 1.5, column),
        Tht("2", 2.54, -5.08, 2.2, 1.5, key),
        Npth("", 0, 0, 4.0),
        Npth("", -5.08, 0, 1.75),
        Npth("", 5.08, 0, 1.75),
    ]


def r0402(a: str, b: str) -> list:
    return [Smd("1", -0.5, 0, 0.5, 0.55, a), Smd("2", 0.5, 0, 0.5, 0.55, b)]


def qfn60(pins: dict[int, str]) -> list:
    """A 7 × 7 mm QFN-60 with 0.4 mm pitch, as the RP2350A: pin 1 at the top
    of the left side, counting anti-clockwise, and a ground pad in the middle."""
    pads = []
    for index in range(60):
        side, position = divmod(index, 15)
        offset = -2.8 + 0.4 * position
        # 0.7 mm long, so the corner pads of two sides keep 0.15 mm apart.
        x, y, width, height = [
            (-3.4, offset, 0.7, 0.2),
            (offset, 3.4, 0.2, 0.7),
            (3.4, -offset, 0.7, 0.2),
            (-offset, -3.4, 0.2, 0.7),
        ][side]
        pads.append(Smd(str(index + 1), x, y, width, height, pins.get(index + 1)))
    pads.append(Smd("61", 0, 0, 3.4, 3.4, "GND"))
    return pads


def keyboard_tkl() -> pcbnew.BOARD:
    """An ISO tenkeyless keyboard as hobbyists build it: 88 MX switches with a
    diode each (NKRO), a 6 × 17 matrix, and an RP2350-like MCU with USB-C,
    QSPI flash, crystal and regulator in a strip along the top, on four layers."""
    unit, top = KEY_PITCH, 24.0
    builder = BoardBuilder(18.25 * unit + 4, top + 6.25 * unit + 2, copper_layers=4)
    for row, keys in enumerate(ISO_TKL):
        for column, (x, y, w, h) in enumerate(keys):
            key = f"K{row}_{column}"
            cx, cy = 2 + (x + w / 2) * unit, top + (y + h / 2) * unit
            number = sum(len(r) for r in ISO_TKL[:row]) + column + 1
            builder.footprint(f"SW{number}", cx, cy, mx_switch(f"COL{column}", key))
            # Below the switch's centre, where real boards put the diode: clear of its holes.
            builder.footprint(f"D{number}", cx, cy + 6.5, [
                Smd("1", -1.65, 0, 0.9, 1.2, f"ROW{row}"),
                Smd("2", 1.65, 0, 0.9, 1.2, key),
            ], back=True)
    pins = {16 + i: f"ROW{i}" for i in range(6)} | {22 + i: f"COL{i}" for i in range(9)}
    pins |= {6 + i: f"COL{9 + i}" for i in range(8)}
    pins |= {1: "3V3", 2: "3V3", 3: "XIN", 4: "XOUT", 5: "1V1", 14: "3V3"}
    pins |= {31: "QSPI_SS", 32: "QSPI_SD1", 33: "QSPI_SD2", 34: "QSPI_SD0", 35: "QSPI_SCLK", 36: "QSPI_SD3",
             38: "3V3", 40: "1V1", 44: "3V3"}
    pins |= {52: "USB_DP", 53: "USB_DM", 50: "3V3"}
    mcu_x, mcu_y = 180.0, 13.0
    builder.footprint("U1", mcu_x, mcu_y, qfn60(pins))
    builder.footprint("J1", mcu_x, 3.2, [
        Smd(str(i + 1), -1.25 + 0.5 * i, 0, 0.3, 1.0, net)
        for i, net in enumerate(["GND", "CC1", "USB_DP_C", "USB_DM_C", "CC2", "VBUS"])
    ])
    builder.footprint("R1", mcu_x - 1.0, 6.8, r0402("USB_DP_C", "USB_DP"), rotation=90)
    builder.footprint("R2", mcu_x + 1.0, 6.8, r0402("USB_DM_C", "USB_DM"), rotation=90)
    builder.footprint("R3", mcu_x - 5.0, 3.5, r0402("CC1", "GND"))
    builder.footprint("R4", mcu_x + 5.0, 3.5, r0402("CC2", "GND"))
    builder.footprint("U3", 166.0, 5.0, [
        Smd("1", -0.95, 1.1, 0.6, 1.0, "VBUS"), Smd("2", 0, 1.1, 0.6, 1.0, "GND"),
        Smd("3", 0.95, 1.1, 0.6, 1.0, "VBUS"), Smd("4", 0.95, -1.1, 0.6, 1.0),
        Smd("5", -0.95, -1.1, 0.6, 1.0, "3V3"),
    ])
    builder.footprint("C1", 162.0, 5.0, r0402("VBUS", "GND"), rotation=90)
    builder.footprint("C2", 170.0, 5.0, r0402("3V3", "GND"), rotation=90)
    builder.footprint("Y1", 168.0, 14.0, [
        Smd("1", -1.1, 0.8, 1.2, 1.0, "XIN"), Smd("2", 1.1, 0.8, 1.2, 1.0, "GND"),
        Smd("3", 1.1, -0.8, 1.2, 1.0, "XOUT"), Smd("4", -1.1, -0.8, 1.2, 1.0, "GND"),
    ])
    builder.footprint("C3", 164.0, 16.0, r0402("XIN", "GND"))
    builder.footprint("C4", 164.0, 11.0, r0402("XOUT", "GND"))
    builder.footprint("U2", 195.0, 13.0, [
        Smd(str(i + 1), -1.905 + 1.27 * i, 2.7, 0.6, 1.5, net)
        for i, net in enumerate(["QSPI_SS", "QSPI_SD1", "QSPI_SD2", "GND"])
    ] + [
        Smd(str(i + 5), 1.905 - 1.27 * i, -2.7, 0.6, 1.5, net)
        for i, net in enumerate(["QSPI_SD0", "QSPI_SCLK", "QSPI_SD3", "3V3"])
    ])
    builder.footprint("C5", 195.0, 19.0, r0402("3V3", "GND"))
    builder.footprint("C6", 175.0, 19.5, r0402("3V3", "GND"))
    builder.footprint("C7", 185.0, 19.5, r0402("3V3", "GND"))
    builder.footprint("C8", 186.0, 8.0, r0402("3V3", "GND"), rotation=90)
    builder.footprint("C9", 173.0, 9.0, r0402("1V1", "GND"), rotation=90)
    builder.footprint("C10", 187.5, 13.0, r0402("1V1", "GND"), rotation=90)
    return builder.board


RECIPES = {
    "keyboard-tkl": Recipe(keyboard_tkl, track=0.2, clearance=0.15),
    "placements": Recipe(placements),
    "crossing": Recipe(crossing),
    "detour": Recipe(detour),
    "ground-pour": Recipe(ground_pour),
    "inner-plane": Recipe(inner_plane),
    "isolation": Recipe(
        isolation,
        classes=(NetClass("HV", "HV", track=0.5, clearance=0.3),),
        custom_rules=ISOLATION_RULES,
    ),
    "matrix-3x4": Recipe(lambda: key_matrix(3, 4)),
    "matrix-6x17": Recipe(lambda: key_matrix(6, 17)),
}
