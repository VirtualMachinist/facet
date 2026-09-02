use std::{borrow::Cow, collections::BTreeMap, path::PathBuf};

use gpui::Action;
use probe_core::{Environment, ImportDiagnostic, ImportDiagnosticSeverity, RequestKey};
use probe_opencollection::ItemKind;
use probe_postman::{ImportedPostmanCollection, PostmanImportPreview};
use probe_yaak::{ImportedYaakWorkspace, YaakImportPreview, YaakWorkspaceSummary};

use crate::{components, session::SessionState};

pub(crate) const IMPORT_DIAGNOSTIC_GROUP_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ImportSource {
    Postman,
    Yaak,
}

#[derive(Clone, Debug)]
pub(crate) struct EnvironmentManagerDialog {
    pub(crate) original_name: String,
    pub(crate) draft: Environment,
}

impl EnvironmentManagerDialog {
    pub(crate) fn new(environment: &Environment) -> Self {
        Self {
            original_name: environment.name.clone(),
            draft: environment.clone(),
        }
    }
}

pub(crate) enum PendingClose {
    Tab(RequestKey),
    OtherTabs {
        keep: RequestKey,
    },
    Workspace,
    Window,
    Quit,
    Open {
        path: PathBuf,
        restored_state: Option<SessionState>,
    },
    Create {
        path: PathBuf,
    },
    Import(ImportSource),
}

pub(crate) enum ApplicationDialog {
    About,
    Unsaved {
        keys: Vec<RequestKey>,
        pending: PendingClose,
    },
    Delete {
        kind: ItemKind,
        selector: String,
        name: String,
        detail: String,
    },
    DeleteEnvironment {
        name: String,
        detail: String,
    },
    UnsavedEnvironment,
    FilesystemConflict {
        path: Option<PathBuf>,
        detail: String,
    },
    SelectYaakWorkspace {
        preview: YaakImportPreview,
        workspaces: Vec<YaakWorkspaceSummary>,
    },
    SelectCollectionFile {
        candidates: Vec<PathBuf>,
    },
    ConfirmPartialYaakImport {
        preview: YaakImportPreview,
        workspace_id: String,
        detail: String,
    },
    ConfirmPartialPostmanImport {
        preview: Box<PostmanImportPreview>,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DesktopMenu {
    File,
    Edit,
    View,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DesktopSubmenu {
    Import,
    EditorLayout,
}

pub(crate) struct DesktopMenuDefinition {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) width: f32,
    pub(crate) items: Vec<DesktopMenuItem>,
}

pub(crate) enum DesktopMenuItem {
    Action(&'static str, Box<dyn Action>, Option<bool>),
    Submenu(&'static str, DesktopSubmenu, DesktopMenuDefinition),
    Separator,
}

impl DesktopMenuItem {
    pub(crate) fn action(label: &'static str, action: impl Action + 'static) -> Self {
        Self::Action(label, Box::new(action), None)
    }

    pub(crate) fn checked_action(
        label: &'static str,
        checked: bool,
        action: impl Action + 'static,
    ) -> Self {
        Self::Action(label, Box::new(action), Some(checked))
    }

    pub(crate) fn submenu(
        label: &'static str,
        state: DesktopSubmenu,
        popup: DesktopMenuDefinition,
    ) -> Self {
        Self::Submenu(label, state, popup)
    }
}

impl ApplicationDialog {
    pub(crate) fn title(&self) -> Cow<'_, str> {
        match self {
            Self::About => Cow::Borrowed("Probe"),
            Self::Unsaved { keys, .. } => {
                let noun = if keys.len() == 1 {
                    "request"
                } else {
                    "requests"
                };
                Cow::Owned(format!("Save changes to {} {noun}?", keys.len()))
            }
            Self::UnsavedEnvironment => Cow::Borrowed("Save changes to this environment?"),
            Self::Delete { name, .. } | Self::DeleteEnvironment { name, .. } => {
                Cow::Owned(format!("Delete “{name}”?"))
            }
            Self::FilesystemConflict { .. } => {
                Cow::Borrowed("Collection changes conflict with local edits")
            }
            Self::SelectYaakWorkspace { .. } => Cow::Borrowed("Select a Yaak workspace"),
            Self::SelectCollectionFile { .. } => Cow::Borrowed("Select a collection"),
            Self::ConfirmPartialYaakImport { .. } => {
                Cow::Borrowed("Some Yaak data cannot be represented")
            }
            Self::ConfirmPartialPostmanImport { .. } => {
                Cow::Borrowed("Some Postman data cannot be represented")
            }
        }
    }

    pub(crate) fn description(&self) -> &str {
        match self {
            Self::About => concat!(
                "Version ",
                env!("CARGO_PKG_VERSION"),
                "\n\nA fast, native, local-first API client."
            ),
            Self::Unsaved { .. } | Self::UnsavedEnvironment => {
                "Unsaved changes will be lost if you discard them."
            }
            Self::Delete { detail, .. }
            | Self::DeleteEnvironment { detail, .. }
            | Self::FilesystemConflict { detail, .. }
            | Self::ConfirmPartialYaakImport { detail, .. }
            | Self::ConfirmPartialPostmanImport { detail, .. } => detail,
            Self::SelectYaakWorkspace { .. } => {
                "Choose the workspace to import into a new Probe collection."
            }
            Self::SelectCollectionFile { .. } => {
                "This folder contains multiple bundled OpenCollection files. Choose one to open."
            }
        }
    }

    pub(crate) const fn width(&self) -> f32 {
        match self {
            Self::SelectYaakWorkspace { .. }
            | Self::SelectCollectionFile { .. }
            | Self::ConfirmPartialYaakImport { .. }
            | Self::ConfirmPartialPostmanImport { .. } => components::WIDE_DIALOG_WIDTH,
            _ => components::COMPACT_DIALOG_WIDTH,
        }
    }

    pub(crate) const fn action_specs(&self) -> Option<&'static [DialogActionSpec]> {
        match self {
            Self::About => Some(ABOUT_DIALOG_ACTIONS),
            Self::Unsaved { .. } | Self::UnsavedEnvironment => Some(UNSAVED_DIALOG_ACTIONS),
            Self::Delete { .. } | Self::DeleteEnvironment { .. } => Some(DELETE_DIALOG_ACTIONS),
            Self::FilesystemConflict { .. } => Some(FILESYSTEM_CONFLICT_DIALOG_ACTIONS),
            Self::SelectYaakWorkspace { .. } => None,
            Self::SelectCollectionFile { .. } => None,
            Self::ConfirmPartialYaakImport { .. } | Self::ConfirmPartialPostmanImport { .. } => {
                Some(PARTIAL_IMPORT_DIALOG_ACTIONS)
            }
        }
    }

    pub(crate) fn primary_action(&self) -> Option<ApplicationDialogAction> {
        self.action_specs()?.iter().find_map(|spec| {
            (spec.style == components::DialogActionStyle::Primary).then_some(spec.action)
        })
    }

    pub(crate) fn destructive_action(&self) -> Option<ApplicationDialogAction> {
        self.action_specs()?.iter().find_map(|spec| {
            (spec.style == components::DialogActionStyle::Destructive).then_some(spec.action)
        })
    }
}

pub(crate) enum YaakConversionResult {
    Imported(ImportedYaakWorkspace),
    NeedsPartialConfirmation {
        preview: YaakImportPreview,
        workspace_id: String,
        detail: String,
    },
    Failed(String),
}

pub(crate) enum PostmanConversionResult {
    Imported(Box<ImportedPostmanCollection>),
    NeedsPartialConfirmation {
        preview: Box<PostmanImportPreview>,
        detail: String,
    },
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApplicationDialogAction {
    Cancel,
    Save,
    Discard,
    Delete,
    UseDisk,
    KeepLocal,
    SelectWorkspace(usize),
    SelectCollectionFile(usize),
    ImportSupportedData,
}

#[derive(Clone, Copy)]
pub(crate) struct DialogActionSpec {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) style: components::DialogActionStyle,
    pub(crate) action: ApplicationDialogAction,
}

impl DialogActionSpec {
    const fn new(
        id: &'static str,
        label: &'static str,
        style: components::DialogActionStyle,
        action: ApplicationDialogAction,
    ) -> Self {
        Self {
            id,
            label,
            style,
            action,
        }
    }
}

pub(crate) const CANCEL_DIALOG_ACTION: DialogActionSpec = DialogActionSpec::new(
    "application-dialog-cancel",
    "Cancel",
    components::DialogActionStyle::Secondary,
    ApplicationDialogAction::Cancel,
);
const ABOUT_DIALOG_ACTIONS: &[DialogActionSpec] = &[DialogActionSpec::new(
    "application-dialog-done",
    "Done",
    components::DialogActionStyle::Primary,
    ApplicationDialogAction::Cancel,
)];
const UNSAVED_DIALOG_ACTIONS: &[DialogActionSpec] = &[
    CANCEL_DIALOG_ACTION,
    DialogActionSpec::new(
        "application-dialog-discard",
        "Discard",
        components::DialogActionStyle::Destructive,
        ApplicationDialogAction::Discard,
    ),
    DialogActionSpec::new(
        "application-dialog-save",
        "Save",
        components::DialogActionStyle::Primary,
        ApplicationDialogAction::Save,
    ),
];
const DELETE_DIALOG_ACTIONS: &[DialogActionSpec] = &[
    CANCEL_DIALOG_ACTION,
    DialogActionSpec::new(
        "application-dialog-delete",
        "Delete",
        components::DialogActionStyle::Destructive,
        ApplicationDialogAction::Delete,
    ),
];
const FILESYSTEM_CONFLICT_DIALOG_ACTIONS: &[DialogActionSpec] = &[
    DialogActionSpec::new(
        "application-dialog-keep-local",
        "Keep Local",
        components::DialogActionStyle::Secondary,
        ApplicationDialogAction::KeepLocal,
    ),
    DialogActionSpec::new(
        "application-dialog-use-disk",
        "Use Disk",
        components::DialogActionStyle::Destructive,
        ApplicationDialogAction::UseDisk,
    ),
];
const PARTIAL_IMPORT_DIALOG_ACTIONS: &[DialogActionSpec] = &[
    CANCEL_DIALOG_ACTION,
    DialogActionSpec::new(
        "application-dialog-import-supported",
        "Import Supported Data",
        components::DialogActionStyle::Primary,
        ApplicationDialogAction::ImportSupportedData,
    ),
];

pub(crate) fn suggested_collection_filename(name: &str) -> String {
    let stem = name
        .trim()
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\' | ':' | '\0') {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    let stem = stem.trim_matches([' ', '.', '-']);
    format!("{}.yml", if stem.is_empty() { "Imported" } else { stem })
}

pub(crate) fn format_import_diagnostics(diagnostics: &[ImportDiagnostic]) -> String {
    if diagnostics.is_empty() {
        return "No compatibility issues found.".to_owned();
    }

    let lossy_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == ImportDiagnosticSeverity::Lossy)
        .count();
    let warning_count = diagnostics.len() - lossy_count;
    let mut groups = BTreeMap::new();
    for diagnostic in diagnostics {
        *groups
            .entry((
                diagnostic.severity,
                diagnostic.resource_type.as_str(),
                diagnostic.field.as_deref(),
                diagnostic.code,
                diagnostic.message.as_str(),
            ))
            .or_insert(0_usize) += 1;
    }

    let mut lines = vec![format!(
        "Found {} compatibility issue(s): {lossy_count} lossy, {warning_count} warning(s).",
        diagnostics.len()
    )];
    lines.push(String::new());
    for ((severity, resource_type, field, _, message), count) in
        groups.iter().take(IMPORT_DIAGNOSTIC_GROUP_LIMIT)
    {
        let resource = field
            .map(|field| format!("{resource_type}.{field}"))
            .unwrap_or_else(|| (*resource_type).to_owned());
        lines.push(format!(
            "• {count} {} — {resource}: {message}",
            severity.as_str()
        ));
    }
    if groups.len() > IMPORT_DIAGNOSTIC_GROUP_LIMIT {
        let hidden_group_count = groups.len() - IMPORT_DIAGNOSTIC_GROUP_LIMIT;
        let hidden_issue_count = groups
            .values()
            .skip(IMPORT_DIAGNOSTIC_GROUP_LIMIT)
            .sum::<usize>();
        lines.push(format!(
            "• {hidden_issue_count} more issue(s) across {hidden_group_count} additional type(s)"
        ));
    }
    if lossy_count > 0 {
        lines.push(String::new());
        lines.push(
            "Import Supported Data will omit or change the lossy fields listed above.".to_owned(),
        );
    }
    lines.join("\n")
}
