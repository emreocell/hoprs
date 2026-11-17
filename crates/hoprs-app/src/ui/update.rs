//! The update notice (Plan 15): the card floating above the sidebar's footer, bottom
//! left, and the footer line that reopens it. Not a modal: no backdrop (a press anywhere
//! else reaches the app and leaves the card open), no text input, nothing focused, no
//! animation. The state is [`crate::app::update`]'s; [`card_body`] picks what to draw.

use iced::widget::{button, column, container, opaque, progress_bar, row, stack, text, tooltip};
use iced::{Alignment, Element, Length, Padding};
use semver::Version;

use crate::app::update::{CardCheck, ManualCheck, Phase, UpdateMsg, install_command};
use crate::app::{App, Message};
use crate::ui::icons::{self, Icon};
use crate::ui::sidebar;
use crate::ui::strings;
use crate::ui::theme::{self, palette};
use crate::ui::widgets;
use crate::ui::{fit, format};
use crate::update::package::Os;

/// The space between the card and the sidebar's edges (left, right, and above the
/// footer row).
pub const MARGIN: f32 = theme::SPACE_XS;
/// The card's width: inside the sidebar, with [`MARGIN`] on both sides, so it never
/// covers the content next to the sidebar.
pub const WIDTH: f32 = sidebar::WIDTH - 2.0 * MARGIN;
/// The card's inner padding, left and right (top and bottom: [`theme::SPACE_SM`]).
const PADDING_X: f32 = theme::SPACE_XS;
/// The body's indent: it lines up with the title's text, right of the icon.
const INDENT: f32 = icons::ROW + theme::SPACE_XS;
/// The width the body's lines get (the border counted on both sides).
pub const BODY_WIDTH: f32 = WIDTH - 2.0 * PADDING_X - 2.0 * theme::HAIRLINE - INDENT;
/// The Linux command's size and the code block's inner padding.
const COMMAND_SIZE: f32 = theme::TEXT_TAG;
const COMMAND_PADDING: f32 = theme::SPACE_XS;
/// An error shows on at most about this many lines; the rest goes to its tooltip.
const ERROR_LINES: f32 = 4.0;
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
    /// Windows: between "Close and install" and the opener's answer.
    Installing,
    /// Windows: the installer is open; hoprs must close (a cancelled close stays here).
    InstallerOpen,
    /// Linux: the `.deb` went to the system installer; the command is the way without one.
    Linux {
        command: String,
        open_error: Option<String>,
    },
    /// macOS: the `.dmg` is open and the user replaces the app; or (`open_error`) it did
    /// not open, and Try again opens the verified file again.
    MacOs { open_error: Option<String> },
    /// The update (or the card's check) failed.
    Failed(String),
}

/// The version the card offers and what it shows under its title, or `None` while it is
/// collapsed or no newer version is known.
pub fn card_body(app: &App) -> Option<(&Version, CardBody)> {
    if !app.updater.card {
        return None;
    }
    let version = app.update_offer()?;
    Some((version, body_of(app)))
}

/// What the card shows for the updater's state (open or not).
fn body_of(app: &App) -> CardBody {
    let updater = &app.updater;
    match &updater.phase {
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
            Os::MacOs | Os::Other => CardBody::MacOs {
                open_error: open_error.as_ref().map(strings::error),
            },
        },
        Phase::InstallerOpen(_) => CardBody::InstallerOpen,
        Phase::Failed(error) => CardBody::Failed(strings::error(error)),
    }
}

/// The Linux text above the command: the system installer opened, or (it failed) the
/// command is the way.
fn linux_intro(open_failed: bool) -> &'static str {
    if open_failed {
        strings::update_linux_run()
    } else {
        strings::update_linux_opened()
    }
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
        Phase::Ready(_) | Phase::InstallerOpen(_) => strings::update_footer_install(&version),
        _ => strings::update_footer_available(&version),
    })
}

/// The line above the sidebar's footer row that reopens the card: while a newer version
/// is known and the card is collapsed.
pub fn footer_line(app: &App) -> Option<Element<'_, Message>> {
    // While the card is open (it shows whenever an offer exists) the line is not needed.
    if app.updater.card {
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

/// The card's left and right edges, from the window's left edge: inside the sidebar, with
/// [`MARGIN`] on both sides.
pub fn horizontal_span() -> (f32, f32) {
    (MARGIN, MARGIN + WIDTH)
}

/// `base` with the card over its bottom left, above the sidebar's footer. Always a stack
/// (as the menu's overlay), so `base` keeps its state when the card comes and goes. The
/// layer has no backdrop: only the card's box takes presses.
pub fn overlay<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    let Some((version, body)) = card_body(app) else {
        return stack![base].into();
    };
    let (left, right) = horizontal_span();
    let card = opaque(
        container(card(app, version.to_string(), body))
            .width(right - left)
            .padding(Padding::from([theme::SPACE_SM, PADDING_X]))
            .style(theme::dialog_box),
    );
    let layer = container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            left,
            bottom: sidebar::FOOTER_HEIGHT + MARGIN,
            ..Padding::ZERO
        })
        .align_x(Alignment::Start)
        .align_y(Alignment::End);
    stack![base, layer].into()
}

fn card(app: &App, version: String, body: CardBody) -> Element<'_, Message> {
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
            // No bar while the size is unknown: the label says how much came.
            if let Some(fraction) = fraction {
                lines = lines.push(
                    progress_bar(0.0..=1.0, fraction)
                        .girth(BAR_GIRTH)
                        .style(theme::progress),
                );
            }
            lines = lines.push(muted(label));
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
        CardBody::InstallerOpen => {
            lines = lines.push(plain(strings::update_installer_open())).push(
                button(
                    text(strings::update_close_hoprs())
                        .size(theme::TEXT_SM)
                        .font(theme::INTER_SEMIBOLD),
                )
                .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_MD]))
                .style(theme::button_primary)
                .on_press(msg(UpdateMsg::CloseForInstall)),
            );
        }
        CardBody::Linux {
            command,
            open_error,
        } => {
            // A code block across the body: the path breaks after a `/` (and only inside a
            // name too long for a line); Copy copies the exact command, below it.
            let block = container(
                text(command)
                    .font(theme::MONO)
                    .size(COMMAND_SIZE)
                    .wrapping(text::Wrapping::WordOrGlyph),
            )
            .width(Length::Fill)
            .padding(COMMAND_PADDING)
            .style(theme::code_block);
            // Why the command shows first; the opener's own words only in the tooltip.
            let open_failed = open_error.is_some();
            if let Some(error) = open_error {
                lines = lines.push(with_tip(
                    muted(strings::update_open_failed().to_owned()),
                    error,
                ));
            }
            lines = lines
                .push(plain(linux_intro(open_failed)))
                .push(block)
                .push(widgets::tool_button(
                    Icon::Copy,
                    strings::update_copy(),
                    None,
                    false,
                    Some(msg(UpdateMsg::CopyCommand)),
                ));
        }
        CardBody::MacOs { open_error: None } => {
            lines = lines.push(plain(strings::update_macos_steps()));
        }
        // The disk image did not open: the steps would not apply. Try again downloads
        // nothing (the verified file is reused) and opens it again.
        CardBody::MacOs {
            open_error: Some(error),
        } => {
            lines = lines
                .push(with_tip(
                    muted(strings::update_open_failed().to_owned()),
                    error,
                ))
                .push(actions(
                    strings::update_try_again(),
                    UpdateMsg::Start,
                    strings::update_whats_new(),
                    UpdateMsg::WhatsNew,
                ));
        }
        CardBody::Failed(error) => {
            let clipped = clip_error(&error);
            let cut = matches!(clipped, std::borrow::Cow::Owned(_));
            let shown: Element<'_, Message> = text(clipped.into_owned())
                .size(theme::TEXT_SM)
                .color(palette().error)
                .into();
            // A long error ends in an ellipsis; the whole text is in the tooltip.
            let shown = if cut { with_tip(shown, error) } else { shown };
            lines = lines.push(shown).push(actions(
                strings::update_try_again(),
                UpdateMsg::Start,
                strings::update_whats_new(),
                UpdateMsg::WhatsNew,
            ));
        }
    }
    if let Some(error) = &app.updater.page_error {
        lines = lines.push(muted(clip_error(&strings::error(error)).into_owned()));
    }

    column![
        title,
        container(lines.width(Length::Fill)).padding(Padding {
            left: INDENT,
            ..Padding::ZERO
        }),
    ]
    .spacing(theme::SPACE_2XS)
    .into()
}

/// `error` cut to about [`ERROR_LINES`] lines of the body (an ellipsis at the end when cut).
fn clip_error(error: &str) -> std::borrow::Cow<'_, str> {
    fit::ellipsize(
        error,
        ERROR_LINES * BODY_WIDTH,
        theme::TEXT_SM,
        fit::INTER_ADVANCE,
    )
}

/// `content` with `full` in a tooltip above it.
fn with_tip<'a>(content: Element<'a, Message>, full: String) -> Element<'a, Message> {
    let body = container(text(full).size(theme::TEXT_XS))
        .max_width(sidebar::WIDTH * 1.5)
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
        .style(theme::dialog_box);
    tooltip(content, body, tooltip::Position::Top).into()
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
    // Wraps to a second row rather than clipping (long translations).
    row![
        button(
            text(primary)
                .size(theme::TEXT_SM)
                .font(theme::INTER_SEMIBOLD)
        )
        .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_SM]))
        .style(theme::button_primary)
        .on_press(msg(on_primary)),
        button(text(secondary).size(theme::TEXT_SM))
            .padding(Padding::from([theme::SPACE_2XS, theme::SPACE_XS]))
            .style(theme::button_text)
            .on_press(msg(on_secondary)),
    ]
    .spacing(theme::SPACE_XS)
    .align_y(Alignment::Center)
    .wrap()
    .vertical_spacing(theme::SPACE_XS)
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

    /// How many characters of the Linux command fit on one line of its code block.
    fn command_chars_per_line() -> usize {
        ((BODY_WIDTH - 2.0 * COMMAND_PADDING) / (COMMAND_SIZE * fit::MONO_ADVANCE)).floor() as usize
    }

    /// The body only.
    fn body(app: &App) -> Option<CardBody> {
        card_body(app).map(|(_, body)| body)
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
        assert_eq!(body(&app), Some(CardBody::Offer));

        app.updater.phase = Phase::Downloading {
            done: 512 * 1024,
            total: Some(1024 * 1024),
        };
        match body(&app) {
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
            body(&app),
            Some(CardBody::Progress { fraction: None, .. })
        ));

        app.updater.phase = Phase::Verifying;
        assert_eq!(body(&app), Some(CardBody::Checking));

        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: None,
        };
        assert_eq!(
            body(&app),
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
            body(&app),
            Some(CardBody::Linux {
                command: install_command(&file),
                open_error: Some(strings::error(&failure())),
            })
        );

        app.updater.phase = Phase::Failed(failure());
        assert_eq!(
            body(&app),
            Some(CardBody::Failed(strings::error(&failure())))
        );

        let mut app = offered(Os::MacOs);
        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: None,
        };
        assert_eq!(body(&app), Some(CardBody::MacOs { open_error: None }));
        // The disk image did not open: the error shows (Try again opens it again).
        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: Some(failure()),
        };
        assert_eq!(
            body(&app),
            Some(CardBody::MacOs {
                open_error: Some(strings::error(&failure())),
            })
        );

        let mut app = offered(Os::Windows);
        app.updater.phase = Phase::Ready(file.clone());
        assert_eq!(
            body(&app),
            Some(CardBody::Ready {
                transfers: app.transfers_to_stop()
            })
        );
        app.updater.phase = Phase::HandedOff {
            file: file.clone(),
            open_error: None,
        };
        assert_eq!(body(&app), Some(CardBody::Installing));
        app.updater.phase = Phase::InstallerOpen(file);
        assert_eq!(body(&app), Some(CardBody::InstallerOpen));
        app.updater.phase = Phase::Failed(failure());
        assert_eq!(
            body(&app),
            Some(CardBody::Failed(strings::error(&failure())))
        );
    }

    #[test]
    fn card_stays_inside_the_sidebar() {
        let (left, right) = horizontal_span();
        assert_eq!(right - left, 240.0);
        // Equal margins: the right edge stops as far from the sidebar's edge as the left
        // one from the window's, before the sidebar's 1 px rule.
        assert_eq!(sidebar::WIDTH - right, left);
        assert!(right <= sidebar::WIDTH - theme::HAIRLINE);
        let body = BODY_WIDTH;
        assert!(body > 180.0, "{body}");
    }

    #[test]
    fn the_deb_name_fits_one_command_line() {
        // A name never breaks inside: the longest plausible one fits on a line.
        let longest = "hoprs_10.10.10-1_amd64.deb".chars().count();
        assert!(
            command_chars_per_line() >= longest,
            "{} < {longest}",
            command_chars_per_line()
        );
    }

    #[test]
    fn long_errors_are_cut_short_ones_kept() {
        let short = "Could not check for updates";
        assert_eq!(clip_error(short), short);
        let long = "x ".repeat(200);
        let cut = clip_error(&long);
        assert!(cut.ends_with('\u{2026}'), "{cut}");
        let lines = cut.chars().count() as f32 * theme::TEXT_SM * fit::INTER_ADVANCE / BODY_WIDTH;
        assert!(lines <= ERROR_LINES, "{lines}");
    }

    #[test]
    fn linux_intro_follows_the_open_result() {
        set_language(Language::En);
        assert_eq!(
            linux_intro(false),
            "Opened with the default app for .deb files. Or run:"
        );
        assert_eq!(linux_intro(true), "Run this command to install:");
    }

    #[test]
    fn card_check_shows_only_before_the_release_is_known() {
        let mut app = offered(Os::Linux);
        app.updater.known = None;
        app.settings.update_seen = Some(Version::new(1, 4, 0));
        assert_eq!(body(&app), Some(CardBody::Offer));
        app.updater.card_check = CardCheck::Checking;
        assert_eq!(body(&app), Some(CardBody::Checking));
        app.updater.card_check = CardCheck::Failed(failure());
        assert_eq!(
            body(&app),
            Some(CardBody::Failed(strings::error(&failure())))
        );
        app.updater.card_check = CardCheck::Idle;
        app.updater.manual = ManualCheck::Checking;
        assert_eq!(body(&app), Some(CardBody::Checking));

        // Once the release is known, the card offers it whatever a check did.
        app.updater.known = Some(release("1.4.0"));
        app.updater.manual = ManualCheck::Failed(failure());
        app.updater.card_check = CardCheck::Failed(failure());
        assert_eq!(body(&app), Some(CardBody::Offer));
    }

    #[test]
    fn no_card_without_offer() {
        let mut app = offered(Os::Linux);
        app.updater.card = false;
        assert_eq!(body(&app), None);

        let mut app = offered(Os::Linux);
        app.updater.known = Some(release("1.3.1"));
        assert_eq!(body(&app), None);
    }

    #[test]
    fn footer_label_available_then_install() {
        let mut app = offered(Os::Windows);
        assert_eq!(footer_label(&app).as_deref(), Some("1.4.0 available"));
        let file = PathBuf::from("hoprs_1.4.0_windows_x64.msi");
        app.updater.phase = Phase::Ready(file.clone());
        assert_eq!(footer_label(&app).as_deref(), Some("Install 1.4.0"));
        app.updater.phase = Phase::InstallerOpen(file);
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
