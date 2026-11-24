import { escapeHtml } from "./utils/ircColors.js";

// Script editor window: file list, text editor with a synchronized highlight
// layer, toolbar and status line. The textarea owns the text and the caret.
// A transparent-text textarea sits above a layer that only contains the rows
// that are visible (plus a small buffer), so a 5000 line file costs the same
// per keystroke as a 50 line one. All listeners are delegated on the root.

const LH = 16; // row height in px, must match --se-lh in app.css
const PAD_Y = 4;
const PAD_X = 8;
const OVERSCAN = 24;
const TAB_SIZE = 4;
const MAX_HL_LINE = 4000; // longer lines are shown without highlighting
const MAX_RENDER_LINE = 20000;
const LAST_FILE_KEY = "ferric-script-last";
const LINE_NUMBERS_KEY = "ferric-script-line-numbers";
const BACKUP_KEY = "ferric-script-backup";

// ---------------------------------------------------------------------------
// Highlighting

const KEYWORDS = new Set([
  "alias", "on", "raw", "ctcp", "menu", "dialog", "if", "elseif", "else", "while",
  "return", "goto", "var", "set", "halt", "break", "continue", "unset",
]);

const RE_CMD = /\/{1,2}[A-Za-z_.][\w.]*/y;
const RE_ID = /\$\$?(?:\+|\d+-?\d*|[A-Za-z_]\w*(?:\.\w+)*|(?=\())/y;
const RE_VAR = /%{1,2}\.?[A-Za-z_]\w*(?:\.\w+)*/y;
const RE_GROUP = /#[^\s:,()|{}]*/y;
const RE_WORD = /[A-Za-z_]\w*/y;
const RE_ON_LEVEL = /\s+[^\s:]*:/y;
const RE_EVENT = /[A-Za-z]+/y;
const RE_RAW_NUM = /\s+(?:\d+|\*)/y;
const RE_PY_WORD = /[a-zA-Z_][a-zA-Z0-9_]*/y;
const RE_PY_DECORATOR = /@[a-zA-Z_][a-zA-Z0-9_.]*/y;
const PY_KEYWORDS = new Set([
  "def", "class", "if", "elif", "else", "for", "while", "return",
  "import", "from", "as", "try", "except", "finally", "with", "in",
  "is", "not", "and", "or", "None", "True", "False", "lambda",
  "pass", "break", "continue", "raise", "yield", "async", "await",
  "global", "nonlocal", "del", "assert"
]);

const ESC_MAP = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#039;" };
function esc(s) {
  return s.replace(/[&<>"']/g, (c) => ESC_MAP[c]);
}

function matchAt(re, text, pos) {
  re.lastIndex = pos;
  const m = re.exec(text);
  return m ? m[0] : null;
}

export function highlightPythonLine(text) {
  const n = text.length;
  if (n === 0) return "";
  if (n > MAX_HL_LINE) return esc(text.slice(0, MAX_RENDER_LINE));

  let out = "";
  let plain = 0;
  let i = 0;

  const emit = (cls, from, to) => {
    if (from > plain) out += esc(text.slice(plain, from));
    out += `<span class="${cls}">${esc(text.slice(from, to))}</span>`;
    plain = to;
  };

  while (i < n) {
    const ch = text.charCodeAt(i);

    if (ch === 32 || ch === 9) {
      i++;
      continue;
    }

    if (ch === 35) {
      emit("se-c", i, n);
      i = n;
      break;
    }

    if (ch === 64) {
      const m = matchAt(RE_PY_DECORATOR, text, i);
      if (m) {
        emit("se-cmd", i, i + m.length);
        i += m.length;
        continue;
      }
    }

    let strPrefixLen = 0;
    if (ch === 34 || ch === 39) {
      strPrefixLen = 0;
    } else if (
      (ch === 114 || ch === 82 || ch === 102 || ch === 70 || ch === 98 || ch === 66 || ch === 117 || ch === 85) &&
      i + 1 < n && (text.charCodeAt(i + 1) === 34 || text.charCodeAt(i + 1) === 39)
    ) {
      strPrefixLen = 1;
    } else if (
      i + 2 < n &&
      ((ch === 114 || ch === 82) && (text.charCodeAt(i + 1) === 102 || text.charCodeAt(i + 1) === 70) ||
       (ch === 102 || ch === 70) && (text.charCodeAt(i + 1) === 114 || text.charCodeAt(i + 1) === 82)) &&
      (text.charCodeAt(i + 2) === 34 || text.charCodeAt(i + 2) === 39)
    ) {
      strPrefixLen = 2;
    }

    const quoteIdx = i + strPrefixLen;
    if (quoteIdx < n && (text.charCodeAt(quoteIdx) === 34 || text.charCodeAt(quoteIdx) === 39)) {
      const q = text.charAt(quoteIdx);
      const isTriple = (quoteIdx + 2 < n && text.charAt(quoteIdx + 1) === q && text.charAt(quoteIdx + 2) === q);
      const qDelim = isTriple ? q + q + q : q;
      let end = quoteIdx + qDelim.length;
      let escaped = false;
      while (end < n) {
        if (!escaped && text.startsWith(qDelim, end)) {
          end += qDelim.length;
          break;
        }
        if (text.charCodeAt(end) === 92) {
          escaped = !escaped;
        } else {
          escaped = false;
        }
        end++;
      }
      emit("se-id", i, end);
      i = end;
      continue;
    }

    if ((ch >= 65 && ch <= 90) || (ch >= 97 && ch <= 122) || ch === 95) {
      const word = matchAt(RE_PY_WORD, text, i);
      const end = i + word.length;
      if (PY_KEYWORDS.has(word)) {
        emit("se-kw", i, end);
      } else if (word === "ferric") {
        emit("se-cmd", i, end);
      }
      i = end;
      continue;
    }

    i++;
  }

  if (plain < n) out += esc(text.slice(plain));
  return out;
}

/** Returns HTML for one script line. `inBlock` marks a line inside a block comment. `lang` is "mrc" or "py". */
export function highlightLine(text, inBlock, lang = "mrc") {
  if (lang === "py") {
    return highlightPythonLine(text);
  }
  const n = text.length;
  if (n === 0) return "";
  if (n > MAX_HL_LINE) return esc(text.slice(0, MAX_RENDER_LINE));
  if (inBlock) return `<span class="se-c">${esc(text)}</span>`;

  let out = "";
  let plain = 0; // start of the pending unstyled run
  let i = 0;
  let kwOk = true; // a keyword or /command may start here
  let cmtOk = true; // a ; comment may start here

  const emit = (cls, from, to) => {
    if (from > plain) out += esc(text.slice(plain, from));
    out += `<span class="${cls}">${esc(text.slice(from, to))}</span>`;
    plain = to;
  };

  while (i < n) {
    const ch = text.charCodeAt(i);

    if (ch === 32 || ch === 9) { i++; continue; }

    if (ch === 59 && cmtOk) { // ;
      emit("se-c", i, n);
      i = n;
      break;
    }

    if (ch === 123 || ch === 125 || ch === 124) { // { } |
      i++; kwOk = true; cmtOk = true; continue;
    }
    if (ch === 41) { // )
      i++; kwOk = true; cmtOk = false; continue;
    }

    if (ch === 47 && kwOk) { // /command
      const m = matchAt(RE_CMD, text, i);
      if (m) {
        emit("se-cmd", i, i + m.length);
        i += m.length; kwOk = false; cmtOk = false; continue;
      }
    }

    if (ch === 36) { // $identifier
      const m = matchAt(RE_ID, text, i);
      if (m) {
        emit("se-id", i, i + m.length);
        i += m.length; kwOk = false; cmtOk = false; continue;
      }
    }

    if (ch === 37) { // %variable
      const m = matchAt(RE_VAR, text, i);
      if (m) {
        emit("se-var", i, i + m.length);
        i += m.length; kwOk = false; cmtOk = false; continue;
      }
    }

    if (ch === 35) { // #group or channel
      const prev = i === 0 ? 32 : text.charCodeAt(i - 1);
      if (prev === 32 || prev === 9 || prev === 58 || prev === 44 || prev === 40) {
        const m = matchAt(RE_GROUP, text, i);
        if (m) {
          emit("se-grp", i, i + m.length);
          i += m.length; kwOk = false; cmtOk = false; continue;
        }
      }
    }

    if ((ch >= 65 && ch <= 90) || (ch >= 97 && ch <= 122) || ch === 95) {
      const word = matchAt(RE_WORD, text, i);
      const end = i + word.length;
      if (kwOk && KEYWORDS.has(word.toLowerCase())) {
        const lower = word.toLowerCase();
        emit("se-kw", i, end);
        i = end;
        if (lower === "on") {
          const lvl = matchAt(RE_ON_LEVEL, text, i);
          if (lvl) {
            const ev = matchAt(RE_EVENT, text, i + lvl.length);
            if (ev) {
              emit("se-ev", i + lvl.length, i + lvl.length + ev.length);
              i += lvl.length + ev.length;
            }
          }
        } else if (lower === "raw") {
          const num = matchAt(RE_RAW_NUM, text, i);
          if (num) {
            const start = i + num.length - num.trimStart().length;
            emit("se-ev", start, i + num.length);
            i += num.length;
          }
        }
        kwOk = lower === "else";
        cmtOk = false;
        continue;
      }
      i = end; kwOk = false; cmtOk = false;
      continue;
    }

    i++; kwOk = false; cmtOk = false;
  }

  if (plain < n) out += esc(text.slice(plain));
  return out;
}

// ---------------------------------------------------------------------------
// In-memory backend for the browser (outside Tauri)

const DEMO_EVENTS = `; demo-events.mrc - sample script, edits stay in memory
#greeter on
on *:JOIN:#sandbox:{
  if ($nick != $me) {
    set %greet.last $nick
    msg $chan Welcome, $nick $+ !
  }
  elseif ($nick == $me) { halt }
}
raw 001:*:{ echo -s Connected as $me }
ctcp *:VERSION:*:{ ctcpreply $nick VERSION sample 1.0 }
#greeter end

/*
  A block comment is shown as one comment,
  also when it covers several lines.
*/
menu channel,nicklist {
  Say hello: msg $chan hello $1
  -
  Query: /query $1
}
`;

const DEMO_ALIASES = `; demo-aliases.mrc - sample aliases, indented with tabs
alias -l countdown {
\tvar %i = $1
\twhile (%i > 0) {
\t\techo -a %i
\t\tdec %i
\t}
\treturn done
}

alias greet {
\tif ($0 < 1) { echo -a Usage: /greet <nick> | return }
\t/msg $1 Hello $1, this is $me $+ .
\tgoto end
\t:end
}
`;

function createDemoBackend() {
  let clock = Date.now();
  const tick = () => ++clock;
  const files = new Map();
  const nameOk = (name) =>
    typeof name === "string" && name.length > 0 && name.length <= 128 &&
    /\.(mrc|py)$/i.test(name) && name.length > 3 && !/[\\/:*?"<>|\u0000-\u001f]/.test(name) && !name.includes("..");
  const fail = (code, message) => Object.assign(new Error(message), { code });
  const needName = (name) => {
    if (!nameOk(name)) throw fail("invalid_name", "Use a plain file name ending in .mrc or .py.");
  };
  files.set("demo-events.mrc", { content: DEMO_EVENTS, mtime: tick() });
  files.set("demo-aliases.mrc", { content: DEMO_ALIASES, mtime: tick() });

  const api = {
    async list() {
      return [...files.entries()]
        .map(([name, f]) => ({ name, size: f.content.length, mtime: f.mtime }))
        .sort((a, b) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
    },
    async read(name) {
      needName(name);
      const f = files.get(name);
      if (!f) throw fail("not_found", `${name} does not exist.`);
      return {
        name, content: f.content, mtime: f.mtime, size: f.content.length,
        encoding: "utf-8", eol: "crlf", mixedEol: false,
      };
    },
    async write(name, content, expectedMtime) {
      needName(name);
      const f = files.get(name);
      const current = f ? f.mtime : null;
      const expected = expectedMtime === undefined ? null : expectedMtime;
      if (expected !== current) throw fail("conflict", "The file changed on disk after it was opened.");
      if (!f) throw fail("conflict", "The file was deleted or renamed after it was opened.");
      if (content.length > 5 * 1024 * 1024) throw fail("too_large", "The text is larger than the 5 MB limit and was not saved.");
      f.content = content;
      f.mtime = tick();
      return f.mtime;
    },
    async create(name) {
      needName(name);
      if (files.has(name)) throw fail("exists", `${name} already exists.`);
      const f = { content: "", mtime: tick() };
      files.set(name, f);
      return { name, size: 0, mtime: f.mtime };
    },
  };

  // Test hook for the browser preview: simulate another program writing a file.
  window.__ferricScriptDemo = {
    add(name, content) { files.set(name, { content, mtime: tick() }); },
    touch(name, content) {
      const f = files.get(name);
      if (f) { f.content = content; f.mtime = tick(); }
    },
    text(name) { return files.has(name) ? files.get(name).content : null; },
  };
  return api;
}

// ---------------------------------------------------------------------------
// Editor

function normalizeError(err) {
  if (err && typeof err === "object" && typeof err.message === "string") {
    return { code: err.code || "io", message: err.message };
  }
  return { code: "io", message: String(err || "Unknown error") };
}

function detectIndent(text, name = "") {
  let tabs = 0;
  let spaces = 0;
  let pos = 0;
  const isPy = /\.py$/i.test(name);
  for (let n = 0; n < 1000 && pos < text.length; n++) {
    const c = text.charCodeAt(pos);
    if (c === 9) tabs++;
    else if (c === 32 && text.charCodeAt(pos + 1) === 32) spaces++;
    const nl = text.indexOf("\n", pos);
    if (nl === -1) break;
    pos = nl + 1;
  }
  if (tabs === 0 && spaces === 0) {
    return isPy ? "    " : "\t";
  }
  return spaces > tabs ? (isPy ? "    " : "  ") : "\t";
}

function encodingLabel(enc) {
  if (enc === "utf-8-bom") return "UTF-8 BOM";
  if (enc === "windows-1252") return "Windows-1252";
  return "UTF-8";
}

function formatKb(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / 1024).toFixed(bytes < 10240 ? 1 : 0)} KB`;
}

function pad2(n) {
  return String(n).padStart(2, "0");
}

export function createScriptEditor({ isTauri, onStatus = null }) {
  // Messages about loading and unloading go to the main window's status when a callback is given.
  const reportStatus = (text) => {
    // The editor's own status line, then the main window's status (behind the editor).
    setMessage(text);
    if (typeof onStatus === "function") onStatus(text);
    else window.alert(text);
  };
  let api = null;
  let root = null;
  let els = null;
  let isOpen = false;
  let appWasInert = false;
  let returnFocus = null;

  let files = [];
  let treeCollapsed = false;
  let treeSig = null;
  let listError = "";

  // Open document. `text` mirrors the textarea so only input events refresh it.
  let doc = null; // { name, saved, mtime, encoding, eol, mixed, indent }
  let text = "";
  let lineStarts = [0];
  let blk = null; // Uint8Array, 1 for lines inside a block comment; null when none
  let dirty = false;
  let loadToken = 0;
  let busy = false;

  let rows = []; // { line, hl, num, sig }
  let charW = 7.2;
  let renderQueued = false;
  let statusQueued = false;
  let dialogState = null;
  let hit = null; // { start, end } current find match

  const getApi = () => {
    if (api) return api;
    if (isTauri) {
      const call = async (cmd, args) => {
        const { invoke } = await import("@tauri-apps/api/core");
        return invoke(cmd, args);
      };
      api = {
        list: () => call("list_script_files"),
        read: (name) => call("read_script_file", { name }),
        write: (name, content, expectedMtime) =>
          call("write_script_file", { name, content, expectedMtime, backup: backupOnSave }),
        create: (name) => call("create_script_file", { name }),
      };
    } else {
      api = createDemoBackend();
    }
    return api;
  };

  // ---- DOM ---------------------------------------------------------------

  function buildDom() {
    root = document.createElement("div");
    root.id = "script-editor-root";
    root.className = "se-overlay";
    root.dataset.seRoot = "";
    root.hidden = true;
    root.innerHTML = `
      <div class="se-window" role="dialog" aria-modal="true" aria-labelledby="se-title" tabindex="-1">
        <div class="se-titlebar">
          <span id="se-title" class="se-title">Ferric Scripts Editor</span>
          <span id="se-title-file" class="se-title-file"></span>
          <button type="button" class="se-close" data-se-action="close" aria-label="Close script editor" title="Close (Esc)">✕</button>
        </div>

        <div class="se-menubar" role="menubar" aria-label="Script editor menu">
          <details class="menu-dropdown">
            <summary role="menuitem">File</summary>
            <div class="menu-popover" role="menu">
              <button type="button" role="menuitem" class="has-key" data-se-action="new"><span>New…</span></button>
              <button type="button" role="menuitem" class="has-key" data-se-action="save"><span>Save</span><kbd class="menu-key">Ctrl+S</kbd></button>
              <button type="button" role="menuitem" class="has-key" data-se-action="reload"><span>Reload from disk</span></button>
              <div class="menu-separator" role="separator"></div>
              <button type="button" role="menuitem" class="has-key" data-se-action="close"><span>Close</span><kbd class="menu-key">Esc</kbd></button>
            </div>
          </details>
          <details class="menu-dropdown">
            <summary role="menuitem">Edit</summary>
            <div class="menu-popover" role="menu">
              <button type="button" role="menuitem" class="has-key" data-se-action="find"><span>Find…</span><kbd class="menu-key">Ctrl+F</kbd></button>
              <button type="button" role="menuitem" class="has-key" data-se-action="goto"><span>Go to line…</span><kbd class="menu-key">Ctrl+G</kbd></button>
            </div>
          </details>
          <details class="menu-dropdown">
            <summary role="menuitem">Options</summary>
            <div class="menu-popover" role="menu">
              <button type="button" role="menuitemcheckbox" aria-checked="true" data-se-action="toggle-line-numbers">Line Numbers</button>
              <button type="button" role="menuitemcheckbox" aria-checked="true" data-se-action="toggle-backup">Backup File on Save</button>
            </div>
          </details>
        </div>

        <div class="app-toolbar se-toolbar" role="toolbar" aria-label="Script editor actions">
          <div class="toolbar-group">
            <button type="button" class="toolbar-button" data-se-action="new">New</button>
            <button type="button" id="se-btn-save" class="toolbar-button" data-se-action="save" disabled>Save</button>
            <button type="button" id="se-btn-reload" class="toolbar-button" data-se-action="reload" disabled>Reload</button>
            <button type="button" id="se-btn-load" class="toolbar-button" data-se-action="load-script" disabled title="Load the saved script into the interpreter (like /scriptload)">Load</button>
            <button type="button" id="se-btn-unload" class="toolbar-button" data-se-action="unload-script" disabled title="Unload this script's aliases (like /scriptunload)">Unload</button>
          </div>
          <span class="toolbar-divider" aria-hidden="true"></span>
          <div class="toolbar-group">
            <button type="button" class="toolbar-button" data-se-action="undo">Undo</button>
            <button type="button" class="toolbar-button" data-se-action="redo">Redo</button>
          </div>
          <span class="toolbar-divider" aria-hidden="true"></span>
          <div class="toolbar-group">
            <button type="button" class="toolbar-button" data-se-action="find">Find</button>
            <button type="button" class="toolbar-button" data-se-action="goto">Go to line</button>
          </div>
        </div>

        <div id="se-find" class="se-find" hidden>
          <label class="se-find-label" for="se-find-input">Find</label>
          <input id="se-find-input" class="form-input se-find-input" autocomplete="off" spellcheck="false" />
          <button type="button" class="btn-icon-sm" data-se-action="find-prev">Previous</button>
          <button type="button" class="btn-icon-sm" data-se-action="find-next">Next</button>
          <label class="se-find-case"><input id="se-find-case" type="checkbox" /> Match case</label>
          <span id="se-find-info" class="se-find-info" aria-live="polite"></span>
          <button type="button" class="se-find-close" data-se-action="find-close" aria-label="Close find" title="Close find">✕</button>
        </div>

        <div class="se-body">
          <nav class="se-tree" aria-label="Script files">
            <div id="se-tree" class="se-tree-list" role="tree"></div>
          </nav>
          <section class="se-main">
            <div id="se-editor" class="se-editor" hidden>
              <div class="se-gutter" aria-hidden="true">
                <div id="se-gutter-cur" class="se-gutter-cur"></div>
                <div id="se-gutter-inner" class="se-gutter-inner"></div>
              </div>
              <div class="se-code">
                <div id="se-curline" class="se-curline"></div>
                <div id="se-hit" class="se-hit" hidden></div>
                <div class="se-hl" aria-hidden="true"><div id="se-hl-inner" class="se-hl-inner"></div></div>
                <textarea id="se-input" class="se-input" wrap="off" spellcheck="false" autocomplete="off" autocapitalize="off" autocorrect="off" aria-label="Script text"></textarea>
              </div>
            </div>
            <div id="se-placeholder" class="se-placeholder"></div>
          </section>
        </div>

        <footer class="se-status">
          <span id="se-st-pos" class="se-st-pos">Ln 1, Col 1</span>
          <span id="se-st-enc">UTF-8</span>
          <span id="se-st-eol">CRLF</span>
          <span id="se-st-mod" class="se-st-mod">Modified</span>
          <span id="se-st-msg" class="se-st-msg" role="status" aria-live="polite"></span>
        </footer>

        <div id="se-dialog-layer" class="se-dialog-layer" hidden></div>
      </div>
    `;
    document.body.appendChild(root);

    const q = (id) => root.querySelector(`#${id}`);
    els = {
      window: root.querySelector(".se-window"),
      titleFile: q("se-title-file"),
      saveBtn: q("se-btn-save"),
      reloadBtn: q("se-btn-reload"),
      loadBtn: q("se-btn-load"),
      unloadBtn: q("se-btn-unload"),
      find: q("se-find"),
      findInput: q("se-find-input"),
      findCase: q("se-find-case"),
      findInfo: q("se-find-info"),
      tree: q("se-tree"),
      editor: q("se-editor"),
      placeholder: q("se-placeholder"),
      gutterInner: q("se-gutter-inner"),
      gutterCur: q("se-gutter-cur"),
      curline: q("se-curline"),
      hit: q("se-hit"),
      hlInner: q("se-hl-inner"),
      ta: q("se-input"),
      stPos: q("se-st-pos"),
      stEnc: q("se-st-enc"),
      stEol: q("se-st-eol"),
      stMod: q("se-st-mod"),
      stMsg: q("se-st-msg"),
      dialogLayer: q("se-dialog-layer"),
    };

    attachEvents();
    applyLineNumbers();
    applyBackupOption();
    if (typeof ResizeObserver !== "undefined") {
      new ResizeObserver(() => { if (isOpen) scheduleRender(); }).observe(els.ta);
    }
  }

  let charMeasured = false;
  function measureChar() {
    charMeasured = true;
    const probe = document.createElement("span");
    probe.className = "se-measure";
    probe.textContent = "0".repeat(100);
    els.editor.hidden = false;
    els.editor.appendChild(probe);
    const w = probe.getBoundingClientRect().width / 100;
    probe.remove();
    els.editor.hidden = true;
    if (w > 0) charW = w;
  }

  // ---- Text index and rendering -----------------------------------------

  function indexText(t) {
    const starts = [0];
    let pos = 0;
    while ((pos = t.indexOf("\n", pos)) !== -1) {
      pos++;
      starts.push(pos);
    }
    lineStarts = starts;

    if (t.indexOf("/*") === -1) {
      blk = null;
      return;
    }
    const n = starts.length;
    const flags = new Uint8Array(n);
    let inBlock = false;
    for (let i = 0; i < n; i++) {
      let p = starts[i];
      const end = i + 1 < n ? starts[i + 1] - 1 : t.length;
      while (p < end) {
        const c = t.charCodeAt(p);
        if (c !== 32 && c !== 9) break;
        p++;
      }
      const c1 = p < end ? t.charCodeAt(p) : 0;
      const c2 = p + 1 < end ? t.charCodeAt(p + 1) : 0;
      // A block ends at the first "*/" on the line that closes it; the next line is code again.
      if (!inBlock) {
        if (c1 === 47 && c2 === 42) {
          inBlock = true;
          flags[i] = 1;
          if (t.slice(p + 2, end).indexOf("*/") !== -1) inBlock = false;
        }
      } else {
        flags[i] = 1;
        if (t.slice(p, end).indexOf("*/") !== -1) inBlock = false;
      }
    }
    blk = flags;
  }

  function lineText(i) {
    const n = lineStarts.length;
    return text.slice(lineStarts[i], i + 1 < n ? lineStarts[i + 1] - 1 : text.length);
  }

  function lineOfOffset(off) {
    let lo = 0;
    let hi = lineStarts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (lineStarts[mid] <= off) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  function makeRow(line) {
    const hl = document.createElement("div");
    hl.className = "se-ln";
    const num = document.createElement("div");
    num.className = "se-num";
    num.textContent = String(line + 1);
    return { line, hl, num, sig: null };
  }

  function dropRow(r) {
    r.hl.remove();
    r.num.remove();
  }

  function syncRows(first, last) {
    while (rows.length && rows[rows.length - 1].line >= last) dropRow(rows.pop());
    while (rows.length && rows[0].line < first) dropRow(rows.shift());

    if (!rows.length) {
      const hlFrag = document.createDocumentFragment();
      const numFrag = document.createDocumentFragment();
      for (let l = first; l < last; l++) {
        const r = makeRow(l);
        rows.push(r);
        hlFrag.appendChild(r.hl);
        numFrag.appendChild(r.num);
      }
      els.hlInner.appendChild(hlFrag);
      els.gutterInner.appendChild(numFrag);
      return;
    }

    if (rows[0].line > first) {
      const hlFrag = document.createDocumentFragment();
      const numFrag = document.createDocumentFragment();
      const added = [];
      for (let l = first; l < rows[0].line; l++) {
        const r = makeRow(l);
        added.push(r);
        hlFrag.appendChild(r.hl);
        numFrag.appendChild(r.num);
      }
      els.hlInner.insertBefore(hlFrag, els.hlInner.firstChild);
      els.gutterInner.insertBefore(numFrag, els.gutterInner.firstChild);
      rows = added.concat(rows);
    }

    const tail = rows[rows.length - 1].line + 1;
    if (tail < last) {
      const hlFrag = document.createDocumentFragment();
      const numFrag = document.createDocumentFragment();
      for (let l = tail; l < last; l++) {
        const r = makeRow(l);
        rows.push(r);
        hlFrag.appendChild(r.hl);
        numFrag.appendChild(r.num);
      }
      els.hlInner.appendChild(hlFrag);
      els.gutterInner.appendChild(numFrag);
    }
  }

  function updateRows() {
    const lang = doc && /\.py$/i.test(doc.name) ? "py" : "mrc";
    for (const r of rows) {
      const t = lineText(r.line);
      const inBlock = blk ? blk[r.line] : 0;
      const sig = `${lang}:${inBlock ? "1" : "0"}${t}`;
      if (r.sig !== sig) {
        r.sig = sig;
        r.hl.innerHTML = highlightLine(t, inBlock, lang);
      }
    }
  }

  function renderViewport() {
    renderQueued = false;
    if (!doc || !isOpen) return;
    const ta = els.ta;
    const st = ta.scrollTop;
    const vh = ta.clientHeight || 400;
    const total = lineStarts.length;
    const first = Math.max(0, Math.floor((st - PAD_Y) / LH) - OVERSCAN);
    const last = Math.min(total, Math.ceil((st + vh - PAD_Y) / LH) + OVERSCAN);
    syncRows(first, last);
    updateRows();
    applyScroll();
  }

  function scheduleRender() {
    if (renderQueued) return;
    renderQueued = true;
    requestAnimationFrame(renderViewport);
  }

  function applyScroll() {
    const ta = els.ta;
    const st = ta.scrollTop;
    const sl = ta.scrollLeft;
    const top = rows.length ? rows[0].line * LH + PAD_Y - st : 0;
    els.hlInner.style.transform = `translate(${PAD_X - sl}px, ${top}px)`;
    els.gutterInner.style.transform = `translateY(${top}px)`;
    positionBars();
  }

  function positionBars() {
    if (!doc) return;
    const ta = els.ta;
    const st = ta.scrollTop;
    const sl = ta.scrollLeft;
    const single = ta.selectionStart === ta.selectionEnd;
    if (single) {
      const y = PAD_Y + lineOfOffset(ta.selectionStart) * LH - st;
      els.curline.style.transform = `translateY(${y}px)`;
      els.gutterCur.style.transform = `translateY(${y}px)`;
    }
    els.curline.hidden = !single;
    els.gutterCur.hidden = !single;

    if (hit) {
      const line = lineOfOffset(hit.start);
      const ls = lineStarts[line];
      const vx = (s) => {
        let v = 0;
        for (let k = 0; k < s.length; k++) v += s.charCodeAt(k) === 9 ? TAB_SIZE - (v % TAB_SIZE) : 1;
        return v;
      };
      const before = text.slice(ls, hit.start);
      const startCol = vx(before);
      const width = vx(before + text.slice(hit.start, hit.end)) - startCol;
      els.hit.hidden = false;
      els.hit.style.width = `${Math.max(1, width * charW)}px`;
      els.hit.style.transform = `translate(${PAD_X + startCol * charW - sl}px, ${PAD_Y + line * LH - st}px)`;
    } else {
      els.hit.hidden = true;
    }
  }

  function onScroll() {
    applyScroll();
    if (!rows.length) return;
    const ta = els.ta;
    const vf = Math.floor((ta.scrollTop - PAD_Y) / LH);
    const vl = Math.ceil((ta.scrollTop + ta.clientHeight - PAD_Y) / LH);
    const r0 = rows[0].line;
    const r1 = rows[rows.length - 1].line + 1;
    if ((vf < r0 + 3 && r0 > 0) || (vl > r1 - 3 && r1 < lineStarts.length)) scheduleRender();
  }

  function scheduleStatus() {
    if (statusQueued) return;
    statusQueued = true;
    requestAnimationFrame(() => {
      statusQueued = false;
      updateStatus();
      positionBars();
    });
  }

  function updateStatus() {
    if (!doc) return;
    const ta = els.ta;
    const s = ta.selectionStart;
    const e = ta.selectionEnd;
    const line = lineOfOffset(s);
    const col = s - lineStarts[line] + 1;
    let label = `Ln ${line + 1}, Col ${col}`;
    if (e > s) label += ` (${e - s} selected)`;
    if (els.stPos.textContent !== label) els.stPos.textContent = label;
  }

  function setDigits() {
    const digits = Math.max(3, String(lineStarts.length).length);
    els.editor.style.setProperty("--se-digits", String(digits));
  }

  // ---- Document state ---------------------------------------------------

  function setMessage(message, kind = "") {
    els.stMsg.textContent = message;
    els.stMsg.className = `se-st-msg${kind ? ` is-${kind}` : ""}`;
  }

  // Loads the saved file into the interpreter, like /scriptload. connectd.mrc is the server list, not a script.
  async function loadIntoInterpreter() {
    if (!doc) return;
    if (doc.name.toLowerCase() === "connectd.mrc") {
      reportStatus("connectd.mrc is the server list, not a script.");
      return;
    }
    if (dirty) {
      reportStatus("Save the script before loading it.");
      return;
    }
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const notices = await invoke("load_script_aliases", { name: doc.name });
      reportStatus(notices.length ? `${doc.name} loaded with ${notices.length} problem(s).` : `${doc.name} loaded.`);
      for (const notice of notices) reportStatus(notice);
    } catch (e) {
      reportStatus(`Load failed: ${String(e?.message || e)}`);
    }
  }

  async function unloadFromInterpreter() {
    if (!doc) return;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const unloaded = await invoke("unload_script_aliases", { name: doc.name });
      reportStatus(unloaded ? `${doc.name} unloaded.` : `${doc.name} was not loaded.`);
    } catch (e) {
      reportStatus(`Unload failed: ${String(e?.message || e)}`);
    }
  }

  function applyDirtyUI() {
    els.stMod.classList.toggle("is-on", dirty);
    els.saveBtn.disabled = !doc || !dirty;
    if (els.loadBtn) els.loadBtn.disabled = !doc || dirty;
    if (els.unloadBtn) els.unloadBtn.disabled = !doc;
    els.titleFile.textContent = doc ? `– ${doc.name}${dirty ? " *" : ""}` : "";
    const cur = els.tree.querySelector(".se-file.is-current");
    if (cur) cur.classList.toggle("is-dirty", dirty);
    reportDirty();
  }

  let reportedDirty = false;
  function reportDirty() {
    if (reportedDirty === dirty) return;
    reportedDirty = dirty;
    if (isTauri) {
      import("@tauri-apps/api/core")
        .then(({ invoke }) => invoke("set_scripts_dirty", { dirty }))
        .catch(() => { });
    }
  }

  function checkDirty() {
    const now = Boolean(doc) && text !== doc.saved;
    if (now === dirty) return;
    dirty = now;
    if (dirty && els.stMsg.classList.contains("is-ok")) setMessage("");
    applyDirtyUI();
  }

  function onInput() {
    text = els.ta.value;
    hit = null;
    indexText(text);
    setDigits();
    renderViewport();
    checkDirty();
    updateStatus();
  }

  function showDocument(next) {
    doc = next;
    text = next.saved;
    els.ta.value = text;
    els.ta.scrollTop = 0;
    els.ta.scrollLeft = 0;
    els.ta.setSelectionRange(0, 0);
    hit = null;
    indexText(text);
    setDigits();
    for (const r of rows) dropRow(r);
    rows = [];
    els.editor.hidden = false;
    els.placeholder.hidden = true;
    els.ta.disabled = false;
    els.stEnc.textContent = encodingLabel(next.encoding);
    els.stEol.textContent = `${next.eol === "lf" ? "LF" : "CRLF"}${next.mixed ? " (mixed)" : ""}`;
    els.stEol.title = next.mixed
      ? "The file mixes line endings. Saving writes the most common style for every line."
      : "";
    els.reloadBtn.disabled = false;
    dirty = false;
    renderViewport();
    updateStatus();
    applyDirtyUI();
    try { localStorage.setItem(LAST_FILE_KEY, next.name); } catch { }
  }

  function showNoDocument(message) {
    doc = null;
    text = "";
    lineStarts = [0];
    blk = null;
    dirty = false;
    for (const r of rows) dropRow(r);
    rows = [];
    hit = null;
    els.editor.hidden = true;
    els.placeholder.hidden = false;
    els.placeholder.textContent = message;
    els.ta.value = "";
    els.reloadBtn.disabled = true;
    els.stPos.textContent = "";
    els.stEnc.textContent = "";
    els.stEol.textContent = "";
    applyDirtyUI();
  }

  // ---- File list and tree -----------------------------------------------

  async function refreshList() {
    try {
      const list = await getApi().list();
      files = Array.isArray(list) ? list : [];
      listError = "";
    } catch (err) {
      files = [];
      listError = normalizeError(err).message;
    }
    renderTree(true);
  }

  function renderTree(force = false) {
    const currentName = doc ? doc.name : "";
    const items = files.map((f) => `
      <div class="se-file${f.name === currentName ? " is-current" : ""}${f.name === currentName && dirty ? " is-dirty" : ""}" role="treeitem" tabindex="0" aria-selected="${f.name === currentName}" data-file="${escapeHtml(f.name)}" title="${escapeHtml(f.name)} (${formatKb(f.size)})">
        <span class="se-file-label"><span class="se-file-icon" aria-hidden="true"></span><span class="se-file-name">${escapeHtml(f.name)}</span></span>
        <span class="se-file-dirty" aria-hidden="true"></span>
      </div>`).join("");
    const empty = listError
      ? `<div class="se-tree-empty is-error">${escapeHtml(listError)}</div>`
      : (files.length === 0 ? '<div class="se-tree-empty">No .mrc or .py files in the script folder. Use File &gt; New to create one.</div>' : "");
    const html = `
      <div class="se-tree-node${treeCollapsed ? "" : " is-open"}">
        <button type="button" class="tree-box" data-se-action="toggle-tree" aria-expanded="${!treeCollapsed}" aria-label="${treeCollapsed ? "Expand" : "Collapse"} script list"></button>
        <span class="se-tree-root-name">Scripts</span>
        <span class="se-tree-count">${files.length}</span>
      </div>
      <div class="se-tree-children" role="group"${treeCollapsed ? " hidden" : ""}>${items}${empty}</div>`;
    const sig = `${currentName}|${dirty}|${html}`;
    if (!force && sig === treeSig) return;
    treeSig = sig;
    const hadFocus = els.tree.contains(document.activeElement) ? document.activeElement.dataset.file : null;
    els.tree.innerHTML = html;
    if (hadFocus) {
      for (const el of els.tree.querySelectorAll(".se-file")) {
        if (el.dataset.file === hadFocus) { el.focus({ preventScroll: true }); break; }
      }
    }
  }

  // ---- Dialogs ----------------------------------------------------------

  function showDialog({ title, message, label, value, buttons, cancelId, validate }) {
    return new Promise((resolve) => {
      const hasInput = label !== undefined;
      const defaultBtn = buttons.find((b) => b.primary) || buttons[0];
      dialogState = { resolve, buttons, cancelId, hasInput, validate, defaultId: defaultBtn.id, opener: document.activeElement };
      els.dialogLayer.innerHTML = `
        <div class="se-dialog" role="alertdialog" aria-modal="true" aria-labelledby="se-dlg-title" aria-describedby="se-dlg-msg">
          <div id="se-dlg-title" class="se-dialog-title">${escapeHtml(title)}</div>
          <div class="se-dialog-body">
            <p id="se-dlg-msg" class="se-dialog-msg">${escapeHtml(message || "")}</p>
            ${hasInput ? `<label class="form-label" for="se-dlg-input">${escapeHtml(label)}</label><input id="se-dlg-input" class="form-input" autocomplete="off" spellcheck="false" value="${escapeHtml(value || "")}" />` : ""}
            <div id="se-dlg-error" class="form-error" role="alert" hidden></div>
          </div>
          <div class="se-dialog-buttons">
            ${buttons.map((b) => `<button type="button" class="btn${b.primary ? " btn-primary" : ""}" data-se-dlg="${escapeHtml(b.id)}">${escapeHtml(b.label)}</button>`).join("")}
          </div>
        </div>`;
      els.dialogLayer.hidden = false;
      const input = els.dialogLayer.querySelector("#se-dlg-input");
      if (input) {
        input.focus();
        input.select();
      } else {
        els.dialogLayer.querySelector(`[data-se-dlg="${CSS.escape(defaultBtn.id)}"]`).focus();
      }
    });
  }

  function dialogError(message) {
    const box = els.dialogLayer.querySelector("#se-dlg-error");
    if (!box) return;
    box.textContent = message;
    box.hidden = !message;
  }

  function finishDialog(id) {
    const d = dialogState;
    if (!d) return;
    const input = els.dialogLayer.querySelector("#se-dlg-input");
    const value = input ? input.value : undefined;
    if (id !== d.cancelId && d.validate) {
      const problem = d.validate(id, value);
      if (problem) {
        dialogError(problem);
        if (input) input.focus();
        return;
      }
    }
    dialogState = null;
    els.dialogLayer.hidden = true;
    els.dialogLayer.innerHTML = "";
    if (d.opener && d.opener.isConnected && root.contains(d.opener)) d.opener.focus();
    else if (doc) els.ta.focus();
    d.resolve({ id, value });
  }

  function handleDialogKey(e) {
    const d = dialogState;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      finishDialog(d.cancelId);
    } else if (e.key === "Enter" && e.target.tagName === "INPUT") {
      e.preventDefault();
      finishDialog(d.defaultId);
    } else if (e.key === "Tab") {
      const focusable = [...els.dialogLayer.querySelectorAll("input, button")];
      if (!focusable.length) return;
      const idx = focusable.indexOf(document.activeElement);
      const next = e.shiftKey ? (idx <= 0 ? focusable.length - 1 : idx - 1) : (idx + 1) % focusable.length;
      e.preventDefault();
      focusable[next].focus();
    }
  }

  function notice(title, message) {
    return showDialog({ title, message, buttons: [{ id: "ok", label: "OK", primary: true }], cancelId: "ok" });
  }

  // ---- Saving, loading, guarding ----------------------------------------

  async function loadFile(name) {
    const token = ++loadToken;
    try {
      const r = await getApi().read(name);
      if (token !== loadToken) return false;
      showDocument({
        name: r.name || name,
        saved: r.content,
        mtime: r.mtime,
        encoding: r.encoding,
        eol: r.eol,
        mixed: Boolean(r.mixedEol),
        indent: detectIndent(r.content, r.name || name),
      });
      renderTree(true);
      setMessage("");
      return true;
    } catch (err) {
      if (token !== loadToken) return false;
      const e = normalizeError(err);
      setMessage(e.message, "error");
      await notice("Cannot open file", e.message);
      return false;
    }
  }

  async function save() {
    if (!doc || busy) return false;
    if (!dirty) {
      setMessage("No changes to save.");
      return true;
    }
    busy = true;
    try {
      const content = els.ta.value;
      try {
        const mtime = await getApi().write(doc.name, content, doc.mtime);
        return await afterSave(content, mtime);
      } catch (err) {
        const e = normalizeError(err);
        if (e.code === "conflict") return await resolveConflict(content, e.message);
        setMessage(e.message, "error");
        await notice("Cannot save", e.message);
        return false;
      }
    } finally {
      busy = false;
    }
  }

  async function afterSave(content, mtime) {
    doc.saved = content;
    doc.mtime = mtime;
    checkDirty();
    const now = new Date();
    setMessage(`Saved ${pad2(now.getHours())}:${pad2(now.getMinutes())}:${pad2(now.getSeconds())}`, "ok");
    refreshList();
    return true;
  }

  async function resolveConflict(content, detail) {
    const choice = await showDialog({
      title: "File changed on disk",
      message: `${doc.name} was changed outside the editor. ${detail} Reload from disk to discard your edits, or overwrite the file on disk with your text.`,
      buttons: [
        { id: "reload", label: "Reload from disk" },
        { id: "overwrite", label: "Overwrite" },
        { id: "cancel", label: "Cancel", primary: true },
      ],
      cancelId: "cancel",
    });
    if (choice.id === "reload") {
      await refreshList();
      return (await loadFile(doc.name)) && false;
    }
    if (choice.id !== "overwrite") {
      setMessage("Not saved: the file changed on disk.", "error");
      return false;
    }
    try {
      // Take the modification time that is on disk now and retry once.
      const list = await getApi().list();
      files = Array.isArray(list) ? list : files;
      const current = files.find((f) => f.name === doc.name);
      const mtime = await getApi().write(doc.name, content, current ? current.mtime : null);
      return await afterSave(content, mtime);
    } catch (err) {
      const e = normalizeError(err);
      setMessage(e.message, "error");
      await notice("Cannot save", e.message);
      return false;
    }
  }

  /** Resolves true when the caller may leave the current document. */
  async function guardUnsaved() {
    if (!doc || !dirty) return true;
    const choice = await showDialog({
      title: "Unsaved changes",
      message: `Save changes to ${doc.name}?`,
      buttons: [
        { id: "save", label: "Save", primary: true },
        { id: "discard", label: "Discard" },
        { id: "cancel", label: "Cancel" },
      ],
      cancelId: "cancel",
    });
    if (choice.id === "save") return save();
    return choice.id === "discard";
  }

  async function selectFile(name) {
    if (doc && doc.name === name) {
      els.ta.focus();
      return;
    }
    if (!(await guardUnsaved())) return;
    await loadFile(name);
    if (doc) els.ta.focus();
  }

  async function reloadFromDisk() {
    if (!doc) return;
    if (dirty) {
      const choice = await showDialog({
        title: "Reload from disk",
        message: `Discard your changes to ${doc.name} and load the file from disk again?`,
        buttons: [
          { id: "reload", label: "Reload", primary: true },
          { id: "cancel", label: "Cancel" },
        ],
        cancelId: "cancel",
      });
      if (choice.id !== "reload") return;
    }
    const name = doc.name;
    await refreshList();
    if (await loadFile(name)) setMessage("Reloaded from disk.", "ok");
  }

  async function newFile() {
    if (!(await guardUnsaved())) return;
    let value = "";
    for (;;) {
      const choice = await showDialog({
        title: "New script",
        message: "Create an empty script file (.mrc or .py) in the script folder.",
        label: "File name",
        value,
        buttons: [
          { id: "create", label: "Create", primary: true },
          { id: "cancel", label: "Cancel" },
        ],
        cancelId: "cancel",
        validate: (_id, v) => (v && v.trim() ? null : "Enter a file name."),
      });
      if (choice.id !== "create") return;
      let name = (choice.value || "").trim();
      if (!/\.(mrc|py)$/i.test(name)) name += ".mrc";
      try {
        const info = await getApi().create(name);
        await refreshList();
        await loadFile(info.name || name);
        if (doc) els.ta.focus();
        return;
      } catch (err) {
        const e = normalizeError(err);
        value = name;
        setMessage(e.message, "error");
        await notice("Cannot create file", e.message);
      }
    }
  }

  async function requestClose() {
    if (!isOpen || dialogState) return;
    if (!(await guardUnsaved())) return;
    closeNow();
  }

  function closeNow() {
    isOpen = false;
    loadToken++;
    root.hidden = true;
    const app = document.getElementById("app");
    if (app) app.inert = appWasInert;
    showNoDocument("");
    hideFind();
    if (returnFocus && returnFocus.isConnected) returnFocus.focus({ preventScroll: true });
    returnFocus = null;
  }

  async function open() {
    if (!root) buildDom();
    if (isOpen) {
      if (doc && !dialogState) els.ta.focus();
      return;
    }
    returnFocus = document.activeElement;
    isOpen = true;
    const app = document.getElementById("app");
    appWasInert = app ? app.inert : false;
    if (app) app.inert = true;
    root.hidden = false;
    if (!charMeasured) measureChar();
    for (const menu of root.querySelectorAll("details[open]")) menu.open = false;
    setMessage("");
    showNoDocument("Loading…");
    await refreshList();
    if (!isOpen) return;

    let last = "";
    try { last = localStorage.getItem(LAST_FILE_KEY) || ""; } catch { }
    const target = files.find((f) => f.name === last) || files[0];
    if (target) {
      await loadFile(target.name);
      if (doc) els.ta.focus();
    } else {
      showNoDocument(listError ? "The script folder cannot be read." : "No script files yet. Use File > New to create one.");
      root.querySelector(".se-window").focus();
    }
  }

  // ---- Editing commands -------------------------------------------------

  function replaceText(start, end, value) {
    const ta = els.ta;
    ta.focus();
    ta.setSelectionRange(start, end);
    // execCommand keeps the native undo stack; setRangeText is the fallback.
    if (!document.execCommand("insertText", false, value)) {
      ta.setRangeText(value, start, end, "end");
      ta.dispatchEvent(new Event("input", { bubbles: true }));
    }
  }

  function handleTab(shift) {
    const ta = els.ta;
    const v = ta.value;
    const unit = doc ? doc.indent : "\t";
    const s = ta.selectionStart;
    const e = ta.selectionEnd;

    if (s === e && !shift) {
      replaceText(s, e, unit);
      return;
    }

    const lineStart = v.lastIndexOf("\n", s - 1) + 1;
    const endRef = e > s && v[e - 1] === "\n" ? e - 1 : e;
    let lineEnd = v.indexOf("\n", endRef);
    if (lineEnd === -1) lineEnd = v.length;
    const lines = v.slice(lineStart, lineEnd).split("\n");

    let firstDelta = 0;
    let total = 0;
    const changed = lines.map((l, idx) => {
      let out = l;
      if (!shift) {
        if (l.length) out = unit + l;
      } else if (l.startsWith("\t")) {
        out = l.slice(1);
      } else {
        const maxSpaces = doc && doc.indent && doc.indent.startsWith(" ") ? doc.indent.length : 2;
        const re = new RegExp(`^ {1,${maxSpaces}}`);
        const m = re.exec(l);
        if (m) out = l.slice(m[0].length);
      }
      const d = out.length - l.length;
      if (idx === 0) firstDelta = d;
      total += d;
      return out;
    });

    const newStart = s === lineStart ? s : Math.max(lineStart, s + firstDelta);
    const newEnd = Math.max(newStart, e + total);
    replaceText(lineStart, lineEnd, changed.join("\n"));
    ta.setSelectionRange(newStart, newEnd);
    scheduleStatus();
  }

  function handleEnter() {
    const ta = els.ta;
    const v = ta.value;
    const s = ta.selectionStart;
    const lineStart = v.lastIndexOf("\n", s - 1) + 1;
    const before = v.slice(lineStart, s);
    let indent = /^[ \t]*/.exec(before)[0];
    const isPy = doc && /\.py$/i.test(doc.name);
    const trimmed = before.trimEnd();
    if (doc && (trimmed.endsWith("{") || (isPy && trimmed.endsWith(":")))) {
      indent += doc.indent;
    }
    replaceText(s, ta.selectionEnd, `\n${indent}`);
  }

  function revealOffset(off) {
    const ta = els.ta;
    const line = lineOfOffset(off);
    const y = PAD_Y + line * LH;
    const vh = ta.clientHeight;
    if (y < ta.scrollTop + LH || y > ta.scrollTop + vh - 3 * LH) {
      ta.scrollTop = Math.max(0, y - Math.floor(vh / 2));
    }
    const col = off - lineStarts[line];
    const lineStr = text.slice(lineStarts[line], lineStarts[line] + col);
    let v = 0;
    for (let k = 0; k < lineStr.length; k++) v += lineStr.charCodeAt(k) === 9 ? TAB_SIZE - (v % TAB_SIZE) : 1;
    const x = PAD_X + v * charW;
    const vw = ta.clientWidth;
    if (x < ta.scrollLeft + 20 || x > ta.scrollLeft + vw - 60) {
      ta.scrollLeft = Math.max(0, x - Math.floor(vw / 2));
    }
    applyScroll();
    scheduleRender();
  }

  async function gotoLine() {
    if (!doc) return;
    const total = lineStarts.length;
    const choice = await showDialog({
      title: "Go to line",
      message: `Lines 1 to ${total}.`,
      label: "Line number",
      value: String(lineOfOffset(els.ta.selectionStart) + 1),
      buttons: [
        { id: "go", label: "Go", primary: true },
        { id: "cancel", label: "Cancel" },
      ],
      cancelId: "cancel",
      validate: (_id, v) => {
        const n = Number(String(v).trim());
        return Number.isInteger(n) && n >= 1 && n <= total ? null : `Enter a number from 1 to ${total}.`;
      },
    });
    if (choice.id !== "go") return;
    const n = Number(String(choice.value).trim());
    const off = lineStarts[Math.min(n, lineStarts.length) - 1];
    els.ta.focus();
    els.ta.setSelectionRange(off, off);
    revealOffset(off);
    updateStatus();
  }

  // ---- Find -------------------------------------------------------------

  function openFind() {
    if (!doc) return;
    const ta = els.ta;
    const sel = ta.value.slice(ta.selectionStart, ta.selectionEnd);
    els.find.hidden = false;
    if (sel && !sel.includes("\n")) els.findInput.value = sel;
    els.findInput.focus();
    els.findInput.select();
    scheduleRender();
  }

  function hideFind() {
    if (!els) return;
    const wasOpen = !els.find.hidden;
    els.find.hidden = true;
    els.findInfo.textContent = "";
    if (hit) {
      hit = null;
      positionBars();
    }
    if (wasOpen && doc && isOpen) els.ta.focus();
  }

  function findNext(forward = true) {
    if (!doc) return;
    const needle = els.findInput.value;
    if (!needle) {
      els.findInfo.textContent = "";
      return;
    }
    const ta = els.ta;
    const hay = ta.value;
    const re = new RegExp(needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), els.findCase.checked ? "g" : "gi");
    let found = null;
    let wrapped = false;

    if (forward) {
      re.lastIndex = ta.selectionEnd;
      found = re.exec(hay);
      if (!found) {
        re.lastIndex = 0;
        found = re.exec(hay);
        wrapped = Boolean(found);
      }
    } else {
      const limit = ta.selectionStart;
      let m;
      let lastBefore = null;
      let lastAny = null;
      re.lastIndex = 0;
      while ((m = re.exec(hay))) {
        lastAny = m;
        if (m.index + m[0].length <= limit) lastBefore = m;
      }
      found = lastBefore || lastAny;
      wrapped = Boolean(found) && !lastBefore;
    }

    if (!found) {
      hit = null;
      positionBars();
      els.findInfo.textContent = "Not found";
      els.findInfo.className = "se-find-info is-error";
      return;
    }
    els.findInfo.textContent = wrapped ? "Search wrapped" : "";
    els.findInfo.className = "se-find-info";
    ta.setSelectionRange(found.index, found.index + found[0].length);
    hit = { start: found.index, end: found.index + found[0].length };
    revealOffset(found.index);
    updateStatus();
  }

  // ---- Events (delegated on the root) -----------------------------------

  function readLineNumbers() {
    try {
      return localStorage.getItem(LINE_NUMBERS_KEY) !== "off";
    } catch { }
    return true;
  }

  let showLineNumbers = readLineNumbers();

  function readBackupOnSave() {
    try {
      return localStorage.getItem(BACKUP_KEY) !== "off";
    } catch { }
    return true;
  }

  let backupOnSave = readBackupOnSave();

  function applyLineNumbers() {
    els.window.classList.toggle("hide-line-numbers", !showLineNumbers);
    root.querySelector('[data-se-action="toggle-line-numbers"]')?.setAttribute("aria-checked", String(showLineNumbers));
  }

  function applyBackupOption() {
    root.querySelector('[data-se-action="toggle-backup"]')?.setAttribute("aria-checked", String(backupOnSave));
  }

  function runAction(action, target) {
    switch (action) {
      case "close": requestClose(); break;
      case "new": newFile(); break;
      case "save": save(); break;
      case "reload": reloadFromDisk(); break;
      case "load-script": loadIntoInterpreter(); break;
      case "unload-script": unloadFromInterpreter(); break;
      case "undo": els.ta.focus(); document.execCommand("undo"); break;
      case "redo": els.ta.focus(); document.execCommand("redo"); break;
      case "find": openFind(); break;
      case "find-next": findNext(true); break;
      case "find-prev": findNext(false); break;
      case "find-close": hideFind(); break;
      case "goto": gotoLine(); break;
      case "toggle-line-numbers":
        showLineNumbers = !showLineNumbers;
        try {
          localStorage.setItem(LINE_NUMBERS_KEY, showLineNumbers ? "on" : "off");
        } catch { }
        applyLineNumbers();
        break;
      case "toggle-backup":
        backupOnSave = !backupOnSave;
        try {
          localStorage.setItem(BACKUP_KEY, backupOnSave ? "on" : "off");
        } catch { }
        applyBackupOption();
        break;
      case "toggle-tree":
        treeCollapsed = !treeCollapsed;
        renderTree(true);
        break;
      default: break;
    }
    void target;
  }

  function attachEvents() {
    root.addEventListener("click", (e) => {
      const dlg = e.target.closest("[data-se-dlg]");
      if (dlg) {
        finishDialog(dlg.dataset.seDlg);
        return;
      }
      if (dialogState) return;

      const actionEl = e.target.closest("[data-se-action]");
      if (actionEl) {
        const menu = actionEl.closest("details");
        if (menu) menu.open = false;
        runAction(actionEl.dataset.seAction, actionEl);
        return;
      }
      const file = e.target.closest(".se-file");
      if (file) {
        selectFile(file.dataset.file);
        return;
      }
      if (e.target === root) {
        // The scrim does not close the editor; an accidental click must not lose work.
        if (doc) els.ta.focus();
      }
    });

    // Keep the caret in the text when a toolbar button is pressed.
    root.addEventListener("mousedown", (e) => {
      if (e.button !== 0) return;
      const btn = e.target.closest(".se-toolbar [data-se-action]");
      if (btn) e.preventDefault();
    });

    root.addEventListener("input", (e) => {
      if (e.target === els.ta) onInput();
      else if (e.target === els.findInput) {
        hit = null;
        els.findInfo.textContent = "";
        positionBars();
      }
    });

    root.addEventListener("scroll", (e) => {
      if (e.target === els.ta) onScroll();
    }, true);

    root.addEventListener("wheel", (e) => {
      if (e.target.closest(".se-gutter") && doc) {
        els.ta.scrollTop += e.deltaY;
        els.ta.scrollLeft += e.deltaX;
      }
    }, { passive: true });

    for (const type of ["keyup", "mouseup", "focusin", "select"]) {
      root.addEventListener(type, (e) => {
        if (e.target === els.ta) scheduleStatus();
      });
    }
    document.addEventListener("selectionchange", () => {
      if (isOpen && document.activeElement === els.ta) scheduleStatus();
    });

    root.addEventListener("keydown", onKeyDown);
  }

  function onKeyDown(e) {
    if (dialogState) {
      handleDialogKey(e);
      return;
    }
    const ctrl = e.ctrlKey || e.metaKey;
    const key = e.key;
    const lower = key.length === 1 ? key.toLowerCase() : key;

    if (key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      const openMenu = root.querySelector("details[open]");
      if (openMenu) {
        openMenu.open = false;
        return;
      }
      if (!els.find.hidden && e.target === els.findInput) {
        hideFind();
        return;
      }
      requestClose();
      return;
    }

    if (e.altKey && !ctrl && e.code === "KeyR") {
      e.preventDefault();
      return;
    }
    if (ctrl && !e.altKey && lower === "s") {
      e.preventDefault();
      save();
      return;
    }
    if (ctrl && !e.altKey && lower === "f") {
      e.preventDefault();
      openFind();
      return;
    }
    if (ctrl && !e.altKey && lower === "g") {
      e.preventDefault();
      gotoLine();
      return;
    }
    if (key === "F3") {
      e.preventDefault();
      findNext(!e.shiftKey);
      return;
    }

    if (e.target === els.findInput && key === "Enter") {
      e.preventDefault();
      findNext(!e.shiftKey);
      return;
    }

    if (e.target === els.ta && !ctrl && !e.altKey && !e.isComposing) {
      if (key === "Tab") {
        e.preventDefault();
        handleTab(e.shiftKey);
        return;
      }
      if (key === "Enter") {
        e.preventDefault();
        handleEnter();
        return;
      }
    }

    const item = e.target.closest ? e.target.closest(".se-file") : null;
    if (item) {
      const all = [...els.tree.querySelectorAll(".se-file")];
      const idx = all.indexOf(item);
      if (key === "ArrowDown" && idx < all.length - 1) { e.preventDefault(); all[idx + 1].focus(); }
      else if (key === "ArrowUp" && idx > 0) { e.preventDefault(); all[idx - 1].focus(); }
      else if (key === "Home") { e.preventDefault(); all[0].focus(); }
      else if (key === "End") { e.preventDefault(); all[all.length - 1].focus(); }
      else if (key === "Enter" || key === " ") { e.preventDefault(); selectFile(item.dataset.file); }
    }
  }

  // ---- Leaving the app with unsaved changes -----------------------------

  window.addEventListener("beforeunload", (e) => {
    if (isOpen && dirty) {
      e.preventDefault();
      e.returnValue = "";
    }
  });

  async function exitApp() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("exit_after_script_prompt");
    } catch { }
  }

  async function onAppCloseRequested() {
    if (!isOpen || !dirty || dialogState) {
      if (!dialogState) await exitApp();
      return;
    }
    const choice = await showDialog({
      title: "Unsaved changes",
      message: `Save changes to ${doc.name} before closing Ferric?`,
      buttons: [
        { id: "save", label: "Save", primary: true },
        { id: "discard", label: "Don't save" },
        { id: "cancel", label: "Cancel" },
      ],
      cancelId: "cancel",
    });
    if (choice.id === "discard") await exitApp();
    else if (choice.id === "save" && (await save())) await exitApp();
  }

  if (isTauri) {
    import("@tauri-apps/api/event")
      .then(({ listen }) => listen("script-close-requested", () => { onAppCloseRequested(); }))
      .catch(() => { });
  }

  if (!isTauri) getApi();

  return {
    open,
    close: requestClose,
    isOpen: () => isOpen,
    isDirty: () => dirty,
  };
}
