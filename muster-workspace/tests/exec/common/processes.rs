use std::{fs, path::Path};

/// Pids of live processes whose command line names `subcommand` and whose
/// command line or environment mentions `root`.
pub fn processes_under(subcommand: &str, root: &Path) -> Vec<u32> {
    let root = root.to_string_lossy().into_owned();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| alive(*pid))
        .filter(|pid| {
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let environ = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            let cmdline = String::from_utf8_lossy(&cmdline);
            cmdline.contains(subcommand)
                && (cmdline.contains(&root) || String::from_utf8_lossy(&environ).contains(&root))
        })
        .collect()
}

/// Whether the process `pid` exists and has not died unreaped.
pub fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit_once(") ")
            .is_some_and(|(_, rest)| rest.starts_with('Z'))
    })
}
