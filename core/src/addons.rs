//! Addons: separate programs that talk to Wavy through the local API (see docs/addons.md).
//! Each addon is a folder in `data_dir/addons/` with an `addon.json` manifest:
//!   { "name": "Track log", "description": "…", "version": "1.0", "author": "…",
//!     "run": ["python", "main.py"] }
//! Wavy starts enabled addons when it launches (working directory = the addon's folder) and stops
//! them when it quits. They get the API address and token through environment variables:
//!   WAVY_API — http://127.0.0.1:<port>/api/v1    WAVY_TOKEN — the API token    WAVY_ADDON_DIR — their folder
//! Running out of process keeps Wavy's own memory unchanged and an addon crash can't take the player down.
use serde::Deserialize;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    /// command and arguments; relative paths are resolved from the addon's folder
    pub run: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Addon {
    /// folder name — stable id used in settings
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Result<Manifest, String>,
}

pub fn dir() -> PathBuf { crate::data_dir().join("addons") }

/// All addon folders (with a readable or broken manifest), sorted by name.
pub fn scan(root: &Path) -> Vec<Addon> {
    let Ok(rd) = std::fs::read_dir(root) else { return vec![] };
    let mut v: Vec<Addon> = rd.flatten().filter(|e| e.path().is_dir()).filter_map(|e| {
        let dir = e.path();
        let file = dir.join("addon.json");
        if !file.exists() { return None; }
        let manifest = std::fs::read_to_string(&file).map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str::<Manifest>(&s).map_err(|e| lf!("ошибка в addon.json: {e}", "error in addon.json: {e}")))
            .and_then(|m| if m.run.is_empty() { Err(l!("в addon.json не указан run", "addon.json has no \"run\"").into()) } else { Ok(m) });
        Some(Addon { id: e.file_name().to_string_lossy().into_owned(), dir, manifest })
    }).collect();
    v.sort_by_key(|a| a.manifest.as_ref().map(|m| m.name.to_lowercase()).unwrap_or_else(|_| a.id.to_lowercase()));
    v
}

/// 64 random hex characters. Uses the OS-seeded keys of std's RandomState, so no extra dependency.
fn random_hex() -> String {
    (0..4).map(|i| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(crate::now_ms() ^ i);
        format!("{:016x}", h.finish())
    }).collect()
}

/// The API token: created once and kept in `root/api-token`, so addons and scripts can read it from there too.
pub fn token(root: &Path) -> String {
    let f = root.join("api-token");
    if let Ok(t) = std::fs::read_to_string(&f) { let t = t.trim().to_string(); if t.len() >= 32 { return t; } }
    let t = random_hex();
    let _ = std::fs::create_dir_all(root);
    let _ = std::fs::write(&f, &t);
    t
}

/// Running addon processes.
#[derive(Default)]
pub struct Runner {
    procs: HashMap<String, Child>,
    /// last exit message per addon (shown in settings)
    pub exited: HashMap<String, String>,
}

impl Runner {
    pub fn start(&mut self, a: &Addon, api: &str, token: &str) -> Result<(), String> {
        self.stop(&a.id);
        let m = a.manifest.as_ref().map_err(|e| e.clone())?;
        // "main.py" next to addon.json → full path; "python" stays a PATH lookup
        let prog = { let p = a.dir.join(&m.run[0]); if p.exists() { p } else { PathBuf::from(&m.run[0]) } };
        let log = std::fs::File::create(a.dir.join("addon.log")).map_err(|e| e.to_string())?;
        #[allow(unused_mut)]
        let mut c = Command::new(prog);
        c.args(&m.run[1..]).current_dir(&a.dir)
            .env("WAVY_API", api).env("WAVY_TOKEN", token).env("WAVY_ADDON_DIR", &a.dir)
            .stdin(Stdio::null()).stdout(log.try_clone().map_err(|e| e.to_string())?).stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW — no console window for script addons
        }
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            // the addon gets SIGTERM if Wavy dies without stopping it (crash, kill)
            c.pre_exec(|| { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM); Ok(()) });
        }
        let child = c.spawn().map_err(|e| lf!("не запустился «{}»: {e}", "couldn't start \"{}\": {e}", m.run[0]))?;
        #[cfg(windows)]
        kill_with_wavy(&child);
        crate::log(&format!("аддон {} запущен (pid {})", a.id, child.id()));
        self.exited.remove(&a.id);
        self.procs.insert(a.id.clone(), child);
        Ok(())
    }

    pub fn stop(&mut self, id: &str) {
        if let Some(mut c) = self.procs.remove(id) { let _ = c.kill(); let _ = c.wait(); }
    }

    pub fn stop_all(&mut self) {
        let ids: Vec<String> = self.procs.keys().cloned().collect();
        for id in ids { self.stop(&id); }
    }

    pub fn is_running(&self, id: &str) -> bool { self.procs.contains_key(id) }

    /// Notice addons that exited on their own (call now and then).
    pub fn reap(&mut self) {
        let done: Vec<(String, String)> = self.procs.iter_mut().filter_map(|(id, c)| {
            c.try_wait().ok().flatten().map(|st| (id.clone(), match st.code() {
                Some(0) => l!("завершился", "finished").to_string(),
                Some(n) => lf!("упал (код {n}), см. addon.log", "crashed (code {n}), see addon.log"),
                None => l!("остановлен", "stopped").to_string(),
            }))
        }).collect();
        for (id, msg) in done { self.procs.remove(&id); crate::log(&format!("аддон {id}: {msg}")); self.exited.insert(id, msg); }
    }
}

/// Windows: put the addon into a job object that kills its processes when Wavy's handle to it closes,
/// i.e. when Wavy exits for any reason (including a crash), so addons never outlive the player.
#[cfg(windows)]
fn kill_with_wavy(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::*;
    static JOB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() { return 0; }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of_val(&info) as u32);
        job as usize
    });
    if job != 0 { unsafe { AssignProcessToJobObject(job as _, child.as_raw_handle() as _); } }
}

impl Drop for Runner {
    fn drop(&mut self) { self.stop_all(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_token_and_run() {
        let root = std::env::temp_dir().join(format!("wavy-addons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("b-good")).unwrap();
        std::fs::create_dir_all(root.join("a-broken")).unwrap();
        std::fs::create_dir_all(root.join("not-an-addon")).unwrap();
        #[cfg(unix)]
        std::fs::write(root.join("b-good/addon.json"), r#"{"name":"Echo","run":["sh","-c","echo $WAVY_TOKEN > got.txt"],"extra":1}"#).unwrap();
        #[cfg(windows)]
        std::fs::write(root.join("b-good/addon.json"), r#"{"name":"Echo","run":["cmd","/C","echo %WAVY_TOKEN%> got.txt"]}"#).unwrap();
        std::fs::write(root.join("a-broken/addon.json"), "{ nope").unwrap();
        let v = scan(&root);
        assert_eq!(v.len(), 2);
        let good = v.iter().find(|a| a.id == "b-good").unwrap();
        assert_eq!(good.manifest.as_ref().unwrap().name, "Echo");
        assert!(v.iter().find(|a| a.id == "a-broken").unwrap().manifest.is_err());
        // token is stable once created
        let t = token(&root);
        assert_eq!(t.len(), 64);
        assert_eq!(token(&root), t);
        assert_ne!(random_hex(), random_hex());
        // the addon gets the token through the environment and runs in its own folder
        let mut r = Runner::default();
        r.start(good, "http://127.0.0.1:1/api/v1", &t).unwrap();
        for _ in 0..100 { r.reap(); if !r.is_running("b-good") { break; } std::thread::sleep(std::time::Duration::from_millis(50)); }
        assert!(!r.is_running("b-good"));
        assert_eq!(std::fs::read_to_string(root.join("b-good/got.txt")).unwrap().trim(), t);
        assert!(r.exited["b-good"].contains("завершился") || r.exited["b-good"].contains("finished"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
