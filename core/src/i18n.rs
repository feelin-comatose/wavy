//! Interface language: Russian (default) or English.
//! Translations live in the code next to the Russian text: `l!("Настройки", "Settings")`,
//! and for strings with arguments `lf!("Добавлено {n}", "Added {n}")` (like `format!`).
use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

/// English is on.
pub fn en() -> bool { EN.load(Ordering::Relaxed) }

/// "en" is English, anything else is Russian.
pub fn set(lang: &str) { EN.store(lang == "en", Ordering::Relaxed); }

/// Languages offered in settings: (code, name).
pub const LANGS: &[(&str, &str)] = &[("ru", "Русский"), ("en", "English")];

/// String in the current language.
#[macro_export]
macro_rules! l {
    ($ru:literal, $en:literal) => { if $crate::i18n::en() { $en } else { $ru } };
}

/// `format!` in the current language.
#[macro_export]
macro_rules! lf {
    ($ru:literal, $en:literal $(, $a:expr)* $(,)?) => { if $crate::i18n::en() { format!($en $(, $a)*) } else { format!($ru $(, $a)*) } };
}

/// Translation of strings from constant tables (EQ presets, hotkeys, smart playlists),
/// where `l!` doesn't work because it isn't const.
pub fn tr(ru: &str) -> &str {
    if !en() { return ru; }
    match ru {
        "Ровно" => "Flat",
        // moods and genres on the "For you" tab (core/src/recs.rs)
        "Расслабиться" => "Relax",
        "Энергия" => "Energize",
        "Тренировка" => "Workout",
        "Хорошее настроение" => "Feel good",
        "Фокус" => "Focus",
        "Вечеринка" => "Party",
        "Грусть" => "Sad",
        "Романтика" => "Romance",
        "Сон" => "Sleep",
        "Дорога" => "Commute",
        "Поп" => "Pop",
        "Хип-хоп" => "Hip-hop",
        "Джаз" => "Jazz",
        "Классика" => "Classical",
        "Метал" => "Metal",
        "Лоу-фай" => "Lo-fi",
        "Аниме" => "Anime",
        // interface themes (native/src/theme.rs)
        "Тёмная" => "Dark",
        "Чёрная (AMOLED)" => "Black (AMOLED)",
        "Светлая" => "Light",
        "Океан" => "Ocean",
        "Сакура" => "Sakura",
        "Лес" => "Forest",
        "Янтарь" => "Amber",
        "Сумерки" => "Twilight",
        "Неон" => "Neon",
        "Храм" => "Shrine",
        "Меха" => "Mecha",
        "Пастель" => "Pastel",
        "Бас" => "Bass",
        "Вокал" => "Vocal",
        "Электроника" => "Electronic",
        "Рок" => "Rock",
        "Мягче верхи" => "Softer highs",
        "Наушники-затычки" => "Earbuds",
        "Пауза / играть" => "Pause / play",
        "Следующий трек" => "Next track",
        "Предыдущий трек" => "Previous track",
        "Громче" => "Volume up",
        "Тише" => "Volume down",
        "Лайк текущему" => "Like the current track",
        "Пропустить опенинг (+85 с)" => "Skip the opening (+85 s)",
        "Мини-плеер" => "Mini player",
        "Оверлей поверх игры" => "Overlay over the game",
        "Закрепить / двигать оверлей" => "Lock / move the overlay",
        "Топ недели" => "Top of the week",
        "Чаще всего за 7 дней" => "Most played in 7 days",
        "Топ месяца" => "Top of the month",
        "Чаще всего за 30 дней" => "Most played in 30 days",
        "На повторе" => "On repeat",
        "Самое слушаемое за всё время" => "Most played of all time",
        "Забытое любимое" => "Forgotten favorites",
        "Лайкнутое, что не играло больше месяца" => "Liked, but not played for over a month",
        "Свежее" => "Fresh",
        "Добавлено за две недели, ещё не слушал" => "Added in the last two weeks, not played yet",
        "Ни разу не слушал" => "Never played",
        "Всё, что ещё ни разу не играло" => "Everything that has never been played",
        // stored in the library when importing files without tags
        "Неизвестный исполнитель" => "Unknown artist",
        _ => ru,
    }
}

#[cfg(test)]
mod tests {
    // The language is global and tests run in parallel (and check Russian text), so it isn't switched here;
    // English is checked with a screenshot of the interface.
    #[test]
    fn russian_by_default() {
        let n = 2;
        assert_eq!(l!("Настройки", "Settings"), "Настройки");
        assert_eq!(lf!("Добавлено {n}", "Added {n}"), "Добавлено 2");
        assert_eq!(super::tr("Бас"), "Бас");
    }
}
