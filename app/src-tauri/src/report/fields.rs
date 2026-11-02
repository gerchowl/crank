//! The field ids of `.github/ISSUE_TEMPLATE/bug.yml` (ADR-0003 §2.5).
//!
//! GitHub prefills a form field only when the URL's query parameter equals the field's `id`,
//! and drops any other parameter without a word. The ids are therefore a contract between the
//! app and the template, and `tests/issue_form.rs` fails when they drift apart.

/// What happened (textarea, required). Filled by the user.
pub const CURRENT_BEHAVIOR: &str = "current-behavior";
/// Steps to reproduce (textarea, required). Filled by the user.
pub const REPRODUCTION: &str = "reproduction";
/// The error code and message (textarea). Filled by the banner entry point.
pub const ERROR: &str = "error";
/// App version, build and platform (textarea). Filled from provenance.
pub const ENVIRONMENT: &str = "environment";
/// The redacted log tail (textarea, `render: text`). Pasted by the user; never in the URL.
pub const DIAGNOSTICS: &str = "diagnostics";
/// The reporter's confirmation (checkboxes, required). Never filled by the app: GitHub cannot
/// prefill checkboxes, and the app must not attest for the user.
pub const CONFIRM: &str = "confirm";
