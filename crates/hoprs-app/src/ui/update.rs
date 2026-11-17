//! The update notice (Plan 15): the card floating above the sidebar's footer, bottom
//! left, and the footer line that reopens it. Not a modal: no backdrop (a press anywhere
//! else reaches the app and leaves the card open), no text input, nothing focused, no
//! animation. The state is [`crate::app::update`]'s; [`card_body`] picks what to draw.

use iced::widget::{button, column, container, opaque, progress_bar, row, stack, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::update::{CardCheck, ManualCheck, Phase, UpdateMsg, install_command};
use crate::app::{App, Message};
use crate::ui::format;
use crate::ui::icons::{self, Icon};
use crate::ui::sidebar;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;
use crate::update::package::Os;

/// The card's width: the sidebar's plus a margin, so it reads as floating over it.
pub const WIDTH: f32 = sidebar::WIDTH + theme::SPACE_LG;
/// The progress bar's thickness (the queue's).
const BAR_GIRTH: f32 = 4.0;

/// What the card shows under its title.
#[derive(Debug, PartialEq)]
pub enum CardBody {
    /// "You have A.B.C." with Update and What's new.
    Offer,
    /// The download: `fraction` of the installer (none while the size is unknown).
    Progress {
        fraction: Option<f32>,
        label: String,
    },
    /// A check the card waits for, or the download being verified.
    Checking,
    /// Windows: the verified installer waits for "Close and install".
    Ready { transfers: usize },
    /// Windows: the installer is being opened and hoprs closes.
    Installing,
    /// Linux: the `.deb` went to the system installer; the command is the way without one.
    Linux {
        command: String,
        open_error: Option<String>,
    },
    /// macOS: the `.dmg` is open; the user replaces the app.
    MacOs,
    /// The update (or the card's check) failed.
    Failed(String),
}

/// What the card shows, or `None` while it is collapsed or no newer version is known.
pub fn card_body(app: &App) -> Option<CardBody> {
    let updater = &app.updater;
    if !updater.card {
        return None;
    }
    app.update_offer()?;
    let body = match &updater.phase {
        // Only `update_seen` so far: the check the card asked for (or Check now's).
        Phase::Idle if updater.known.is_none() => match &updater.card_check {
            CardCheck::Checking => CardBody::Checking,
            CardCheck::Failed(error) => CardBody::Failed(strings::error(error)),
            CardCheck::Idle if updater.manual == ManualCheck::Checking => CardBody::Checking,
            CardCheck::Idle => CardBody::Offer,
        },
        Phase::Idle => CardBody::Offer,
        Phase::Downloading { done, total } => CardBody::Progress {
            fraction: fraction(*done, *total),
            label: match total {
                Some(total) => {
                    strings::update_downloading(&format::size(*done), &format::size(*total))
                }
                None => strings::update_downloading_size(&format::size(*done)),
            },
        },
        Phase::Verifying => CardBody::Checking,
        Phase::Ready(_) => CardBody::Ready {
            transfers: app.transfers_to_stop(),
        },
        Phase::HandedOff { file, open_error } => match updater.env.os {
            Os::Windows => CardBody::Installing,
            Os::Linux => CardBody::Linux {
                command: install_command(file),
                open_error: open_error.as_ref().map(strings::error),
            },
            // Other never downloads (no package): the macOS text is the nearest.
            Os::MacOs | Os::Other => CardBody::MacOs,
        },
        Phase::Failed(error) => CardBody::Failed(strings::error(error)),
    };
    Some(body)
}

/// The share of the installer downloaded, in `0.0..=1.0`; `None` while the size is unknown.
fn fraction(done: u64, total: Option<u64>) -> Option<f32> {
    let total = total.filter(|total| *total > 0)?;
    Some((done as f64 / total as f64).clamp(0.0, 1.0) as f32)
}

/// The footer line's text: "Install X.Y.Z" while a verified installer waits (Windows,
/// after Later), else "X.Y.Z available"; `None` while no newer version is known.
pub fn footer_label(app: &App) -> Option<String> {
    let version = app.update_offer()?.to_string();
    Some(match app.updater.phase {
        Phase::Ready(_) => strings::update_footer_install(&version),
        _ => strings::update_footer_available(&version),
    })
}

/// The line above the sidebar's footer row that reopens the card: while a newer version
/// is known and the card is collapsed.
pub fn footer_line(app: &App) -> Option<Element<'_, Message>> {
    if card_body(app).is_some() {
        return None;
    }
    let label = footer_label(app)?;
    let line = row![
        icons::icon(Icon::ArrowDown, icons::SMALL),
        text(label).size(theme::TEXT_XS),
    ]
    .spacing(theme::SPACE_2XS)
    .align_y(Alignment::Center);
    Some(
        button(line)
            .padding(Padding::from([theme::SPACE_3XS, theme::SPACE_2XS]))
            .style(theme::button_text)
            .on_press(msg(UpdateMsg::ShowCard))
            .into(),
    )
}

fn msg(m: UpdateMsg) -> Message {
    Message::Update(m)
}

/// `base` with the card over its bottom left, above the sidebar's footer. Always a stack
/// (as the menu's overlay), so `base` keeps its state when the card comes and goes. The
/// layer has no backdrop: only the card's box takes presses.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some(body) = card_body(app) else {
        return stack![base].into();
    };
    let card = opaque(
        container(card(app, body))
            .width(WIDTH)
            .padding(theme::SPACE_SM)
            .style(theme::dialog_box),
    );
    let layer = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            left: theme::SPACE_XS,
            bottom: sidebar::FOOTER_HEIGHT + theme::SPACE_XS,
            ..Padding::ZERO
        })
        .align_x(Alignment::Start)
        .align_y(Alignment::End);
    stack![base, layer].into()
}

fn card(app: &App, body: CardBody) -> Element<'_, Message> {
    let version = app
        .update_offer()
        .map(ToString::to_string)
        .unwrap_or_default();
    let title = row![
        icons::icon(Icon::DownloadSimple, icons::ROW),
        container(
            text(strings::update_available(&version))
                .size(theme::TEXT_SM)
                .font(theme::INTER_SEMIBOLD)
        )
        .width(Length::Fill),
        widgets::icon_button_small(
            Icon::X,
            strings::update_dismiss(),
            None,
            Some(msg(UpdateMsg::Dismiss)),
        ),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center);

    let mut lines = column![].spacing(theme::SPACE_XS);
    match body {
        CardBody::Offer => {
            lines = lines.push(muted(strings::update_you_have(
                &app.updater.env.running.to_string(),
            )));
            lines = lines.push(actions(
                strings::update_action(),
                UpdateMsg::Start,
                strings::update_whats_new(),
                UpdateMsg::WhatsNew,
            ));
        }
        CardBody::Progress { fraction, label } => {
            lines = lines
                .push(
                    progress_bar(0.0..=1.0, fraction.unwrap_or(0.0))
                        .girth(BAR_GIRTH)
                        .style(theme::progress),
                )
                .push(muted(label));
        }
        CardBody::Checking => lines = lines.push(muted(strings::update_verifying().to_owned())),
        CardBody::Ready { transfers } => {
            lines = lines.push(plain(strings::update_ready_windows()));
            if transfers > 0 {
                lines = lines.push(muted(strings::update_transfers_stop(transfers)));
            }
            lines = lines.push(actions(
                strings::update_close_and_install(),
                UpdateMsg::Install,
                strings::update_later(),
                UpdateMsg::Later,
            ));
        }
        CardBody::Installing => lines = lines.push(plain(strings::update_installing())),
        CardBody::Linux {
            command,
            open_error,
        } => {
            let command = row![
                container(
                    text(command)
                        .font(theme::MONO)
                        .size(theme::TEXT_MONO)
                        .wrapping(text::Wrapping::WordOrGlyph)
                )
                .width(Length::Fill),
                widgets::tool_button(
                    Icon::Copy,
                    strings::update_copy(),
                    None,
                    false,
                    Some(msg(UpdateMsg::CopyCommand)),
                ),
            ]
            .spacing(theme::SPACE_XS)
            .align_y(Alignment::Center);
            lines = lines
                .push(plain(strings::update_linux_opened()))
                .push(command);
            if let Some(error) = open_error {
                lines = lines.push(muted(error));
            }
        }
        CardBody::MacOs => lines = lines.push(plain(strings::update_macos_steps())),
        CardBody::Failed(error) => {
            lines = lines
                .push(text(error).size(theme::TEXT_SM).color(palette().error))
                .push(actions(
                    strings::update_try_again(),
                    UpdateMsg::Start,
                    strings::update_whats_new(),
                    UpdateMsg::WhatsNew,
                ));
        }
    }
    if let Some(error) = &app.updater.page_error {
        lines = lines.push(muted(strings::error(error)));
    }

    // The body lines up with the title's text, right of the icon.
    let indent = icons::ROW + theme::SPACE_XS;
    column![
        title,
        container(lines).padding(Padding {
            left: indent,
            ..Padding::ZERO
        }),
    ]
    .spacing(theme::SPACE_2XS)
    .into()
}

fn plain<'a>(label: &'a str) -> Element<'a, Message> {
    text(label).size(theme::TEXT_SM).into()
}

fn muted<'a>(label: String) -> Element<'a, Message> {
    text(label)
        .size(theme::TEXT_SM)
        .color(palette().muted)
        .into()
}

/// The primary action, then a text-button one.
fn actions<'a>(
    primary: &'a str,
    on_primary: UpdateMsg,
    secondary: &'a str,
    on_secondary: UpdateMsg,
) -> Element<'a, Message> {
    row![
        button(
            text(primary)
                .size(theme::TEXT_SM)
                .font(theme::INTER_SEMIBOLD)
        )
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
        .style(theme::button_primary)
        .on_press(msg(on_primary)),
        button(text(secondary).size(theme::TEXT_SM))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(msg(on_secondary)),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use hoprs_core::{Error, ErrorKind};
    use semver::Version;

    use super::*;
    use crate::app::update::{UpdateEnv, Updater};
    use crate::i18n::{Language, set_language};
    use crate::update::package::PackageKind;
    use crate::update::{Asset, Release};

    fn release(version: &str) -> Release {
        Release {
            version: Version::parse(version).unwrap(),
            page: format!("https://github.com/engviniciusbs/hoprs/releases/tag/v{version}"),
            assets: vec![Asset {
                name: "SHA256SUMS".to_owned(),
                url: format!(
                    "https://github.com/engviniciusbs/hoprs/releases/download/v{version}/SHA256SUMS"
                ),
                size: 10,
            }],
        }
    }

    /// An app running 1.3.1 on `os` that knows of 1.4.0, card open.
    fn offered(os: Os) -> App {
        set_language(Language::En);
        let mut app = App::new();
        app.updater = Updater::new(UpdateEnv::test("1.3.1", os, Some(PackageKind::Deb)));
        app.updater.known = Some(release("1.4.0"));
        app.updater.card = true;
        app
    }

    fn failure() -> Error {
        Error::coded(
            ErrorKind::Network,
            "update-download-failed",
            "The update could not be downloaded",
        )
    }

    #[test]
    fn card_body_per_phase() {
        let file = PathBuf::from("/home/me/Downloads/hoprs_1.4.0-1_amd64.deb");
        let mut app = offered(Os::Linux);
        assert_eq!(card_body(&app), Some(CardBody::Offer));

        app.updater.phase = Phase::Downloading {
            done: 512 * 1024,
            total: Some(1024 * 1024),
        };
        match card_body(&app) {
            Some(CardBody::Progress { fraction, label }) => {
                assert_eq!(fraction, Some(0.5));
                assert!(label.starts_with("Downloading…"), "{label}");
                assert!(label.contains(" of "), "{label}");
            }
            other => panic!("{other:?}"),
        }
        app.updater.phase = Phase::Downloading {
            done: 10,
            total: None,
        };
        assert!(matches!(
            card_body(&app),
            Some(CardBody::Progress { fraction: None, .. })
        ));

        app.updater.phase = Phase::Verifying;
        assert_eq!(card_body(&app), Some(CardBody::Checking));

        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: None,
        };
        assert_eq!(
            card_body(&app),
            Some(CardBody::Linux {
                command: install_command(&file),
                open_error: None,
            })
        );
        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: Some(failure()),
        };
        assert_eq!(
            card_body(&app),
            Some(CardBody::Linux {
                command: install_command(&file),
                open_error: Some(strings::error(&failure())),
            })
        );

        app.updater.phase = Phase::Failed(failure());
        assert_eq!(
            card_body(&app),
            Some(CardBody::Failed(strings::error(&failure())))
        );

        let mut app = offered(Os::MacOs);
        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: None,
        };
        assert_eq!(card_body(&app), Some(CardBody::MacOs));

        let mut app = offered(Os::Windows);
        app.updater.phase = Phase::Ready(file.clone());
        assert_eq!(
            card_body(&app),
            Some(CardBody::Ready {
                transfers: app.transfers_to_stop()
            })
        );
        app.updater.phase = Phase::HandedOff {
            file,
            open_error: None,
        };
        assert_eq!(card_body(&app), Some(CardBody::Installing));
    }

    #[test]
    fn card_check_shows_only_before_the_release_is_known() {
        let mut app = offered(Os::Linux);
        app.updater.known = None;
        app.settings.update_seen = Some(Version::new(1, 4, 0));
        assert_eq!(card_body(&app), Some(CardBody::Offer));
        app.updater.card_check = CardCheck::Checking;
        assert_eq!(card_body(&app), Some(CardBody::Checking));
        app.updater.card_check = CardCheck::Failed(failure());
        assert_eq!(
            card_body(&app),
            Some(CardBody::Failed(strings::error(&failure())))
        );
        app.updater.card_check = CardCheck::Idle;
        app.updater.manual = ManualCheck::Checking;
        assert_eq!(card_body(&app), Some(CardBody::Checking));

        // Once the release is known, the card offers it whatever a check did.
        app.updater.known = Some(release("1.4.0"));
        app.updater.manual = ManualCheck::Failed(failure());
        app.updater.card_check = CardCheck::Failed(failure());
        assert_eq!(card_body(&app), Some(CardBody::Offer));
    }

    #[test]
    fn no_card_without_offer() {
        let mut app = offered(Os::Linux);
        app.updater.card = false;
        assert_eq!(card_body(&app), None);

        let mut app = offered(Os::Linux);
        app.updater.known = Some(release("1.3.1"));
        assert_eq!(card_body(&app), None);
    }

    #[test]
    fn footer_label_available_then_install() {
        let mut app = offered(Os::Windows);
        assert_eq!(footer_label(&app).as_deref(), Some("1.4.0 available"));
        app.updater.phase = Phase::Ready(PathBuf::from("hoprs_1.4.0_windows_x64.msi"));
        assert_eq!(footer_label(&app).as_deref(), Some("Install 1.4.0"));
    }

    #[test]
    fn footer_line_absent_when_up_to_date() {
        let mut app = offered(Os::Linux);
        app.updater.known = None;
        app.updater.card = false;
        assert!(footer_label(&app).is_none());
        assert!(footer_line(&app).is_none());

        // The open card hides the line; ✕ shows it.
        let mut app = offered(Os::Linux);
        assert!(footer_line(&app).is_none());
        app.updater.card = false;
        assert!(footer_line(&app).is_some());
    }
}
