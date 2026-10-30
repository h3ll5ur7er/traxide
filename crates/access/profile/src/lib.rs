//! Profile Access: where routing profiles come from (volatility V13 of
//! docs/Architecture/Volatilities.md). Profiles are built in, and a
//! `traxide.toml` can change them or add its own.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use traxide_contracts::Profile;

const BUILT_IN: &str = include_str!("default.toml");
const BASE_PROFILE: &str = "default";

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Syntax {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("no profile named {name}; there are {}", known.join(", "))]
    Unknown { name: String, known: Vec<String> },
    #[error("profile {profile}: {field} must be positive")]
    NotPositive {
        profile: String,
        field: &'static str,
    },
    #[error("profile {profile}: {field} must be at least 1")]
    BelowOne {
        profile: String,
        field: &'static str,
    },
    #[error("profile {profile}: {field} must be zero or more")]
    Negative {
        profile: String,
        field: &'static str,
    },
}

/// The profile called `name`. Profiles in `file` override fields of the built-in
/// ones; a profile that only exists there starts from the default profile.
pub fn load_profile(name: &str, file: Option<&Path>) -> Result<Profile, ProfileError> {
    let mut profiles = parse(BUILT_IN, Path::new("built-in profiles"))?;
    if let Some(path) = file {
        let text = std::fs::read_to_string(path).map_err(|source| ProfileError::Io {
            path: path.into(),
            source,
        })?;
        for (key, overrides) in parse(&text, path)? {
            let base = profiles
                .get(&key)
                .or_else(|| profiles.get(BASE_PROFILE))
                .cloned()
                .unwrap_or_default();
            profiles.insert(key, base.overridden_by(overrides));
        }
    }
    let fields = profiles.get(name).ok_or_else(|| ProfileError::Unknown {
        name: name.into(),
        known: profiles.keys().cloned().collect(),
    })?;
    fields.to_profile(name)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    #[serde(default)]
    profiles: BTreeMap<String, Fields>,
}

/// One profile as written: millimetres, every field optional so a file can
/// change just one of them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    grid_pitch: Option<f64>,
    via_cost: Option<f64>,
    bend_cost: Option<f64>,
    detour_margin: Option<f64>,
    rip_up_cost: Option<f64>,
    max_rip_ups: Option<u32>,
    history_cost: Option<f64>,
    heuristic_weight: Option<f64>,
    global_routing: Option<bool>,
    batch_size: Option<u32>,
    against_layer_direction: Option<f64>,
}

impl Fields {
    fn overridden_by(self, other: Self) -> Self {
        Self {
            grid_pitch: other.grid_pitch.or(self.grid_pitch),
            via_cost: other.via_cost.or(self.via_cost),
            bend_cost: other.bend_cost.or(self.bend_cost),
            detour_margin: other.detour_margin.or(self.detour_margin),
            rip_up_cost: other.rip_up_cost.or(self.rip_up_cost),
            max_rip_ups: other.max_rip_ups.or(self.max_rip_ups),
            history_cost: other.history_cost.or(self.history_cost),
            heuristic_weight: other.heuristic_weight.or(self.heuristic_weight),
            global_routing: other.global_routing.or(self.global_routing),
            batch_size: other.batch_size.or(self.batch_size),
            against_layer_direction: other
                .against_layer_direction
                .or(self.against_layer_direction),
        }
    }

    fn to_profile(&self, name: &str) -> Result<Profile, ProfileError> {
        let positive_nm = |value: Option<f64>, field: &'static str| -> Result<i64, ProfileError> {
            let nm = (value.unwrap_or(0.0) * 1_000_000.0).round();
            if nm >= 1.0 {
                Ok(nm as i64)
            } else {
                Err(ProfileError::NotPositive {
                    profile: name.into(),
                    field,
                })
            }
        };
        let length = |value, field| {
            positive_nm(value, field).map(|nm| i32::try_from(nm).unwrap_or(i32::MAX))
        };
        Ok(Profile {
            name: name.into(),
            grid_pitch: length(self.grid_pitch, "grid_pitch")?,
            via_cost: positive_nm(self.via_cost, "via_cost")?,
            bend_cost: positive_nm(self.bend_cost, "bend_cost")?,
            detour_margin: length(self.detour_margin, "detour_margin")?,
            rip_up_cost: positive_nm(self.rip_up_cost, "rip_up_cost")?,
            // Zero is allowed: it turns rip-up off.
            max_rip_ups: self.max_rip_ups.unwrap_or(0),
            // Zero is allowed: it turns history off.
            history_cost: match self.history_cost.unwrap_or(0.0) {
                0.0 => 0,
                _ => positive_nm(self.history_cost, "history_cost")?,
            },
            heuristic_weight: match self.heuristic_weight.unwrap_or(1.0) {
                weight if weight >= 1.0 && weight.is_finite() => weight,
                _ => {
                    return Err(ProfileError::BelowOne {
                        profile: name.into(),
                        field: "heuristic_weight",
                    });
                }
            },
            global_routing: self.global_routing.unwrap_or(false),
            batch_size: match self.batch_size.unwrap_or(1) {
                0 => {
                    return Err(ProfileError::BelowOne {
                        profile: name.into(),
                        field: "batch_size",
                    });
                }
                size => size,
            },
            // Zero is allowed: it turns layer directions off.
            against_layer_direction: match self.against_layer_direction.unwrap_or(0.0) {
                share if share >= 0.0 && share.is_finite() => share,
                _ => {
                    return Err(ProfileError::Negative {
                        profile: name.into(),
                        field: "against_layer_direction",
                    });
                }
            },
        })
    }
}

fn parse(text: &str, path: &Path) -> Result<BTreeMap<String, Fields>, ProfileError> {
    toml::from_str::<ProfileFile>(text)
        .map(|file| file.profiles)
        .map_err(|source| ProfileError::Syntax {
            path: path.into(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_with(text: &str) -> tempfile::TempPath {
        let path = tempfile::NamedTempFile::new()
            .expect("temp file")
            .into_temp_path();
        std::fs::write(&path, text).expect("writable");
        path
    }

    #[test]
    fn the_default_profile_is_built_in() {
        let profile = load_profile("default", None).expect("built in");
        assert_eq!(profile.grid_pitch, 100_000);
        assert_eq!(profile.via_cost, 2_000_000);
    }

    #[test]
    fn a_search_cannot_be_weighted_below_the_cheapest_path() {
        let file = file_with("[profiles.default]\nheuristic_weight = 0.9\n");
        assert!(matches!(
            load_profile("default", Some(&file)),
            Err(ProfileError::BelowOne { .. })
        ));
    }

    #[test]
    fn layer_directions_can_be_turned_off_but_not_made_negative() {
        let off = file_with("[profiles.default]\nagainst_layer_direction = 0\n");
        let profile = load_profile("default", Some(&off)).expect("valid");
        assert_eq!(profile.against_layer_direction, 0.0);
        let negative = file_with("[profiles.default]\nagainst_layer_direction = -0.5\n");
        assert!(matches!(
            load_profile("default", Some(&negative)),
            Err(ProfileError::Negative {
                field: "against_layer_direction",
                ..
            })
        ));
    }

    #[test]
    fn a_batch_holds_at_least_one_connection() {
        let file = file_with("[profiles.default]\nbatch_size = 0\n");
        assert!(matches!(
            load_profile("default", Some(&file)),
            Err(ProfileError::BelowOne {
                field: "batch_size",
                ..
            })
        ));
    }

    #[test]
    fn history_can_be_turned_off_but_not_made_negative() {
        let off = file_with("[profiles.default]\nhistory_cost = 0\n");
        assert_eq!(
            load_profile("default", Some(&off))
                .expect("valid")
                .history_cost,
            0
        );
        let negative = file_with("[profiles.default]\nhistory_cost = -1\n");
        assert!(matches!(
            load_profile("default", Some(&negative)),
            Err(ProfileError::NotPositive {
                field: "history_cost",
                ..
            })
        ));
    }

    #[test]
    fn rip_up_can_be_turned_off() {
        let file = file_with("[profiles.default]\nmax_rip_ups = 0\n");
        assert_eq!(
            load_profile("default", Some(&file))
                .expect("valid")
                .max_rip_ups,
            0
        );
    }

    #[test]
    fn a_file_changes_single_fields_of_a_built_in_profile() {
        let file = file_with("[profiles.default]\ngrid_pitch = 0.05\n");
        let profile = load_profile("default", Some(&file)).expect("valid");
        assert_eq!(profile.grid_pitch, 50_000);
        assert_eq!(profile.via_cost, 2_000_000);
    }

    #[test]
    fn a_file_adds_profiles_that_start_from_the_default() {
        let file = file_with("[profiles.cheap-vias]\nvia_cost = 0.5\n");
        let profile = load_profile("cheap-vias", Some(&file)).expect("valid");
        assert_eq!(
            (profile.name.as_str(), profile.via_cost, profile.grid_pitch),
            ("cheap-vias", 500_000, 100_000)
        );
    }

    #[test]
    fn unknown_profiles_are_reported_with_the_known_ones() {
        let error = load_profile("turbo", None).unwrap_err();
        assert_eq!(
            error.to_string(),
            "no profile named turbo; there are default"
        );
    }

    #[test]
    fn misspelled_fields_are_errors() {
        let file = file_with("[profiles.default]\ngrid_pich = 0.05\n");
        assert!(matches!(
            load_profile("default", Some(&file)),
            Err(ProfileError::Syntax { .. })
        ));
    }

    #[test]
    fn lengths_must_be_positive() {
        let file = file_with("[profiles.default]\ngrid_pitch = 0\n");
        assert!(matches!(
            load_profile("default", Some(&file)),
            Err(ProfileError::NotPositive {
                field: "grid_pitch",
                ..
            })
        ));
    }
}
