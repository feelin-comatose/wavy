//! Язык интерфейса: русский (по умолчанию) или английский.
//! Переводы лежат прямо в коде рядом с русским текстом: `l!("Настройки", "Settings")`,
//! для строк с подстановками — `lf!("Добавлено {n}", "Added {n}")` (как `format!`).
use std::sync::atomic::{AtomicBool, Ordering};

static EN: AtomicBool = AtomicBool::new(false);

/// Включён английский.
pub fn en() -> bool { EN.load(Ordering::Relaxed) }

/// "en" — английский, всё остальное — русский.
pub fn set(lang: &str) { EN.store(lang == "en", Ordering::Relaxed); }

/// Языки для выбора в настройках: (код, название).
pub const LANGS: &[(&str, &str)] = &[("ru", "Русский"), ("en", "English")];

/// Строка на текущем языке.
#[macro_export]
macro_rules! l {
    ($ru:literal, $en:literal) => { if $crate::i18n::en() { $en } else { $ru } };
}

/// `format!` на текущем языке.
#[macro_export]
macro_rules! lf {
    ($ru:literal, $en:literal $(, $a:expr)* $(,)?) => { if $crate::i18n::en() { format!($en $(, $a)*) } else { format!($ru $(, $a)*) } };
}

/// Перевод строк из таблиц-констант (пресеты эквалайзера, хоткеи, умные плейлисты),
/// где `l!` не подходит, потому что он не const.
pub fn tr(ru: &str) -> &str {
    if !en() { return ru; }
    match ru {
        "Ровно" => "Flat",
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
        // сохранённое в библиотеке при импорте файлов без тегов
        "Неизвестный исполнитель" => "Unknown artist",
        _ => ru,
    }
}

#[cfg(test)]
mod tests {
    // Язык — глобальный, а тесты идут параллельно (и сверяют русские тексты), поэтому здесь его не переключаем;
    // английский проверяется скриншотом интерфейса.
    #[test]
    fn russian_by_default() {
        let n = 2;
        assert_eq!(l!("Настройки", "Settings"), "Настройки");
        assert_eq!(lf!("Добавлено {n}", "Added {n}"), "Добавлено 2");
        assert_eq!(super::tr("Бас"), "Бас");
    }
}
