//! The app's text catalogs (Fluent) and the process-wide active language.
//!
//! `en.ftl` is the reference; `pt-BR.ftl` translates it. A lookup tries the active
//! language, then English, then returns the key itself, and never panics: a missing key
//! or a formatting error is logged once per key.

// The language switch, `group_digits` and the `Arg` conversions are used by the tasks that
// follow (settings, formatting, errors).
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

use crate::settings::LanguageChoice;

const EN_SRC: &str = include_str!("../i18n/en.ftl");
const PT_BR_SRC: &str = include_str!("../i18n/pt-BR.ftl");

/// A language the interface is available in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    En,
    PtBr,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::En, Language::PtBr];

    /// The BCP 47 tag: also the settings value and the catalog's file name.
    pub fn tag(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::PtBr => "pt-BR",
        }
    }

    fn source(self) -> Option<&'static str> {
        match self {
            Language::En => None,
            Language::PtBr => Some(PT_BR_SRC),
        }
    }
}

/// A value passed to a message.
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    Str(&'a str),
    Num(i64),
    Float(f64),
}

impl<'a> From<&'a str> for Arg<'a> {
    fn from(s: &'a str) -> Self {
        Arg::Str(s)
    }
}
impl<'a> From<&'a String> for Arg<'a> {
    fn from(s: &'a String) -> Self {
        Arg::Str(s)
    }
}
impl From<usize> for Arg<'_> {
    fn from(n: usize) -> Self {
        Arg::Num(i64::try_from(n).unwrap_or(i64::MAX))
    }
}
impl From<u64> for Arg<'_> {
    fn from(n: u64) -> Self {
        Arg::Num(i64::try_from(n).unwrap_or(i64::MAX))
    }
}
impl From<u32> for Arg<'_> {
    fn from(n: u32) -> Self {
        Arg::Num(i64::from(n))
    }
}
impl From<i64> for Arg<'_> {
    fn from(n: i64) -> Self {
        Arg::Num(n)
    }
}
impl From<u16> for Arg<'_> {
    fn from(n: u16) -> Self {
        Arg::Num(i64::from(n))
    }
}
impl From<i32> for Arg<'_> {
    fn from(n: i32) -> Self {
        Arg::Num(i64::from(n))
    }
}
impl From<f64> for Arg<'_> {
    fn from(n: f64) -> Self {
        Arg::Float(n)
    }
}

/// `1000` as `1,000` (English) or `1.000` (Brazilian Portuguese).
pub fn group_digits(lang: Language, n: u64) -> String {
    let sep = match lang {
        Language::En => ',',
        Language::PtBr => '.',
    };
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

fn format_number(lang: Language, value: f64) -> String {
    let negative = value.is_sign_negative() && value != 0.0;
    let abs = value.abs();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if abs.fract() == 0.0 && abs < u64::MAX as f64 {
        out.push_str(&group_digits(lang, abs as u64));
    } else {
        let text = abs.to_string();
        let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
        let int: u64 = int.parse().unwrap_or(0);
        out.push_str(&group_digits(lang, int));
        if !frac.is_empty() {
            out.push(match lang {
                Language::En => '.',
                Language::PtBr => ',',
            });
            out.push_str(frac);
        }
    }
    out
}

// `set_formatter` takes a plain function pointer, so one function per language.
fn number_formatter<M>(lang: Language, value: &FluentValue, _memoizer: &M) -> Option<String> {
    match value {
        FluentValue::Number(n) => Some(format_number(lang, n.value)),
        _ => None,
    }
}
fn en_formatter<M>(value: &FluentValue, memoizer: &M) -> Option<String> {
    number_formatter(Language::En, value, memoizer)
}
fn pt_br_formatter<M>(value: &FluentValue, memoizer: &M) -> Option<String> {
    number_formatter(Language::PtBr, value, memoizer)
}

type Bundle = FluentBundle<FluentResource>;

fn bundle(lang: Language, source: &str) -> Bundle {
    let id: LanguageIdentifier = lang.tag().parse().expect("language tags are valid");
    let mut bundle = FluentBundle::new_concurrent(vec![id]);
    bundle.set_use_isolating(false);
    match lang {
        Language::En => bundle.set_formatter(Some(en_formatter)),
        Language::PtBr => bundle.set_formatter(Some(pt_br_formatter)),
    }
    match FluentResource::try_new(source.to_owned()) {
        Ok(resource) => {
            if let Err(errors) = bundle.add_resource(resource) {
                warn_once(&format!("{}:add", lang.tag()), || {
                    format!("catalog {}: {errors:?}", lang.tag())
                });
            }
        }
        Err((resource, errors)) => {
            warn_once(&format!("{}:parse", lang.tag()), || {
                format!("catalog {} has syntax errors: {errors:?}", lang.tag())
            });
            // Keep every entry that did parse.
            let _ = bundle.add_resource(resource);
        }
    }
    bundle
}

fn warn_once(key: &str, message: impl FnOnce() -> String) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let first = seen
        .lock()
        .map(|mut seen| seen.insert(key.to_owned()))
        .unwrap_or(false);
    if first {
        tracing::warn!("{}", message());
    }
}

/// A key that no catalog has, as a `'static` string (leaked once per distinct key).
fn leak_key(key: &str) -> &'static str {
    static KEYS: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let keys = KEYS.get_or_init(Default::default);
    let Ok(mut keys) = keys.lock() else {
        return Box::leak(key.to_owned().into_boxed_str());
    };
    keys.entry(key.to_owned())
        .or_insert_with(|| Box::leak(key.to_owned().into_boxed_str()))
}

/// Message ids that take no variables, from a source's syntax tree.
fn message_ids(source: &str) -> Vec<(String, bool)> {
    ast::Resource::<&str>::default_from(source)
        .body
        .iter()
        .filter_map(|entry| match entry {
            ast::Entry::Message(message) => {
                let mut vars = HashSet::new();
                if let Some(pattern) = &message.value {
                    pattern_variables(pattern, &mut vars);
                }
                Some((message.id.name.to_owned(), vars.is_empty()))
            }
            _ => None,
        })
        .collect()
}

trait ParseLenient<'a> {
    fn default_from(source: &'a str) -> Self;
}
impl<'a> ParseLenient<'a> for ast::Resource<&'a str> {
    fn default_from(source: &'a str) -> Self {
        match fluent_syntax::parser::parse(source) {
            Ok(resource) | Err((resource, _)) => resource,
        }
    }
}

fn pattern_variables<'a>(pattern: &ast::Pattern<&'a str>, out: &mut HashSet<&'a str>) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            expression_variables(expression, out);
        }
    }
}

fn expression_variables<'a>(expression: &ast::Expression<&'a str>, out: &mut HashSet<&'a str>) {
    match expression {
        ast::Expression::Inline(inline) => inline_variables(inline, out),
        ast::Expression::Select { selector, variants } => {
            inline_variables(selector, out);
            for variant in variants {
                pattern_variables(&variant.value, out);
            }
        }
    }
}

fn inline_variables<'a>(inline: &ast::InlineExpression<&'a str>, out: &mut HashSet<&'a str>) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            out.insert(id.name);
        }
        ast::InlineExpression::Placeable { expression } => expression_variables(expression, out),
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            call_variables(arguments, out)
        }
        ast::InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => call_variables(arguments, out),
        _ => {}
    }
}

fn call_variables<'a>(arguments: &ast::CallArguments<&'a str>, out: &mut HashSet<&'a str>) {
    for positional in &arguments.positional {
        inline_variables(positional, out);
    }
    for named in &arguments.named {
        inline_variables(&named.value, out);
    }
}

/// The messages of one language, with English behind it.
pub struct Catalog {
    lang: Language,
    primary: Option<Bundle>,
    english: Bundle,
    /// Messages without variables, formatted once (already through the fallback).
    plain: HashMap<&'static str, &'static str>,
}

impl Catalog {
    pub fn new(lang: Language) -> Catalog {
        Catalog::from_sources(lang, EN_SRC, lang.source())
    }

    /// A catalog from explicit sources (the tests build partial ones).
    fn from_sources(lang: Language, english: &str, primary: Option<&str>) -> Catalog {
        let mut catalog = Catalog {
            lang,
            primary: primary.map(|source| bundle(lang, source)),
            english: bundle(Language::En, english),
            plain: HashMap::new(),
        };
        let mut ids = message_ids(english);
        if let Some(source) = primary {
            ids.extend(message_ids(source));
        }
        for (id, plain) in ids {
            if !plain || catalog.plain.contains_key(id.as_str()) {
                continue;
            }
            if let Some(text) = catalog.format_checked(&id, &[]) {
                catalog.plain.insert(
                    Box::leak(id.into_boxed_str()),
                    Box::leak(text.into_boxed_str()),
                );
            }
        }
        catalog
    }

    pub fn language(&self) -> Language {
        self.lang
    }

    /// A message without variables. A key that no catalog has comes back as itself.
    pub fn tr(&self, key: &str) -> &'static str {
        if let Some(text) = self.plain.get(key) {
            return text;
        }
        warn_once(&format!("missing:{key}"), || {
            format!("no text for the key `{key}` (or it needs arguments)")
        });
        leak_key(key)
    }

    pub fn tr_args(&self, key: &str, args: &[(&str, Arg)]) -> String {
        self.format_checked(key, args).unwrap_or_else(|| {
            warn_once(&format!("failed:{key}"), || {
                format!("cannot format the key `{key}`")
            });
            key.to_owned()
        })
    }

    /// The message in the catalog's language, else in English; `None` when neither has
    /// the key or formatting reports any error (a missing variable, say).
    fn format_checked(&self, key: &str, args: &[(&str, Arg)]) -> Option<String> {
        let mut fluent_args = FluentArgs::new();
        for (name, arg) in args {
            match *arg {
                Arg::Str(s) => fluent_args.set(*name, s),
                Arg::Num(n) => fluent_args.set(*name, n),
                Arg::Float(f) => fluent_args.set(*name, f),
            }
        }
        for bundle in self.primary.iter().chain(std::iter::once(&self.english)) {
            let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) else {
                continue;
            };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
            if errors.is_empty() {
                return Some(text.into_owned());
            }
            warn_once(&format!("error:{}:{key}", self.lang.tag()), || {
                format!("formatting `{key}`: {errors:?}")
            });
        }
        None
    }
}

fn catalog_for(lang: Language) -> &'static Catalog {
    static EN: OnceLock<Catalog> = OnceLock::new();
    static PT_BR: OnceLock<Catalog> = OnceLock::new();
    match lang {
        Language::En => EN.get_or_init(|| Catalog::new(Language::En)),
        Language::PtBr => PT_BR.get_or_init(|| Catalog::new(Language::PtBr)),
    }
}

// The active language. Tests run in parallel threads, so under `cfg(test)` each thread
// has its own (starting at English): a test that switches language cannot disturb the
// ones asserting English.
#[cfg(not(test))]
mod active {
    use super::Language;
    use std::sync::atomic::{AtomicU8, Ordering};

    static ACTIVE: AtomicU8 = AtomicU8::new(0);

    pub fn set(lang: Language) {
        ACTIVE.store(lang as u8, Ordering::Relaxed);
    }
    pub fn get() -> Language {
        match ACTIVE.load(Ordering::Relaxed) {
            1 => Language::PtBr,
            _ => Language::En,
        }
    }
}

#[cfg(test)]
mod active {
    use super::Language;
    use std::cell::Cell;

    thread_local! {
        static ACTIVE: Cell<Language> = const { Cell::new(Language::En) };
    }

    pub fn set(lang: Language) {
        ACTIVE.with(|a| a.set(lang));
    }
    pub fn get() -> Language {
        ACTIVE.with(Cell::get)
    }
}

/// The language an OS locale (`pt_BR.UTF-8`, `pt-BR`, `C`, …) asks for: Brazilian
/// Portuguese when the language subtag is `pt` (any case, any region or suffix), English
/// otherwise, also for no locale, an empty one, `C` and `POSIX`.
pub fn language_from_locale(locale: Option<&str>) -> Language {
    let tag = locale.unwrap_or("");
    let subtag = tag.split(['_', '-', '.', '@']).next().unwrap_or("");
    if subtag.eq_ignore_ascii_case("pt") {
        Language::PtBr
    } else {
        Language::En
    }
}

/// The language a setting means; `System` asks the OS.
pub fn resolve(choice: LanguageChoice) -> Language {
    match choice {
        LanguageChoice::Fixed(lang) => lang,
        LanguageChoice::System => language_from_locale(sys_locale::get_locale().as_deref()),
    }
}

pub fn set_language(lang: Language) {
    active::set(lang);
}

pub fn language() -> Language {
    active::get()
}

/// A message without variables, in the active language.
pub fn tr(key: &str) -> &'static str {
    catalog_for(language()).tr(key)
}

/// A message with variables, in the active language.
pub fn tr_args(key: &str, args: &[(&str, Arg)]) -> String {
    catalog_for(language()).tr_args(key, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAME_IN_BOTH: &[&str] = &[
        "app-title",
        "language-english",
        "language-pt-br",
        // Product names, symbols and shortcut labels.
        "brand",
        "go-to-anything-key",
        "palette-key-copy",
        "palette-key-paste",
        "palette-key-find",
        "ssh-options",
        "ftp-options",
        "close-tab",
        // Terms kept in English, or words that are the same in Portuguese.
        "palette-group-site",
        "palette-group-workspace",
        "palette-kind-terminal",
        "workspaces",
        "field-host",
        "workspace-sites",
        "open-as-terminal",
        "auth-keyboard-interactive",
        // File panes, log levels and file actions: symbols, units and loanwords.
        "local",
        "log",
        "log-info",
        "log-debug",
        "upload",
        "download",
        "perm-octal",
        "sort-ascending",
        "sort-descending",
        "parent-dir",
        "path-separator",
        "home-dir",
        "no-size",
        "upload-arrow",
        "download-arrow",
        "key-new-folder",
        "key-rename",
        "key-delete",
        "key-edit",
        "key-transfer",
        "size-unit-b",
        "size-unit-kb",
        "size-unit-mb",
        "size-unit-gb",
        "size-unit-tb",
        "size-unit-pb",
        "size-unit-eb",
    ];

    fn parse(source: &str) -> ast::Resource<&str> {
        ast::Resource::default_from(source)
    }

    fn variables(source: &str) -> std::collections::BTreeMap<String, Vec<String>> {
        let mut map = std::collections::BTreeMap::new();
        for entry in &parse(source).body {
            if let ast::Entry::Message(message) = entry {
                let mut vars = HashSet::new();
                if let Some(pattern) = &message.value {
                    pattern_variables(pattern, &mut vars);
                }
                let mut vars: Vec<String> = vars.into_iter().map(str::to_owned).collect();
                vars.sort();
                map.insert(message.id.name.to_owned(), vars);
            }
        }
        map
    }

    #[test]
    fn en_and_pt_br_parse_without_errors() {
        for (name, source) in [("en", EN_SRC), ("pt-BR", PT_BR_SRC)] {
            let resource = match FluentResource::try_new(source.to_owned()) {
                Ok(resource) => resource,
                Err((_, errors)) => panic!("{name}.ftl: {errors:?}"),
            };
            // Adding to a bundle fails on a duplicate message id.
            let mut bundle =
                FluentBundle::new_concurrent(vec![Language::En.tag().parse().unwrap()]);
            if let Err(errors) = bundle.add_resource(resource) {
                panic!("{name}.ftl: {errors:?}");
            }
        }
    }

    #[test]
    fn same_keys_and_variables_in_both_languages() {
        assert_eq!(variables(EN_SRC), variables(PT_BR_SRC));
        assert!(variables(EN_SRC).contains_key("site-count"));
        assert_eq!(variables(EN_SRC)["site-count"], ["count"]);
    }

    #[test]
    fn pt_br_values_differ_from_english() {
        let en = Catalog::new(Language::En);
        let pt = Catalog::new(Language::PtBr);
        for (id, plain) in message_ids(EN_SRC) {
            if plain && !SAME_IN_BOTH.contains(&id.as_str()) {
                assert_ne!(en.tr(&id), pt.tr(&id), "{id} is not translated");
            }
        }
    }

    #[test]
    fn missing_key_falls_back_to_english_then_key() {
        let partial = Catalog::from_sources(Language::PtBr, EN_SRC, Some("app-title = x\n"));
        assert_eq!(partial.tr("app-title"), "x");
        assert_eq!(partial.tr("already-running"), "hoprs is already running");
        assert_eq!(
            partial.tr_args("site-count", &[("count", 3.into())]),
            "3 sites"
        );
        assert_eq!(partial.tr("no-such-key"), "no-such-key");
        assert_eq!(partial.tr_args("no-such-key", &[]), "no-such-key");
    }

    #[test]
    fn missing_argument_does_not_leak_a_placeholder() {
        let text = Catalog::new(Language::En).tr_args("site-count", &[]);
        assert!(!text.contains('$') && !text.contains('{'), "{text}");
    }

    #[test]
    fn no_isolation_marks() {
        for lang in Language::ALL {
            let text = Catalog::new(lang).tr_args("site-count", &[("count", 3.into())]);
            assert!(!text.contains('\u{2068}') && !text.contains('\u{2069}'));
        }
    }

    #[test]
    fn numbers_group_per_language() {
        let args = [("count", Arg::from(1000usize))];
        assert!(
            Catalog::new(Language::PtBr)
                .tr_args("site-count", &args)
                .contains("1.000")
        );
        assert!(
            Catalog::new(Language::En)
                .tr_args("site-count", &args)
                .contains("1,000")
        );
        assert_eq!(group_digits(Language::En, 999), "999");
        assert_eq!(group_digits(Language::En, 1_234_567), "1,234,567");
        assert_eq!(format_number(Language::PtBr, 4.1), "4,1");
        assert_eq!(format_number(Language::En, 1234.5), "1,234.5");
    }

    #[test]
    fn site_count_plurals() {
        let en = Catalog::new(Language::En);
        let pt = Catalog::new(Language::PtBr);
        let count = |c: &Catalog, n: usize| c.tr_args("site-count", &[("count", n.into())]);
        assert_eq!(count(&en, 0), "0 sites");
        assert_eq!(count(&en, 1), "1 site");
        assert_eq!(count(&en, 2), "2 sites");
        assert_eq!(count(&pt, 0), "0 sites");
        assert_eq!(count(&pt, 1), "1 site");
        assert_eq!(count(&pt, 2), "2 sites");
    }

    #[test]
    fn language_is_per_test_thread_and_switches() {
        assert_eq!(language(), Language::En);
        set_language(Language::PtBr);
        assert_eq!(tr("already-running"), "O hoprs já está em execução");
        set_language(Language::En);
        assert_eq!(tr("already-running"), "hoprs is already running");
    }

    #[test]
    fn strings_rs_keys_exist() {
        let source = include_str!("ui/strings.rs");
        let en = variables(EN_SRC);
        let mut found = 0;
        for call in ["tr", "tr_args"] {
            for (at, _) in source.match_indices(call) {
                let before = source[..at].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                // `(`, then optional whitespace (rustfmt wraps long calls), then the quote.
                let rest = source[at + call.len()..].strip_prefix('(');
                let Some(rest) = rest.map(str::trim_start) else {
                    continue;
                };
                let Some(rest) = rest.strip_prefix('"') else {
                    continue;
                };
                let key = &rest[..rest.find('"').expect("closing quote")];
                assert!(
                    en.contains_key(key),
                    "strings.rs uses `{key}`, not in en.ftl"
                );
                found += 1;
            }
        }
        assert!(found >= 4, "found only {found} keys");
    }

    #[test]
    fn locale_mapping() {
        for pt in [
            "pt_BR.UTF-8",
            "pt-BR",
            "pt_PT",
            "pt",
            "PT",
            "pt_BR.UTF-8@euro",
        ] {
            assert_eq!(language_from_locale(Some(pt)), Language::PtBr, "{pt}");
        }
        for other in [
            "en_US.UTF-8",
            "de_DE",
            "C",
            "C.UTF-8",
            "POSIX",
            "",
            "ptx",
            "en",
        ] {
            assert_eq!(language_from_locale(Some(other)), Language::En, "{other}");
        }
        assert_eq!(language_from_locale(None), Language::En);
    }

    #[test]
    fn resolve_fixed_ignores_the_os() {
        use crate::settings::LanguageChoice;
        assert_eq!(
            resolve(LanguageChoice::Fixed(Language::PtBr)),
            Language::PtBr
        );
        assert_eq!(resolve(LanguageChoice::Fixed(Language::En)), Language::En);
    }
}
