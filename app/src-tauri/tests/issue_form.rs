//! The app's field ids against `.github/ISSUE_TEMPLATE/bug.yml` (ADR-0003 §2.5).
//!
//! GitHub drops a prefilled query parameter that names no field, silently, after the reporter
//! has approved the preview. Nothing else in the suite sees a renamed id, so this test parses the
//! template with squelch's own form parser and pins every constant's presence, kind and required
//! flag, plus the reverse: the template declares no answerable field the app does not know.

use crank_desktop::report::fields::{
    CONFIRM, CURRENT_BEHAVIOR, DIAGNOSTICS, ENVIRONMENT, ERROR, REPRODUCTION,
};
use squelch::FieldKind;
use squelch::Form;
use squelch::form::github::IssueForm;
use std::collections::BTreeSet;

/// Baked in at compile time, so a moved template is a build error, and so the nix sandbox
/// (no `CARGO_MANIFEST_DIR` source tree at run time) sees the same bytes.
const TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../.github/ISSUE_TEMPLATE/bug.yml"
));

/// Whether a field is a textarea, or checkboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Textarea,
    Checkboxes,
}

/// Every id the app uses, with the kind and required flag ADR-0003 §2.5 gives it.
const CONTRACT: [(&str, Kind, bool); 6] = [
    (CURRENT_BEHAVIOR, Kind::Textarea, true),
    (REPRODUCTION, Kind::Textarea, true),
    (ERROR, Kind::Textarea, false),
    (ENVIRONMENT, Kind::Textarea, false),
    (DIAGNOSTICS, Kind::Textarea, false),
    (CONFIRM, Kind::Checkboxes, true),
];

fn form() -> Form {
    let parsed: IssueForm =
        serde_yaml_ng::from_str(TEMPLATE).expect("bug.yml is a GitHub issue form");
    parsed.into()
}

#[test]
fn every_app_id_is_declared_with_its_kind_and_required_flag() {
    let form = form();
    for (id, kind, required) in CONTRACT {
        let field = form
            .field(id)
            .unwrap_or_else(|| panic!("bug.yml declares no field `{id}`; GitHub would drop it"));
        let actual = match field.kind {
            FieldKind::Textarea => Kind::Textarea,
            FieldKind::Checkboxes { .. } => Kind::Checkboxes,
            ref other => panic!("`{id}` is {other:?} in bug.yml"),
        };
        assert_eq!(actual, kind, "`{id}` has the wrong kind in bug.yml");
        assert_eq!(
            field.required, required,
            "`{id}` has the wrong required flag in bug.yml"
        );
    }
}

#[test]
fn the_template_offers_no_answer_the_app_does_not_know() {
    let known: BTreeSet<&str> = CONTRACT.iter().map(|(id, ..)| *id).collect();
    let form = form();
    let declared: BTreeSet<&str> = form
        .fields
        .iter()
        .filter(|f| f.kind.is_answerable())
        .map(|f| f.id.as_str())
        .collect();
    assert_eq!(
        declared, known,
        "bug.yml and `report::fields` disagree about the field set"
    );
}

#[test]
fn the_confirmation_is_a_checkbox_no_url_can_reach() {
    // A dropdown or input would be prefillable, and the app would then attest for the user.
    let confirm = form().field(CONFIRM).cloned().expect("confirm is declared");
    assert!(matches!(confirm.kind, FieldKind::Checkboxes { .. }));
    assert!(!confirm.kind.is_machine_fillable());
}
