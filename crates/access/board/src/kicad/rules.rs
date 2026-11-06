//! KiCad's custom design rules (`.kicad_dru`), as its documentation and its
//! DRC describe them: `(version 1)`, then `(rule NAME (layer …) (condition
//! "…") (constraint KIND (min …) (opt …) (max …)) (severity …))`.

use super::project::matches_wildcards;
use super::sexpr;
use crate::BoardError;
use crate::sexp::Sexp;
use traxide_contracts::{
    Condition, Constraint, CustomRule, Item, Layer, LayerId, Limits, Net, NetClass, NetClassId,
    NetId,
};

/// What the rules' names refer to on the board.
pub(crate) struct Names<'a> {
    pub classes: &'a [NetClass],
    pub nets: &'a [Net],
    pub layers: &'a [Layer],
    /// The references of the board's footprints.
    pub footprints: &'a [&'a str],
}

/// The rules of a `.kicad_dru` that traxide could read, and what it could not.
#[derive(Debug, Default)]
pub(crate) struct Read {
    pub rules: Vec<CustomRule>,
    pub warnings: Vec<String>,
}

/// The rules in `text`, in their order, one per constraint. Constraints traxide
/// does not route by are left out, and so are rules KiCad does not check. A
/// broken file does not stop routing: KiCad's DRC would ignore it all, but the
/// rules that can be read still keep the routes where the designer meant them,
/// so traxide leaves out only what it cannot read, and says so.
pub(crate) fn read(text: &str, names: &Names) -> Read {
    let uncommented: String = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(|line| {
            if line.trim_start().starts_with('#') {
                ""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut read = Read::default();
    // The file is a sequence of lists; one around them keeps the line numbers.
    let root = match sexpr::parse(&format!("(kicad_dru {uncommented}\n)")) {
        Ok(root) => root,
        Err(error) => {
            read.warnings.push(format!(
                "the custom rules cannot be read ({error}); routing without them"
            ));
            return read;
        }
    };
    let version = root.child("version").and_then(|v| v.atoms().next());
    if version != Some("1") {
        read.warnings.push(
            "the custom rules lack (version 1), so KiCad's DRC ignores them; traxide routes by them anyway"
                .to_owned(),
        );
    }
    for rule in root.children("rule") {
        let ignored = rule
            .child("severity")
            .and_then(|s| s.atoms().next())
            .is_some_and(|s| s == "ignore");
        if ignored {
            continue;
        }
        match read_rule(rule, names) {
            Ok(rules) => read.rules.extend(rules),
            Err(error) => {
                let name = rule.atoms().next().unwrap_or("without a name");
                read.warnings
                    .push(format!("custom rule {name} left out: {error}"));
            }
        }
    }
    read
}

/// One rule, as one custom rule per constraint traxide routes by.
fn read_rule(rule: &Sexp, names: &Names) -> Result<Vec<CustomRule>, BoardError> {
    let mut rules = Vec::new();
    let name = rule.arg_atom(0, "rule name")?;
    let mut condition = match rule.child("condition").and_then(|c| c.atoms().next()) {
        Some(expression) => condition(expression, names),
        None => Condition::Always,
    };
    if let Some(layer) = rule.child("layer").and_then(|l| l.atoms().next()) {
        condition = Condition::All(vec![on_layer(layer, names.layers), condition]);
    }
    for constraint in rule.children("constraint") {
        let Some(kind) = constraint.atoms().next().and_then(constraint_kind) else {
            continue;
        };
        rules.push(CustomRule {
            name: name.to_owned(),
            condition: condition.clone(),
            constraint: kind,
            limits: Limits {
                min: limit(constraint, "min")?,
                opt: limit(constraint, "opt")?,
                max: limit(constraint, "max")?,
            },
        });
    }
    Ok(rules)
}

fn constraint_kind(keyword: &str) -> Option<Constraint> {
    Some(match keyword {
        "clearance" => Constraint::Clearance,
        "track_width" => Constraint::TrackWidth,
        "via_diameter" => Constraint::ViaDiameter,
        "hole_size" => Constraint::HoleSize,
        "edge_clearance" => Constraint::EdgeClearance,
        "hole_clearance" => Constraint::HoleClearance,
        _ => return None,
    })
}

fn limit(constraint: &Sexp, keyword: &str) -> Result<Option<i32>, BoardError> {
    let Some(limit) = constraint.child(keyword) else {
        return Ok(None);
    };
    let text = limit.arg_atom(0, keyword)?;
    length(text).map(Some).ok_or_else(|| {
        limit.invalid(format!(
            "cannot read the length {text:?}: traxide reads a number in mm, mil or in"
        ))
    })
}

/// A length such as `0.2mm`, `"1 mm"` or `8mil`, in nanometres. KiCad's DRC
/// ignores the whole file over a length without a unit, so it is an error here too.
fn length(text: &str) -> Option<i32> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+')))
        .unwrap_or(text.len());
    let number: f64 = text[..split].parse().ok()?;
    let nanometres_per_unit = match text[split..].trim() {
        "mm" => 1e6,
        "mil" => 25_400.0,
        "in" => 25.4e6,
        _ => return None,
    };
    Some((number * nanometres_per_unit).round() as i32)
}

/// The rule's layer: `outer`, `inner` or a layer's name. A name that is not a
/// copper layer's might be another name for one, so the rule may apply.
fn on_layer(layer: &str, layers: &[Layer]) -> Condition {
    let last = layers.len().saturating_sub(1);
    let indices: Vec<usize> = match layer {
        "outer" if last == 0 => vec![0],
        "outer" => vec![0, last],
        "inner" => (1..last).collect(),
        name => match layers.iter().position(|l| l.name == name) {
            Some(index) => vec![index],
            None => return Condition::Unknown,
        },
    };
    Condition::OnLayers(indices.into_iter().map(LayerId::from_index).collect())
}

/// The condition `expression` means: comparisons of `NetClass` and `NetName`,
/// joined with `&&`, `||` and `!`. Any other test may hold or not.
fn condition(expression: &str, names: &Names) -> Condition {
    let Some(tokens) = tokens(expression) else {
        return Condition::Unknown;
    };
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        names,
    };
    match parser.any() {
        Some(condition) if parser.at == tokens.len() => condition,
        _ => Condition::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Text(String),
    Symbol(&'static str),
}

const SYMBOLS: [&str; 16] = [
    "==", "!=", "&&", "||", "<=", ">=", "!", "(", ")", ",", "<", ">", "+", "-", "*", "/",
];

/// `None` for text that is no expression.
fn tokens(expression: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut rest = expression;
    while let Some(c) = rest.chars().next() {
        if c.is_whitespace() {
            rest = &rest[c.len_utf8()..];
        } else if c == '\'' || c == '"' {
            let end = rest[1..].find(c)? + 1;
            tokens.push(Token::Text(rest[1..end].to_owned()));
            rest = &rest[end + 1..];
        } else if let Some(symbol) = SYMBOLS.iter().find(|s| rest.starts_with(**s)) {
            tokens.push(Token::Symbol(symbol));
            rest = &rest[symbol.len()..];
        } else if c.is_alphanumeric() || c == '_' || c == '.' {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                .unwrap_or(rest.len());
            tokens.push(Token::Word(rest[..end].to_owned()));
            rest = &rest[end..];
        } else {
            return None;
        }
    }
    Some(tokens)
}

struct Parser<'t> {
    tokens: &'t [Token],
    at: usize,
    names: &'t Names<'t>,
}

/// One side of a comparison.
enum Operand {
    Property(String),
    Text(String),
    /// A function that is a condition of its own, such as A.memberOfFootprint('U*').
    Condition(Condition),
    Other,
}

impl Parser<'_> {
    fn eat(&mut self, symbol: &str) -> bool {
        let found = matches!(self.tokens.get(self.at), Some(Token::Symbol(s)) if *s == symbol);
        self.at += usize::from(found);
        found
    }

    fn any(&mut self) -> Option<Condition> {
        let mut any = vec![self.all()?];
        while self.eat("||") {
            any.push(self.all()?);
        }
        Some(if any.len() == 1 {
            any.remove(0)
        } else {
            Condition::Any(any)
        })
    }

    fn all(&mut self) -> Option<Condition> {
        let mut all = vec![self.unary()?];
        while self.eat("&&") {
            all.push(self.unary()?);
        }
        Some(if all.len() == 1 {
            all.remove(0)
        } else {
            Condition::All(all)
        })
    }

    fn unary(&mut self) -> Option<Condition> {
        if self.eat("!") {
            return Some(Condition::Not(Box::new(self.unary()?)));
        }
        if self.eat("(") {
            let inner = self.any()?;
            return self.eat(")").then_some(inner);
        }
        let left = self.operand()?;
        let comparison = ["==", "!=", "<=", ">=", "<", ">"]
            .into_iter()
            .find(|op| self.eat(op));
        let Some(op) = comparison else {
            return Some(match left {
                Operand::Condition(condition) => condition,
                _ => Condition::Unknown,
            });
        };
        let right = self.operand()?;
        let equal = match (left, right) {
            (Operand::Property(property), Operand::Text(text))
            | (Operand::Text(text), Operand::Property(property)) => self.equals(&property, &text),
            _ => Condition::Unknown,
        };
        Some(match op {
            "==" => equal,
            "!=" => Condition::Not(Box::new(equal)),
            _ => Condition::Unknown,
        })
    }

    fn operand(&mut self) -> Option<Operand> {
        let token = self.tokens.get(self.at)?.clone();
        self.at += 1;
        match token {
            Token::Word(function) if self.eat("(") => {
                let arguments = self.at;
                // Any other function, such as A.inDiffPair('*'): its arguments are skipped.
                let mut depth = 1;
                while depth > 0 {
                    match self.tokens.get(self.at)? {
                        Token::Symbol("(") => depth += 1,
                        Token::Symbol(")") => depth -= 1,
                        _ => {}
                    }
                    self.at += 1;
                }
                Some(match &self.tokens[arguments..self.at - 1] {
                    [Token::Text(pattern)] => self.function(&function, pattern),
                    _ => Operand::Other,
                })
            }
            Token::Word(word) => Some(Operand::Property(word)),
            Token::Text(text) => Some(Operand::Text(text)),
            Token::Symbol(_) => None,
        }
    }

    /// `A.memberOfFootprint('LED*')`: the footprints whose reference the
    /// pattern matches. A pattern with a library name, `Lib:Footprint`, names
    /// footprints by what the board does not keep, so it may hold or not.
    fn function(&self, function: &str, pattern: &str) -> Operand {
        let item = match function.split_once('.') {
            Some(("A", "memberOfFootprint")) => Item::A,
            Some(("B", "memberOfFootprint")) => Item::B,
            _ => return Operand::Other,
        };
        if pattern.contains(':') {
            return Operand::Condition(Condition::Unknown);
        }
        let pattern = pattern.to_lowercase();
        let footprints = (self.names.footprints.iter())
            .filter(|reference| matches_wildcards(&pattern, &reference.to_lowercase()))
            .map(|&reference| reference.to_owned())
            .collect();
        Operand::Condition(Condition::MemberOfFootprint { item, footprints })
    }

    /// `property == pattern`, where KiCad matches the pattern's wildcards
    /// without regard to case.
    fn equals(&self, property: &str, pattern: &str) -> Condition {
        let pattern = pattern.to_lowercase();
        let matches = |name: &str| matches_wildcards(&pattern, &name.to_lowercase());
        let (item, property) = match property.split_once('.') {
            Some(("A", property)) => (Item::A, property),
            Some(("B", property)) => (Item::B, property),
            _ => return Condition::Unknown,
        };
        match property {
            "NetClass" => Condition::NetClassIn {
                item,
                classes: (self.names.classes.iter().enumerate())
                    .filter(|(_, class)| matches(&class.name))
                    .map(|(i, _)| NetClassId::from_index(i))
                    .collect(),
            },
            "NetName" => Condition::NetIn {
                item,
                nets: (matches("").then_some(None).into_iter())
                    .chain(
                        (self.names.nets.iter().enumerate())
                            .filter(|(_, net)| matches(&net.name))
                            .map(|(i, _)| Some(NetId::from_index(i))),
                    )
                    .collect(),
            },
            _ => Condition::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str) -> NetClass {
        NetClass {
            name: name.into(),
            track_width: 0,
            clearance: 0,
            via_kinds: vec![],
            pair: None,
        }
    }

    fn net(name: &str) -> Net {
        Net {
            name: name.into(),
            class: NetClassId(0),
            pads: vec![],
        }
    }

    fn layer(name: &str) -> Layer {
        Layer {
            name: name.into(),
            routable: true,
        }
    }

    fn rules(text: &str) -> Vec<CustomRule> {
        let classes = [class("Default"), class("HV"), class("USB")];
        let nets = [net("GND"), net("/HV+"), net("/HV-")];
        let layers = ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"].map(layer);
        let footprints = ["LED1", "LED2", "U1"];
        let names = Names {
            classes: &classes,
            nets: &nets,
            layers: &layers,
            footprints: &footprints,
        };
        read(&format!("(version 1)\n{text}"), &names).rules
    }

    #[test]
    fn member_of_footprint_names_the_footprints_its_pattern_matches() {
        let rule = only(
            r#"(rule "LED cutout" (constraint edge_clearance (min 0.2mm))
                 (condition "A.memberOfFootprint('LED*') || B.memberOfFootprint('led2')"))"#,
        );
        let member = |item, footprints: &[&str]| Condition::MemberOfFootprint {
            item,
            footprints: footprints.iter().map(|&f| f.to_owned()).collect(),
        };
        assert_eq!(
            rule.condition,
            Condition::Any(vec![
                member(Item::A, &["LED1", "LED2"]),
                member(Item::B, &["LED2"])
            ])
        );
    }

    #[test]
    fn member_of_footprint_by_library_name_may_hold_or_not() {
        let rule = only(
            r#"(rule r (constraint edge_clearance (min 0.2mm))
                 (condition "A.memberOfFootprint('LEDs:SK6812')"))"#,
        );
        assert_eq!(rule.condition, Condition::Unknown);
    }

    fn only(text: &str) -> CustomRule {
        let mut rules = rules(text);
        assert_eq!(rules.len(), 1, "{rules:?}");
        rules.remove(0)
    }

    fn when(condition: &str) -> Condition {
        only(&format!(
            "(rule r (constraint clearance (min 1mm)) (condition \"{condition}\"))"
        ))
        .condition
    }

    fn classes(item: Item, classes: &[u32]) -> Condition {
        Condition::NetClassIn {
            item,
            classes: classes.iter().copied().map(NetClassId).collect(),
        }
    }

    #[test]
    fn a_rule_states_its_constraint_and_limits_in_nanometres() {
        let rule = only(
            "(rule \"HV clearance\" (constraint clearance (min 1.5mm) (opt \"2 mm\") (max 0.1in)))",
        );
        assert_eq!(rule.name, "HV clearance");
        assert_eq!(rule.constraint, Constraint::Clearance);
        assert_eq!(rule.condition, Condition::Always);
        assert_eq!(
            rule.limits,
            Limits {
                min: Some(1_500_000),
                opt: Some(2_000_000),
                max: Some(2_540_000),
            }
        );
        assert_eq!(
            only("(rule r (constraint track_width (min 8mil)))")
                .limits
                .min,
            Some(203_200)
        );
    }

    #[test]
    fn a_rule_with_several_constraints_becomes_one_rule_each_in_order() {
        let read = rules(
            "(rule r (constraint track_width (min 0.3mm)) (constraint disallow via) \
             (constraint edge_clearance (min 1mm)) (constraint hole_size (max 0.4mm)))",
        );
        let kinds: Vec<Constraint> = read.iter().map(|r| r.constraint).collect();
        assert_eq!(
            kinds,
            [
                Constraint::TrackWidth,
                Constraint::EdgeClearance,
                Constraint::HoleSize
            ]
        );
    }

    #[test]
    fn rules_kicad_does_not_check_are_left_out() {
        assert!(rules("(rule r (severity ignore) (constraint clearance (min 1mm)))").is_empty());
    }

    #[test]
    fn comments_are_skipped() {
        assert_eq!(
            rules("# wider\n  # still a comment\n(rule r (constraint clearance (min 1mm)))").len(),
            1
        );
    }

    #[test]
    fn net_classes_match_by_wildcard_without_regard_to_case() {
        assert_eq!(when("A.NetClass == 'HV'"), classes(Item::A, &[1]));
        assert_eq!(when("B.NetClass == 'hv'"), classes(Item::B, &[1]));
        assert_eq!(when("'?SB' == A.NetClass"), classes(Item::A, &[2]));
        assert_eq!(when("A.NetClass == 'Power'"), classes(Item::A, &[]));
    }

    #[test]
    fn net_names_match_by_wildcard_and_copper_of_no_net_has_an_empty_name() {
        let nets = |nets: &[Option<u32>]| Condition::NetIn {
            item: Item::A,
            nets: nets.iter().map(|n| n.map(NetId)).collect(),
        };
        assert_eq!(when("A.NetName == '/HV*'"), nets(&[Some(1), Some(2)]));
        assert_eq!(when("A.NetName == 'gnd'"), nets(&[Some(0)]));
        assert_eq!(when("A.NetName == ''"), nets(&[None]));
    }

    #[test]
    fn conditions_combine_with_and_or_not_and_parentheses() {
        assert_eq!(
            when("A.NetClass == 'HV' && B.NetClass != 'HV' || !(A.NetClass == 'USB')"),
            Condition::Any(vec![
                Condition::All(vec![
                    classes(Item::A, &[1]),
                    Condition::Not(Box::new(classes(Item::B, &[1]))),
                ]),
                Condition::Not(Box::new(classes(Item::A, &[2]))),
            ])
        );
    }

    #[test]
    fn tests_traxide_cannot_tell_may_hold_or_not() {
        assert_eq!(when("A.Type == 'Via'"), Condition::Unknown);
        assert_eq!(
            when("A.inDiffPair('*') && A.NetClass == 'HV'"),
            Condition::All(vec![Condition::Unknown, classes(Item::A, &[1])])
        );
        assert_eq!(when("A.NetClass == 'HV' &&"), Condition::Unknown);
        assert_eq!(when("A.NetClass == 'HV' ; 1"), Condition::Unknown);
    }

    #[test]
    fn a_layer_limits_a_rule_to_copper_on_it() {
        let on = |layer: &str| {
            only(&format!(
                "(rule r (layer {layer}) (constraint clearance (min 1mm)))"
            ))
            .condition
        };
        let layers = |ids: &[u32]| {
            Condition::All(vec![
                Condition::OnLayers(ids.iter().copied().map(LayerId).collect()),
                Condition::Always,
            ])
        };
        assert_eq!(on("outer"), layers(&[0, 3]));
        assert_eq!(on("inner"), layers(&[1, 2]));
        assert_eq!(on("\"In2.Cu\""), layers(&[2]));
        assert_eq!(
            on("\"Top\""),
            Condition::All(vec![Condition::Unknown, Condition::Always])
        );
    }

    const NO_NAMES: Names = Names {
        classes: &[],
        nets: &[],
        layers: &[],
        footprints: &[],
    };

    fn names_and_warnings(text: &str) -> (Vec<String>, Vec<String>) {
        let read = read(text, &NO_NAMES);
        (
            read.rules.into_iter().map(|r| r.name).collect(),
            read.warnings,
        )
    }

    #[test]
    fn a_rule_with_a_length_traxide_cannot_read_is_left_out_with_a_warning() {
        let (names, warnings) = names_and_warnings(
            "(version 1)\n(rule r\n (constraint clearance (min 1)))\n(rule s (constraint clearance (min 1um)))\n(rule t (constraint clearance (min 1mm)))",
        );
        assert_eq!(names, ["t"]);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("custom rule r") && warnings[0].contains("line 3"));
        assert!(warnings[1].contains("custom rule s"));
    }

    #[test]
    fn rules_without_a_version_are_read_with_a_warning() {
        let (names, warnings) = names_and_warnings("(rule r (constraint clearance (min 1mm)))");
        assert_eq!(names, ["r"]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("(version 1)"));
    }

    #[test]
    fn rules_that_are_no_lists_at_all_are_left_out_with_a_warning() {
        let (names, warnings) = names_and_warnings("(version 1)\n(rule r (constraint");
        assert_eq!(names, Vec::<String>::new());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("custom rules"), "{warnings:?}");
    }
}
