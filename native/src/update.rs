//! Автообновление с GitHub Releases: сборки называются build-N, номер N вшивается в exe
//! при сборке (переменная WAVY_BUILD в GitHub Actions). Локальные сборки не обновляются.
use serde_json::Value;

pub const REPO: &str = "feelin-comatose/wavy";

/// Номер сборки этого exe (None — собран не в GitHub Actions).
pub fn current_build() -> Option<u32> {
    option_env!("WAVY_BUILD").and_then(|s| s.parse().ok())
}

pub struct Release {
    pub build: u32,
    pub url: String,
}

/// Есть ли сборка новее текущей.
pub fn check() -> Result<Option<Release>, String> {
    let Some(cur) = current_build() else { return Ok(None) };
    let mut r = wavy_core::http()
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "wavy-player")
        .call()
        .map_err(|e| format!("не удалось проверить обновления: {e}"))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let build = v["tag_name"].as_str().and_then(|t| t.strip_prefix("build-")).and_then(|n| n.parse::<u32>().ok()).ok_or("непонятный формат релиза")?;
    if build <= cur { return Ok(None); }
    let url = v["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == "Wavy.exe"))
        .and_then(|x| x["browser_download_url"].as_str()).ok_or("в релизе нет Wavy.exe")?.to_string();
    Ok(Some(Release { build, url }))
}

/// Скачать новый exe и подменить им текущий (вступит в силу после перезапуска).
pub fn install(rel: &Release) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let tmp = exe.with_file_name("Wavy.update.exe");
    let resp = wavy_core::http().get(&rel.url).header("User-Agent", "wavy-player").call().map_err(|e| format!("не удалось скачать обновление: {e}"))?;
    let mut f = std::fs::File::create(&tmp).map_err(|e| format!("нет доступа к папке с exe: {e}"))?;
    let n = std::io::copy(&mut resp.into_body().into_reader(), &mut f).map_err(|e| e.to_string())?;
    drop(f);
    if n < 1_000_000 {
        let _ = std::fs::remove_file(&tmp);
        return Err("скачанный файл подозрительно маленький".into());
    }
    let res = self_replace::self_replace(&tmp).map_err(|e| format!("не удалось заменить exe: {e}"));
    let _ = std::fs::remove_file(&tmp);
    res?;
    wavy_core::log(&format!("обновление: установлена сборка {}", rel.build));
    Ok(())
}

/// Запустить свежий exe (после выхода этого процесса сессия восстановится).
pub fn restart() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--restarted").spawn();
    }
}

/// «Что нового» между сборками: заголовки и описания коммитов (без служебных строк).
pub fn changes(from: u32, to: u32) -> Result<Vec<(String, String)>, String> {
    let mut r = wavy_core::http()
        .get(&format!("https://api.github.com/repos/{REPO}/compare/build-{from}...build-{to}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "wavy-player")
        .call()
        .map_err(|e| format!("не удалось загрузить список изменений: {e}"))?;
    let v: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let commits = v["commits"].as_array().ok_or("непонятный ответ GitHub")?;
    Ok(parse_changes(commits.iter().rev().filter_map(|c| c["commit"]["message"].as_str())))
}

fn parse_changes<'a>(msgs: impl Iterator<Item = &'a str>) -> Vec<(String, String)> {
    msgs.filter_map(|m| {
        let mut lines = m.lines();
        let title = lines.next()?.trim().to_string();
        // мелочи сборки людям не интересны
        if title.starts_with("Cargo.lock") || title.starts_with("Merge ") || title.is_empty() { return None; }
        let body: Vec<&str> = lines.map(str::trim_end)
            .filter(|l| !l.starts_with("Co-Authored-By:") && !l.starts_with("Claude-Session:") && !l.starts_with("Signed-off-by:"))
            .collect();
        // абзацы: строки внутри абзаца склеиваем
        let body = body.split(|l| l.trim().is_empty()).map(|p| p.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n");
        Some((title, body))
    }).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn changes_text() {
        let m = "Плеер: ровные кнопки\n\nКнопки: ряд центрировал\nкаждую кнопку.\n\nОбрыв: теперь\nдокачивается.\n\nCo-Authored-By: X <x@y>\nClaude-Session: https://x";
        let v = super::parse_changes([m, "Cargo.lock: lofty"].into_iter());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].0, "Плеер: ровные кнопки");
        assert_eq!(v[0].1, "Кнопки: ряд центрировал каждую кнопку.\n\nОбрыв: теперь докачивается.");
    }
}
