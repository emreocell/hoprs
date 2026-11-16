//! The settings tab's page in the center: transfers (workers per site, conflict policy,
//! keepalive), the terminal (font size, scrollback, theme) and the encrypted backup.

use std::fmt;

use hoprs_transfer::ConflictPolicy;
use hoprs_vault::{BackupImportReport, ExportReport, ImportMode};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, rule, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::settings::{Backup, BackupMsg, BackupPanel, BackupState, SettingsMsg};
use crate::app::{App, Message, PickTarget};
use crate::settings::{
    FONT_SIZE_MAX, FONT_SIZE_MIN, LanguageChoice, TerminalTheme, WORKERS_MAX, WORKERS_MIN,
};
use crate::ui::site_form::Password;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{self, busy, confirm_with, failure, field};

fn msg(m: SettingsMsg) -> Message {
    Message::Settings(m)
}

fn backup_msg(m: BackupMsg) -> Message {
    msg(SettingsMsg::Backup(m))
}

/// `pick_list` needs `Display`: the UI label of a policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PolicyChoice(ConflictPolicy);

impl fmt::Display for PolicyChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(policy_label(self.0))
    }
}

pub fn policy_label(policy: ConflictPolicy) -> &'static str {
    match policy {
        ConflictPolicy::Ask => strings::policy_ask(),
        ConflictPolicy::Overwrite => strings::policy_overwrite(),
        ConflictPolicy::Skip => strings::policy_skip(),
        ConflictPolicy::OverwriteIfNewer => strings::policy_overwrite_if_newer(),
        ConflictPolicy::Resume => strings::policy_resume(),
    }
}

/// Scrollback sizes the page offers (lines).
const SCROLLBACK_CHOICES: [usize; 7] = [1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000];

/// `pick_list` needs `Display`: a scrollback size, in lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Lines(usize);

impl fmt::Display for Lines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Keepalive intervals the page offers (seconds).
const KEEPALIVE_CHOICES: [u64; 7] = [10, 15, 30, 60, 120, 180, 300];

/// `pick_list` needs `Display`: a keepalive interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Secs(u64);

impl fmt::Display for Secs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&strings::seconds(self.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LanguageOption(LanguageChoice);

impl fmt::Display for LanguageOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.label())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ThemeChoice(TerminalTheme);

impl fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.label())
    }
}

const POLICIES: [PolicyChoice; 5] = [
    PolicyChoice(ConflictPolicy::Ask),
    PolicyChoice(ConflictPolicy::Overwrite),
    PolicyChoice(ConflictPolicy::OverwriteIfNewer),
    PolicyChoice(ConflictPolicy::Resume),
    PolicyChoice(ConflictPolicy::Skip),
];

fn muted(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_SM)
        .color(palette().muted)
        .into()
}

fn heading(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_XS)
        .font(theme::INTER_SEMIBOLD)
        .into()
}

fn hrule<'a>() -> Element<'a, Message> {
    rule::horizontal(theme::HAIRLINE)
        .style(theme::rule_1px)
        .into()
}

fn primary(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label).font(theme::INTER_SEMIBOLD))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::button_primary)
        .on_press_maybe(on_press)
        .into()
}

fn outlined(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    let enabled = on_press.is_some();
    widgets::outlined(
        button(text(label))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
            .on_press_maybe(on_press),
        enabled,
    )
}

fn plain(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::button_text)
        .on_press_maybe(on_press)
        .into()
}

fn input<'a>(
    value: &'a str,
    secure: bool,
    on_input: impl Fn(String) -> Message + 'a,
) -> iced::widget::TextInput<'a, Message> {
    let field = text_input("", value)
        .on_input(on_input)
        .secure(secure)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::search);
    if secure {
        field.size(theme::TEXT_SM)
    } else {
        // Paths are data: mono.
        field.font(theme::MONO).size(theme::TEXT_MONO)
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
    let settings = &app.settings;
    let workers = settings.workers_per_site;
    let stepper = row![
        outlined(
            strings::decrease(),
            (workers > WORKERS_MIN).then(|| msg(SettingsMsg::Workers(workers - 1)))
        ),
        container(
            text(workers.to_string())
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
        )
        .width(36)
        .align_x(Alignment::Center),
        outlined(
            strings::increase(),
            (workers < WORKERS_MAX).then(|| msg(SettingsMsg::Workers(workers + 1)))
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);

    let policy = pick_list(
        POLICIES,
        Some(PolicyChoice(settings.default_policy)),
        |choice| msg(SettingsMsg::Policy(choice.0)),
    )
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);

    // A value from the file that is not on the list is offered too.
    let mut intervals: Vec<Secs> = KEEPALIVE_CHOICES.into_iter().map(Secs).collect();
    if !KEEPALIVE_CHOICES.contains(&settings.keepalive_secs) {
        intervals.push(Secs(settings.keepalive_secs));
        intervals.sort();
    }
    let keepalive = pick_list(intervals, Some(Secs(settings.keepalive_secs)), |secs| {
        msg(SettingsMsg::Keepalive(secs.0))
    })
    .text_size(theme::TEXT_SM)
    .width(120)
    .style(theme::pick_list)
    .menu_style(theme::menu);

    let mut transfers = column![heading(strings::settings_transfers())].spacing(theme::SPACE_SM);
    if app.settings_load_error.is_some() {
        transfers = transfers.push(
            column![
                text(strings::settings_unreadable())
                    .size(theme::TEXT_SM)
                    .color(palette().error),
                failure(app.settings_load_error.as_ref()),
            ]
            .spacing(theme::SPACE_2XS),
        );
    }
    transfers = transfers
        .push(field(strings::settings_workers(), stepper.into(), None))
        .push(muted(strings::settings_workers_hint()))
        .push(field(strings::settings_policy(), policy.into(), None))
        .push(muted(strings::settings_policy_hint()))
        .push(field(strings::settings_keepalive(), keepalive.into(), None))
        .push(muted(strings::settings_keepalive_hint()));
    if let Some(error) = &app.settings_save_error {
        transfers = transfers.push(
            column![
                text(strings::settings_save_failed())
                    .size(theme::TEXT_SM)
                    .color(palette().error),
                failure(Some(error)),
            ]
            .spacing(theme::SPACE_2XS),
        );
    }

    let font_size = settings.terminal_font_size;
    let font_stepper = row![
        outlined(
            strings::decrease(),
            (font_size > FONT_SIZE_MIN).then(|| msg(SettingsMsg::TerminalFontSize(font_size - 1)))
        ),
        container(
            text(font_size.to_string())
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
        )
        .width(36)
        .align_x(Alignment::Center),
        outlined(
            strings::increase(),
            (font_size < FONT_SIZE_MAX).then(|| msg(SettingsMsg::TerminalFontSize(font_size + 1)))
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);
    let mut lines: Vec<Lines> = SCROLLBACK_CHOICES.iter().copied().map(Lines).collect();
    if !SCROLLBACK_CHOICES.contains(&settings.terminal_scrollback) {
        // A value typed into the file: shown as it is.
        lines.push(Lines(settings.terminal_scrollback));
        lines.sort();
    }
    let scrollback = pick_list(lines, Some(Lines(settings.terminal_scrollback)), |lines| {
        msg(SettingsMsg::TerminalScrollback(lines.0))
    })
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);
    let themes: Vec<ThemeChoice> = TerminalTheme::ALL.into_iter().map(ThemeChoice).collect();
    let term_theme = pick_list(
        themes,
        Some(ThemeChoice(settings.terminal_theme)),
        |choice| msg(SettingsMsg::TerminalTheme(choice.0)),
    )
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);
    let terminal = column![
        heading(strings::settings_terminal()),
        field(strings::settings_font_size(), font_stepper.into(), None),
        field(strings::settings_scrollback(), scrollback.into(), None),
        muted(strings::settings_scrollback_hint()),
        field(strings::settings_theme(), term_theme.into(), None),
    ]
    .spacing(theme::SPACE_SM);

    let editor_value = app
        .editor_input
        .as_deref()
        .unwrap_or(&settings.editor_command);
    let mut editor_field = row![
        input(editor_value, false, |command| {
            msg(SettingsMsg::EditorCommand(command))
        })
        .width(320)
    ]
    .spacing(theme::SPACE_SM)
    .align_y(Alignment::Center);
    // The setting stays a string; `system` reads as what it does.
    if crate::settings::is_system_editor(editor_value) {
        editor_field = editor_field.push(muted(strings::editor_system_default()));
    }
    let mut editing = column![
        heading(strings::settings_editing()),
        field(strings::editor_command(), editor_field.into(), None),
        muted(strings::editor_command_hint()),
    ]
    .spacing(theme::SPACE_SM);
    if app.editor_input.is_some() {
        editing = editing.push(
            text(strings::editor_command_empty())
                .size(theme::TEXT_SM)
                .color(palette().error),
        );
    }

    let ignore_value = app
        .compare_ignore_input
        .clone()
        .unwrap_or_else(|| settings.compare_ignore.join(", "));
    let ignore_default = crate::settings::compare_ignore_default().join(", ");
    let comparing = column![
        heading(strings::settings_compare()),
        field(
            strings::settings_compare_auto(),
            checkbox(settings.compare_auto)
                .on_toggle(|on| msg(SettingsMsg::CompareAuto(on)))
                .style(theme::checkbox)
                .into(),
            None
        ),
        field(
            strings::settings_compare_ignore(),
            text_input(&ignore_default, &ignore_value)
                .on_input(|typed| msg(SettingsMsg::CompareIgnore(typed)))
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                .style(theme::search)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .width(320)
                .into(),
            None
        ),
        muted(strings::settings_compare_ignore_hint()),
    ]
    .spacing(theme::SPACE_SM);

    let languages: Vec<LanguageOption> = LanguageChoice::ALL
        .into_iter()
        .map(LanguageOption)
        .collect();
    let language = pick_list(
        languages,
        Some(LanguageOption(settings.language)),
        |choice| msg(SettingsMsg::Language(choice.0)),
    )
    .text_size(theme::TEXT_SM)
    .width(240)
    .style(theme::pick_list)
    .menu_style(theme::menu);

    let content = column![
        text(strings::settings_title())
            .size(theme::TEXT_MD)
            .font(theme::INTER_SEMIBOLD),
        hrule(),
        field(strings::settings_language(), language.into(), None),
        hrule(),
        transfers,
        hrule(),
        terminal,
        hrule(),
        editing,
        hrule(),
        comparing,
        hrule(),
        backup_view(&app.backup, app.busy_phase),
    ]
    .spacing(theme::SPACE_MD)
    .max_width(640);

    scrollable(
        container(content)
            .width(Length::Fill)
            .padding(theme::SPACE_LG),
    )
    .height(Length::Fill)
    .into()
}

fn backup_view(backup: &Backup, phase: u32) -> Element<'_, Message> {
    let running = backup.running();
    let buttons = row![
        outlined(
            strings::export_backup(),
            (!running).then(|| backup_msg(BackupMsg::OpenExport))
        ),
        outlined(
            strings::restore_backup(),
            (!running).then(|| backup_msg(BackupMsg::OpenRestore))
        ),
    ]
    .spacing(theme::SPACE_XS);
    let mut col = column![
        heading(strings::backup()),
        muted(strings::backup_hint()),
        buttons
    ]
    .spacing(theme::SPACE_SM);

    // The running operation and the outcomes, above the panel (in view without
    // scrolling).
    match (&backup.export, &backup.restore) {
        (BackupState::Running, _) => col = col.push(busy(strings::exporting(), phase)),
        (_, BackupState::Running) => col = col.push(busy(strings::restoring(), phase)),
        _ => {}
    }
    if backup.panel == BackupPanel::Export {
        match &backup.export {
            BackupState::Done(report) => col = col.push(export_report(report)),
            BackupState::Failed(error) => {
                col = col.push(outcome_error(strings::export_failed(), error));
            }
            BackupState::Idle | BackupState::Running => {}
        }
    }
    if backup.panel == BackupPanel::Restore {
        match &backup.restore {
            BackupState::Done(report) => col = col.push(restore_report(report)),
            BackupState::Failed(error) => {
                col = col.push(outcome_error(strings::restore_failed(), error));
            }
            BackupState::Idle | BackupState::Running => {}
        }
    }
    match backup.panel {
        BackupPanel::Closed => {}
        BackupPanel::Export => col = col.push(export_panel(backup)),
        BackupPanel::Restore => col = col.push(restore_panel(backup)),
    }
    col.into()
}

fn panel<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .padding(theme::SPACE_SM)
        .style(theme::outline_box)
        .into()
}

fn path_row<'a>(
    value: &'a str,
    target: PickTarget,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    row![
        input(value, false, on_input),
        outlined(
            strings::choose_file(),
            Some(backup_msg(BackupMsg::Choose(target)))
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn field_error(backup: &Backup) -> Element<'_, Message> {
    match backup.field_error {
        Some(error) => text(error)
            .size(theme::TEXT_XS)
            .color(palette().error)
            .into(),
        None => space().into(),
    }
}

fn export_panel(backup: &Backup) -> Element<'_, Message> {
    panel(
        column![
            heading(strings::export_backup()),
            field(
                strings::backup_file(),
                path_row(&backup.export_path, PickTarget::ExportBackup, |v| {
                    backup_msg(BackupMsg::ExportPath(v))
                }),
                None,
            ),
            row![
                field(
                    strings::backup_password(),
                    input(&backup.export_password.0, true, |v| {
                        backup_msg(BackupMsg::ExportPassword(Password(v)))
                    })
                    .into(),
                    None,
                ),
                field(
                    strings::backup_password_confirm(),
                    input(&backup.export_confirm.0, true, |v| {
                        backup_msg(BackupMsg::ExportConfirm(Password(v)))
                    })
                    .on_submit(backup_msg(BackupMsg::Export))
                    .into(),
                    None,
                ),
            ]
            .spacing(theme::SPACE_SM),
            field_error(backup),
            export_actions(backup),
        ]
        .spacing(theme::SPACE_SM),
    )
}

/// Export / Cancel, or the inline question about an existing file.
fn export_actions(backup: &Backup) -> Element<'_, Message> {
    if backup.confirm_overwrite {
        return confirm_with(
            strings::export_overwrite_confirm(),
            strings::overwrite(),
            backup_msg(BackupMsg::ConfirmOverwrite),
            backup_msg(BackupMsg::CancelOverwrite),
        );
    }
    let idle = !backup.running() && !backup.export_checking;
    row![
        primary(
            strings::export(),
            idle.then(|| backup_msg(BackupMsg::Export))
        ),
        plain(strings::cancel(), Some(backup_msg(BackupMsg::Cancel))),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn restore_panel(backup: &Backup) -> Element<'_, Message> {
    let running = backup.running();
    let mode = |label, mode: ImportMode| {
        button(text(label))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
            .style(theme::segment(backup.restore_mode == mode))
            .on_press(backup_msg(BackupMsg::RestoreMode(mode)))
    };
    let mode_hint = match backup.restore_mode {
        ImportMode::Merge => strings::restore_merge_hint(),
        ImportMode::Replace => strings::restore_replace_hint(),
    };
    let actions: Element<'_, Message> = if backup.confirm_replace {
        confirm_with(
            strings::restore_replace_confirm(),
            strings::replace(),
            backup_msg(BackupMsg::ConfirmReplace),
            backup_msg(BackupMsg::CancelReplace),
        )
    } else {
        row![
            primary(
                strings::restore(),
                (!running).then(|| backup_msg(BackupMsg::Restore))
            ),
            plain(strings::cancel(), Some(backup_msg(BackupMsg::Cancel))),
        ]
        .spacing(theme::SPACE_XS)
        .align_y(Alignment::Center)
        .into()
    };
    panel(
        column![
            heading(strings::restore_backup()),
            field(
                strings::backup_file(),
                path_row(&backup.restore_path, PickTarget::RestoreBackup, |v| {
                    backup_msg(BackupMsg::RestorePath(v))
                }),
                None,
            ),
            field(
                strings::backup_password(),
                input(&backup.restore_password.0, true, |v| {
                    backup_msg(BackupMsg::RestorePassword(Password(v)))
                })
                .on_submit(backup_msg(BackupMsg::Restore))
                .into(),
                None,
            ),
            field(
                strings::restore_mode(),
                row![
                    mode(strings::restore_merge(), ImportMode::Merge),
                    mode(strings::restore_replace(), ImportMode::Replace),
                ]
                .spacing(theme::SPACE_XS)
                .into(),
                None,
            ),
            muted(mode_hint),
            field_error(backup),
            actions,
        ]
        .spacing(theme::SPACE_SM),
    )
}

fn outcome_error<'a>(title: &'a str, error: &'a hoprs_core::Error) -> Element<'a, Message> {
    column![
        text(title).size(theme::TEXT_SM).color(palette().error),
        failure(Some(error)),
    ]
    .spacing(theme::SPACE_2XS)
    .into()
}

/// A titled list of names (mono), skipped when empty.
fn names<'a>(
    col: iced::widget::Column<'a, Message>,
    title: &'a str,
    items: impl IntoIterator<Item = String>,
    error: bool,
) -> iced::widget::Column<'a, Message> {
    let items: Vec<String> = items.into_iter().collect();
    if items.is_empty() {
        return col;
    }
    let count = items.len();
    let color = if error {
        palette().error
    } else {
        palette().ink_2
    };
    let list = column(items.into_iter().map(|item| {
        text(item)
            .font(theme::MONO)
            .size(theme::TEXT_MONO)
            .color(color)
            .into()
    }))
    .spacing(theme::SPACE_3XS);
    col.push(
        column![
            text(strings::section_count(title, count))
                .size(theme::TEXT_XS)
                .font(theme::INTER_SEMIBOLD),
            list
        ]
        .spacing(theme::SPACE_2XS),
    )
}

fn export_report(report: &ExportReport) -> Element<'_, Message> {
    let mut col = column![text(strings::export_done(
        report.sites,
        report.workspaces,
        report.secrets
    ))]
    .spacing(theme::SPACE_SM);
    col = names(
        col,
        strings::backup_missing_secrets(),
        report.missing_secrets.iter().cloned(),
        false,
    );
    col = names(
        col,
        strings::backup_skipped_unreadable(),
        report
            .skipped_unreadable
            .iter()
            .map(|entry| strings::unreadable_name(entry.name.as_deref(), entry.file, entry.index)),
        false,
    );
    col.into()
}

fn restore_report(report: &BackupImportReport) -> Element<'_, Message> {
    let mut col = column![text(strings::restore_done(
        report.added_sites,
        report.added_workspaces,
        report.secrets_restored
    ))]
    .spacing(theme::SPACE_SM);
    col = names(
        col,
        strings::backup_secret_errors(),
        report.secret_errors.iter().cloned(),
        true,
    );
    col = names(
        col,
        strings::backup_skipped_sites(),
        report.skipped_sites.iter().cloned(),
        false,
    );
    col = names(
        col,
        strings::backup_skipped_workspaces(),
        report.skipped_workspaces.iter().cloned(),
        false,
    );
    col.into()
}
