use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::connection::client::ConnectConfig;
use crate::crypto::fish::decrypt_ini_key;
use crate::text::decode_lossy;

#[derive(Debug, Clone)]
pub struct ConfigServerEntry {
    pub network_id: String,
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    pub password: Option<String>,
    pub nick: String,
    pub altnick: String,
    pub username: String,
    pub realname: String,
    /// The network name from the saved server order, or empty for the default name.
    pub label: String,
    /// Whether Ferric connects this server at startup (from the server order file; default on).
    pub enabled: bool,
}

/// Parses server commands from connectd.mrc (or similar script).
pub fn parse_connectd_mrc<P: AsRef<Path>>(path: P) -> Result<Vec<ConfigServerEntry>, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read mrc file: {}", e))?;
    // UTF-8 (BOM allowed) or Windows-1252, as blow.ini is read: an ANSI connectd.mrc must not fail the load.
    // The BOM is dropped before decoding, as blow.ini is read.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let content = decode_lossy(bytes);
    let mut entries = Vec::new();

    for line in content.lines() {
        let Some(row) = crate::server_list::parse_server_line(line) else {
            continue;
        };
        let (host, port, use_tls) = (row.host, row.port, row.use_tls);
        let (password, nick, altnick, username, realname) = (row.password, row.nick, row.altnick, row.username, row.realname);

        // The label is the first host segment and the user name. Two servers can share it,
        // so a repeated label gets a number; a repeated id would drop the second server.
        let host_label = host.split('.').next().unwrap_or("irc").replace(':', "_");
        let base = format!("{}-{}", host_label, username);
        let mut net_label = base.clone();
        let mut n = 2;
        while entries.iter().any(|e: &ConfigServerEntry| e.network_id == net_label) {
            net_label = format!("{base}-{n}");
            n += 1;
        }

        entries.push(ConfigServerEntry {
            network_id: net_label,
            label: String::new(),
            enabled: true,
            host,
            port,
            use_tls,
            password,
            nick,
            altnick,
            username,
            realname,
        });
    }

    Ok(entries)
}

/// One server's place in the startup order and its network name. These live in
/// `ferric-servers.txt` next to connectd.mrc, so connectd.mrc keeps its lines as they are.
/// A server is matched by host, port and ident; the list order is the startup order.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ServerMeta {
    pub host: String,
    pub port: u16,
    pub ident: String,
    pub name: String,
    /// Whether Ferric connects this server at startup.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// The key that matches a connectd.mrc entry to its `ServerMeta`.
pub fn server_meta_key(host: &str, port: u16, ident: &str) -> String {
    format!("{}:{}:{}", host.to_ascii_lowercase(), port, ident.to_ascii_lowercase())
}

/// Reads the server order file: one server per line, fields separated by tabs as
/// `host`, `port`, `ident`, `name`, `enabled` (1 or 0). A missing file gives an empty list, and a
/// line without the last field counts as enabled.
pub fn load_server_meta<P: AsRef<Path>>(path: P) -> Vec<ServerMeta> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let tab = char::from(9);
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(5, tab);
            let host = parts.next()?.to_string();
            let port = parts.next()?.parse().ok()?;
            let ident = parts.next()?.to_string();
            let name = parts.next().unwrap_or("").to_string();
            let enabled = parts.next().map(|v| v.trim() != "0").unwrap_or(true);
            Some(ServerMeta { host, port, ident, name, enabled })
        })
        .collect()
}

/// Writes the server order file. Tabs and line breaks inside a field become spaces.
pub fn save_server_meta<P: AsRef<Path>>(path: P, servers: &[ServerMeta]) -> Result<(), String> {
    let tab = char::from(9);
    let newline = char::from(10);
    let clean = |s: &str| s.replace([tab, char::from(13), newline], " ");
    let mut text = String::new();
    for s in servers {
        let flag = if s.enabled { "1" } else { "0" };
        text.push_str(&format!("{}{tab}{}{tab}{}{tab}{}{tab}{}", clean(&s.host), s.port, clean(&s.ident), clean(s.name.trim()), flag));
        text.push(newline);
    }
    fs::write(path, text).map_err(|e| format!("Failed to write the server order: {e}"))
}

/// Sorts the entries into the saved order and sets each entry's network name. Entries without a
/// saved place keep their file order after the saved ones.
pub fn apply_server_meta(entries: &mut [ConfigServerEntry], meta: &[ServerMeta]) {
    let mut rank: HashMap<String, usize> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut enabled: HashMap<String, bool> = HashMap::new();
    for (i, m) in meta.iter().enumerate() {
        let key = server_meta_key(&m.host, m.port, &m.ident);
        rank.entry(key.clone()).or_insert(i);
        names.entry(key.clone()).or_insert_with(|| m.name.trim().to_string());
        enabled.entry(key).or_insert(m.enabled);
    }
    for e in entries.iter_mut() {
        let key = server_meta_key(&e.host, e.port, &e.username);
        e.label = names.get(&key).cloned().unwrap_or_default();
        e.enabled = enabled.get(&key).copied().unwrap_or(true);
    }
    entries.sort_by_key(|e| rank.get(&server_meta_key(&e.host, e.port, &e.username)).copied().unwrap_or(usize::MAX));
}

pub(crate) fn parse_server_port(s: &str) -> (String, u16, bool) {
    // A bracketed IPv6 host is "[addr]:port"; a bare IPv6 address has several colons and no port.
    if let Some(rest) = s.strip_prefix('[') {
        let Some((host, after)) = rest.split_once(']') else {
            return (rest.to_string(), 6667, false);
        };
        return match after.strip_prefix(':') {
            Some(p) => {
                let (port, tls) = port_and_tls(p);
                (host.to_string(), port, tls)
            }
            None => (host.to_string(), 6667, false),
        };
    }
    if s.matches(':').count() > 1 {
        return (s.to_string(), 6667, false);
    }
    match s.split_once(':') {
        Some((h, p)) => {
            let (port, tls) = port_and_tls(p);
            (h.to_string(), port, tls)
        }
        None => (s.to_string(), 6667, false),
    }
}

/// "+6697" is TLS on port 6697; a port that does not parse falls back to 6667, as before.
fn port_and_tls(p: &str) -> (u16, bool) {
    let (digits, tls) = match p.strip_prefix('+') {
        Some(d) => (d, true),
        None => (p, false),
    };
    (digits.parse::<u16>().unwrap_or(6667), tls)
}

/// Reads blow.ini as bytes and decodes them leniently: UTF-8 (BOM allowed), else Windows-1252.
fn read_blow_ini_text(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Failed to read blow.ini: {}", e))?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(decode_lossy(bytes))
}

/// Replaces `path` with `data` atomically: writes a temp file in the same folder, syncs it, then
/// renames it over the original. The temp file is removed on failure.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let name = path.file_name().ok_or_else(|| "Failed to write blow.ini: invalid path".to_string())?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(name);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Ok(dir) = fs::File::open(parent) {
                    let _ = dir.sync_all();
                }
            }
        }
        Ok::<(), std::io::Error>(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to write blow.ini: {}", e));
    }
    Ok(())
}

/// Parses all Blowfish keys from blow.ini and decrypts them.
/// Returns a map of:
/// target_lowercase (e.g. "#demo", "#merex5") -> decrypted_key
pub fn parse_blow_ini<P: AsRef<Path>>(path: P) -> Result<HashMap<String, String>, String> {
    let _read = BLOW_INI_LOCK.read().unwrap_or_else(|e| e.into_inner());
    let content = read_blow_ini_text(path.as_ref())?;
    let mut keys = HashMap::new();
    let mut unscoped_candidates: HashMap<String, Option<String>> = HashMap::new();
    let mut current_target: Option<(String, String)> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(';') || trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = &trimmed[1..trimmed.len() - 1];
            // Format can be [Network:#channel] or [#channel] or [Nick]
            let target = if let Some((_, chan)) = section.split_once(':') {
                chan
            } else {
                section
            };
            current_target = Some((target.to_lowercase(), section.to_lowercase()));
            continue;
        }

        if let Some((target, section)) = &current_target {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case("key") {
                    let enc_key = v.trim();
                    let decrypted = decrypt_ini_key(enc_key);
                    keys.insert(section.clone(), decrypted.clone());
                    // Only a bare [target] section gives the unscoped fallback. A [net:target] section stays
                    // with its network: find_fish_key tries the network names before the bare target.
                    if section == target {
                        match unscoped_candidates.entry(target.clone()) {
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                entry.insert(Some(decrypted));
                            }
                            std::collections::hash_map::Entry::Occupied(mut entry) => {
                                if entry.get().as_deref() != Some(decrypted.as_str()) {
                                    entry.insert(None);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // A target-only fallback is safe only if every matching section uses the
    // same key. Otherwise a missing network name must not select another
    // network's key by file order.
    for (target, candidate) in unscoped_candidates {
        if let Some(key) = candidate {
            keys.insert(target, key);
        }
    }

    Ok(keys)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlowEntry {
    pub section: String,
    pub target: String,
    pub key: String,
    pub date: String,
}

pub fn current_date_str() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = secs / 86400;
    let mut year = 1970;
    let mut remaining_days = days as i64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if remaining_days < days_in_year {
            let days_in_months: [i64; 12] = if leap {
                [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            } else {
                [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
            };
            let mut month = 1;
            for &dim in &days_in_months {
                if remaining_days < dim {
                    return format!("{:02}/{:02}/{}", remaining_days + 1, month, year);
                }
                remaining_days -= dim;
                month += 1;
            }
            return format!("01/01/{}", year);
        }
        remaining_days -= days_in_year;
        year += 1;
    }
}

/// Lists all blow.ini entries with their section, target, decrypted key and date.
pub fn list_blow_ini_entries<P: AsRef<Path>>(path: P) -> Result<Vec<BlowEntry>, String> {
    let _read = BLOW_INI_LOCK.read().unwrap_or_else(|e| e.into_inner());
    let content = read_blow_ini_text(path.as_ref())?;
    let mut entries = Vec::new();
    let mut current_section: Option<String> = None;
    let mut current_target: Option<String> = None;
    let mut current_key: Option<String> = None;
    let mut current_date: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(';') || trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let (Some(sec), Some(tgt), Some(k)) = (current_section.take(), current_target.take(), current_key.take()) {
                entries.push(BlowEntry {
                    section: sec,
                    target: tgt,
                    key: k,
                    date: current_date.take().unwrap_or_default(),
                });
            } else {
                current_section = None;
                current_target = None;
                current_key = None;
                current_date = None;
            }

            let section = trimmed[1..trimmed.len() - 1].to_string();
            if section.eq_ignore_ascii_case("FiSH") || section.eq_ignore_ascii_case("Test") {
                continue;
            }

            let target = if let Some((_, chan)) = section.split_once(':') {
                chan.to_string()
            } else {
                section.clone()
            };

            current_section = Some(section);
            current_target = Some(target);
            continue;
        }

        if current_section.is_some() {
            if let Some((k, v)) = trimmed.split_once('=') {
                let k_trim = k.trim();
                let v_trim = v.trim();
                if k_trim.eq_ignore_ascii_case("key") {
                    let decrypted = decrypt_ini_key(v_trim);
                    current_key = Some(decrypted);
                } else if k_trim.eq_ignore_ascii_case("date") {
                    current_date = Some(v_trim.to_string());
                }
            }
        }
    }

    if let (Some(sec), Some(tgt), Some(k)) = (current_section, current_target, current_key) {
        entries.push(BlowEntry {
            section: sec,
            target: tgt,
            key: k,
            date: current_date.unwrap_or_default(),
        });
    }

    Ok(entries)
}

/// Guards blow.ini inside the process. A save or delete (a read-modify-write) takes it for writing,
/// so two saves cannot drop each other's section. Reads take it for reading: on Windows a rename
/// over a file another thread has open fails, so a reader must not overlap the write.
static BLOW_INI_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Saves or updates a key in blow.ini with FiSH encryption (+OK <enc>) and timestamp.
/// `section` must equal an existing section name (case-insensitive) to update it; otherwise a new
/// `[section]` is appended. `target` and `net:target` are different sections.
pub fn save_blow_ini_key<P: AsRef<Path>>(path: P, section: &str, plain_key: &str) -> Result<(), String> {
    let _write = BLOW_INI_LOCK.write().unwrap_or_else(|e| e.into_inner());
    let p = path.as_ref();
    let content = if p.exists() {
        read_blow_ini_text(p)?
    } else {
        String::from("[FiSH]\r\nprocess_incoming=1\r\nprocess_outgoing=1\r\n\r\n")
    };

    let encrypted = crate::crypto::fish::encrypt_ini_key(plain_key);
    let date_str = current_date_str();

    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let match_sec = section.trim().to_lowercase();
    let mut found_idx: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let sec_name = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            if sec_name == match_sec {
                found_idx = Some(i);
                break;
            }
        }
    }

    if let Some(idx) = found_idx {
        let mut j = idx + 1;
        let mut key_updated = false;
        let mut date_updated = false;

        while j < lines.len() && !lines[j].trim().starts_with('[') {
            let trimmed = lines[j].trim();
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case("key") {
                    lines[j] = format!("key={}", encrypted);
                    key_updated = true;
                } else if k.trim().eq_ignore_ascii_case("date") {
                    lines[j] = format!("date={}", date_str);
                    date_updated = true;
                }
            }
            j += 1;
        }

        if !key_updated {
            lines.insert(idx + 1, format!("key={}", encrypted));
            j += 1;
        }
        if !date_updated {
            lines.insert(j, format!("date={}", date_str));
        }
    } else {
        if !lines.is_empty() && !lines.last().unwrap().is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("[{}]", section.trim()));
        lines.push(format!("key={}", encrypted));
        lines.push(format!("date={}", date_str));
    }

    let new_content = lines.join("\r\n") + "\r\n";
    write_atomic(p, new_content.as_bytes())
}

/// Deletes the section named exactly `section` (case-insensitive) from blow.ini. A bare `target`
/// matches only `[target]`, never `[net:target]`. Returns whether a section was removed.
pub fn delete_blow_ini_key<P: AsRef<Path>>(path: P, section: &str) -> Result<bool, String> {
    let _write = BLOW_INI_LOCK.write().unwrap_or_else(|e| e.into_inner());
    let p = path.as_ref();
    if !p.exists() {
        return Ok(false);
    }
    let content = read_blow_ini_text(p)?;
    let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let match_sec = section.trim().to_lowercase();

    let mut new_lines = Vec::new();
    let mut skipping = false;
    let mut deleted = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let sec_name = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
            if sec_name == match_sec {
                skipping = true;
                deleted = true;
                continue;
            } else {
                skipping = false;
            }
        }

        if !skipping {
            new_lines.push(line);
        }
    }

    if deleted {
        let new_content = new_lines.join("\r\n") + "\r\n";
        write_atomic(p, new_content.as_bytes())?;
    }

    Ok(deleted)
}

impl From<ConfigServerEntry> for ConnectConfig {
    fn from(e: ConfigServerEntry) -> Self {
        Self {
            network: e.network_id,
            host: e.host,
            port: e.port,
            use_tls: e.use_tls,
            nick: e.nick,
            username: e.username,
            realname: e.realname,
            password: e.password,
            altnick: Some(e.altnick).filter(|a| !a.is_empty()),
            sasl: None,
            tls_fingerprint: None,
            allow_self_signed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_connectd() {
        if let Some(path) = std::env::var_os("FERRIC_TEST_CONNECTD_MRC") {
            let servers = parse_connectd_mrc(path).unwrap();
            assert!(!servers.is_empty());
            assert!(servers.iter().all(|server| !server.host.is_empty()));
        }
    }

    #[test]
    fn test_parse_blow_ini() {
        if let Some(path) = std::env::var_os("FERRIC_TEST_BLOW_INI") {
            let keys = parse_blow_ini(path).unwrap();
            assert!(!keys.is_empty());
        }
    }

    #[test]
    fn test_list_and_save_blow_ini() {
        let temp_dir = std::env::temp_dir();
        let test_ini = temp_dir.join("test_blow.ini");
        let _ = fs::remove_file(&test_ini);

        save_blow_ini_key(&test_ini, "#testchan", "supersecret123").unwrap();
        save_blow_ini_key(&test_ini, "MyNet:#other", "otherpass456").unwrap();

        let entries = list_blow_ini_entries(&test_ini).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "supersecret123");
        assert_eq!(entries[1].key, "otherpass456");

        delete_blow_ini_key(&test_ini, "#testchan").unwrap();
        let entries2 = list_blow_ini_entries(&test_ini).unwrap();
        assert_eq!(entries2.len(), 1);
        assert_eq!(entries2[0].target, "#other");

        let _ = fs::remove_file(&test_ini);
    }
    #[test]
    fn a_scoped_section_gives_no_unscoped_fallback() {
        let dir = unique_dir("scoped_fallback");
        let ini = dir.join("blow.ini");
        save_blow_ini_key(&ini, "net-a:#chat", "keyA").unwrap();
        save_blow_ini_key(&ini, "net-b:#chat", "keyB").unwrap();
        let keys = parse_blow_ini(&ini).unwrap();
        assert!(!keys.contains_key("#chat"), "{keys:?}");
        assert!(keys.contains_key("net-a:#chat") && keys.contains_key("net-b:#chat"));
        save_blow_ini_key(&ini, "#bare", "keyC").unwrap();
        assert!(parse_blow_ini(&ini).unwrap().contains_key("#bare"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_saves_keep_every_section() {
        let ini = unique_dir("blow_lock").join("blow.ini");
        let writers: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|tag| {
                let ini = ini.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        save_blow_ini_key(&ini, &format!("#{tag}{i}"), "key").unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(sections(&ini).len(), 50);
    }

    #[test]
    fn ipv6_hosts_are_not_cut_at_their_colons() {
        assert_eq!(parse_server_port("2001:db8::1"), ("2001:db8::1".to_string(), 6667, false));
        assert_eq!(parse_server_port("[2001:db8::1]:+6697"), ("2001:db8::1".to_string(), 6697, true));
        assert_eq!(parse_server_port("irc.example.net:6667"), ("irc.example.net".to_string(), 6667, false));
    }

    fn unique_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ferric_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sections(path: &Path) -> Vec<String> {
        list_blow_ini_entries(path).unwrap().into_iter().map(|e| e.section).collect()
    }

    #[test]
    fn delete_and_save_match_the_exact_section_only() {
        let dir = unique_dir("exact");
        let ini = dir.join("blow.ini");
        save_blow_ini_key(&ini, "NetA:#chan", "keyA").unwrap();
        save_blow_ini_key(&ini, "NetB:#chan", "keyB").unwrap();
        save_blow_ini_key(&ini, "#chan", "bare").unwrap();

        // A bare target saves into its own section, not into NetA:#chan.
        let entries = list_blow_ini_entries(&ini).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries.iter().find(|e| e.section == "NetA:#chan").unwrap().key, "keyA");

        // Deleting the bare target leaves the network-scoped sections alone.
        assert!(delete_blow_ini_key(&ini, "#CHAN").unwrap());
        assert_eq!(sections(&ini), vec!["NetA:#chan", "NetB:#chan"]);
        assert!(!delete_blow_ini_key(&ini, "#chan").unwrap());
        assert!(!delete_blow_ini_key(&ini, "chan").unwrap());

        // Exact, case-insensitive section name works.
        assert!(delete_blow_ini_key(&ini, "neta:#CHAN").unwrap());
        assert_eq!(sections(&ini), vec!["NetB:#chan"]);

        // A missing file is not an error.
        assert!(!delete_blow_ini_key(dir.join("missing.ini"), "#chan").unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_are_atomic_and_leave_no_temp_files() {
        let dir = unique_dir("atomic");
        let ini = dir.join("blow.ini");
        save_blow_ini_key(&ini, "#a", "one").unwrap();
        save_blow_ini_key(&ini, "#b", "two").unwrap();
        delete_blow_ini_key(&ini, "#a").unwrap();
        let names: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("blow.ini")]);
        assert_eq!(sections(&ini), vec!["#b"]);

        // A failed write (target is a directory) reports an error and cleans up its temp file.
        let blocked = dir.join("blocked");
        fs::create_dir(&blocked).unwrap();
        assert!(write_atomic(&blocked, b"x").is_err());
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_utf8_blow_ini_is_read_lossily() {
        let dir = unique_dir("lossy");
        let ini = dir.join("blow.ini");
        // Section name with a Windows-1252 byte (0xE9 = e-acute), and a BOM-less ANSI comment.
        let mut bytes = b"; caf\xE9\r\n[Net:#caf\xE9]\r\nkey=".to_vec();
        bytes.extend_from_slice(crate::crypto::fish::encrypt_ini_key("secret").as_bytes());
        bytes.extend_from_slice(b"\r\ndate=01/01/2024\r\n");
        fs::write(&ini, bytes).unwrap();

        let entries = list_blow_ini_entries(&ini).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].section, "Net:#caf\u{e9}");
        let keys = parse_blow_ini(&ini).unwrap();
        assert_eq!(keys.get("net:#caf\u{e9}").map(String::as_str), Some("secret"));
        // Saving into it keeps working.
        save_blow_ini_key(&ini, "Net:#caf\u{e9}", "newer").unwrap();
        assert_eq!(list_blow_ini_entries(&ini).unwrap()[0].key, "newer");
        let _ = fs::remove_dir_all(&dir);
    }

}

#[cfg(test)]
mod network_id_tests {
    use super::*;

    #[test]
    fn two_servers_with_the_same_label_get_different_ids() {
        let path = std::env::temp_dir().join("ferric_connectd_ids_test.mrc");
        fs::write(
            &path,
            "server -m irc.libera.chat:6697 -i ferric ferric ferric@ Ferric User\n\
             server -m irc.efnet.org:6667 -i ferric ferric ferric@ Ferric User\n",
        )
        .unwrap();
        let servers = parse_connectd_mrc(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(servers.len(), 2);
        assert_ne!(servers[0].network_id, servers[1].network_id);
    }
}

#[cfg(test)]
mod realname_tests {
    use super::*;

    #[test]
    fn alternative_nick_is_not_the_user_name_and_the_real_name_is_kept() {
        let path = std::env::temp_dir().join("ferric_connectd_real_test.mrc");
        fs::write(&path, "server -m irc.libera.chat:6697 -i ferric alt ferric@ Real Name Here\n").unwrap();
        let servers = parse_connectd_mrc(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(servers[0].nick, "ferric");
        assert_eq!(servers[0].username, "ferric");
        assert_eq!(servers[0].realname, "Real Name Here");
    }
}

/// The key of a server's trust-on-first-use pin: its host and port.
pub fn tls_pin_key(host: &str, port: u16) -> String {
    format!("{}:{}", host.to_ascii_lowercase(), port)
}

static TLS_PINS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn read_tls_pins_unlocked(path: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let (key, fingerprint) = line.split_once('\t')?;
            Some((key.to_string(), fingerprint.trim().to_ascii_lowercase()))
        })
        .collect()
}

/// Reads the TLS pins in `ferric-tls-pins.txt`: one server per line, `host:port`, a tab, then the
/// SHA-256 fingerprint in hex. A missing file gives no pins.
pub fn load_tls_pins<P: AsRef<Path>>(path: P) -> HashMap<String, String> {
    let _guard = TLS_PINS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_tls_pins_unlocked(path.as_ref())
}

/// Stores the fingerprint pinned for a server, replacing an earlier pin for it.
pub fn save_tls_pin<P: AsRef<Path>>(path: P, key: &str, fingerprint: &str) -> Result<(), String> {
    let path = path.as_ref();
    let _guard = TLS_PINS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut pins = read_tls_pins_unlocked(path);
    pins.insert(key.to_string(), fingerprint.to_ascii_lowercase());
    let mut lines: Vec<String> = pins.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
    lines.sort();
    write_atomic(path, lines.concat().as_bytes())
}

#[cfg(test)]
mod server_meta_tests {
    use super::*;

    fn entry(host: &str, ident: &str) -> ConfigServerEntry {
        ConfigServerEntry {
            network_id: format!("{host}-{ident}"),
            label: String::new(),
            enabled: true,
            host: host.into(),
            port: 6667,
            use_tls: false,
            password: None,
            nick: "n".into(),
            altnick: "n_".into(),
            username: ident.into(),
            realname: "r".into(),
        }
    }

    #[test]
    fn saved_order_and_names_apply_and_unknown_servers_keep_file_order() {
        let mut entries = vec![entry("a.example", "x"), entry("b.example", "y"), entry("c.example", "z")];
        let meta = vec![
            ServerMeta { host: "c.example".into(), port: 6667, ident: "z".into(), name: "Third".into(), enabled: true },
            ServerMeta { host: "a.example".into(), port: 6667, ident: "x".into(), name: String::new(), enabled: false },
        ];
        apply_server_meta(&mut entries, &meta);
        let hosts: Vec<&str> = entries.iter().map(|e| e.host.as_str()).collect();
        assert_eq!(hosts, ["c.example", "a.example", "b.example"]);
        assert_eq!(entries[0].label, "Third");
        assert_eq!(entries[1].label, "");
        assert_eq!(entries[2].label, "");
        assert!(entries[0].enabled);
        assert!(!entries[1].enabled);
        assert!(entries[2].enabled);
    }

    #[test]
    fn order_file_round_trips_and_a_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("ferric-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ferric-servers.txt");
        let meta = vec![ServerMeta { host: "h.example".into(), port: 6697, ident: "i".into(), name: "Net name".into(), enabled: false }];
        save_server_meta(&path, &meta).unwrap();
        assert_eq!(load_server_meta(&path), meta);
        assert!(load_server_meta(dir.join("missing.txt")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tls_pins_round_trip_and_a_new_pin_replaces_the_old() {
        let dir = std::env::temp_dir().join(format!("ferric-pins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ferric-tls-pins.txt");
        assert!(load_tls_pins(&path).is_empty());
        save_tls_pin(&path, &tls_pin_key("Chill.Example", 59000), "AB12").unwrap();
        save_tls_pin(&path, "other:1", "cd34").unwrap();
        save_tls_pin(&path, "chill.example:59000", "EF56").unwrap();
        let pins = load_tls_pins(&path);
        assert_eq!(pins.len(), 2);
        assert_eq!(pins.get("chill.example:59000").map(String::as_str), Some("ef56"));
        assert_eq!(pins.get("other:1").map(String::as_str), Some("cd34"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
