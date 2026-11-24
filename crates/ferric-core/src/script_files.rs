//! Script file access for the script editor.
//!
//! Lists, reads, writes and creates `*.mrc` files inside one folder. Nothing
//! here depends on Tauri, so all rules (name validation, containment, size
//! limit, conflict detection, encoding and line ending round trip, atomic
//! write with a backup) can be unit tested directly.
//!
//! The module never logs or prints file contents. Error messages name the file
//! but never quote its text.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

/// Largest script file that can be read or written.
pub const MAX_SCRIPT_BYTES: u64 = 5 * 1024 * 1024;
const MAX_NAME_LEN: usize = 128;
pub const EXTENSIONS: &[&str] = &[".mrc", ".py"];

/// Returns true if the file name has a `.py` extension (case-insensitive).
pub fn is_python_script(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".py")
}

// ---------------------------------------------------------------------------
// Errors

/// Error returned to the frontend as `{ code, message }`.
///
/// Codes: `invalid_name`, `not_found`, `exists`, `too_large`, `conflict`,
/// `encoding`, `unsupported`, `io`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptError {
    pub code: &'static str,
    pub message: String,
}

impl ScriptError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn io(context: &str, error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::NotFound {
            return Self::new("not_found", format!("{}: the file or folder does not exist.", context));
        }
        Self::new("io", format!("{}: {}", context, error))
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ScriptError {}

// ---------------------------------------------------------------------------
// Name validation and path containment

/// Accepts only a plain `*.mrc` file name: no separators, no `..`, no drive
/// or stream syntax (`:`), no wildcard or control characters, no Windows
/// device names. Pure function.
pub fn validate_script_name(name: &str) -> Result<(), ScriptError> {
    let bad = |message: &str| Err(ScriptError::new("invalid_name", message));

    if name.is_empty() {
        return bad("Enter a file name.");
    }
    if name.len() > MAX_NAME_LEN {
        return bad("The file name is too long.");
    }
    if name != name.trim() {
        return bad("The file name must not start or end with a space.");
    }
    if name.contains("..") {
        return bad("The file name must not contain \"..\".");
    }
    for c in name.chars() {
        if c.is_control() {
            return bad("The file name contains a control character.");
        }
        match c {
            '/' | '\\' => return bad("Use a plain file name without folders."),
            ':' => return bad("Drive letters and stream names (\":\") are not allowed."),
            '*' | '?' | '"' | '<' | '>' | '|' => {
                return bad("The file name contains a character that is not allowed.")
            }
            _ => {}
        }
    }
    let lower = name.to_ascii_lowercase();
    let Some(ext) = EXTENSIONS.iter().find(|ext| lower.ends_with(**ext)) else {
        return bad("Only .mrc and .py script files can be used.");
    };
    let stem = &name[..name.len() - ext.len()];
    if stem.is_empty() {
        return bad("Enter a name before the extension.");
    }
    if stem.ends_with('.') || stem.ends_with(' ') {
        return bad("The name must not end with a dot or a space.");
    }
    let device = stem.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((device.starts_with("COM") || device.starts_with("LPT"))
            && device.len() == 4
            && device.as_bytes()[3].is_ascii_digit()
            && device.as_bytes()[3] != b'0');
    if reserved {
        return bad("This name is reserved by the operating system.");
    }
    Ok(())
}

fn is_inside(root: &Path, candidate: &Path) -> bool {
    candidate.parent() == Some(root) && candidate.starts_with(root)
}

/// Validates `name` and returns the full path inside `dir`. The folder is
/// canonicalised first; an existing file must be a regular file (not a
/// symlink) that still resolves to a direct child of the folder.
pub fn resolve_script_path(dir: &Path, name: &str) -> Result<PathBuf, ScriptError> {
    validate_script_name(name)?;
    let root = dir
        .canonicalize()
        .map_err(|e| ScriptError::io("The script folder is not available", e))?;
    let candidate = root.join(name);
    if !is_inside(&root, &candidate) {
        return Err(ScriptError::new(
            "invalid_name",
            "The file is outside the script folder.",
        ));
    }
    match fs::symlink_metadata(&candidate) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(ScriptError::new(
                    "invalid_name",
                    "Links are not followed; the file must be a regular file in the script folder.",
                ));
            }
            if !meta.is_file() {
                return Err(ScriptError::new("invalid_name", "The name does not point to a file."));
            }
            let real = candidate
                .canonicalize()
                .map_err(|e| ScriptError::io("Cannot resolve the file", e))?;
            if !is_inside(&root, &real) {
                return Err(ScriptError::new(
                    "invalid_name",
                    "The file is outside the script folder.",
                ));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(ScriptError::io("Cannot access the file", e)),
    }
    Ok(candidate)
}

// ---------------------------------------------------------------------------
// Conflict logic

/// `expected` is the modification time (ms) the editor saw when it loaded the
/// file, `None` meaning "the file did not exist". `current` is the time on
/// disk now, `None` meaning "missing". Anything but an exact match is a
/// conflict. Pure function.
pub fn check_conflict(expected: Option<u64>, current: Option<u64>) -> Result<(), ScriptError> {
    match (expected, current) {
        (Some(e), Some(c)) if e == c => Ok(()),
        (None, None) => Ok(()),
        (Some(_), Some(_)) => Err(ScriptError::new(
            "conflict",
            "The file changed on disk after it was opened.",
        )),
        (Some(_), None) => Err(ScriptError::new(
            "conflict",
            "The file was deleted or renamed after it was opened.",
        )),
        (None, Some(_)) => Err(ScriptError::new(
            "conflict",
            "The file already exists on disk.",
        )),
    }
}

// ---------------------------------------------------------------------------
// Text encoding and line endings

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TextEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-8-bom")]
    Utf8Bom,
    #[serde(rename = "windows-1252")]
    Windows1252,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LineEnding {
    #[serde(rename = "crlf")]
    Crlf,
    #[serde(rename = "lf")]
    Lf,
}

const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

pub(crate) fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x80..=0x9F).contains(&b) {
                CP1252_HIGH[(b - 0x80) as usize]
            } else {
                b as char
            }
        })
        .collect()
}

fn encode_windows_1252(text: &str) -> Result<Vec<u8>, ScriptError> {
    let mut out = Vec::with_capacity(text.len());
    let mut line = 1usize;
    for c in text.chars() {
        let code = c as u32;
        if code < 0x80 || (0xA0..=0xFF).contains(&code) {
            out.push(code as u8);
        } else if let Some(index) = CP1252_HIGH.iter().position(|&h| h == c) {
            out.push(0x80 + index as u8);
        } else {
            return Err(ScriptError::new(
                "encoding",
                format!(
                    "This file is stored as Windows-1252 and cannot hold the character U+{:04X} (line {}). Remove it to save.",
                    code, line
                ),
            ));
        }
        if c == '\n' {
            line += 1;
        }
    }
    Ok(out)
}

/// Decodes as UTF-8 (with or without BOM) and falls back to Windows-1252.
fn decode_text(bytes: &[u8], name: &str) -> Result<(String, TextEncoding), ScriptError> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(ScriptError::new(
            "unsupported",
            format!("{} is stored as UTF-16, which the editor does not support.", name),
        ));
    }
    if bytes.contains(&0) {
        return Err(ScriptError::new(
            "unsupported",
            format!("{} contains binary data and cannot be opened as text.", name),
        ));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        if let Ok(text) = std::str::from_utf8(rest) {
            return Ok((text.to_string(), TextEncoding::Utf8Bom));
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), TextEncoding::Utf8)),
        Err(_) => Ok((decode_windows_1252(bytes), TextEncoding::Windows1252)),
    }
}

fn encode_text(text: &str, encoding: TextEncoding) -> Result<Vec<u8>, ScriptError> {
    match encoding {
        TextEncoding::Utf8 => Ok(text.as_bytes().to_vec()),
        TextEncoding::Utf8Bom => {
            let mut out = Vec::with_capacity(text.len() + 3);
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            out.extend_from_slice(text.as_bytes());
            Ok(out)
        }
        TextEncoding::Windows1252 => encode_windows_1252(text),
    }
}

/// Returns the dominant line ending and whether the file mixes styles (or
/// contains a lone CR). A file without any line break defaults to CRLF.
pub fn detect_line_ending(text: &str) -> (LineEnding, bool) {
    let bytes = text.as_bytes();
    let (mut crlf, mut lf, mut lone_cr) = (0usize, 0usize, 0usize);
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\n' => {
                if i > 0 && bytes[i - 1] == b'\r' {
                    crlf += 1;
                } else {
                    lf += 1;
                }
            }
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => lone_cr += 1,
            _ => {}
        }
    }
    if crlf == 0 && lf == 0 {
        return (LineEnding::Crlf, lone_cr > 0);
    }
    let eol = if crlf >= lf { LineEnding::Crlf } else { LineEnding::Lf };
    (eol, (crlf > 0 && lf > 0) || lone_cr > 0)
}

fn to_lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn apply_line_ending(text: &str, eol: LineEnding) -> String {
    let lf = to_lf(text);
    match eol {
        LineEnding::Lf => lf,
        LineEnding::Crlf => lf.replace('\n', "\r\n"),
    }
}

// ---------------------------------------------------------------------------
// Store

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptFileInfo {
    pub name: String,
    pub size: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub mtime: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptContent {
    pub name: String,
    /// Text with `\n` line breaks. The original style is in `eol`.
    pub content: String,
    pub mtime: u64,
    pub size: u64,
    pub encoding: TextEncoding,
    pub eol: LineEnding,
    /// The file mixed line ending styles; saving writes the dominant one.
    pub mixed_eol: bool,
}

#[derive(Debug, Clone, Copy)]
struct FileMeta {
    encoding: TextEncoding,
    eol: LineEnding,
    backed_up: bool,
}

fn modified_ms(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn current_mtime(path: &Path) -> Result<Option<u64>, ScriptError> {
    match fs::metadata(path) {
        Ok(meta) => Ok(Some(modified_ms(&meta))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ScriptError::io("Cannot read the file state", e)),
    }
}

fn read_limited(path: &Path, name: &str) -> Result<(Vec<u8>, u64), ScriptError> {
    let file = fs::File::open(path).map_err(|e| ScriptError::io(&format!("Cannot open {}", name), e))?;
    let meta = file.metadata().map_err(|e| ScriptError::io(&format!("Cannot open {}", name), e))?;
    if meta.len() > MAX_SCRIPT_BYTES {
        return Err(ScriptError::new(
            "too_large",
            format!("{} is larger than the 5 MB limit.", name),
        ));
    }
    // Read at most one byte past the limit, so a file that grows after the size check is refused too.
    let mut bytes = Vec::new();
    file.take(MAX_SCRIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ScriptError::io(&format!("Cannot read {}", name), e))?;
    if bytes.len() as u64 > MAX_SCRIPT_BYTES {
        return Err(ScriptError::new(
            "too_large",
            format!("{} is larger than the 5 MB limit.", name),
        ));
    }
    Ok((bytes, modified_ms(&meta)))
}

/// Script folder access plus the per-session memory of each file's encoding,
/// line ending style and backup state.
pub struct ScriptStore {
    dir: PathBuf,
    meta: Mutex<HashMap<String, FileMeta>>,
}

impl ScriptStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            meta: Mutex::new(HashMap::new()),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, FileMeta>> {
        self.meta.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `*.mrc` files directly in the folder (no recursion), sorted by name.
    pub fn list(&self) -> Result<Vec<ScriptFileInfo>, ScriptError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ScriptError::io("Cannot list the script folder", e)),
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else { continue };
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else { continue };
            if validate_script_name(&name).is_err() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            files.push(ScriptFileInfo {
                name,
                size: meta.len(),
                mtime: modified_ms(&meta),
            });
        }
        files.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(files)
    }

    pub fn read(&self, name: &str) -> Result<ScriptContent, ScriptError> {
        let path = resolve_script_path(&self.dir, name)?;
        let (bytes, mtime) = read_limited(&path, name)?;
        let (text, encoding) = decode_text(&bytes, name)?;
        let (eol, mixed_eol) = detect_line_ending(&text);
        let mut meta = self.lock();
        let backed_up = meta
            .get(&name.to_lowercase())
            .map(|m| m.backed_up)
            .unwrap_or(false);
        meta.insert(
            name.to_lowercase(),
            FileMeta {
                encoding,
                eol,
                backed_up,
            },
        );
        Ok(ScriptContent {
            name: name.to_string(),
            content: to_lf(&text),
            mtime,
            size: bytes.len() as u64,
            encoding,
            eol,
            mixed_eol,
        })
    }

    /// Writes `content` atomically. `expected_mtime` is the time reported by
    /// the last read or write; a different time on disk is a conflict and the
    /// file is left untouched. Returns the new modification time.
    pub fn write(
        &self,
        name: &str,
        content: &str,
        expected_mtime: Option<u64>,
    ) -> Result<u64, ScriptError> {
        self.write_with_backup(name, content, expected_mtime, true)
    }

    /// Like `write`. With `backup` false the `<name>.bak` copy is skipped and
    /// the file stays eligible, so the first overwrite after the option is
    /// turned back on copies the then-current file.
    pub fn write_with_backup(
        &self,
        name: &str,
        content: &str,
        expected_mtime: Option<u64>,
        backup: bool,
    ) -> Result<u64, ScriptError> {
        let mut meta_map = self.lock();
        let path = resolve_script_path(&self.dir, name)?;
        let current = current_mtime(&path)?;
        check_conflict(expected_mtime, current)?;

        let key = name.to_lowercase();
        let mut meta = match meta_map.get(&key) {
            Some(meta) => *meta,
            None => {
                // Not read in this session: keep whatever style is on disk.
                let detected = if current.is_some() {
                    read_limited(&path, name)
                        .and_then(|(bytes, _)| decode_text(&bytes, name))
                        .ok()
                        .map(|(text, encoding)| (encoding, detect_line_ending(&text).0))
                } else {
                    None
                };
                let (encoding, eol) = detected.unwrap_or((TextEncoding::Utf8, LineEnding::Crlf));
                FileMeta {
                    encoding,
                    eol,
                    backed_up: false,
                }
            }
        };

        let bytes = encode_text(&apply_line_ending(content, meta.eol), meta.encoding)?;
        if bytes.len() as u64 > MAX_SCRIPT_BYTES {
            return Err(ScriptError::new(
                "too_large",
                "The text is larger than the 5 MB limit and was not saved.",
            ));
        }

        if backup && current.is_some() && !meta.backed_up {
            // `path` came from resolve_script_path, so the backup sits next to it.
            let backup_path = path.with_file_name(format!("{}.bak", name));
            if let Ok(info) = fs::symlink_metadata(&backup_path) {
                if info.file_type().is_symlink() || !info.is_file() {
                    return Err(ScriptError::new(
                        "io",
                        "The backup name is occupied by something that is not a regular file.",
                    ));
                }
            }
            let old_len = fs::metadata(&path)
                .map_err(|e| ScriptError::io("Cannot read the file state", e))?
                .len();
            if old_len > MAX_SCRIPT_BYTES {
                return Err(ScriptError::new(
                    "too_large",
                    format!("{} is larger than the 5 MB limit and was not backed up or saved.", name),
                ));
            }
            // fs::copy replaces an older backup of the same name.
            fs::copy(&path, &backup_path)
                .map_err(|e| ScriptError::io("Cannot create the backup copy", e))?;
            meta.backed_up = true;
        }

        write_atomic(&self.dir, &path, name, &bytes)?;
        let new_mtime = current_mtime(&path)?.unwrap_or(0);
        meta_map.insert(key, meta);
        Ok(new_mtime)
    }

    /// Creates an empty script file. Fails when the name already exists.
    pub fn create(&self, name: &str) -> Result<ScriptFileInfo, ScriptError> {
        let mut meta_map = self.lock();
        // The folder must exist before the name is resolved: resolving canonicalises the folder.
        fs::create_dir_all(&self.dir)
            .map_err(|e| ScriptError::io("Cannot create the script folder", e))?;
        let path = resolve_script_path(&self.dir, name)?;
        let file = fs::OpenOptions::new().write(true).create_new(true).open(&path);
        match file {
            Ok(file) => {
                let _ = file.sync_all();
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(ScriptError::new(
                    "exists",
                    format!("{} already exists.", name),
                ))
            }
            Err(e) => return Err(ScriptError::io("Cannot create the file", e)),
        }
        meta_map.insert(
            name.to_lowercase(),
            FileMeta {
                encoding: TextEncoding::Utf8,
                eol: LineEnding::Crlf,
                backed_up: true,
            },
        );
        let meta = fs::metadata(&path).map_err(|e| ScriptError::io("Cannot read the new file", e))?;
        Ok(ScriptFileInfo {
            name: name.to_string(),
            size: meta.len(),
            mtime: modified_ms(&meta),
        })
    }
}

/// Temp file in the same folder, flushed to disk, then renamed over the target.
fn write_atomic(dir: &Path, target: &Path, name: &str, bytes: &[u8]) -> Result<(), ScriptError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let parent = target.parent().unwrap_or(dir);
    let temp = parent.join(format!(".{}.{}.{}.tmp", name, std::process::id(), stamp));
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(ScriptError::io("Cannot write the file", e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "ferric-script-test-{}-{}",
                std::process::id(),
                n
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn code<T: std::fmt::Debug>(r: Result<T, ScriptError>) -> &'static str {
        r.expect_err("expected an error").code
    }

    // --- name validation ---

    #[test]
    fn accepts_plain_mrc_names() {
        for name in ["alias.mrc", "My Script.MRC", "über.mrc", "a.b.mrc", "x-1_2.mrc", "bot.py", "My Script.PY", "helper.py"] {
            assert!(validate_script_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_bad_names() {
        let long = format!("{}.mrc", "a".repeat(200));
        let cases = [
            "",
            ".mrc",
            ".py",
            "../x.mrc",
            "..\\x.mrc",
            "../x.py",
            "a/b.mrc",
            "a\\b.py",
            "/etc/x.mrc",
            "C:x.mrc",
            "C:\\x.mrc",
            "x.mrc:stream",
            "x.mrc::$DATA",
            "blow.ini",
            "connectd.txt",
            "x.mrc.bak",
            "x.mrc ",
            " x.mrc",
            "x..mrc",
            "x. .mrc",
            "x .mrc",
            "a*.mrc",
            "a?.mrc",
            "a|b.mrc",
            "a\"b.mrc",
            "a<b>.mrc",
            "a\nb.mrc",
            "a\0b.mrc",
            "CON.mrc",
            "nul.mrc",
            "CON.py",
            "nul.py",
            "Com1.mrc",
            "lpt9.mrc",
            "aux.txt.mrc",
            long.as_str(),
        ];
        for name in cases {
            let err = validate_script_name(name).expect_err(name);
            assert_eq!(err.code, "invalid_name", "{name:?}");
        }
        assert!(validate_script_name("com0.mrc").is_ok());
        assert!(validate_script_name("console.mrc").is_ok());
    }

    #[test]
    fn resolves_inside_folder_only() {
        let tmp = TempDir::new();
        let path = resolve_script_path(&tmp.0, "a.mrc").unwrap();
        assert_eq!(path.parent().unwrap(), tmp.0.canonicalize().unwrap());
        assert_eq!(code(resolve_script_path(&tmp.0, "../a.mrc")), "invalid_name");
        assert_eq!(code(resolve_script_path(&tmp.0, "sub/a.mrc")), "invalid_name");
    }

    #[test]
    fn directory_with_mrc_name_is_rejected() {
        let tmp = TempDir::new();
        fs::create_dir(tmp.0.join("dir.mrc")).unwrap();
        assert_eq!(code(resolve_script_path(&tmp.0, "dir.mrc")), "invalid_name");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_out_of_folder_is_rejected() {
        let tmp = TempDir::new();
        let outside = TempDir::new();
        fs::write(outside.0.join("secret.mrc"), b"x").unwrap();
        std::os::unix::fs::symlink(outside.0.join("secret.mrc"), tmp.0.join("link.mrc")).unwrap();
        assert_eq!(code(resolve_script_path(&tmp.0, "link.mrc")), "invalid_name");
        let store = ScriptStore::new(tmp.0.clone());
        assert!(store.list().unwrap().is_empty());
        assert_eq!(code(store.read("link.mrc")), "invalid_name");
    }

    // --- conflict logic ---

    #[test]
    fn conflict_matrix() {
        assert!(check_conflict(Some(5), Some(5)).is_ok());
        assert!(check_conflict(None, None).is_ok());
        assert_eq!(code(check_conflict(Some(5), Some(6))), "conflict");
        assert_eq!(code(check_conflict(Some(6), Some(5))), "conflict");
        assert_eq!(code(check_conflict(Some(5), None)), "conflict");
        assert_eq!(code(check_conflict(None, Some(5))), "conflict");
    }

    // --- encoding and line endings ---

    #[test]
    fn windows_1252_round_trip() {
        let bytes: Vec<u8> = (0u8..=255).filter(|b| *b != 0).collect();
        let (text, enc) = decode_text(&bytes, "t.mrc").unwrap();
        assert_eq!(enc, TextEncoding::Windows1252);
        assert_eq!(encode_text(&text, enc).unwrap(), bytes);
        assert_eq!(decode_windows_1252(&[0x80, 0xE4, 0x99]), "\u{20AC}\u{E4}\u{2122}");
    }

    #[test]
    fn unencodable_character_is_an_error_not_a_conversion() {
        let err = encode_text("ok\nj\u{65E5}", TextEncoding::Windows1252).unwrap_err();
        assert_eq!(err.code, "encoding");
        assert!(err.message.contains("line 2"));
    }

    #[test]
    fn utf8_and_bom_detection() {
        let (t, e) = decode_text("h\u{E4}llo".as_bytes(), "t.mrc").unwrap();
        assert_eq!((t.as_str(), e), ("h\u{E4}llo", TextEncoding::Utf8));
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice(b"abc");
        let (t, e) = decode_text(&with_bom, "t.mrc").unwrap();
        assert_eq!((t.as_str(), e), ("abc", TextEncoding::Utf8Bom));
        assert_eq!(encode_text("abc", e).unwrap(), with_bom);
    }

    #[test]
    fn utf16_and_binary_are_refused() {
        assert_eq!(code(decode_text(&[0xFF, 0xFE, 0x61, 0x00], "t.mrc")), "unsupported");
        assert_eq!(code(decode_text(b"ab\0cd", "t.mrc")), "unsupported");
    }

    #[test]
    fn line_ending_detection_and_apply() {
        assert_eq!(detect_line_ending("a\r\nb\r\n"), (LineEnding::Crlf, false));
        assert_eq!(detect_line_ending("a\nb\n"), (LineEnding::Lf, false));
        assert_eq!(detect_line_ending("a\r\nb\nc\n"), (LineEnding::Lf, true));
        assert_eq!(detect_line_ending("a\r\nb\r\nc\n"), (LineEnding::Crlf, true));
        assert_eq!(detect_line_ending("single line"), (LineEnding::Crlf, false));
        assert_eq!(detect_line_ending("a\rb\n"), (LineEnding::Lf, true));
        assert_eq!(apply_line_ending("a\nb\r\nc", LineEnding::Crlf), "a\r\nb\r\nc");
        assert_eq!(apply_line_ending("a\nb\r\nc", LineEnding::Lf), "a\nb\nc");
    }

    // --- store ---

    #[test]
    fn list_only_returns_mrc_files_in_the_folder() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("b.mrc"), b"x").unwrap();
        fs::write(tmp.0.join("A.MRC"), b"xy").unwrap();
        fs::write(tmp.0.join("blow.ini"), b"secret").unwrap();
        fs::write(tmp.0.join("b.mrc.bak"), b"x").unwrap();
        fs::create_dir(tmp.0.join("sub")).unwrap();
        fs::write(tmp.0.join("sub").join("c.mrc"), b"x").unwrap();
        fs::create_dir(tmp.0.join("d.mrc")).unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let names: Vec<_> = store.list().unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["A.MRC", "b.mrc"]);
        assert_eq!(code(store.read("blow.ini")), "invalid_name");
        assert_eq!(code(store.write("blow.ini", "x", None)), "invalid_name");
        assert_eq!(code(store.create("blow.ini")), "invalid_name");
    }

    #[test]
    fn missing_folder_lists_nothing() {
        let store = ScriptStore::new(std::env::temp_dir().join("ferric-script-test-missing-dir"));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn unchanged_round_trip_is_byte_identical() {
        let tmp = TempDir::new();
        let originals: Vec<(&str, Vec<u8>)> = vec![
            ("crlf.mrc", b"alias a {\r\n  echo 1\r\n}\r\n".to_vec()),
            ("lf.mrc", b"alias a {\n  echo 1\n}\n".to_vec()),
            ("ansi.mrc", vec![b';', b' ', 0xE4, 0x80, b'\r', b'\n', b'x']),
            ("utf8.mrc", "; h\u{E4}llo \u{20AC}\r\n".as_bytes().to_vec()),
            ("bom.mrc", [&[0xEF, 0xBB, 0xBF][..], b"a\r\nb"].concat()),
        ];
        let store = ScriptStore::new(tmp.0.clone());
        for (name, bytes) in &originals {
            fs::write(tmp.0.join(name), bytes).unwrap();
            let read = store.read(name).unwrap();
            assert!(!read.content.contains('\r'), "{name}");
            store.write(name, &read.content, Some(read.mtime)).unwrap();
            assert_eq!(&fs::read(tmp.0.join(name)).unwrap(), bytes, "{name}");
        }
        let ansi = store.read("ansi.mrc").unwrap();
        assert_eq!(ansi.encoding, TextEncoding::Windows1252);
        assert_eq!(ansi.eol, LineEnding::Crlf);
        assert_eq!(store.read("lf.mrc").unwrap().eol, LineEnding::Lf);
    }

    #[test]
    fn write_keeps_encoding_and_line_endings_after_edit() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("a.mrc"), [b'a', 0xE4, b'\n', b'b']).unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let read = store.read("a.mrc").unwrap();
        assert_eq!(read.content, "a\u{E4}\nb");
        store.write("a.mrc", "a\u{E4}\nb\n\u{20AC}", Some(read.mtime)).unwrap();
        assert_eq!(
            fs::read(tmp.0.join("a.mrc")).unwrap(),
            vec![b'a', 0xE4, b'\n', b'b', b'\n', 0x80]
        );
        let err = store.write("a.mrc", "x\u{65E5}", Some(store.read("a.mrc").unwrap().mtime)).unwrap_err();
        assert_eq!(err.code, "encoding");
    }

    #[test]
    fn conflict_leaves_file_untouched() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("a.mrc"), b"one").unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let read = store.read("a.mrc").unwrap();
        let err = store.write("a.mrc", "two", Some(read.mtime + 1)).unwrap_err();
        assert_eq!(err.code, "conflict");
        assert_eq!(fs::read(tmp.0.join("a.mrc")).unwrap(), b"one");
        assert!(!tmp.0.join("a.mrc.bak").exists());
        // Overwrite flow: take the current time from the list, then retry.
        let current = store.list().unwrap()[0].mtime;
        store.write("a.mrc", "two", Some(current)).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc")).unwrap(), b"two");
    }

    #[test]
    fn deleted_file_is_a_conflict() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("a.mrc"), b"one").unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let read = store.read("a.mrc").unwrap();
        fs::remove_file(tmp.0.join("a.mrc")).unwrap();
        assert_eq!(code(store.write("a.mrc", "x", Some(read.mtime))), "conflict");
        assert!(!tmp.0.join("a.mrc").exists());
    }

    #[test]
    fn backup_is_made_once_per_session_before_first_overwrite() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("a.mrc"), b"original").unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let read = store.read("a.mrc").unwrap();
        let m1 = store.write("a.mrc", "second", Some(read.mtime)).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc.bak")).unwrap(), b"original");
        store.write("a.mrc", "third", Some(m1)).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc.bak")).unwrap(), b"original");
        assert_eq!(fs::read(tmp.0.join("a.mrc")).unwrap(), b"third");
        // The backup never shows up in the list or can be opened.
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(code(store.read("a.mrc.bak")), "invalid_name");
    }

    #[test]
    fn backup_replaces_an_older_bak_and_a_new_session_backs_up_again() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("a.mrc"), b"original").unwrap();
        fs::write(tmp.0.join("a.mrc.bak"), b"stale").unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let read = store.read("a.mrc").unwrap();
        let m1 = store.write("a.mrc", "second", Some(read.mtime)).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc.bak")).unwrap(), b"original");
        assert!(tmp.0.join("a.mrc.bak").parent() == tmp.0.join("a.mrc").parent());
        store.write("a.mrc", "third", Some(m1)).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc.bak")).unwrap(), b"original");
        // A fresh store is a new session: the backup is replaced once more.
        let again = ScriptStore::new(tmp.0.clone());
        let read = again.read("a.mrc").unwrap();
        again.write("a.mrc", "fourth", Some(read.mtime)).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc.bak")).unwrap(), b"third");
    }

    #[test]
    fn backup_off_skips_the_copy_until_turned_on() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("a.mrc"), b"original").unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let read = store.read("a.mrc").unwrap();
        let m1 = store.write_with_backup("a.mrc", "second", Some(read.mtime), false).unwrap();
        assert!(!tmp.0.join("a.mrc.bak").exists());
        store.write_with_backup("a.mrc", "third", Some(m1), true).unwrap();
        assert_eq!(fs::read(tmp.0.join("a.mrc.bak")).unwrap(), b"second");
    }

    #[test]
    fn oversized_old_file_is_not_backed_up_or_overwritten() {
        let tmp = TempDir::new();
        fs::write(tmp.0.join("huge.mrc"), vec![b'a'; MAX_SCRIPT_BYTES as usize + 1]).unwrap();
        let store = ScriptStore::new(tmp.0.clone());
        let mtime = store.list().unwrap()[0].mtime;
        assert_eq!(code(store.write("huge.mrc", "x", Some(mtime))), "too_large");
        assert!(!tmp.0.join("huge.mrc.bak").exists());
        assert_eq!(fs::metadata(tmp.0.join("huge.mrc")).unwrap().len(), MAX_SCRIPT_BYTES + 1);
    }

    #[test]
    fn no_temp_files_are_left_behind() {
        let tmp = TempDir::new();
        let store = ScriptStore::new(tmp.0.clone());
        let info = store.create("n.mrc").unwrap();
        store.write("n.mrc", "alias x { }", Some(info.mtime)).unwrap();
        let mut names: Vec<_> = fs::read_dir(&tmp.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["n.mrc"]);
    }

    #[test]
    fn create_fails_when_the_file_exists() {
        let tmp = TempDir::new();
        let store = ScriptStore::new(tmp.0.clone());
        let info = store.create("new.mrc").unwrap();
        assert_eq!(info.size, 0);
        assert_eq!(code(store.create("new.mrc")), "exists");
        #[cfg(windows)]
        assert_eq!(code(store.create("NEW.mrc")), "exists");
        // A new file starts as CRLF UTF-8 and is not backed up while empty.
        store.write("new.mrc", "a\nb", Some(info.mtime)).unwrap();
        assert_eq!(fs::read(tmp.0.join("new.mrc")).unwrap(), b"a\r\nb");
    }

    #[test]
    fn size_limit_applies_to_read_and_write() {
        let tmp = TempDir::new();
        let store = ScriptStore::new(tmp.0.clone());
        let info = store.create("big.mrc").unwrap();
        let big = "a".repeat(MAX_SCRIPT_BYTES as usize + 1);
        assert_eq!(code(store.write("big.mrc", &big, Some(info.mtime))), "too_large");
        assert_eq!(fs::metadata(tmp.0.join("big.mrc")).unwrap().len(), 0);
        fs::write(tmp.0.join("huge.mrc"), vec![b'a'; MAX_SCRIPT_BYTES as usize + 1]).unwrap();
        assert_eq!(code(store.read("huge.mrc")), "too_large");
        let ok = "a".repeat(MAX_SCRIPT_BYTES as usize);
        assert!(store.write("big.mrc", &ok, Some(info.mtime)).is_ok());
    }
}
