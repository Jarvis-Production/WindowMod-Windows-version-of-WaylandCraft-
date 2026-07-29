use std::path::Path;

use walkdir::WalkDir;

use super::shortcut::resolve_lnk;

#[derive(Clone)]
pub struct DesktopApp {
    pub app_id: String,
    pub name: Option<String>,
    pub generic_name: Option<String>,
    /// Full executable path of the shortcut target (correct UTF-16 path,
    /// resolved by the Windows shell — NOT the fragile `lnk` crate).
    pub exec: Option<String>,
    /// Command-line arguments stored on the shortcut (e.g. Discord's
    /// `--processStart Discord.exe`). Empty when the shortcut has none.
    pub exec_args: Option<String>,
    /// Working directory the shortcut wants the target launched in.
    pub working_dir: Option<String>,
    pub exec_terminal: bool,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    pub visible: bool,
    pub icon_path: Option<String>,
}


pub fn load_start_menu_apps() -> Vec<DesktopApp> {
    let mut apps = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for base in start_menu_dirs() {
        eprintln!("[windowmod] Scanning start menu: {:?}", base);
        for entry in WalkDir::new(&base).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lnk") {
                continue;
            }
            if let Some(app) = parse_lnk_file(path) {
                if seen.insert(app.app_id.clone()) {
                    apps.push(app);
                }
            }
        }
    }

    for builtin in builtin_apps() {
        if seen.insert(builtin.app_id.clone()) {
            apps.push(builtin);
        }
    }

    apps.sort_by(|a, b| {
        a.name
            .as_deref()
            .unwrap_or(&a.app_id)
            .to_lowercase()
            .cmp(&b.name.as_deref().unwrap_or(&b.app_id).to_lowercase())
    });
    apps
}

/// Parse a Windows `.lnk` shortcut into a launchable [`DesktopApp`].
///
/// We resolve shortcuts with the Windows shell (`IShellLinkW`, in
/// `shortcut::resolve_lnk`) rather than the `lnk` crate. The crate panicked on
/// most real shortcuts and, worse, corrupted non-ASCII target paths (a user
/// folder `C:\Users\Макс\...` came back as `C:\Users\????\...`), so launching
/// failed with "path not found". The shell API returns the real target path,
/// arguments and working directory in correct UTF-16 — exactly what we need to
/// CreateProcessW the app directly onto the hidden desktop (no console window,
/// no leaking onto the user's visible desktop).
pub fn parse_lnk_file(path: &Path) -> Option<DesktopApp> {
    let stem = path.file_stem().and_then(|s| s.to_str())?.to_string();
    let app_id = format!("{stem}-{}", path.display());

    // Ask the Windows shell to resolve the shortcut. This never panics and
    // produces correct Unicode paths/arguments.
    let resolved = resolve_lnk(path);

    let (exec, args, work_dir, icon) = match resolved {
        Some(r) if !r.target.is_empty() => {
            let icon = r.icon.filter(|p| Path::new(p).exists());
            let mut target = r.target;
            let mut arguments = r.arguments;
            let mut work_dir_raw = r.working_dir;

            // Squirrel-launcher rewrite (Discord, GitHub Desktop, Slack, many
            // Electron apps): the shortcut points at `Update.exe` with
            // `--processStart <App>.exe`. `Update.exe` RE-LAUNCHES the real app
            // through a broker that does NOT inherit our hidden desktop, so the
            // window ends up on the VISIBLE desktop parked off-screen at
            // (-32000,-32000). Chromium/Electron then ignore synthesized input
            // for that off-screen background window — the app shows as a static
            // picture you cannot click. Launching the REAL exe directly instead
            // keeps it on the hidden desktop (where input works like Notepad).
            //
            // We detect this pattern and rewrite exec -> <working_dir>\<App>.exe
            // (the Squirrel layout places the app exe in the version folder that
            // the shortcut already sets as the working directory).
            if target.to_lowercase().ends_with("update.exe") {
                if let Some(app_exe) = squirrel_process_start(&arguments) {
                    // The real app exe lives in one of a few well-known places
                    // for a Squirrel install. We try them in order and use the
                    // first that exists:
                    //   1) directly in the shortcut's working directory,
                    //   2) directly next to Update.exe (the install root),
                    //   3) inside the NEWEST `app-<version>` sub-folder under the
                    //      install root (the canonical Squirrel layout — Discord
                    //      ships `Discord.exe` in `Discord\app-1.0.xxxx\`, NOT in
                    //      the root, which is why the previous working-dir-only
                    //      lookup failed and Update.exe ran, sending the real
                    //      window onto the VISIBLE desktop — "Discord won't open").
                    let update_dir = Path::new(&target)
                        .parent()
                        .map(|p| p.to_path_buf());

                    let mut resolved: Option<std::path::PathBuf> = None;

                    let in_workdir = Path::new(&work_dir_raw).join(&app_exe);
                    if in_workdir.exists() {
                        resolved = Some(in_workdir);
                    }
                    if resolved.is_none() {
                        if let Some(dir) = &update_dir {
                            let in_root = dir.join(&app_exe);
                            if in_root.exists() {
                                resolved = Some(in_root);
                            }
                        }
                    }
                    if resolved.is_none() {
                        if let Some(dir) = &update_dir {
                            resolved = newest_squirrel_app_exe(dir, &app_exe);
                        }
                    }

                    if let Some(candidate) = resolved {
                        eprintln!(
                            "[windowmod] Squirrel launcher detected; launching real exe {:?} instead of Update.exe",
                            candidate,
                        );
                        // Launch from the exe's own version folder so Electron
                        // finds its resources; set the working dir accordingly.
                        if let Some(parent) = candidate.parent() {
                            work_dir_raw = parent.to_string_lossy().into_owned();
                        }
                        target = candidate.to_string_lossy().into_owned();
                        arguments = String::new(); // real exe needs no Update args
                    } else {
                        eprintln!(
                            "[windowmod] Squirrel launcher: could not locate {:?} near {:?} — falling back to Update.exe",
                            app_exe, update_dir,
                        );
                    }
                }
            }

            let args = if arguments.trim().is_empty() {
                None
            } else {
                Some(arguments)
            };
            let work_dir = if work_dir_raw.trim().is_empty() {
                None
            } else {
                Some(work_dir_raw)
            };
            (Some(target), args, work_dir, icon)
        }

        // The shell could not resolve a target (rare: broken shortcut). Skip it
        // rather than guessing — listing an unlaunchable entry only confuses.
        _ => {
            eprintln!("[windowmod] could not resolve shortcut target for {:?}", path);
            return None;
        }
    };

    Some(DesktopApp {
        app_id,
        name: Some(stem.clone()),
        generic_name: None,
        exec,
        exec_args: args,
        working_dir: work_dir,
        exec_terminal: false,
        comment: None,
        keywords: vec![stem.to_lowercase()],
        categories: vec!["Other".into()],
        visible: true,
        icon_path: icon,
    })
}


/// Locate the Squirrel app exe inside the NEWEST `app-<version>` sub-folder
/// under `root` (the Update.exe install directory). Squirrel keeps each
/// installed version in its own `app-1.0.xxxx` folder; the highest version
/// folder is the current one. We pick the lexicographically greatest
/// `app-*` directory name that actually contains `app_exe`.
fn newest_squirrel_app_exe(root: &Path, app_exe: &str) -> Option<std::path::PathBuf> {
    let mut best: Option<(String, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.to_lowercase().starts_with("app-") {
            continue;
        }
        let candidate = path.join(app_exe);
        if !candidate.exists() {
            continue;
        }
        let name_owned = name.to_string();
        // Keep the version folder with the greatest name (newest version).
        match &best {
            Some((best_name, _)) if *best_name >= name_owned => {}
            _ => best = Some((name_owned, candidate)),
        }
    }
    best.map(|(_, path)| path)
}

/// Extract the application exe name from a Squirrel `Update.exe` argument
/// string. Squirrel shortcuts look like `--processStart Discord.exe` or
/// `--processStart=Discord.exe` (sometimes followed by `--process-start-args`).
/// Returns the exe file name (e.g. `Discord.exe`) if present.
fn squirrel_process_start(arguments: &str) -> Option<String> {
    let tokens: Vec<&str> = arguments.split_whitespace().collect();
    for (i, tok) in tokens.iter().enumerate() {
        // `--processStart=Discord.exe`
        if let Some(rest) = tok.strip_prefix("--processStart=") {
            let name = rest.trim_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
        // `--processStart Discord.exe` (value in the next token)
        if *tok == "--processStart" || *tok == "--processStartAndWait" {
            if let Some(next) = tokens.get(i + 1) {
                let name = next.trim_matches('"');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn start_menu_dirs() -> Vec<std::path::PathBuf> {

    let mut dirs = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        dirs.push(std::path::PathBuf::from(appdata).join("Microsoft/Windows/Start Menu/Programs"));
    }
    if let Ok(programdata) = std::env::var("ProgramData") {
        dirs.push(
            std::path::PathBuf::from(programdata)
                .join("Microsoft/Windows/Start Menu/Programs"),
        );
    }
    dirs
}

fn builtin_apps() -> Vec<DesktopApp> {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    vec![
        make_builtin(
            "notepad",
            "Notepad",
            format!("{windir}\\System32\\notepad.exe"),
            "Utility",
        ),
        make_builtin(
            "calc",
            "Calculator",
            format!("{windir}\\System32\\calc.exe"),
            "Utility",
        ),
        make_builtin(
            "cmd",
            "Command Prompt",
            format!("{windir}\\System32\\cmd.exe"),
            "System",
        ),
        make_builtin(
            "mspaint",
            "Paint",
            format!("{windir}\\System32\\mspaint.exe"),
            "Graphics",
        ),
        // Windows File Explorer. Lets the user browse the whole filesystem,
        // manage files (copy/move/delete/rename) and launch ANY program or game
        // by double-clicking it — all rendered inside the mod.
        //
        // `explorer.exe` is special: launching it bare usually does NOT create a
        // new process. Instead it hands the request to the ALREADY-RUNNING
        // system Explorer (the shell), whose window then opens on the user's
        // VISIBLE desktop — not the hidden desktop the mod captures from. The
        // `/separate` switch forces Explorer to spin up its OWN new process for
        // this window, so it inherits our hidden desktop (lpDesktop) like every
        // other moded app and its window can be found and adopted. We open it on
        // "This PC" (the shell folder) so the user lands on a drive list.
        make_builtin_with_args(
            "explorer",
            "File Explorer",
            format!("{windir}\\explorer.exe"),
            // ::{20D04FE0-3AEA-1069-A2D8-08002B30309D} is the "This PC" shell
            // folder CLSID — a stable entry point that always exists.
            Some("/separate,::{20D04FE0-3AEA-1069-A2D8-08002B30309D}".into()),
            "System",
        ),
    ]
}

fn make_builtin(id: &str, name: &str, exec: String, category: &str) -> DesktopApp {
    make_builtin_with_args(id, name, exec, None, category)
}

fn make_builtin_with_args(
    id: &str,
    name: &str,
    exec: String,
    exec_args: Option<String>,
    category: &str,
) -> DesktopApp {
    DesktopApp {
        app_id: id.into(),
        name: Some(name.into()),
        generic_name: None,
        exec: Some(exec),
        exec_args,
        working_dir: None,
        exec_terminal: false,
        comment: None,
        keywords: vec![name.to_lowercase()],
        categories: vec![category.into()],
        visible: true,
        icon_path: None,
    }
}

pub fn find_app<'a>(apps: &'a [DesktopApp], app_id: &str) -> Option<&'a DesktopApp> {
    apps.iter().find(|a| a.app_id == app_id)
}

pub fn to_raw(entry: &DesktopApp) -> RawDesktopEntry {
    RawDesktopEntry {
        app_id: entry.app_id.clone(),
        name: entry.name.clone(),
        generic_name: entry.generic_name.clone(),
        exec: entry.exec.clone(),
        exec_terminal: entry.exec_terminal,
        comment: entry.comment.clone(),
        keywords: entry.keywords.clone(),
        categories: entry.categories.clone(),
        visible: entry.visible,
        icon_path: entry.icon_path.clone(),
    }
}

pub struct RawDesktopEntry {
    pub app_id: String,
    pub name: Option<String>,
    pub generic_name: Option<String>,
    pub exec: Option<String>,
    pub exec_terminal: bool,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    pub visible: bool,
    pub icon_path: Option<String>,
}
