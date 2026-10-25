//! The board open in a running KiCad, through KiCad's IPC API (KiCad 10).
//!
//! KiCad hands over the board as the text of its file, which the board-file
//! reader reads, so that both give the same design; net classes come from KiCad
//! too, as the editor holds them. KiCad 10 serves no other rules, so the board's
//! minimums and custom rules come from the project's files.

mod client;
mod items;
mod messages;
mod nng;

pub(crate) use client::Environment;

use super::board::{self, Board};
use super::project::{ClassValues, LiveClass, Project};
use crate::{Beside, BoardError, RUNNING_KICAD, Stored};
use client::Kicad;
use messages::board::commands::{
    GetNetClassForNets, GetNets, NetClassForNetsResponse, NetsResponse, RefillZones,
};
use messages::board::types::Net;
use messages::common::commands::{
    BeginCommit, BeginCommitResponse, CommitAction, CreateItems, CreateItemsResponse, EndCommit,
    EndCommitResponse, ItemStatusCode, SaveDocumentToString, SavedDocumentResponse,
};
use messages::common::project::{NetClass, NetClassType};
use messages::common::types::document_specifier::Identifier;
use messages::common::types::{Distance, DocumentSpecifier, ItemHeader, ItemRequestStatus};
use std::path::PathBuf;
use traxide_contracts::{Design, Routes};

/// What KiCad's undo menu calls the routes: *Edit → Undo traxide: route*.
const COMMIT_MESSAGE: &str = "traxide: route";

/// The design of the board open in KiCad's PCB editor.
pub(crate) fn load_design(environment: &Environment) -> Result<Design, BoardError> {
    let mut kicad = Kicad::connect(environment)?;
    let document = kicad.open_board()?;
    Ok(read(&mut kicad, &document, environment)?.design)
}

/// Adds `routes`, routed on `design`, to the board open in KiCad as one commit,
/// then refills the board's copper zones around them; refuses if the board is
/// no longer the one `design` was read from.
///
/// The fill is an undo step of its own after the routes': refilling inside the
/// commit would fill around the board as it was before it. So on a board with
/// copper zones the first undo takes back the fill and the second the routes.
pub(crate) fn store_routes(
    environment: &Environment,
    design: &Design,
    routes: &Routes,
) -> Result<Stored, BoardError> {
    let mut kicad = Kicad::connect(environment)?;
    let document = kicad.open_board()?;
    let board = read(&mut kicad, &document, environment)?;
    if board.design != *design {
        return Err(BoardError::Changed {
            path: PathBuf::from(RUNNING_KICAD),
        });
    }
    let items = items::of(&board, routes)?;
    if items.is_empty() {
        return Ok(Stored::default());
    }
    let commit: BeginCommitResponse = kicad.call(&BeginCommit::default())?;
    let added = create(&mut kicad, &document, items);
    let ended = kicad.call::<_, EndCommitResponse>(&EndCommit {
        id: commit.id,
        action: if added.is_ok() {
            CommitAction::CmaCommit
        } else {
            CommitAction::CmaDrop
        }
        .into(),
        message: COMMIT_MESSAGE.into(),
        header: None,
    });
    added?;
    ended?;
    if board.has_copper_zones {
        refill_zones(&mut kicad, &document)?;
    }
    Ok(Stored {
        zones_refilled: board.has_copper_zones,
        zones_to_refill: false,
    })
}

fn refill_zones(kicad: &mut Kicad, document: &DocumentSpecifier) -> Result<(), BoardError> {
    kicad
        .call::<_, ()>(&RefillZones {
            board: Some(document.clone()),
            zones: Vec::new(),
        })
        .map_err(|error| {
            BoardError::Kicad(format!(
                "the routes are stored, but the zones were not refilled: {error}"
            ))
        })
}

fn create(
    kicad: &mut Kicad,
    document: &DocumentSpecifier,
    items: Vec<messages::Any>,
) -> Result<(), BoardError> {
    let created: CreateItemsResponse = kicad.call(&CreateItems {
        header: Some(ItemHeader {
            document: Some(document.clone()),
            ..ItemHeader::default()
        }),
        items,
        container: None,
    })?;
    if created.status != i32::from(ItemRequestStatus::IrsOk) {
        return Err(BoardError::Kicad(format!(
            "the board refused new items ({})",
            created.status
        )));
    }
    match created
        .created_items
        .iter()
        .filter_map(|item| item.status.as_ref())
        .find(|status| status.code != i32::from(ItemStatusCode::IscOk))
    {
        Some(refused) => Err(BoardError::Kicad(format!(
            "KiCad refused a track or via: {}",
            refused.error_message
        ))),
        None => Ok(()),
    }
}

fn read(
    kicad: &mut Kicad,
    document: &DocumentSpecifier,
    environment: &Environment,
) -> Result<Board, BoardError> {
    let saved: SavedDocumentResponse = kicad.call(&SaveDocumentToString {
        document: Some(document.clone()),
    })?;
    let nets: NetsResponse = kicad.call(&GetNets {
        board: Some(document.clone()),
        netclass_filter: Vec::new(),
    })?;
    let classes: NetClassForNetsResponse = kicad.call(&GetNetClassForNets {
        net: nets.nets.clone(),
    })?;
    let rule_files = rule_files(document, environment)?;
    let mut project = Project::read(rule_files.project.as_deref())?;
    project.follow(&live_classes(&nets.nets, &classes));
    board::read_with(&saved.contents, project, rule_files.custom_rules.as_deref())
}

/// The project's rule files: in the folder KiCad passes to plugins, else in
/// the project's folder as KiCad names it, named after the project.
fn rule_files(
    document: &DocumentSpecifier,
    environment: &Environment,
) -> Result<Beside, BoardError> {
    let project = document.project.clone().unwrap_or_default();
    let folder = environment
        .project_folder
        .clone()
        .or_else(|| (!project.path.is_empty()).then(|| PathBuf::from(&project.path)));
    let board_filename = match &document.identifier {
        Some(Identifier::BoardFilename(board)) => Some(board),
        _ => None,
    };
    let name = match (project.name.as_str(), board_filename) {
        ("", Some(board)) => board.strip_suffix(".kicad_pcb").unwrap_or(board),
        (name, _) => name,
    };
    match folder {
        Some(folder) if !name.is_empty() => Beside::of(&folder.join(format!("{name}.kicad_pcb"))),
        _ => Ok(Beside::default()),
    }
}

/// Each net's class in the order KiCad lists the nets. A net of several
/// classes gets an implicit class that combines them; traxide takes the first,
/// as it does from the project file, with that class's own values.
fn live_classes(nets: &[Net], classes: &NetClassForNetsResponse) -> Vec<LiveClass> {
    nets.iter()
        .filter(|net| !net.name.is_empty())
        .filter_map(|net| {
            let class = classes.classes.get(&net.name)?;
            Some(if class.r#type == i32::from(NetClassType::NctImplicit) {
                LiveClass {
                    net: net.name.clone(),
                    class: class.constituents.first().unwrap_or(&class.name).clone(),
                    values: None,
                }
            } else {
                LiveClass {
                    net: net.name.clone(),
                    class: class.name.clone(),
                    values: class_values(class),
                }
            })
        })
        .collect()
}

fn class_values(class: &NetClass) -> Option<ClassValues> {
    let board = class.board.as_ref()?;
    let via = board.via_stack.as_ref().and_then(|stack| {
        let drill = stack.drill.as_ref()?.diameter?.x_nm;
        let diameter = stack.copper_layers.first()?.size?.x_nm;
        Some((nanometres(diameter), nanometres(drill)))
    });
    Some(ClassValues {
        track_width: board.track_width.map(length)?,
        clearance: board.clearance.map(length)?,
        via,
    })
}

fn length(distance: Distance) -> i32 {
    nanometres(distance.value_nm)
}

fn nanometres(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

/// What the end-to-end test does to the running KiCad as a designer would.
#[cfg(feature = "testing")]
pub(crate) mod testing {
    use super::messages::common::commands::{
        RunAction, RunActionResponse, RunActionStatus, SaveDocument,
    };
    use super::*;

    /// Saves the board open in KiCad to its file, as *File → Save* does.
    pub fn save_board(environment: &Environment) -> Result<(), BoardError> {
        let mut kicad = Kicad::connect(environment)?;
        let document = kicad.open_board()?;
        kicad
            .call::<_, ()>(&SaveDocument {
                document: Some(document),
            })
            .map(drop)
    }

    /// Undoes the last change to the board, as *Edit → Undo* does.
    pub fn undo(environment: &Environment) -> Result<(), BoardError> {
        run_action(environment, "common.Interactive.undo")
    }

    /// Runs one of the PCB editor's actions by its name, as its menu, its
    /// toolbar or a hotkey would.
    pub fn run_action(environment: &Environment, action: &str) -> Result<(), BoardError> {
        let mut kicad = Kicad::connect(environment)?;
        // KiCad calls RunAction a prototyping aid rather than an API; it is the
        // only way to press a button from outside.
        let ran: RunActionResponse = kicad.call(&RunAction {
            action: action.into(),
        })?;
        if ran.status == i32::from(RunActionStatus::RasOk) {
            Ok(())
        } else {
            Err(BoardError::Kicad(format!(
                "KiCad did not run {action} ({})",
                ran.status
            )))
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::messages::board::types::{DrillProperties, PadStack, PadStackLayer};
    use super::messages::common::commands::{
        GetOpenDocumentsResponse, ItemCreationResult, ItemStatus,
    };
    use super::messages::common::project::NetClassBoardSettings;
    use super::messages::common::types::{DocumentType, Kiid, ProjectSpecifier, Vector2};
    use super::messages::common::{ApiRequest, ApiResponse, ApiResponseStatus, ApiStatusCode};
    use super::messages::{Any, Packed};
    use super::*;
    use prost::{Message, Name};
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use traxide_contracts::geometry::Point;
    use traxide_contracts::{LayerId, NetId, Track, Via, ViaKindId};

    const BOARD: &str = "matrix-3x4";

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kicad10")
    }

    fn board_text() -> String {
        std::fs::read_to_string(fixtures().join(format!("{BOARD}.kicad_pcb"))).expect("a fixture")
    }

    fn file_design() -> Design {
        crate::load_design(&fixtures().join(format!("{BOARD}.kicad_pcb"))).expect("a valid board")
    }

    /// What the fake KiCad holds and how it behaves.
    #[derive(Debug, Default)]
    struct State {
        board: String,
        /// The class KiCad states for each net, `Default` where none is named.
        classes: Vec<(String, NetClass)>,
        refuse_items: bool,
        /// The requests received, by message name; commits with their action.
        received: Vec<String>,
    }

    fn default_class() -> NetClass {
        let (width, clearance, via, drill) = (200_000, 200_000, 600_000, 300_000);
        NetClass {
            name: "Default".into(),
            board: Some(NetClassBoardSettings {
                clearance: Some(Distance {
                    value_nm: clearance,
                }),
                track_width: Some(Distance { value_nm: width }),
                via_stack: Some(PadStack {
                    drill: Some(DrillProperties {
                        diameter: Some(Vector2 {
                            x_nm: drill,
                            y_nm: drill,
                        }),
                        ..Default::default()
                    }),
                    copper_layers: vec![PadStackLayer {
                        size: Some(Vector2 {
                            x_nm: via,
                            y_nm: via,
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..NetClassBoardSettings::default()
            }),
            r#type: NetClassType::NctExplicit.into(),
            ..NetClass::default()
        }
    }

    fn ok<M: Message + Name>(message: &M) -> Vec<u8> {
        ApiResponse {
            header: None,
            status: Some(ApiResponseStatus {
                status: ApiStatusCode::AsOk.into(),
                error_message: String::new(),
            }),
            message: Some(Any::of(message)),
        }
        .encode_to_vec()
    }

    fn answer(state: &mut State, request: &[u8]) -> Vec<u8> {
        let request = ApiRequest::decode(request).expect("a request");
        let message = request.message.expect("a message");
        let name = message
            .type_url
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_owned();
        state.received.push(name.clone());
        match name.as_str() {
            "GetOpenDocuments" => ok(&GetOpenDocumentsResponse {
                documents: vec![DocumentSpecifier {
                    r#type: DocumentType::DoctypePcb.into(),
                    identifier: Some(Identifier::BoardFilename(format!("{BOARD}.kicad_pcb"))),
                    project: Some(ProjectSpecifier {
                        name: BOARD.into(),
                        path: fixtures().display().to_string(),
                    }),
                }],
            }),
            "SaveDocumentToString" => ok(&SavedDocumentResponse {
                contents: state.board.clone(),
                document: None,
            }),
            "GetNets" => {
                let design = board::read(&state.board, None, None)
                    .expect("a valid board")
                    .design;
                ok(&NetsResponse {
                    nets: design
                        .nets
                        .iter()
                        .map(|n| Net {
                            name: n.name.clone(),
                            code: None,
                        })
                        .collect(),
                })
            }
            "GetNetClassForNets" => {
                let asked: GetNetClassForNets = message.to().expect("the request");
                let class_of = |net: &str| {
                    state
                        .classes
                        .iter()
                        .find(|(n, _)| n == net)
                        .map_or_else(default_class, |(_, class)| class.clone())
                };
                ok(&NetClassForNetsResponse {
                    classes: asked
                        .net
                        .iter()
                        .map(|net| (net.name.clone(), class_of(&net.name)))
                        .collect(),
                })
            }
            "BeginCommit" => ok(&BeginCommitResponse {
                id: Some(Kiid {
                    value: "commit".into(),
                }),
            }),
            "CreateItems" => {
                let asked: CreateItems = message.to().expect("the request");
                let code = if state.refuse_items {
                    ItemStatusCode::IscInvalidData
                } else {
                    ItemStatusCode::IscOk
                };
                ok(&CreateItemsResponse {
                    header: None,
                    status: ItemRequestStatus::IrsOk.into(),
                    created_items: asked
                        .items
                        .iter()
                        .map(|_| ItemCreationResult {
                            status: Some(ItemStatus {
                                code: code.into(),
                                error_message: "off the board".into(),
                            }),
                            item: None,
                        })
                        .collect(),
                })
            }
            "EndCommit" => {
                let asked: EndCommit = message.to().expect("the request");
                let action = if asked.action == i32::from(CommitAction::CmaCommit) {
                    "push"
                } else {
                    "drop"
                };
                *state.received.last_mut().expect("just received") =
                    format!("EndCommit {action} {}", asked.message);
                ok(&EndCommitResponse {})
            }
            "RefillZones" => ok(&()),
            other => panic!("the fake KiCad does not know {other}"),
        }
    }

    /// A KiCad with the fixture board open, as `change` leaves it.
    fn fake_kicad(
        change: impl FnOnce(&mut State),
    ) -> (tempfile::TempDir, Environment, Arc<Mutex<State>>) {
        let mut state = State {
            board: board_text(),
            ..State::default()
        };
        change(&mut state);
        let state = Arc::new(Mutex::new(state));
        let folder = tempfile::tempdir().expect("a temporary folder");
        let socket = folder.path().join("api.sock");
        let served = Arc::clone(&state);
        nng::serve(
            UnixListener::bind(&socket).expect("bound"),
            move |request| answer(&mut served.lock().expect("not poisoned"), request),
        );
        let environment = Environment {
            socket,
            token: String::new(),
            project_folder: None,
        };
        (folder, environment, state)
    }

    fn with_a_copper_zone(board: &str) -> String {
        let end = board.rfind(')').expect("a board");
        format!(
            "{}  (zone (net \"ROW0\") (layer \"B.Cu\") (polygon (pts (xy 0 0) (xy 50 0) (xy 50 50))))\n)\n",
            &board[..end]
        )
    }

    fn some_routes() -> Routes {
        Routes {
            tracks: vec![Track {
                net: NetId::from_index(0),
                layer: LayerId::from_index(0),
                width: 200_000,
                path: vec![
                    Point::new(1_000_000, 1_000_000),
                    Point::new(2_000_000, 1_000_000),
                ],
            }],
            vias: vec![Via {
                net: NetId::from_index(0),
                kind: ViaKindId::from_index(0),
                position: Point::new(2_000_000, 1_000_000),
            }],
        }
    }

    fn received(state: &Arc<Mutex<State>>) -> Vec<String> {
        state.lock().expect("not poisoned").received.clone()
    }

    #[test]
    fn reads_the_same_design_from_kicad_as_from_the_board_file() {
        let (_folder, environment, _) = fake_kicad(|_| {});
        assert_eq!(load_design(&environment).expect("read"), file_design());
    }

    #[test]
    fn net_classes_come_from_kicad_rather_than_the_project_file() {
        let (_folder, environment, _) = fake_kicad(|state| {
            let mut power = default_class();
            power.name = "Power".into();
            if let Some(board) = power.board.as_mut() {
                board.track_width = Some(Distance { value_nm: 500_000 });
            }
            state.classes.push(("ROW0".into(), power));
        });
        let design = load_design(&environment).expect("read");
        let row = design
            .nets
            .iter()
            .find(|n| n.name == "ROW0")
            .expect("a net");
        let class = &design.rules.classes[row.class.index()];
        assert_eq!((class.name.as_str(), class.track_width), ("Power", 500_000));
    }

    #[test]
    fn a_net_of_several_classes_takes_the_first() {
        let (_folder, environment, _) = fake_kicad(|state| {
            let combined = NetClass {
                name: "HV,Default".into(),
                r#type: NetClassType::NctImplicit.into(),
                constituents: vec!["HV".into(), "Default".into()],
                ..default_class()
            };
            state.classes.push(("ROW0".into(), combined));
        });
        let design = load_design(&environment).expect("read");
        let row = design
            .nets
            .iter()
            .find(|n| n.name == "ROW0")
            .expect("a net");
        assert_eq!(design.rules.classes[row.class.index()].name, "HV");
    }

    #[test]
    fn stores_routes_as_one_commit() {
        let (_folder, environment, state) = fake_kicad(|_| {});
        let stored = store_routes(&environment, &file_design(), &some_routes()).expect("stored");
        assert!(
            !stored.zones_refilled,
            "a board without zones has none to refill"
        );
        let received = received(&state);
        let commit = received
            .iter()
            .position(|r| r == "BeginCommit")
            .expect("a commit");
        assert_eq!(
            received[commit..],
            [
                "BeginCommit",
                "CreateItems",
                "EndCommit push traxide: route"
            ]
        );
    }

    #[test]
    fn refills_the_zones_after_the_routes_commit() {
        let zoned = with_a_copper_zone(&board_text());
        let copy = tempfile::tempdir().expect("a temporary folder");
        let project = format!("{BOARD}.kicad_pro");
        std::fs::copy(fixtures().join(&project), copy.path().join(&project)).expect("copied");
        let board = copy.path().join(format!("{BOARD}.kicad_pcb"));
        std::fs::write(&board, &zoned).expect("written");
        let design = crate::load_design(&board).expect("a valid board");
        let (_folder, environment, state) = fake_kicad(|state| state.board = zoned);
        let stored = store_routes(&environment, &design, &some_routes()).expect("stored");
        assert!(stored.zones_refilled);
        let received = received(&state);
        assert_eq!(
            received[received.len() - 2..],
            ["EndCommit push traxide: route", "RefillZones"]
        );
    }

    #[test]
    fn drops_the_commit_when_kicad_refuses_an_item() {
        let (_folder, environment, state) = fake_kicad(|state| state.refuse_items = true);
        let error =
            store_routes(&environment, &file_design(), &some_routes()).expect_err("refused");
        assert!(error.to_string().contains("off the board"), "{error}");
        let received = received(&state);
        assert_eq!(
            received.last().map(String::as_str),
            Some("EndCommit drop traxide: route")
        );
    }

    #[test]
    fn refuses_to_store_routes_on_a_board_changed_since_it_was_read() {
        let design = file_design();
        let (_folder, environment, state) = fake_kicad(|state| {
            state.board = state.board.replacen("(net \"ROW0\")", "(net \"ROW9\")", 1);
        });
        let error = store_routes(&environment, &design, &some_routes()).expect_err("changed");
        assert!(matches!(error, BoardError::Changed { .. }), "{error}");
        assert!(!received(&state).iter().any(|r| r == "BeginCommit"));
    }

    #[test]
    fn stores_nothing_when_there_are_no_routes() {
        let (_folder, environment, state) = fake_kicad(|_| {});
        store_routes(&environment, &file_design(), &Routes::default()).expect("stored");
        assert!(!received(&state).iter().any(|r| r == "BeginCommit"));
    }

    #[test]
    fn finds_the_project_files_in_the_folder_kicad_passes_to_plugins() {
        let (_folder, mut environment, _) = fake_kicad(|_| {});
        let elsewhere = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(
            elsewhere.path().join(format!("{BOARD}.kicad_pro")),
            r#"{"board": {"design_settings": {"rules": {"min_copper_edge_clearance": 1.5}}}}"#,
        )
        .expect("written");
        environment.project_folder = Some(elsewhere.path().into());
        let design = load_design(&environment).expect("read");
        assert_eq!(design.rules.edge_clearance, 1_500_000);
    }
}
