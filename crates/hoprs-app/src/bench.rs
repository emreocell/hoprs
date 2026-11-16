//! Support for the benchmarks in `benches/`. Not an API: it changes with them.

use std::borrow::Cow;
use std::time::{Duration, SystemTime};

use hoprs_core::{EntryKind, Protocol, RemoteEntry, RemotePath, Site};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, scroll_to};
use iced::advanced::{mouse, renderer};
use iced_runtime::user_interface::{Cache, UserInterface};

use crate::app::{App, Effect, Message, PaneMsg};
use crate::backend::{ProfileSnapshot, Side};
use crate::ui;

pub use crate::probe::{ENV as PROBE_ENV, IDLE as PROBE_IDLE, ProbeLine, parse_probe_line};

/// The window the app opens with.
pub const WINDOW: iced::Size = crate::INITIAL_WINDOW;

/// The app with one tab whose local pane lists a folder of `entries` files, and what it
/// takes to lay it out and draw it without a window: the tiny-skia renderer (recording
/// only, nothing rasterized) and the widget tree kept between frames, as iced keeps it.
pub struct FileList {
    app: App,
    tab: crate::app::TabId,
    renderer: iced::Renderer,
    theme: iced::Theme,
    cache: Option<Cache>,
}

impl FileList {
    pub fn new(entries: usize) -> Self {
        {
            let mut fonts = iced::advanced::graphics::text::font_system()
                .write()
                .expect("font system");
            for font in ui::theme::FONTS {
                fonts.load_font(Cow::Borrowed(font));
            }
        }
        let (mut app, boot) = App::boot();
        let request = boot
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadProfiles(request) => Some(*request),
                _ => None,
            })
            .expect("the app reads the profiles at boot");
        let site = Site::new("bench", "bench.example.com", "bench", Protocol::Ftp);
        let site_id = site.id.clone();
        app.home = Some(RemotePath::new("/bench").expect("path"));
        app.update(Message::ProfilesLoaded {
            request,
            result: Ok(ProfileSnapshot {
                sites: vec![site],
                workspaces: Vec::new(),
                unreadable: Vec::new(),
            }),
        });
        let effects = app.update(Message::ConnectSite(site_id));
        let (tab, request_id) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::List {
                    tab,
                    side: Side::Local,
                    request_id,
                    ..
                } => Some((*tab, *request_id)),
                _ => None,
            })
            .expect("the new tab lists its local folder");
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let listing = (0..entries)
            .map(|i| {
                let name = format!("file-{i:06}.log");
                RemoteEntry {
                    path: RemotePath::new(&format!("/bench/{name}")).expect("path"),
                    name,
                    kind: EntryKind::File,
                    size: (i as u64).wrapping_mul(7919) % 10_000_000,
                    modified: Some(modified + Duration::from_secs(i as u64)),
                    mode: Some(0o100_644),
                }
            })
            .collect();
        app.update(Message::Listed {
            tab,
            side: Side::Local,
            request_id,
            result: Ok(listing),
        });
        let rows = app
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.pane(Side::Local))
            .map_or(0, |pane| pane.entries.len());
        assert_eq!(rows, entries, "the local pane lists every entry");
        let renderer = iced::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            ui::theme::INTER,
            ui::theme::TEXT_SM.into(),
        ));
        Self {
            app,
            tab,
            renderer,
            theme: ui::theme::theme(),
            cache: Some(Cache::new()),
        }
    }

    /// The largest scroll offset of the pane in [`WINDOW`] (an upper bound: the pane is
    /// shorter than the window).
    pub fn max_offset(&self) -> f32 {
        let rows = self
            .app
            .tabs
            .iter()
            .find(|t| t.id == self.tab)
            .and_then(|t| t.pane(Side::Local))
            .map_or(0, |pane| pane.row_count());
        rows as f32 * ui::virtual_list::ROW_HEIGHT
    }

    /// One frame scrolled to `offset_y`: the scroll message, `view()`, the layout (which
    /// builds the rows in view), the scroll position applied, and the draw.
    pub fn frame(&mut self, offset_y: f32) {
        self.app.update(Message::Pane {
            tab: self.tab,
            side: Side::Local,
            msg: PaneMsg::Scrolled {
                offset_y,
                viewport_h: WINDOW.height,
                origin: iced::Point::ORIGIN,
            },
        });
        let cache = self.cache.take().unwrap_or_default();
        let mut ui = UserInterface::build(
            ui::layout::view(&self.app),
            WINDOW,
            cache,
            &mut self.renderer,
        );
        let mut scroll = scroll_to::<()>(
            ui::files::scroll_id(self.tab, Side::Local),
            AbsoluteOffset {
                x: None,
                y: Some(offset_y),
            },
        );
        ui.operate(&self.renderer, &mut scroll);
        ui.draw(
            &mut self.renderer,
            &self.theme,
            &renderer::Style {
                text_color: self.theme.palette().text,
            },
            mouse::Cursor::Unavailable,
        );
        self.cache = Some(ui.into_cache());
    }
}

#[cfg(test)]
mod tests {
    use iced::Rectangle;
    use iced::advanced::widget::{Id, Operation};

    use super::*;

    /// Every text the widget tree shows.
    #[derive(Default)]
    struct Texts(Vec<String>);

    impl Operation for Texts {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn text(&mut self, _id: Option<&Id>, _bounds: Rectangle, text: &str) {
            self.0.push(text.to_owned());
        }
    }

    impl FileList {
        fn texts(&mut self) -> Vec<String> {
            let mut ui = UserInterface::build(
                ui::layout::view(&self.app),
                WINDOW,
                self.cache.take().unwrap_or_default(),
                &mut self.renderer,
            );
            let mut texts = Texts::default();
            ui.operate(&self.renderer, &mut texts);
            self.cache = Some(ui.into_cache());
            texts.0
        }
    }

    /// The bench measures real work: the frame lays out the rows at the offset, and only
    /// those.
    #[test]
    fn frame_lays_out_the_rows_in_view() {
        let mut list = FileList::new(100_000);
        let row = ui::virtual_list::ROW_HEIGHT;
        list.frame(50_000.0 * row);
        let texts = list.texts();
        let names: Vec<&String> = texts.iter().filter(|t| t.starts_with("file-")).collect();
        assert!(
            names.iter().any(|name| name.as_str() == "file-050000.log"),
            "{names:?}"
        );
        assert!(!names.iter().any(|name| name.as_str() == "file-000000.log"));
        assert!(names.len() < 100, "{} rows built", names.len());
    }
}
