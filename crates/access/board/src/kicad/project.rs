use crate::BoardError;
use serde_json::Value;
use std::collections::HashMap;

/// The rules of a KiCad project (`.kicad_pro`) that routing follows: its net
/// classes, which nets they hold, and the board's minimums.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Project {
    /// The default class first.
    pub classes: Vec<ClassRules>,
    pub edge_clearance: i32,
    /// How far copper of other nets stays from a hole.
    pub hole_clearance: i32,
    /// How far apart holes stay.
    pub hole_to_hole: i32,
    min_clearance: i32,
    /// The narrowest a track may be, whatever its class.
    pub min_track_width: i32,
    assignments: HashMap<String, String>,
    patterns: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClassRules {
    pub name: String,
    pub track_width: i32,
    pub clearance: i32,
    pub via_diameter: i32,
    pub via_drill: i32,
}

/// The class of a net as a running KiCad states it, with the class's values
/// where KiCad states them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LiveClass {
    pub net: String,
    pub class: String,
    pub values: Option<ClassValues>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClassValues {
    pub track_width: i32,
    pub clearance: i32,
    /// The via's diameter and drill.
    pub via: Option<(i32, i32)>,
}

/// What KiCad uses where the project says nothing, as a new project has it.
const DEFAULT_CLASS: &str = "Default";
const DEFAULT_CLEARANCE: f64 = 0.2;
const DEFAULT_TRACK_WIDTH: f64 = 0.2;
const DEFAULT_VIA_DIAMETER: f64 = 0.6;
const DEFAULT_VIA_DRILL: f64 = 0.3;
const DEFAULT_EDGE_CLEARANCE: f64 = 0.5;
const DEFAULT_HOLE_CLEARANCE: f64 = 0.25;
/// What a new KiCad 10 project states as the narrowest track.
const DEFAULT_MIN_TRACK_WIDTH: f64 = 0.2;
/// KiCad's default minimum between holes, which JLCPCB asks too.
const DEFAULT_HOLE_TO_HOLE: f64 = 0.25;

pub(crate) fn nanometres(millimetres: f64) -> i32 {
    (millimetres * 1e6).round() as i32
}

impl Project {
    /// The rules of the project in `text`; KiCad's defaults without one.
    pub fn read(text: Option<&str>) -> Result<Self, BoardError> {
        let json: Value = match text {
            Some(text) => serde_json::from_str(text)
                .map_err(|error| BoardError::Project(error.to_string()))?,
            None => Value::Null,
        };
        let rules = &json["board"]["design_settings"]["rules"];
        let minimum = |key: &str, default: f64| rules[key].as_f64().unwrap_or(default);
        let min_clearance = nanometres(minimum("min_clearance", 0.0));
        let min_track_width = nanometres(minimum("min_track_width", DEFAULT_MIN_TRACK_WIDTH));
        let settings = &json["net_settings"];
        let listed = settings["classes"].as_array().cloned().unwrap_or_default();
        let default = listed
            .iter()
            .find(|class| class["name"] == DEFAULT_CLASS)
            .cloned()
            .unwrap_or(Value::Null);
        // A class leaves out what it takes from the default class.
        let value = |class: &Value, key: &str, fallback: f64| {
            class[key]
                .as_f64()
                .or_else(|| default[key].as_f64())
                .unwrap_or(fallback)
        };
        let rules_of = |name: &str, class: &Value| ClassRules {
            name: name.to_owned(),
            track_width: nanometres(value(class, "track_width", DEFAULT_TRACK_WIDTH))
                .max(min_track_width),
            clearance: nanometres(value(class, "clearance", DEFAULT_CLEARANCE)).max(min_clearance),
            via_diameter: nanometres(value(class, "via_diameter", DEFAULT_VIA_DIAMETER)),
            via_drill: nanometres(value(class, "via_drill", DEFAULT_VIA_DRILL)),
        };
        let mut classes = vec![rules_of(DEFAULT_CLASS, &default)];
        for class in &listed {
            match class["name"].as_str() {
                Some(name) if name != DEFAULT_CLASS => classes.push(rules_of(name, class)),
                _ => {}
            }
        }
        // KiCad 9 and later may give a net several classes, as a list.
        let assignments = settings["netclass_assignments"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(net, class)| {
                let class = class
                    .as_str()
                    .or_else(|| class.as_array()?.first()?.as_str())?;
                Some((net.clone(), class.to_owned()))
            })
            .collect();
        let patterns = settings["netclass_patterns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                Some((
                    p["pattern"].as_str()?.to_owned(),
                    p["netclass"].as_str()?.to_owned(),
                ))
            })
            .collect();
        Ok(Self {
            classes,
            edge_clearance: nanometres(minimum(
                "min_copper_edge_clearance",
                DEFAULT_EDGE_CLEARANCE,
            )),
            hole_clearance: nanometres(minimum("min_hole_clearance", DEFAULT_HOLE_CLEARANCE)),
            hole_to_hole: nanometres(minimum("min_hole_to_hole", DEFAULT_HOLE_TO_HOLE)),
            min_clearance,
            min_track_width,
            assignments,
            patterns,
        })
    }

    /// Takes the classes of nets, and the values of those classes, from
    /// `live`, as the running KiCad states them, over what the project file
    /// says; the board's minimums still raise them.
    pub fn follow(&mut self, live: &[LiveClass]) {
        self.assignments.clear();
        self.patterns.clear();
        for LiveClass { net, class, values } in live {
            let index = match self.classes.iter().position(|c| &c.name == class) {
                Some(index) => index,
                None => {
                    self.classes.push(ClassRules {
                        name: class.clone(),
                        ..self.classes[0].clone()
                    });
                    self.classes.len() - 1
                }
            };
            if let Some(values) = values {
                let rules = &mut self.classes[index];
                rules.track_width = values.track_width.max(self.min_track_width);
                rules.clearance = values.clearance.max(self.min_clearance);
                if let Some((diameter, drill)) = values.via {
                    (rules.via_diameter, rules.via_drill) = (diameter, drill);
                }
            }
            self.assignments.insert(net.clone(), class.clone());
        }
    }

    /// The index of the class of `net`: assigned by name, else by the first
    /// pattern it matches, else the default class.
    pub fn class_of(&self, net: &str) -> usize {
        let class = self.assignments.get(net).or_else(|| {
            self.patterns
                .iter()
                .find(|(pattern, _)| matches_wildcards(pattern, net))
                .map(|(_, class)| class)
        });
        class
            .and_then(|class| self.classes.iter().position(|c| &c.name == class))
            .unwrap_or(0)
    }
}

/// Whether `text` matches `pattern`, where `*` stands for any text and `?` for
/// any one character.
pub(crate) fn matches_wildcards(pattern: &str, text: &str) -> bool {
    let (pattern, text): (Vec<char>, Vec<char>) =
        (pattern.chars().collect(), text.chars().collect());
    // The pattern position after the last `*`, and the text position it matched up to.
    let (mut p, mut t, mut star) = (0, 0, None);
    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p + 1, t));
                p += 1;
            }
            Some(&c) if c == '?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((after, matched)) => {
                    p = after;
                    t = matched + 1;
                    star = Some((after, matched + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_project_kicads_defaults_apply() {
        let project = Project::read(None).expect("defaults");
        assert_eq!(
            project.classes,
            [ClassRules {
                name: "Default".into(),
                track_width: 200_000,
                clearance: 200_000,
                via_diameter: 600_000,
                via_drill: 300_000,
            }]
        );
        assert_eq!(project.edge_clearance, 500_000);
        assert_eq!(project.hole_clearance, 250_000);
    }

    const PROJECT: &str = r#"{
        "board": {"design_settings": {"rules": {"min_clearance": 0.3, "min_track_width": 0.15, "min_copper_edge_clearance": 0.3, "min_hole_clearance": 0.4}}},
        "net_settings": {
            "classes": [
                {"name": "Power", "track_width": 0.5},
                {"name": "Default", "clearance": 0.1, "track_width": 0.1, "via_diameter": 0.8, "via_drill": 0.4}
            ],
            "netclass_assignments": {"VBUS": ["Power", "USB"], "GND": "Power"},
            "netclass_patterns": [{"netclass": "Power", "pattern": "+*V?"}]
        }
    }"#;

    #[test]
    fn a_class_takes_what_it_leaves_out_from_the_default_class() {
        let project = Project::read(Some(PROJECT)).expect("valid");
        let power = &project.classes[1];
        assert_eq!(power.name, "Power");
        assert_eq!(power.track_width, 500_000);
        assert_eq!(power.via_diameter, 800_000);
    }

    #[test]
    fn the_boards_minimums_raise_every_class() {
        let project = Project::read(Some(PROJECT)).expect("valid");
        let default = &project.classes[0];
        assert_eq!((default.clearance, default.track_width), (300_000, 150_000));
        assert_eq!(project.edge_clearance, 300_000);
        assert_eq!(project.hole_clearance, 400_000);
        // Kept on its own too, as the limit verification judges widths by.
        assert_eq!(project.min_track_width, 150_000);
    }

    #[test]
    fn nets_join_classes_by_name_or_by_pattern() {
        let project = Project::read(Some(PROJECT)).expect("valid");
        assert_eq!(project.class_of("VBUS"), 1);
        assert_eq!(project.class_of("GND"), 1);
        assert_eq!(project.class_of("+3V3"), 1);
        assert_eq!(project.class_of("+3V"), 0);
        assert_eq!(project.class_of("SDA"), 0);
    }

    #[test]
    fn a_running_kicad_overrides_the_classes_of_the_project_file() {
        let mut project = Project::read(Some(PROJECT)).expect("valid");
        project.follow(&[
            LiveClass {
                net: "SDA".into(),
                class: "Power".into(),
                values: Some(ClassValues {
                    track_width: 400_000,
                    clearance: 100_000,
                    via: Some((900_000, 450_000)),
                }),
            },
            LiveClass {
                net: "VBUS".into(),
                class: "Default".into(),
                values: None,
            },
            LiveClass {
                net: "HV1".into(),
                class: "HV".into(),
                values: None,
            },
        ]);
        let power = &project.classes[1];
        assert_eq!(
            (
                power.track_width,
                power.clearance,
                power.via_diameter,
                power.via_drill
            ),
            (400_000, 300_000, 900_000, 450_000),
            "raised to the board's minimum clearance"
        );
        assert_eq!(project.class_of("SDA"), 1);
        assert_eq!(
            project.class_of("VBUS"),
            0,
            "assigned by KiCad, not by the file"
        );
        assert_eq!(
            project.class_of("+3V3"),
            0,
            "KiCad has resolved the patterns"
        );
        assert_eq!(project.class_of("HV1"), 2);
        assert_eq!(
            project.classes[2],
            ClassRules {
                name: "HV".into(),
                ..project.classes[0].clone()
            },
            "a class the file does not know starts from the default class"
        );
    }

    #[test]
    fn wildcards_match_any_text_or_any_one_character() {
        assert!(matches_wildcards("*", ""));
        assert!(matches_wildcards("a*c", "abbbc"));
        assert!(matches_wildcards("a?c", "abc"));
        assert!(!matches_wildcards("a?c", "abbc"));
        assert!(matches_wildcards("*b*", "abc"));
        assert!(!matches_wildcards("a*d", "abc"));
    }
}
