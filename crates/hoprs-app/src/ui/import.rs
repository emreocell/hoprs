//! "Import from FileZilla" in the center: the file, the folders question, the report.

use hoprs_import::ImportReport;
use iced::widget::{button, column, container, row, rule, scrollable, space, text, text_input};
use iced::{Alignment, Element, Length, Padding};

use crate::app::import::{ImportMsg, ImportStep, KeepSource, SourceDelete};
use crate::app::{App, Message};
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets::{busy, confirm, failure, field};

/// One part of the report: its title, how many, and its lines.
#[derive(Debug, PartialEq)]
pub struct Section<'a> {
    pub title: &'static str,
    pub count: usize,
    pub lines: Vec<Line<'a>>,
}

/// A site (or workspace) with its notes or the reason it was skipped.
#[derive(Debug, PartialEq)]
pub struct Line<'a> {
    pub name: &'a str,
    pub notes: Vec<&'a str>,
}

impl<'a> Line<'a> {
    fn plain(name: &'a str) -> Self {
        Self {
            name,
            notes: Vec::new(),
        }
    }
}

/// The report, in its five sections, each with its count (empty ones included).
pub fn report_sections(report: &ImportReport) -> Vec<Section<'_>> {
    fn section<'a>(title: &'static str, lines: Vec<Line<'a>>) -> Section<'a> {
        Section {
            title,
            count: lines.len(),
            lines,
        }
    }
    fn plain(names: &[String]) -> Vec<Line<'_>> {
        names.iter().map(|n| Line::plain(n)).collect()
    }
    vec![
        section(
            strings::import_imported(),
            report
                .imported
                .iter()
                .map(|site| Line {
                    name: &site.name,
                    notes: site.notes.iter().map(String::as_str).collect(),
                })
                .collect(),
        ),
        section(
            strings::import_skipped(),
            report
                .skipped
                .iter()
                .map(|site| Line {
                    name: &site.name,
                    notes: vec![site.reason.as_str()],
                })
                .collect(),
        ),
        section(
            strings::import_workspaces_created(),
            plain(&report.workspaces_created),
        ),
        section(
            strings::import_workspaces_not_created(),
            plain(&report.skipped_workspaces),
        ),
        section(strings::import_failures(), plain(&report.failures)),
    ]
}

fn msg(m: ImportMsg) -> Message {
    Message::Import(m)
}

fn primary(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label).font(theme::INTER_SEMIBOLD))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::button_primary)
        .on_press_maybe(on_press)
        .into()
}

fn secondary(label: &str, on_press: Option<Message>) -> Element<'_, Message> {
    button(text(label))
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::button_text)
        .on_press_maybe(on_press)
        .into()
}

fn muted(label: &str) -> Element<'_, Message> {
    text(label)
        .size(theme::TEXT_SM)
        .color(palette().muted)
        .into()
}

/// "1 File  2 Folders  3 Report", the current one in ink.
fn steps(step: ImportStep) -> Element<'static, Message> {
    let current = match step {
        ImportStep::File => 0,
        ImportStep::Folders => 1,
        ImportStep::Running | ImportStep::Report => 2,
    };
    row([
        strings::import_step_file(),
        strings::import_step_folders(),
        strings::import_step_report(),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, label)| {
        let color = if i == current {
            palette().ink
        } else {
            palette().muted
        };
        text(label).size(theme::TEXT_XS).color(color).into()
    }))
    .spacing(theme::SPACE_LG)
    .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let flow = &app.import;
    let body: Element<'_, Message> = match flow.step {
        ImportStep::File => {
            let path = row![
                text_input("", &flow.path)
                    .on_input(|v| msg(ImportMsg::Path(v)))
                    .on_submit(msg(ImportMsg::Next))
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
                    .style(theme::search),
                button(text(strings::choose_file()))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
                    .style(theme::button_outline)
                    .on_press(msg(ImportMsg::Choose)),
            ]
            .spacing(theme::SPACE_XS)
            .align_y(Alignment::Center);
            column![
                field(strings::import_file_label(), path.into(), flow.path_error),
                muted(strings::import_file_hint()),
                failure(flow.error.as_ref()),
                row![
                    primary(strings::next(), Some(msg(ImportMsg::Next))),
                    secondary(strings::cancel(), Some(msg(ImportMsg::Close))),
                ]
                .spacing(theme::SPACE_XS)
                .align_y(Alignment::Center),
            ]
            .spacing(theme::SPACE_SM)
            .into()
        }
        ImportStep::Folders => column![
            text(&flow.path)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(palette().muted),
            text(strings::import_folders_question()).font(theme::INTER_SEMIBOLD),
            muted(strings::import_folders_hint()),
            row![
                primary(
                    strings::import_folders_yes(),
                    Some(msg(ImportMsg::Start {
                        folders_as_workspaces: true
                    }))
                ),
                button(text(strings::import_folders_no()))
                    .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                    .style(theme::button_outline)
                    .on_press(msg(ImportMsg::Start {
                        folders_as_workspaces: false
                    })),
                space::horizontal(),
                secondary(strings::back(), Some(msg(ImportMsg::Back))),
            ]
            .spacing(theme::SPACE_XS)
            .align_y(Alignment::Center),
        ]
        .spacing(theme::SPACE_SM)
        .into(),
        ImportStep::Running => busy(strings::import_running(), app.busy_phase),
        ImportStep::Report => match &flow.report {
            Some(report) => report_view(report, &flow.delete, flow.source.as_ref()),
            None => space().into(),
        },
    };

    let content = column![
        text(strings::import_title())
            .size(theme::TEXT_MD)
            .font(theme::INTER_SEMIBOLD),
        steps(flow.step),
        rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
        body,
    ]
    .spacing(theme::SPACE_SM)
    .max_width(640);

    scrollable(
        container(content)
            .width(Length::Fill)
            .padding(theme::SPACE_LG),
    )
    .height(Length::Fill)
    .into()
}

fn report_view<'a>(
    report: &'a ImportReport,
    delete: &'a SourceDelete,
    source: Option<&'a std::path::PathBuf>,
) -> Element<'a, Message> {
    let sections =
        column(report_sections(report).into_iter().map(section_view)).spacing(theme::SPACE_MD);

    let delete_row: Element<'a, Message> = match delete {
        SourceDelete::Unavailable(KeepSource::NoReport) => space().into(),
        SourceDelete::Unavailable(KeepSource::Failures) => {
            muted(strings::import_failed_keep_source())
        }
        SourceDelete::Unavailable(KeepSource::PasswordsNotSaved(sites)) => column![
            muted(strings::import_passwords_keep_source()),
            column(sites.iter().map(|name| {
                text(name)
                    .font(theme::MONO)
                    .size(theme::TEXT_MONO)
                    .color(palette().ink_2)
                    .into()
            }))
            .spacing(theme::SPACE_3XS),
        ]
        .spacing(theme::SPACE_2XS)
        .into(),
        SourceDelete::Offered => row![
            primary(strings::done(), Some(msg(ImportMsg::Close))),
            space::horizontal(),
            secondary(strings::delete_sitemanager(), Some(msg(ImportMsg::Delete))),
        ]
        .align_y(Alignment::Center)
        .into(),
        SourceDelete::Confirming => column![
            source.map_or_else(
                || space().into(),
                |path| -> Element<'a, Message> {
                    text(path.to_string_lossy())
                        .font(theme::MONO)
                        .size(theme::TEXT_MONO)
                        .color(palette().muted)
                        .into()
                }
            ),
            confirm(
                strings::delete_sitemanager_confirm(),
                msg(ImportMsg::ConfirmDelete),
                msg(ImportMsg::CancelDelete),
            ),
        ]
        .spacing(theme::SPACE_2XS)
        .into(),
        SourceDelete::Deleting => muted(strings::deleting_sitemanager()),
        SourceDelete::Deleted => muted(strings::sitemanager_deleted()),
        SourceDelete::Failed(error) => failure(Some(error)),
    };
    let mut col = column![
        sections,
        rule::horizontal(theme::HAIRLINE).style(theme::rule_1px),
        delete_row
    ]
    .spacing(theme::SPACE_MD);
    if !matches!(delete, SourceDelete::Offered) {
        col = col.push(primary(strings::done(), Some(msg(ImportMsg::Close))));
    }
    col.into()
}

fn section_view(section: Section<'_>) -> Element<'_, Message> {
    let head = text(strings::section_count(section.title, section.count))
        .size(theme::TEXT_XS)
        .font(theme::INTER_SEMIBOLD);
    let error = section.title == strings::import_failures() && section.count > 0;
    let lines: Element<'_, Message> = if section.lines.is_empty() {
        text(strings::import_none())
            .size(theme::TEXT_SM)
            .color(palette().muted)
            .into()
    } else {
        column(section.lines.into_iter().map(|line| {
            let name = text(line.name)
                .font(theme::MONO)
                .size(theme::TEXT_MONO)
                .color(if error {
                    palette().error
                } else {
                    palette().ink
                });
            let mut entry = column![name].spacing(theme::SPACE_3XS);
            for note in line.notes {
                entry = entry.push(text(note).size(theme::TEXT_XS).color(palette().muted));
            }
            entry.into()
        }))
        .spacing(theme::SPACE_XS)
        .into()
    };
    column![head, lines].spacing(theme::SPACE_XS).into()
}
