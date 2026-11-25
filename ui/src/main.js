import { renderSpansToHtml, parseIrcToHtml, escapeHtml, IRC_COLORS } from "./utils/ircColors.js";
import { createScriptEditor } from "./scriptEditor.js";
import { fetchScriptMenuNodes, renderScriptMenuNodes, bindScriptMenuItems, runScriptMenuNode } from "./scriptMenus.js";
import { icons } from "./icons.js";
import { openChannelCentral, closeChannelCentral, refreshChannelCentral, channelCentralTarget } from "./channelCentral.js";
import { initScriptDialog, isScriptDialogOpen, getOpenScriptDialogTarget, refreshOpenScriptDialog, closeOpenScriptDialog } from "./scriptDialog.js";
import { openServerManager, closeServerManager, isServerManagerOpen, renderServerManager } from "./serverManager.js";
import { parseServerArgs, buildServerTarget } from "./serverCommand.js";
import {
  initColorSchemes,
  openColorsModal as openColorsDialog,
  closeColorsModal,
  isColorsModalOpen,
  renderColorsModal,
  getActiveSchemeName,
  setOnColorsChanged
} from "./colorsModal.js";
import {
  initAddressBook,
  openAddressBookModal as openAddressBookDialog,
  closeAddressBookModal,
  isAddressBookModalOpen,
  renderAddressBookModal,
  setAddressBookCallbacks,
  getAddressBookColors,
  getAddressBookHighlight,
  matchNickColor,
  checkAddressBookHighlight,
  isUserIgnored,
  setWhoisInfo
} from "./addressBookModal.js";

// Check if running inside Tauri
const isTauri = window.__TAURI_INTERNALS__ !== undefined;
let invoke = async () => { };
let listen = async () => { };
const scriptEditor = createScriptEditor({
  isTauri,
  onStatus: (text) => {
    const net = state.networks[state.active_network];
    if (net) pushStatusNotice(net.id, text, net.active_buffer);
    else window.alert(text);
  },
});
const THEME_STORAGE_KEY = "ferric-theme";

function readThemePreference() {
  try {
    const saved = localStorage.getItem(THEME_STORAGE_KEY);
    if (saved === "dark" || saved === "light") return saved;
  } catch { }
  return "dark";
}

const SWITCHBAR_STORAGE_KEY = "ferric-switchbar";

function readSwitchbarPreference() {
  try {
    return localStorage.getItem(SWITCHBAR_STORAGE_KEY) === "on";
  } catch { }
  return false;
}

// View menu flags. A flag that is on by default is stored as "off" when cleared.
const VIEW_FLAG_KEYS = {
  showMenubar: "ferric-menubar",
  showToolbar: "ferric-toolbar",
  showTreebar: "ferric-treebar",
  compact: "ferric-compact",
  doNotDisturb: "ferric-do-not-disturb",
  nickColors: "ferric-nick-colors",
};

function readViewFlag(name, defaultOn) {
  try {
    const saved = localStorage.getItem(VIEW_FLAG_KEYS[name]);
    if (saved === "on") return true;
    if (saved === "off") return false;
  } catch { }
  return defaultOn;
}

function saveViewFlag(name) {
  try {
    localStorage.setItem(VIEW_FLAG_KEYS[name], state[name] ? "on" : "off");
  } catch { }
}

const FOLLOW_SYSTEM_THEME_STORAGE_KEY = "ferric-follow-system-theme";

function readFollowSystemThemePreference() {
  try {
    return localStorage.getItem(FOLLOW_SYSTEM_THEME_STORAGE_KEY) === "on";
  } catch { }
  return false;
}

// Mouse page: command run on a double-click, one setting per target (Options > Mouse).
// "used: false" targets have no Ferric counterpart yet; their value is stored but never run.
const MOUSE_DBLCLICK_PREFIX = "ferric-mouse-dblclick-";
const MOUSE_DBLCLICK_TARGETS = [
  { id: "status", label: "Status", used: true, fallback: "/lusers" },
  { id: "query", label: "Query", used: true, fallback: "/whois $$1" },
  { id: "channel", label: "Channel", used: true, fallback: "/channel" },
  { id: "message", label: "Message", used: false },
  { id: "nicklist", label: "Nickname List", used: true, fallback: "/query $$1" },
  { id: "notify", label: "Notify List", used: false },
];

function readMouseDblclick(id) {
  try {
    const value = localStorage.getItem(MOUSE_DBLCLICK_PREFIX + id);
    if (value !== null) return value.slice(0, 200);
  } catch { }
  if (id === "message" || id === "notify") return readMouseDblclick("query");
  return MOUSE_DBLCLICK_TARGETS.find((target) => target.id === id)?.fallback || "";
}

function saveMouseDblclick(id, value) {
  try {
    localStorage.setItem(MOUSE_DBLCLICK_PREFIX + id, value.slice(0, 200));
  } catch { }
}

const DROP_RULES_PREFIX = "ferric-drop-rules-";
const DROP_RULE_DEFAULTS = { normal: "*.wav:/sound $1 $2-\n*.*:/dcc send $1 $2-", shift: "*.*:/dcc send $1 $2-" };
let selectedDropRules = "normal";

function readDropRules(mode) {
  try {
    return localStorage.getItem(DROP_RULES_PREFIX + mode) ?? DROP_RULE_DEFAULTS[mode];
  } catch { return DROP_RULE_DEFAULTS[mode]; }
}

// Nick ignore list (Ferric design choice): lowercase nicks in localStorage, at most 200.
// The host of a nick, as the last message showed it. Outside a channel the user list is empty, so
// queries and parted or quit senders keep their host for the hostmask ignores.
const senderHostCache = new Map();
const SENDER_HOST_MAX = 2000;

function rememberSenderHost(nick, host) {
  if (senderHostCache.size >= SENDER_HOST_MAX) senderHostCache.clear();
  senderHostCache.set(String(nick).toLowerCase(), host);
}

function senderHostFor(sender) {
  return senderUser(sender)?.host || senderHostCache.get(String(sender || "").toLowerCase()) || "";
}

const IGNORE_LIST_KEY = "ferric-ignore-list";
const IGNORE_LIST_MAX = 200;

function readIgnoreList() {
  try {
    const parsed = JSON.parse(localStorage.getItem(IGNORE_LIST_KEY) || "[]");
    if (Array.isArray(parsed)) return parsed.filter((n) => typeof n === "string").slice(0, IGNORE_LIST_MAX);
  } catch { }
  return [];
}

function saveIgnoreList(list) {
  try {
    localStorage.setItem(IGNORE_LIST_KEY, JSON.stringify(list));
  } catch { }
}

function isIgnoredNick(nick) {
  return typeof nick === "string" && nick.length > 0 && readIgnoreList().includes(nick.toLowerCase());
}

function addIgnoredNick(nick) {
  if (!validIrcToken(nick)) return false;
  const list = readIgnoreList();
  const key = nick.toLowerCase();
  if (list.includes(key)) return true;
  if (list.length >= IGNORE_LIST_MAX) return false;
  list.push(key);
  saveIgnoreList(list);
  return true;
}

function removeIgnoredNick(nick) {
  if (typeof nick !== "string") return;
  const key = nick.toLowerCase();
  const list = readIgnoreList();
  if (list.includes(key)) saveIgnoreList(list.filter((n) => n !== key));
}

// View > Font...: one font per area, stored as one JSON object under ferric-fonts
// ({ area: { family, size, style } }). Each area is shown through a CSS custom property;
// an area without its own font falls back to --font-default, then to the stylesheet value.
const FONTS_STORAGE_KEY = "ferric-fonts";
const FONT_FAMILIES = [
  ["Microsoft Sans Serif", "sans-serif"], ["Tahoma", "sans-serif"], ["Segoe UI", "sans-serif"], ["Arial", "sans-serif"],
  ["Verdana", "sans-serif"], ["Calibri", "sans-serif"], ["Consolas", "monospace"],
  ["Lucida Console", "monospace"], ["Courier New", "monospace"], ["Cascadia Mono", "monospace"],
];
const FONT_SIZE_MIN = 6;
const FONT_SIZE_MAX = 20;
const FONT_STYLES = { Standard: "normal 400", Bold: "normal 700", Italic: "italic 400", "Bold Italic": "italic 700" };
const FONT_AREAS = [
  { id: "default", label: "Default", prop: "--font-default" },
  { id: "chat", label: "Chat", prop: "--font-chat" },
  { id: "nicklist", label: "Nick list", prop: "--font-nicklist" },
  { id: "input", label: "Input line", prop: "--font-input" },
  { id: "tree", label: "Channel tree", prop: "--font-tree" },
  { id: "switchbar", label: "Switchbar", prop: "--font-switchbar" },
];
// Shown in the dialog while nothing is set; the stylesheet default is Segoe UI 12px.
const FONT_BUILTIN = { family: "Segoe UI", size: 12, style: "Standard" };

// Returns a clean { family, size, style } or null when any part is not on the fixed lists.
function sanitizeFont(value) {
  if (!value || typeof value !== "object") return null;
  const entry = FONT_FAMILIES.find(([name]) => name === value.family);
  const size = value.size;
  if (!entry || !Number.isInteger(size) || size < FONT_SIZE_MIN || size > FONT_SIZE_MAX) return null;
  if (typeof value.style !== "string" || !Object.hasOwn(FONT_STYLES, value.style)) return null;
  return { family: entry[0], size, style: value.style };
}

function readFonts() {
  const fonts = {};
  try {
    const parsed = JSON.parse(localStorage.getItem(FONTS_STORAGE_KEY) || "null");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      for (const area of FONT_AREAS) {
        const font = Object.hasOwn(parsed, area.id) ? sanitizeFont(parsed[area.id]) : null;
        if (font) fonts[area.id] = font;
      }
    }
  } catch { }
  return fonts;
}

function saveFonts(fonts) {
  try {
    localStorage.setItem(FONTS_STORAGE_KEY, JSON.stringify(fonts));
  } catch { }
}

// Value for the CSS font shorthand. Family and generic come from the fixed list above.
function fontCss(font) {
  const generic = FONT_FAMILIES.find(([name]) => name === font.family)?.[1] ?? "sans-serif";
  // The size is in points, as the font dialog and the client show it: 10 pt is 13.33 px.
  const px = (font.size * 4) / 3;
  return `${FONT_STYLES[font.style]} ${px.toFixed(2)}px "${font.family}", ${generic}`;
}

function applyFonts(fonts) {
  const root = document.documentElement.style;
  for (const area of FONT_AREAS) {
    if (fonts[area.id]) root.setProperty(area.prop, fontCss(fonts[area.id]));
    else root.removeProperty(area.prop);
  }
}

let fontSettings = readFonts();
applyFonts(fontSettings);

// Select Font dialog. Works on a draft: Cancel drops it, OK saves and applies it.
let fontDialog = null;

function closeFontDialog() {
  if (!fontDialog) return;
  fontDialog.host.remove();
  fontDialog = null;
}

function openFontDialog() {
  if (fontDialog) return;
  const host = document.createElement("div");
  host.id = "font-modal-container";
  const option = (value, label = value) => `<option value="${escapeHtml(value)}">${escapeHtml(label)}</option>`;
  const sizes = Array.from({ length: FONT_SIZE_MAX - FONT_SIZE_MIN + 1 }, (_, i) => String(FONT_SIZE_MIN + i));
  host.innerHTML = `
    <div class="modal-overlay" id="font-modal-backdrop">
      <div class="modal-card font-dialog" role="dialog" aria-modal="true" aria-labelledby="font-title">
        <h2 class="modal-title" id="font-title">Select Font</h2>
        <div class="font-dialog-body">
          <label class="font-wide"><span>Area:</span><select id="font-area" class="form-input">${FONT_AREAS.map((a) => option(a.id, a.label)).join("")}</select></label>
          <label><span>Font:</span><select id="font-family" class="form-input">${FONT_FAMILIES.map(([name]) => option(name)).join("")}</select></label>
          <label><span>Size:</span><select id="font-size" class="form-input">${sizes.map((s) => option(s)).join("")}</select></label>
          <label class="font-wide"><span>Style:</span><select id="font-style" class="form-input">${Object.keys(FONT_STYLES).map((s) => option(s)).join("")}</select></label>
          <div class="font-preview" id="font-preview">AaBbYyZz 0123456789</div>
          <label class="options-toggle font-wide"><input id="font-set-default" type="checkbox" checked /> Set as default</label>
          <label class="options-toggle font-wide"><input id="font-use-default" type="checkbox" /> Use default</label>
        </div>
        <div class="options-dialog-actions">
          <button id="font-ok" class="btn btn-primary" type="button">OK</button>
          <button id="font-cancel" class="btn" type="button">Cancel</button>
        </div>
      </div>
    </div>`;
  document.body.appendChild(host);

  const draft = {};
  for (const [id, font] of Object.entries(fontSettings)) draft[id] = { ...font };
  fontDialog = { host, draft, area: "default" };

  const $ = (id) => host.querySelector(`#${id}`);
  const controls = [$("font-family"), $("font-size"), $("font-style")];
  const readControls = () => sanitizeFont({ family: $("font-family").value, size: Number($("font-size").value), style: $("font-style").value });

  // Shows the area's own font, or the font it inherits, and what the preview looks like.
  const sync = () => {
    const area = fontDialog.area;
    const own = draft[area];
    const font = own || draft.default || FONT_BUILTIN;
    $("font-family").value = font.family;
    $("font-size").value = String(font.size);
    $("font-style").value = font.style;
    const isDefault = area === "default";
    $("font-use-default").checked = !isDefault && !own;
    $("font-use-default").disabled = isDefault;
    $("font-set-default").disabled = !isDefault;
    for (const el of controls) el.disabled = !isDefault && !own;
    $("font-preview").style.font = fontCss(font);
  };

  $("font-area").addEventListener("change", (event) => {
    fontDialog.area = FONT_AREAS.some((a) => a.id === event.target.value) ? event.target.value : "default";
    sync();
  });
  for (const el of controls) {
    el.addEventListener("change", () => {
      const font = readControls();
      if (font) draft[fontDialog.area] = font;
      // Default with "Set as default" checked is applied to every area: drop their own fonts.
      if (fontDialog.area === "default" && $("font-set-default").checked) {
        for (const area of FONT_AREAS) if (area.id !== "default") delete draft[area.id];
      }
      sync();
    });
  }
  $("font-use-default").addEventListener("change", (event) => {
    if (event.target.checked) {
      delete draft[fontDialog.area];
    } else {
      const font = readControls();
      if (font) draft[fontDialog.area] = font;
    }
    sync();
  });
  $("font-ok").addEventListener("click", () => {
    const next = {};
    for (const area of FONT_AREAS) {
      const font = sanitizeFont(draft[area.id]);
      if (font) next[area.id] = font;
    }
    fontSettings = next;
    saveFonts(fontSettings);
    applyFonts(fontSettings);
    closeFontDialog();
  });
  $("font-cancel").addEventListener("click", closeFontDialog);
  $("font-modal-backdrop").addEventListener("click", (event) => {
    if (event.target === event.currentTarget) closeFontDialog();
  });

  sync();
  $("font-area").focus();
}

const systemDarkQuery = window.matchMedia?.("(prefers-color-scheme: dark)") ?? null;
let followSystemTheme = readFollowSystemThemePreference();

function systemThemeName() {
  return systemDarkQuery?.matches ? "dark" : "light";
}

let activeTheme = followSystemTheme ? systemThemeName() : readThemePreference();
document.documentElement.dataset.theme = activeTheme;

function applyTheme(theme, persist = true) {
  activeTheme = theme === "dark" ? "dark" : "light";
  document.documentElement.dataset.theme = activeTheme;
  if (persist) {
    try {
      localStorage.setItem(THEME_STORAGE_KEY, activeTheme);
    } catch { }
  }
}

function syncDarkModeCheckbox() {
  const box = document.getElementById("options-dark-mode");
  if (!box) return;
  box.checked = activeTheme === "dark";
  box.disabled = followSystemTheme;
}

function onSystemThemeChange() {
  if (!followSystemTheme) return;
  applyTheme(systemThemeName(), false);
  syncDarkModeCheckbox();
}

if (systemDarkQuery) {
  if (typeof systemDarkQuery.addEventListener === "function") {
    systemDarkQuery.addEventListener("change", onSystemThemeChange);
  } else if (typeof systemDarkQuery.addListener === "function") {
    systemDarkQuery.addListener(onSystemThemeChange);
  }
}

// Application State
const state = {
  networks: {},
  // PREFIX pairs per network id from the backend: [[mode, symbol], ...].
  networkPrefix: {},
  active_network: null,
  commandHistory: [],
  historyIndex: -1,
  showConnectModal: false,
  showFishModal: false,
  showOptionsModal: false,
  showNotesModal: false,
  optionsPage: "connect-options",
  optionsOpen: { connect: true, irc: true, sounds: true, mouse: true, dcc: true },
  fishKeysList: [],
  fishSearchQuery: "",
  currentBufferKey: null,
  fishStatusTarget: null,
  fishStatusRequest: 0,
  quickTarget: "",
  quickKey: "",
  quickNetwork: "",
  quickFishMode: "ecb",
  showNicklist: true,
  showSwitchbar: readSwitchbarPreference(),
  showMenubar: readViewFlag("showMenubar", true),
  showToolbar: readViewFlag("showToolbar", true),
  showTreebar: readViewFlag("showTreebar", true),
  compact: readViewFlag("compact", false),
  // Window state is not persisted: the window always starts normal.
  fullscreen: false,
  onTop: false,
  // Stops the attention flashing for queries.
  doNotDisturb: readViewFlag("doNotDisturb", false),
  nickColors: readViewFlag("nickColors", true),
  collapsedNets: {},
  // Query buffers that got a message while not active, keyed by "network/buffer" (lowercased).
  attention: new Set(),
};

// Last rendered output per region. The DOM is only touched when a freshly
// computed value differs from the one stored here.
const rendered = {
  sidebar: null,
  sidebarStructure: null,
  switchbar: null,
  switchbarStructure: null,
  nicklist: null,
  nickSrc: null,
  nickNet: null,
  topic: null,
  fishBadge: null,
  log: null, // { key, count, first, last }
};

// Backend counters the user has already seen, keyed by "net/buffer".
const readBaseline = {};
// Locally created buffers the backend does not know about yet.
const pendingBuffers = new Set();
let lastBackendRevision = null;
// WHOIS answers are read once each: per network, the id of the newest status message already looked at.
const whoisScannedByNet = new Map();
// Local notices get negative ids, so they are never taken for backend messages.
let localNoticeId = 0;
function nextLocalId() {
  localNoticeId -= 1;
  return localNoticeId;
}
// The first sync asks for every message; later syncs get only what is new (see applyMessageMode).
let needsFullSnapshot = true;
// Newest messages kept for a buffer nobody has open; each sync sends this many at most.
const UI_TAIL_KEEP = 200;
// Scrollback of the open buffer (Options > Display > General), within the backend's own cap.
function scrollbackLines() {
  const n = parseInt(appOptions.scrollbackLines, 10);
  return Math.min(2000, Math.max(200, Number.isFinite(n) ? n : 2000));
}

function setText(el, value) {
  if (el && el.textContent !== value) el.textContent = value;
}

function invalidateLog() {
  rendered.log = null;
}

function parseFishKey(value) {
  const match = /^(cbc|ecb):(.*)$/is.exec(value || "");
  return match
    ? { mode: match[1].toLowerCase(), key: match[2] }
    : { mode: "ecb", key: value || "" };
}

function formatFishKey(key, mode) {
  const rawKey = parseFishKey(key).key;
  return mode === "cbc" ? `cbc:${rawKey}` : rawKey;
}

// Bare keys stay ECB, the mode every FiSH client reads. CBC needs an explicit "cbc:".
function formatFishCommandKey(key) {
  if (/^cbc:/i.test(key)) return `cbc:${key.slice(4)}`;
  if (/^ecb:/i.test(key)) return key.slice(4);
  return key;
}

// Initial DOM Shell Mount (runs once)
function mountAppShell() {
  const app = document.getElementById("app");
  if (!app) return;

  app.innerHTML = `
    <div class="app-menu-bar" role="menubar" aria-label="Application menu">
      <details class="menu-dropdown">
        <summary role="menuitem">File</summary>
        <div class="menu-popover" role="menu">
          <button type="button" role="menuitem" data-menu-action="connect">Connect to server…</button>
          <button type="button" role="menuitem" data-menu-action="connect-all">Connect all</button>
        </div>
      </details>
      <details class="menu-dropdown">
        <summary role="menuitem">View</summary>
        <div class="menu-popover" role="menu">
          <button type="button" role="menuitem" data-menu-action="toggle-nicklist">Show or hide nick list</button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitemcheckbox" class="has-key" aria-checked="true" data-menu-action="toggle-menubar"><span>Menubar</span><kbd class="menu-key">Alt+M</kbd></button>
          <button type="button" role="menuitemcheckbox" aria-checked="true" data-menu-action="toggle-toolbar">Toolbar</button>
          <button type="button" role="menuitemcheckbox" aria-checked="false" data-menu-action="toggle-switchbar">Switchbar</button>
          <button type="button" role="menuitemcheckbox" aria-checked="true" data-menu-action="toggle-treebar">Treebar</button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitemradio" aria-checked="true" data-menu-action="set-normal">Normal</button>
          <button type="button" role="menuitemradio" aria-checked="false" data-menu-action="set-compact">Compact</button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitem" data-menu-action="open-font">Font…</button>
          <button type="button" role="menuitem" class="has-key" data-menu-action="open-colors"><span>Colors…</span><kbd class="menu-key">Alt+K</kbd></button>
          <button type="button" role="menuitem" class="has-key" data-menu-action="open-abook"><span>Address Book…</span><kbd class="menu-key">Alt+B</kbd></button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitemcheckbox" aria-checked="false" data-menu-action="toggle-fullscreen">Full Screen</button>
          <button type="button" role="menuitemcheckbox" aria-checked="false" data-menu-action="toggle-ontop">On Top</button>
        </div>
      </details>
      <details class="menu-dropdown">
        <summary role="menuitem">Tools</summary>
        <div class="menu-popover" role="menu">
          <button type="button" role="menuitemcheckbox" aria-checked="false" data-menu-action="toggle-dnd">Do Not Disturb</button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitem" class="has-key" data-menu-action="open-options"><span>Options…</span><kbd class="menu-key">Alt+O</kbd></button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitem" class="has-key" data-menu-action="open-script-editor"><span>Scripts Editor…</span><kbd class="menu-key">Alt+R</kbd></button>
          <button type="button" role="menuitem" data-menu-action="open-server-manager">Server manager…</button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitem" class="has-key" data-menu-action="open-colors"><span>Colors…</span><kbd class="menu-key">Alt+K</kbd></button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitem" class="has-key" data-menu-action="open-abook"><span>Address Book…</span><kbd class="menu-key">Alt+B</kbd></button>
          <div class="menu-separator" role="separator"></div>
          <button type="button" role="menuitem" data-menu-action="open-fish">FiSH key manager…</button>
        </div>
      </details>
    </div>

    <div class="app-toolbar main-toolbar" role="toolbar" aria-label="IRC actions">
      <div class="toolbar-group">
        <button id="btn-open-connect" class="toolbar-button toolbar-icon" type="button" title="Connect to server" aria-label="Connect">${icons.connect}</button>
        <button id="btn-load-connectd" class="toolbar-button toolbar-icon" type="button" title="Connect all" aria-label="Connect all">${icons.connectAll}</button>
        <span class="toolbar-divider" aria-hidden="true"></span>
        <button id="btn-join" class="toolbar-button toolbar-icon" type="button" title="Join channel" aria-label="Join channel">${icons.join}</button>
        <button id="btn-list" class="toolbar-button toolbar-icon" type="button" title="Channel list" aria-label="Channel list">${icons.channelList}</button>
        <span class="toolbar-divider" aria-hidden="true"></span>
        <button id="btn-open-options" class="toolbar-button toolbar-icon" type="button" title="Options" aria-label="Options">${icons.options}</button>
        <button id="btn-open-script-editor" class="toolbar-button toolbar-icon" type="button" title="Scripts Editor" aria-label="Scripts Editor">${icons.scripts}</button>
        <button id="btn-open-font" class="toolbar-button toolbar-icon" type="button" title="Font" aria-label="Font">${icons.font}</button>
        <button id="btn-open-colors" class="toolbar-button toolbar-icon" type="button" title="Colors (Alt+K)" aria-label="Colors">${icons.colors}</button>
        <button id="btn-open-abook" class="toolbar-button toolbar-icon" type="button" title="Address Book (Alt+B)" aria-label="Address Book">${icons.addressBook}</button>
        <button id="btn-open-fish" class="toolbar-button toolbar-icon" type="button" title="FiSH Blowfish Key Manager" aria-label="FiSH keys">${icons.fishKeys}</button>
      </div>
      <div class="toolbar-rss" id="toolbar-rss" style="${appOptions.rssEnabled ? '' : 'display:none;'}">
        <button type="button" class="rss-badge" id="btn-rss-badge" title="RSS News Feed (Click to configure in Options)">
          <span class="rss-dot"></span>
          <span>RSS</span>
        </button>
        <div class="rss-viewport" id="rss-viewport">
          <div class="rss-ticker-track" id="rss-ticker-track">
            <span class="rss-empty-hint">Loading RSS feeds...</span>
          </div>
        </div>
      </div>
      <span class="toolbar-divider" aria-hidden="true"></span>
      <span class="toolbar-product"><span>Ferric</span></span>
    </div>

    <div class="workspace">
      <aside class="sidebar" id="sidebar" aria-label="Servers and channels">
        <div class="sidebar-heading">
          <span>Servers</span>
          <span id="brand-stats">0/0</span>
        </div>
        <div id="nav-section" class="nav-section"></div>
      </aside>

      <div class="workspace-resizer resizer-left" id="sidebar-resizer" role="separator" aria-orientation="vertical" aria-label="Resize channel list" tabindex="0" title="Drag to resize (double-click to reset)"></div>

      <main class="chat-container">
        <header class="topic-bar">
          <div class="topic-heading">
            <span id="topic-title" class="active-title">No buffer</span>
            <span id="topic-text" class="topic-text">Connect to a server to start chatting.</span>
          </div>
          <div class="topic-actions">
            <button id="btn-topic-fish" class="fish-status-badge badge-off" style="display:none;" title="FiSH encryption status">
              <span class="fish-icon">FiSH</span> <span class="fish-label">Off</span>
            </button>
          </div>
        </header>

        <div id="messages-log" class="messages-log" aria-live="polite">
          <div class="empty-state">No buffer selected</div>
        </div>

        <footer class="input-area">
          <textarea
            id="chat-input"
            class="chat-input"
            rows="1"
            placeholder="Connect to a server to start chatting"
            autocomplete="off"
            spellcheck="false"
            disabled
          ></textarea>
        </footer>
      </main>

      <div class="workspace-resizer resizer-right" id="nicklist-resizer" role="separator" aria-orientation="vertical" aria-label="Resize nick list" tabindex="0" title="Drag to resize (double-click to reset)"></div>

      <aside class="nicklist" id="nicklist" aria-label="Nick list">
        <div class="nicklist-header">
          <span>Nick list</span>
          <span id="nicklist-count">0</span>
        </div>
        <div id="nicklist-users" class="nicklist-users"></div>
      </aside>
    </div>

    <nav id="switchbar" class="switchbar" aria-label="Open chat windows"></nav>

    <footer class="app-status" aria-live="polite">
      <span id="status-connection" class="status-connection">Disconnected</span>
      <span id="status-network">No network</span>
      <span id="status-server">No server</span>
      <span id="status-buffer" class="status-buffer">No buffer</span>
    </footer>

    <div id="modal-container"></div>
    <div id="context-menu" class="context-menu" style="display:none;"></div>
  `;

  applyTheme(activeTheme, false);
  initColorSchemes();
  initAddressBook();
  setAddressBookCallbacks(
    (cmd) => {
      const net = state.networks[state.active_network];
      if (net && isTauri) {
        if (cmd.startsWith("/")) {
          handleUserInput(cmd);
        } else {
          void invoke("send_raw", { network: state.active_network, raw: cmd });
        }
      }
    },
    () => {
      rowHtmlCache.clear();
      render();
    }
  );
  setOnColorsChanged(() => {
    rowHtmlCache.clear();
    render();
  });
  applyWorkspaceWidths();
  attachStaticHandlers();
  initWorkspaceResizers();
  initContextMenu();
  rssManager.init();
  // Double-clicking the topic bar opens Channel Central for the active channel.
  document.querySelector(".topic-bar")?.addEventListener("dblclick", openChannelCentralForActive);
  // The empty part of the chat (the log itself, not a message row) does the same.
  // A click in the chat puts the cursor in the message box, unless the click ends a text selection.
  document.getElementById("messages-log")?.addEventListener("click", () => {
    const input = document.getElementById("chat-input");
    if (!input || input.disabled || window.getSelection()?.toString()) return;
    input.focus({ preventScroll: true });
  });
  document.getElementById("messages-log")?.addEventListener("dblclick", (e) => {
    if (e.target === e.currentTarget) openChannelCentralForActive();
  });
  document.getElementById("messages-log")?.addEventListener(
    "scroll",
    () => prependOlderMessagesIfNeeded(),
    { passive: true }
  );
}

// What Channel Central shows for one channel, read from the current state. Null when the channel
// buffer no longer exists.
function channelCentralViewFor(networkId, channelName) {
  const net = state.networks[networkId];
  const buf = net ? net.buffers[String(channelName).toLowerCase()] : null;
  if (!net || !buf || !buf.is_channel) return null;
  const ownNick = (net.current_nick || net.nick || "").toLowerCase();
  const send = (line) => {
    if (isTauri) void invoke("send_raw", { network: networkId, raw: line });
  };
  return {
    network: networkId,
    channel: buf.name,
    topic: buf.topic || "",
    setBy: buf.topic_set_by || "",
    history: buf.topic_history || [],
    flags: buf.channel_flags || "",
    key: buf.channel_key || null,
    limit: buf.channel_limit || null,
    bans: buf.bans || [],
    excepts: buf.excepts || [],
    invites: buf.invites || [],
    canOp: /[@&~]/.test(buf.users?.[ownNick]?.prefix || ""),
    send,
    onSetTopic: (text) => send(`TOPIC ${buf.name} :${text}`),
    // 324 gives the flags, key and limit; MODE with b, e and I gives the three lists (367/348/346).
    requestLists: () => {
      send(`MODE ${buf.name}`);
      send(`MODE ${buf.name} b`);
      send(`MODE ${buf.name} e`);
      send(`MODE ${buf.name} I`);
    },
  };
}

// Opens Channel Central for one channel: checks script alias "channel" first.
async function openChannelCentralFor(networkId, channelName) {
  const net = state.networks[networkId];
  const buf = net ? net.buffers[String(channelName).toLowerCase()] : null;
  if (!net || !buf || !buf.is_channel) return;
  if (isTauri) {
    try {
      const res = await invoke("run_script_alias", {
        name: "channel",
        args: [buf.name],
        network: networkId,
        buffer: buf.name,
      });
      if (res) {
        await syncFromBackend();
        return;
      }
    } catch (err) {
      console.error("run_script_alias error:", err);
    }
  }
  const next = channelCentralViewFor(networkId, buf.name);
  if (next) openChannelCentral(next);
}

// Opens Channel Central for the active channel buffer; does nothing on status or query buffers.
function openChannelCentralForActive() {
  const net = state.networks[state.active_network];
  const buf = net ? net.buffers[net.active_buffer] : null;
  if (!net || !buf || !buf.is_channel) return;
  void openChannelCentralFor(state.active_network, buf.name);
}

// Keeps an open Channel Central current while the server answers.
function refreshOpenChannelCentral() {
  const target = channelCentralTarget();
  if (!target) return;
  const next = channelCentralViewFor(target.network, target.channel);
  if (next) refreshChannelCentral(next);
}

const SIDEBAR_WIDTH_KEY = "ferric-sidebar-width";
const NICKLIST_WIDTH_KEY = "ferric-nicklist-width";
const DEFAULT_SIDEBAR_WIDTH = 176;
const DEFAULT_NICKLIST_WIDTH = 120;
const MIN_SIDEBAR_WIDTH = 100;
const MIN_NICKLIST_WIDTH = 80;

function readSavedWidth(key, fallback) {
  try {
    const val = parseInt(localStorage.getItem(key), 10);
    if (!isNaN(val) && val >= 50 && val <= 1200) return val;
  } catch { }
  return fallback;
}

function applyWorkspaceWidths() {
  const sbWidth = readSavedWidth(SIDEBAR_WIDTH_KEY, DEFAULT_SIDEBAR_WIDTH);
  const nlWidth = readSavedWidth(NICKLIST_WIDTH_KEY, DEFAULT_NICKLIST_WIDTH);
  document.documentElement.style.setProperty("--sidebar-width", `${sbWidth}px`);
  document.documentElement.style.setProperty("--nicklist-width", `${nlWidth}px`);
}

function initWorkspaceResizers() {
  const sidebarResizer = document.getElementById("sidebar-resizer");
  const nicklistResizer = document.getElementById("nicklist-resizer");
  const sidebarEl = document.getElementById("sidebar");
  const nicklistEl = document.getElementById("nicklist");

  function setupResizer(resizer, targetEl, side, storageKey, defaultWidth, minWidth) {
    if (!resizer || !targetEl) return;

    let isDragging = false;
    let startX = 0;
    let startWidth = 0;

    const onPointerDown = (e) => {
      if (e.button !== 0) return;
      isDragging = true;
      startX = e.clientX;
      startWidth = targetEl.getBoundingClientRect().width;
      resizer.classList.add("is-active");
      try { resizer.setPointerCapture(e.pointerId); } catch { }
      document.body.classList.add("is-resizing");
      e.preventDefault();
    };

    const onPointerMove = (e) => {
      if (!isDragging) return;
      const delta = e.clientX - startX;
      const maxW = Math.floor(window.innerWidth * (side === "left" ? 0.55 : 0.45));
      const newWidth = Math.max(minWidth, Math.min(maxW, Math.round(side === "left" ? startWidth + delta : startWidth - delta)));
      const cssVar = side === "left" ? "--sidebar-width" : "--nicklist-width";
      document.documentElement.style.setProperty(cssVar, `${newWidth}px`);
    };

    const onPointerUp = (e) => {
      if (!isDragging) return;
      isDragging = false;
      resizer.classList.remove("is-active");
      try { resizer.releasePointerCapture(e.pointerId); } catch { }
      document.body.classList.remove("is-resizing");

      const currentWidth = Math.round(targetEl.getBoundingClientRect().width);
      try {
        localStorage.setItem(storageKey, String(currentWidth));
      } catch { }
    };

    resizer.addEventListener("pointerdown", onPointerDown);
    resizer.addEventListener("pointermove", onPointerMove);
    resizer.addEventListener("pointerup", onPointerUp);
    resizer.addEventListener("pointercancel", onPointerUp);

    resizer.addEventListener("dblclick", () => {
      const cssVar = side === "left" ? "--sidebar-width" : "--nicklist-width";
      document.documentElement.style.setProperty(cssVar, `${defaultWidth}px`);
      try { localStorage.setItem(storageKey, String(defaultWidth)); } catch { }
    });

    resizer.addEventListener("keydown", (e) => {
      if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        const step = (e.shiftKey ? 20 : 6) * (e.key === "ArrowRight" ? 1 : -1) * (side === "left" ? 1 : -1);
        const currentWidth = Math.round(targetEl.getBoundingClientRect().width);
        const maxW = Math.floor(window.innerWidth * (side === "left" ? 0.55 : 0.45));
        const newWidth = Math.max(minWidth, Math.min(maxW, currentWidth + step));
        const cssVar = side === "left" ? "--sidebar-width" : "--nicklist-width";
        document.documentElement.style.setProperty(cssVar, `${newWidth}px`);
        try { localStorage.setItem(storageKey, String(newWidth)); } catch { }
        e.preventDefault();
      }
    });
  }

  setupResizer(sidebarResizer, sidebarEl, "left", SIDEBAR_WIDTH_KEY, DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH);
  setupResizer(nicklistResizer, nicklistEl, "right", NICKLIST_WIDTH_KEY, DEFAULT_NICKLIST_WIDTH, MIN_NICKLIST_WIDTH);
}

// Input line history kept for the Up and Down keys.
const MAX_COMMAND_HISTORY = 200;

// The attach functions below bind to elements and to document/window that live for the whole
// session, so each runs its bindings once even if it is called again.
let delegatedHandlersAttached = false;
let staticHandlersAttached = false;
let contextMenuInitialized = false;

function attachDelegatedHandlers() {
  if (delegatedHandlersAttached) return;
  delegatedHandlersAttached = true;
  const nav = document.getElementById("nav-section");
  const bar = document.getElementById("switchbar");
  const nicks = document.getElementById("nicklist-users");

  if (nav) {
    // The wheel is not intercepted: the browser's own scrolling is immediate, and the custom
    // smooth scroll that used to sit here lagged behind every notch.

    // Activate on pointer down so a re-render between down and up cannot swallow the click.
    nav.addEventListener("mousedown", (e) => {
      if (e.button !== 0 || e.ctrlKey) return;
      if (e.target.closest(".btn-close-buf") || e.target.closest(".tree-box")) return;
      const item = e.target.closest(".channel-item");
      if (item) {
        e.preventDefault(); // keep focus in the input line
        activateBuffer(item.dataset.net, item.dataset.buf);
        return;
      }
      // The collapse marker (.tree-box) returns early above, so only the network name activates status.
      const header = e.target.closest(".network-header");
      if (header && state.networks[header.dataset.net]) {
        e.preventDefault();
        activateBuffer(header.dataset.net, "status");
      }
    });

    nav.addEventListener("click", (e) => {
      const closeButton = e.target.closest(".btn-close-buf");
      if (closeButton) {
        closeBuffer(closeButton.dataset.net, closeButton.dataset.buf);
        return;
      }
      const box = e.target.closest(".tree-box");
      if (box) {
        const netId = box.dataset.toggleNet;
        if (!state.networks[netId]) return;
        state.collapsedNets[netId] = !state.collapsedNets[netId];
        snapshotScroll();
        updateSidebar();
        flushRestores();
      }
    });

    nav.addEventListener("dblclick", (e) => {
      if (e.target.closest(".btn-close-buf") || e.target.closest(".tree-box")) return;
      const item = e.target.closest(".channel-item");
      const buf = item && state.networks[item.dataset.net]?.buffers?.[(item.dataset.buf || "").toLowerCase()];
      if (!buf) return;
      const isStatus = buf.name.toLowerCase() === "status";
      runMouseDblclick(isStatus ? "status" : buf.is_channel ? "channel" : "query", {
        network: item.dataset.net,
        nick: isStatus || buf.is_channel ? "" : buf.name,
        chan: buf.is_channel ? buf.name : "",
      });
    });

    nav.addEventListener("keydown", (e) => {
      if (e.key !== "Enter" && e.key !== " ") return;
      const item = e.target.closest(".channel-item");
      if (!item || e.target !== item) return;
      e.preventDefault();
      activateBuffer(item.dataset.net, item.dataset.buf);
    });
  }

  if (bar) {
    bar.addEventListener("mousedown", (e) => {
      if (e.button !== 0 || e.ctrlKey) return;
      const tab = e.target.closest(".switchbar-tab");
      if (!tab) return;
      e.preventDefault();
      activateBuffer(tab.dataset.net, tab.dataset.buf);
    });
    // Keyboard activation of a focused tab arrives as a click without pointer (detail 0).
    bar.addEventListener("click", (e) => {
      if (e.detail !== 0) return;
      const tab = e.target.closest(".switchbar-tab");
      if (tab) activateBuffer(tab.dataset.net, tab.dataset.buf);
    });
  }

  if (nicks) {
    nicks.addEventListener("dblclick", (e) => {
      const el = e.target.closest(".nick-item");
      if (!el || !el.dataset.nick) return;
      const net = state.networks[state.active_network];
      const active = net?.buffers?.[net.active_buffer];
      runMouseDblclick("nicklist", {
        network: state.active_network,
        nick: el.dataset.nick,
        chan: active?.is_channel ? active.name : "",
      });
    });
    nicks.addEventListener("click", selectNickFromClick);
  }
}

function attachStaticHandlers() {
  if (staticHandlersAttached) return;
  staticHandlersAttached = true;
  attachDelegatedHandlers();

  document.addEventListener("click", (event) => {
    const currentMenu = event.target.closest(".menu-dropdown");
    document.querySelectorAll(".menu-dropdown[open]").forEach((menu) => {
      if (menu !== currentMenu) menu.open = false;
    });
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      document.querySelectorAll(".menu-dropdown[open]").forEach((menu) => { menu.open = false; });
      if (state.showOptionsModal) closeOptionsModal();
      if (state.showNotesModal) closeNotesModal();
      if (isColorsModalOpen()) closeColorsModal();
      if (isAddressBookModalOpen()) closeAddressBookModal();
      if (isServerManagerOpen()) closeServerManager();
      closeFontDialog();
    }
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.code === "KeyO") {
      event.preventDefault();
      openOptionsModal();
    }
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.code === "KeyK") {
      event.preventDefault();
      openColorsModal();
    }
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.code === "KeyB") {
      event.preventDefault();
      openAddressBookModal();
    }
    // Alt+M also brings the menubar back: with it hidden the View menu is out of reach.
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.code === "KeyM") {
      event.preventDefault();
      toggleViewFlag("showMenubar");
    }
    // Alt+R opens the script editor.
    if (event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey && event.code === "KeyR") {
      event.preventDefault();
      scriptEditor.open();
    }
  });

  document.querySelectorAll("[data-menu-action]").forEach((item) => {
    item.addEventListener("click", () => {
      const menu = item.closest("details");
      if (menu) menu.open = false;

      switch (item.dataset.menuAction) {
        case "connect":
          document.getElementById("btn-open-connect")?.click();
          break;
        case "connect-all":
          document.getElementById("btn-load-connectd")?.click();
          break;
        case "open-fish":
          document.getElementById("btn-open-fish")?.click();
          break;
        case "open-options":
          openOptionsModal();
          break;
        case "open-script-editor":
          scriptEditor.open();
          break;
        case "open-server-manager":
          openServerManagerDialog();
          break;
        case "toggle-nicklist":
          state.showNicklist = !state.showNicklist;
          render();
          break;
        case "toggle-switchbar":
          state.showSwitchbar = !state.showSwitchbar;
          try {
            localStorage.setItem(SWITCHBAR_STORAGE_KEY, state.showSwitchbar ? "on" : "off");
          } catch { }
          render();
          break;
        case "toggle-menubar":
          toggleViewFlag("showMenubar");
          break;
        case "toggle-toolbar":
          toggleViewFlag("showToolbar");
          break;
        case "toggle-treebar":
          toggleViewFlag("showTreebar");
          break;
        case "toggle-dnd":
          toggleViewFlag("doNotDisturb");
          if (state.doNotDisturb) {
            state.attention.clear();
            render();
          }
          break;
        case "set-normal":
        case "set-compact": {
          const wantCompact = item.dataset.menuAction === "set-compact";
          if (state.compact !== wantCompact && !(wantCompact && state.fullscreen)) {
            state.compact = wantCompact;
            saveViewFlag("compact");
            render();
          }
          break;
        }
        case "open-font":
          openFontDialog();
          break;
        case "open-colors":
          openColorsModal();
          break;
        case "open-abook":
          openAddressBookModal();
          break;
        case "toggle-fullscreen":
          if (!state.compact) void setWindowFlag("set_fullscreen", "fullscreen");
          break;
        case "toggle-ontop":
          void setWindowFlag("set_always_on_top", "onTop");
          break;
      }
    });
  });

  // Connect All button
  const btnLoadConnectd = document.getElementById("btn-load-connectd");
  if (btnLoadConnectd) {
    btnLoadConnectd.addEventListener("click", async () => {
      if (isTauri) {
        btnLoadConnectd.disabled = true;
        btnLoadConnectd.title = btnLoadConnectd.ariaLabel = "Connecting…";
        try {
          await loadConnectdWithProxy();
          await syncFromBackend();
        } catch (e) {
          console.error("load_connectd error:", e);
          pushStatusNotice(state.active_network, String(e?.message || e));
        } finally {
          btnLoadConnectd.disabled = false;
          btnLoadConnectd.title = btnLoadConnectd.ariaLabel = "Connect all";
        }
      }
    });
  }

  // Open modal button
  const btnOpenConnect = document.getElementById("btn-open-connect");
  if (btnOpenConnect) {
    btnOpenConnect.addEventListener("click", () => {
      dismissSlotDialogs();
      state.showConnectModal = true;
      state.showFishModal = false;
      state.showOptionsModal = false;
      state.showNotesModal = false;
      renderModal();
    });
  }

  // Open FiSH modal button
  const btnOpenFish = document.getElementById("btn-open-fish");
  if (btnOpenFish) {
    btnOpenFish.addEventListener("click", () => {
      openFishModal();
    });
  }

  // Toolbar buttons that share the code of a menu item.
  document.getElementById("btn-open-options")?.addEventListener("click", () => openOptionsModal());
  document.getElementById("btn-open-script-editor")?.addEventListener("click", () => scriptEditor.open());
  document.getElementById("btn-open-font")?.addEventListener("click", () => openFontDialog());
  document.getElementById("btn-open-colors")?.addEventListener("click", () => openColorsModal());
  document.getElementById("btn-open-abook")?.addEventListener("click", () => openAddressBookModal());
  // Join and Channel list run the popup-menu items "Join Channel…" and "List Channels".
  const toolbarPopup = (kind, match) => {
    const item = REFERENCE_POPUPS[kind].find(match);
    if (item && state.networks[state.active_network]) {
      void runReferencePopupAction(item, { network: state.active_network, channel: "", nick: "", nicks: [], fishTarget: "" });
    }
  };
  document.getElementById("btn-join")?.addEventListener("click", () => toolbarPopup("menubar", (i) => i.action === "join"));
  document.getElementById("btn-list")?.addEventListener("click", () => toolbarPopup("status", (i) => i.command === "LIST"));

  // Topic bar FiSH status badge click
  const btnTopicFish = document.getElementById("btn-topic-fish");
  if (btnTopicFish) {
    btnTopicFish.addEventListener("click", () => {
      const net = state.networks[state.active_network];
      const activeBuf = net ? net.buffers[net.active_buffer] : null;
      openFishModal(activeBuf ? activeBuf.name : null);
    });
  }

  // RSS badge and ticker item click handlers
  document.getElementById("btn-rss-badge")?.addEventListener("click", () => {
    state.optionsPage = "display-rss";
    openOptionsModal();
  });

  document.addEventListener("click", (e) => {
    const item = e.target.closest(".rss-ticker-item");
    if (item) {
      e.preventDefault();
      const url = item.dataset.url;
      if (url && (url.startsWith("http://") || url.startsWith("https://"))) {
        openExternalUrl(url);
      }
    }
  });

  // Persistent Input Field Handlers
  const input = document.getElementById("chat-input");
  if (input) {
    // Enter sends every line of the input, one command after another; Shift+Enter adds a line.
    input.addEventListener("input", () => resizeChatInput(input));
    // IRC control codes on Ctrl+key. Ctrl+K opens the color index instead. Ctrl+R is left out: it reloads the page.
    const CHAT_FORMAT_CODES = { KeyB: String.fromCharCode(2), KeyU: String.fromCharCode(31), KeyI: String.fromCharCode(29), KeyE: String.fromCharCode(30), KeyO: String.fromCharCode(15) };

    input.addEventListener("keydown", async (e) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.code === "KeyK") {
        e.preventDefault();
        openChatColorPopup(input);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && CHAT_FORMAT_CODES[e.code]) {
        e.preventDefault();
        input.setRangeText(CHAT_FORMAT_CODES[e.code], input.selectionStart, input.selectionEnd, "end");
        resizeChatInput(input);
        return;
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const lines = input.value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
        if (lines.length === 0) return;
        input.value = "";
        resizeChatInput(input);
        for (const line of lines) {
          state.commandHistory.push(line);
          if (state.commandHistory.length > MAX_COMMAND_HISTORY) {
            state.commandHistory.splice(0, state.commandHistory.length - MAX_COMMAND_HISTORY);
          }
          state.historyIndex = state.commandHistory.length;
          await handleUserInput(line);
        }
      } else if (e.key === "ArrowUp") {
        if (state.historyIndex > 0) {
          state.historyIndex--;
          input.value = state.commandHistory[state.historyIndex] || "";
        }
      } else if (e.key === "ArrowDown") {
        if (state.historyIndex < state.commandHistory.length - 1) {
          state.historyIndex++;
          input.value = state.commandHistory[state.historyIndex] || "";
        } else {
          state.historyIndex = state.commandHistory.length;
          input.value = "";
        }
      }
    });
  }
}

function toggleViewFlag(name) {
  state[name] = !state[name];
  saveViewFlag(name);
  render();
}

// Asks the backend to change the window, and records the new state only when it worked.
async function setWindowFlag(command, name) {
  if (!isTauri) return;
  try {
    await invoke(command, { enabled: !state[name] });
    state[name] = !state[name];
  } catch (e) {
    console.warn(`${command} failed:`, e);
  }
  render();
}

let viewMenuSig = null;

function syncViewMenu() {
  const sig = [
    state.showMenubar, state.showToolbar, state.showTreebar, state.showSwitchbar, state.compact,
    state.fullscreen, state.onTop, state.doNotDisturb, isTauri,
    Boolean(state.networks[state.active_network]),
  ].join("|");
  if (sig === viewMenuSig) return;
  viewMenuSig = sig;
  document.querySelector('[data-menu-action="toggle-switchbar"]')?.setAttribute("aria-checked", String(state.showSwitchbar));
  const app = document.getElementById("app");
  if (app) {
    app.classList.toggle("hide-menubar", !state.showMenubar);
    app.classList.toggle("hide-toolbar", !state.showToolbar);
    app.classList.toggle("hide-treebar", !state.showTreebar);
    app.classList.toggle("compact", state.compact);
  }
  const checks = {
    "toggle-menubar": state.showMenubar,
    "toggle-toolbar": state.showToolbar,
    "toggle-treebar": state.showTreebar,
    "set-normal": !state.compact,
    "set-compact": state.compact,
    "toggle-fullscreen": state.fullscreen,
    "toggle-ontop": state.onTop,
    "toggle-dnd": state.doNotDisturb,
  };
  for (const [action, checked] of Object.entries(checks)) {
    document.querySelector(`[data-menu-action="${action}"]`)?.setAttribute("aria-checked", String(checked));
  }
  // Full Screen and Compact disable each other. The window items need the Tauri backend.
  const disabled = {
    "toggle-fullscreen": state.compact || !isTauri,
    "set-compact": state.fullscreen,
    "toggle-ontop": !isTauri,
  };
  for (const [action, value] of Object.entries(disabled)) {
    const el = document.querySelector(`[data-menu-action="${action}"]`);
    if (el) el.disabled = value;
  }
  // Join and Channel list send to the active network, so they need the backend and a network.
  const noNetwork = !isTauri || !state.networks[state.active_network];
  for (const id of ["btn-join", "btn-list"]) {
    const el = document.getElementById(id);
    if (el) el.disabled = noNetwork;
  }
}

// Backend Synchronization
let syncRunning = null;
let syncAgain = false;
const SYNC_MIN_GAP_MS = 150;
let syncTimer = null;

// Query windows to open or flag after a sync, one entry per window. A burst of messages from one
// person leaves one entry, and the sync loop runs the entries: no callback is queued per event.
const pendingQueryFollowUps = new Map();

// Short beep for a query message (Options > Events > Play sound for query messages). No audio file.
let queryAudioCtx = null;
function playQuerySound() {
  try {
    queryAudioCtx = queryAudioCtx || new (window.AudioContext || window.webkitAudioContext)();
    if (queryAudioCtx.state === "suspended") queryAudioCtx.resume().catch(() => {});
    const osc = queryAudioCtx.createOscillator();
    const gain = queryAudioCtx.createGain();
    osc.type = "sine";
    osc.frequency.value = 880;
    gain.gain.value = 0.08;
    osc.connect(gain);
    gain.connect(queryAudioCtx.destination);
    osc.start();
    osc.stop(queryAudioCtx.currentTime + 0.12);
  } catch { /* no audio output available */ }
}

// Taskbar flash for a query message (Options > Messages > Flash on message: Query).
// Only while the Ferric window is not focused; the taskbar button keeps flashing until it is.
async function flashForQuery() {
  if (!isTauri || document.hasFocus()) return;
  try {
    const { getCurrentWindow, UserAttentionType } = await import("@tauri-apps/api/window");
    await getCurrentWindow().requestUserAttention(UserAttentionType.Informational);
  } catch (e) {
    console.warn("requestUserAttention failed:", e);
  }
}

function runQueryFollowUps() {
  if (pendingQueryFollowUps.size === 0) return;
  const jobs = [...pendingQueryFollowUps.values()];
  pendingQueryFollowUps.clear();
  let dirty = false;
  for (const job of jobs) {
    const syncedNet = state.networks[job.network];
    if (!syncedNet?.buffers[job.key]) continue;
    if (job.opened) {
      // activateBuffer focuses the chat input; text already typed there is kept.
      activateBuffer(job.network, job.sender);
      if (appOptions.eventsSound) playQuerySound();
      if (appOptions.flashQuery) flashForQuery();
      continue;
    }
    const viewing = state.active_network === job.network && syncedNet.active_buffer === job.key;
    if (viewing || state.doNotDisturb) continue;
    state.attention.add(`${job.network}/${job.key}`);
    if (appOptions.eventsSound) playQuerySound();
    if (appOptions.flashQuery) flashForQuery();
    dirty = true;
  }
  // One redraw for the whole burst, not one per window.
  if (dirty) render();
}

const frontendPerf = {
  syncCount: 0,
  syncTimestamps: [],
  lastIpcMs: 0,
  lastJsMs: 0,
  lastSnapshotIpcMs: 0,
  lastSnapshotJsMs: 0,
  lastRenderTotalMs: 0,
  lastSidebarMs: 0,
  lastMessagesMs: 0,
  lastNicklistMs: 0,
};

function recordSyncTick() {
  const now = performance.now();
  frontendPerf.syncCount++;
  frontendPerf.syncTimestamps.push(now);
  const cutoff = now - 60000;
  while (frontendPerf.syncTimestamps.length > 0 && frontendPerf.syncTimestamps[0] < cutoff) {
    frontendPerf.syncTimestamps.shift();
  }
}

function getFrontendSyncsPerMin() {
  const now = performance.now();
  const cutoff = now - 60000;
  while (frontendPerf.syncTimestamps.length > 0 && frontendPerf.syncTimestamps[0] < cutoff) {
    frontendPerf.syncTimestamps.shift();
  }
  return frontendPerf.syncTimestamps.length;
}

function syncFromBackend() {
  if (!isTauri) return Promise.resolve();
  if (syncRunning) {
    syncAgain = true;
    return syncRunning;
  }
  syncRunning = (async () => {
    try {
      do {
        syncAgain = false;
        await syncOnce();
        runQueryFollowUps();
        refreshOpenChannelCentral();
        const scriptDlgTarget = getOpenScriptDialogTarget();
        if (scriptDlgTarget) {
          const targetNet = state.networks[scriptDlgTarget.network];
          const targetBuf = targetNet?.buffers?.[scriptDlgTarget.channel.toLowerCase()];
          if (targetBuf) {
            const sig = `${targetBuf.topic || ""}|${targetBuf.topic_set_by || ""}|${targetBuf.channel_flags || ""}|${targetBuf.channel_key || ""}|${targetBuf.channel_limit || ""}|${targetBuf.lists_hash || 0}`;
            refreshOpenScriptDialog(sig);
          }
        }
        // Events that arrived meanwhile get one follow-up sync, at most every SYNC_MIN_GAP_MS.
        if (syncAgain) await new Promise((resolve) => setTimeout(resolve, SYNC_MIN_GAP_MS));
      } while (syncAgain);
    } finally {
      syncRunning = null;
    }
  })();
  return syncRunning;
}


// WHOIS lines as the backend writes them (client.rs whois_line). One pattern per shape, so a
// status line such as "Alice is on channel: #x" is not read as an address.
function inspectForWhois(text) {
  if (!text || typeof text !== "string") return;
  const t = text.trim();
  let m = t.match(/^(\S+) \[([^\]]+)\]\s*(.*)$/);
  if (m) return setWhoisInfo({ nick: m[1], address: m[2], name: m[3], channels: "", idle: "", away: "", server: "", status: "", ctcp: "" });
  m = t.match(/^(\S+) is away:\s*(.*)$/);
  if (m) return setWhoisInfo({ nick: m[1], away: m[2] });
  m = t.match(/^(\S+) is an IRC operator$/);
  if (m) return setWhoisInfo({ nick: m[1], status: "IRC operator" });
  m = t.match(/^(\S+) is logged in as\s*(.*)$/);
  if (m) return setWhoisInfo({ nick: m[1], status: `logged in as ${m[2]}` });
  m = t.match(/^(\S+) idle:\s*(.*)$/);
  if (m) return setWhoisInfo({ nick: m[1], idle: m[2] });
  m = t.match(/^(\S+) is on channel:\s*(.*)$/);
  if (m) return setWhoisInfo({ nick: m[1], channels: m[2] });
  m = t.match(/^(\S+) using server\s*(.*)$/);
  if (m) return setWhoisInfo({ nick: m[1], server: m[2] });
}

// Puts a buffer's messages together from what the UI held before and what the backend sent now.
function applyMessageMode(prevBuf, buf) {
  const mode = buf.messages_mode;
  delete buf.messages_mode;
  buf._lastFishId = prevBuf?._lastFishId ?? null;

  if (mode === "same") {
    buf.messages = prevBuf?.messages || [];
    return;
  }
  const fresh = buf.messages || [];
  for (const m of fresh) {
    if (m.clock === undefined) m.clock = formatClock(m.timestamp);
    if (m.text && m.text.includes("*** FiSH: ")) {
      buf._lastFishId = m.id;
    }
  }
  if (mode === "append") {
    const list = prevBuf?.messages || [];
    if (fresh.length > 0) list.push(...fresh);
    buf.messages = list;
  } else {
    const locals = (prevBuf?.messages || []).filter((m) => m.local);
    buf.messages = locals.length
      ? [...fresh, ...locals].sort((a, b) => (a.timestamp || 0) - (b.timestamp || 0))
      : fresh;
  }
}

// Fetches a buffer's full scrollback when it is opened, then redraws. Messages that arrived while
// the history was on its way stay at the end.
async function loadBufferHistory(networkId, key) {
  if (!isTauri) return;
  const limit = scrollbackLines();
  try {
    const history = await invoke("get_buffer_history", { network: networkId, buffer: key, limit });
    const net = state.networks[networkId];
    const buf = net?.buffers[key];
    if (!buf || net.active_buffer !== key || state.active_network !== networkId) return;

    const lastId = history.length ? history[history.length - 1].id : 0;
    const later = (buf.messages || []).filter((m) => m.local || m.id > lastId);
    for (const m of history) {
      if (m.clock === undefined) m.clock = formatClock(m.timestamp);
    }

    const oldLen = (buf.messages || []).length;
    buf.messages = [...history, ...later]
      .sort((a, b) => (a.timestamp || 0) - (b.timestamp || 0))
      .slice(-limit);

    // Falls der Buffer bereits sichtbar ist, nur den startIndex verschieben statt DOM neu zu bauen
    const curLog = rendered.log && rendered.log !== "empty" ? rendered.log : null;
    const bufKey = `${networkId}/${buf.name}`;
    if (curLog && curLog.key === bufKey && oldLen > 0) {
      const addedBefore = Math.max(0, buf.messages.length - oldLen);
      curLog.startIndex += addedBefore;
      curLog.count = buf.messages.length;
      curLog.first = buf.messages.length > 0 ? rowSig(buf.messages[0]) : "";
      prependOlderMessagesIfNeeded();
    } else {
      invalidateLog();
      render();
    }
  } catch (err) {
    console.warn("get_buffer_history failed:", err);
  }
}

// Loads the history of whichever buffer is open right now (used once, after the first sync).
function loadActiveHistory() {
  const net = state.networks[state.active_network];
  if (net?.active_buffer) void loadBufferHistory(state.active_network, net.active_buffer);
}

// /clear: empties the window here and in the backend. Opening a window loads its scrollback from
// the backend, so without the backend clear the lines would come back.
async function clearBufferLines(networkId, key) {
  const buf = state.networks[networkId]?.buffers[key];
  if (!buf) return;
  buf.messages = [];
  invalidateLog();
  render();
  if (!isTauri) return;
  try {
    await invoke("clear_buffer_history", { network: networkId, buffer: key });
  } catch (err) {
    console.warn("clear_buffer_history failed:", err);
  }
}

// Puts a buffer's lists together from what the UI held before. "same" keeps them: the backend
// leaves them out of a sync when they did not change.
function applyListsMode(prevBuf, buf) {
  const same = buf.lists_mode === "same";
  delete buf.lists_mode;
  if (!same) return;
  // A new window with no lists yet starts empty; its lists come when it is shown.
  buf.users = prevBuf?.users || {};
  buf.topic_history = prevBuf?.topic_history || [];
  buf.bans = prevBuf?.bans || [];
  buf.excepts = prevBuf?.excepts || [];
  buf.invites = prevBuf?.invites || [];
}

async function syncOnce() {
  recordSyncTick();
  let t1 = performance.now();
  try {
    // The revision the UI last applied: the backend answers null when it has not moved.
    const t0 = performance.now();
    const backendState = await invoke("get_state", { full: needsFullSnapshot, seenRevision: lastBackendRevision });
    t1 = performance.now();
    const ipcMs = t1 - t0;
    frontendPerf.lastIpcMs = ipcMs;

    const fullRequested = needsFullSnapshot;
    needsFullSnapshot = false;
    if (!backendState || !backendState.networks || typeof backendState.networks !== "object") {
      frontendPerf.lastJsMs = performance.now() - t1;
      return;
    }

    // The backend moves `revision` only when what it would send has changed. The revision of the last
    // applied sync means nothing new for the UI: leave local state and the DOM alone.
    if (!fullRequested && backendState.revision === lastBackendRevision) {
      frontendPerf.lastJsMs = performance.now() - t1;
      return;
    }
    lastBackendRevision = backendState.revision;
    frontendPerf.lastSnapshotIpcMs = ipcMs;

    const prevNetworks = state.networks;
    const networks = backendState.networks;
    const networkIds = Object.keys(networks);
    // The window the backend holds as shown, read before the UI's own choice below changes the objects.
    const backendActiveNetwork = backendState.active_network;
    const backendShown = Object.fromEntries(Object.entries(networks).map(([id, n]) => [id, n.active_buffer]));

    // Messages arrive as a delta: "same" keeps the UI's list, "append" adds the new messages to it.
    for (const net of Object.values(networks)) {
      const openKey = prevNetworks[net.id]?.active_buffer;
      for (const [key, buf] of Object.entries(net.buffers || {})) {
        applyMessageMode(prevNetworks[net.id]?.buffers?.[key], buf);
        applyListsMode(prevNetworks[net.id]?.buffers?.[key], buf);
        // Only the open buffer keeps its scrollback; the others keep a short tail.
        const open = state.active_network === net.id && openKey === key;
        const keep = open ? scrollbackLines() : UI_TAIL_KEEP;
        if (buf.messages.length > keep) {
          buf.messages.splice(0, buf.messages.length - keep);
        }
      }
    }

    if (networkIds.length === 0) {
      state.active_network = null;
    } else if (!networks[state.active_network]) {
      state.active_network = networks[backendState.active_network]
        ? backendState.active_network
        : networkIds[0];
    }

    for (const net of Object.values(networks)) {
      if (!net.buffers || typeof net.buffers !== "object") net.buffers = {};
      const prev = prevNetworks[net.id];

      // Locally created queries the backend has not acknowledged yet stay visible.
      if (prev) {
        for (const key of Object.keys(prev.buffers || {})) {
          if (!net.buffers[key] && pendingBuffers.has(`${net.id}/${key}`)) {
            net.buffers[key] = prev.buffers[key];
          }
        }
      }

      // The locally selected buffer wins over the backend's idea of it.
      if (prev && prev.active_buffer && net.buffers[prev.active_buffer]) {
        net.active_buffer = prev.active_buffer;
      } else if (!net.active_buffer || !net.buffers[net.active_buffer]) {
        net.active_buffer = Object.keys(net.buffers)[0] || "status";
      }

      for (const [key, buf] of Object.entries(net.buffers)) {
        const mark = `${net.id}/${key}`;
        const rawUnread = buf._rawUnread !== undefined ? buf._rawUnread : (buf.unread_count || 0);
        const rawHighlight = buf._rawHighlight !== undefined ? buf._rawHighlight : (buf.highlight_count || 0);
        buf._rawUnread = rawUnread;
        buf._rawHighlight = rawHighlight;

        // Subtract what the user has already read; a smaller backend value means it was reset there.
        const base = readBaseline[mark];
        if (base) {
          if (rawUnread < base.u || rawHighlight < base.h) {
            delete readBaseline[mark];
          } else {
            buf.unread_count = rawUnread - base.u;
            buf.highlight_count = rawHighlight - base.h;
          }
        }

        // The buffer being viewed is always read.
        if (state.active_network === net.id && net.active_buffer === key) {
          readBaseline[mark] = { u: rawUnread, h: rawHighlight };
          buf.unread_count = 0;
          buf.highlight_count = 0;
        }
      }
    }

    // Forget per-buffer bookkeeping for buffers and networks that no longer exist.
    const liveMarks = new Set();
    for (const net of Object.values(networks)) {
      for (const key of Object.keys(net.buffers)) liveMarks.add(`${net.id}/${key}`);
    }
    for (const mark of Object.keys(readBaseline)) {
      if (!liveMarks.has(mark)) delete readBaseline[mark];
    }
    for (const mark of [...state.attention]) {
      if (!liveMarks.has(mark)) state.attention.delete(mark);
    }
    for (const netId of Object.keys(state.collapsedNets)) {
      if (!networks[netId]) delete state.collapsedNets[netId];
    }
    // WHOIS scan positions of networks that were closed.
    for (const netId of [...whoisScannedByNet.keys()]) {
      if (!networks[netId]) whoisScannedByNet.delete(netId);
    }

    state.networks = networks;
    state.networkPrefix = backendState.network_prefix && typeof backendState.network_prefix === "object" ? backendState.network_prefix : {};

    // WHOIS answers arrive in the status window as server notices (sender "*"). Each message is read once.
    const scanNet = state.active_network;
    const statusMessages = state.networks[scanNet]?.buffers?.status?.messages;
    if (statusMessages && statusMessages.length > 0) {
      let scanned = whoisScannedByNet.get(scanNet) || 0;
      let startIdx = 0;
      for (let i = statusMessages.length - 1; i >= 0; i--) {
        const msg = statusMessages[i];
        if (msg.local || msg.id <= 0) continue;
        if (msg.id <= scanned) {
          startIdx = i + 1;
          break;
        }
      }
      for (let i = startIdx; i < statusMessages.length; i++) {
        const msg = statusMessages[i];
        if (msg.local || msg.id <= 0 || msg.id <= scanned) continue;
        scanned = msg.id;
        if (msg.sender === "*" && msg.text) inspectForWhois(msg.text);
      }
      if (scanNet) whoisScannedByNet.set(scanNet, scanned);
    }
    if (networkIds.length === 0) {
      state.active_network = null;
      const jsMs = performance.now() - t1;
      frontendPerf.lastJsMs = jsMs;
      frontendPerf.lastSnapshotJsMs = jsMs;
      render();
      return;
    }

    if (!state.networks[state.active_network]) {
      state.active_network = state.networks[backendState.active_network]
        ? backendState.active_network
        : networkIds[0];
    }

    // Selecting a different network by fallback: that buffer is now being viewed.
    const curNet = state.networks[state.active_network];
    const curBuf = curNet && curNet.buffers[curNet.active_buffer];
    if (curBuf) {
      readBaseline[`${curNet.id}/${curNet.active_buffer}`] = { u: curBuf._rawUnread || 0, h: curBuf._rawHighlight || 0 };
      curBuf.unread_count = 0;
      curBuf.highlight_count = 0;
    }

    // The backend sends a window's lists only while it holds that window as shown. When the UI shows
    // another one (a network it fell back to, or a window it kept over the backend's own choice), tell it.
    const shownNet = state.active_network;
    const shownBuf = state.networks[shownNet]?.active_buffer;
    if (shownNet && shownBuf && (backendActiveNetwork !== shownNet || backendShown[shownNet] !== shownBuf)) {
      reportShownBuffer();
    }

    const jsMs = performance.now() - t1;
    frontendPerf.lastJsMs = jsMs;
    frontendPerf.lastSnapshotJsMs = jsMs;
    render();
  } catch (e) {
    console.error("Failed to sync backend state:", e);
    // The state may be half applied and the backend has recorded what it sent: take everything next time.
    needsFullSnapshot = true;
  }
}

// Incremental UI Rendering (does not replace whole DOM)
// Scroll offsets and focus are read before the first DOM write of a render and put back after
// the last one, so a render forces layout once instead of after every region.
const scrollSnap = new Map();
let pendingRestores = [];

function snapshotScroll() {
  scrollSnap.clear();
  for (const id of ["nav-section", "switchbar", "nicklist-users"]) {
    const el = document.getElementById(id);
    if (el) scrollSnap.set(el, { top: el.scrollTop, left: el.scrollLeft });
  }
}

function flushRestores() {
  const list = pendingRestores;
  pendingRestores = [];
  for (const fn of list) fn();
}

// Runs a full render after the browser has painted the current frame.
let renderQueued = false;
function renderSoon() {
  if (renderQueued) return;
  renderQueued = true;
  const run = () => {
    if (!renderQueued) return;
    renderQueued = false;
    render();
  };
  requestAnimationFrame(() => setTimeout(run, 0));
  setTimeout(run, 120); // background window: animation frames do not fire
}

function render() {
  renderQueued = false;
  const renderStart = performance.now();
  const logEl = document.getElementById("messages-log");
  const logWasAtBottom = !logEl || logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 80;
  snapshotScroll();
  const net = state.networks[state.active_network];
  const activeBuf = net ? net.buffers[net.active_buffer] : null;
  const app = document.getElementById("app");
  if (app) app.classList.toggle("hide-nicklist", !state.showNicklist);
  if (app) app.classList.toggle("hide-switchbar", !state.showSwitchbar);
  syncViewMenu();

  // 1. Stats Counter
  const totalNetworks = Object.keys(state.networks).length;
  const connectedCount = Object.values(state.networks).filter(n => n.connected).length;
  setText(document.getElementById("brand-stats"), `${connectedCount}/${totalNetworks}`);

  const connectionStatus = document.getElementById("status-connection");
  if (connectionStatus) {
    setText(connectionStatus, net?.connected ? "Connected" : "Disconnected");
    connectionStatus.classList.toggle("is-connected", Boolean(net?.connected));
  }
  setText(document.getElementById("status-network"), net ? net.id : "No network");
  setText(document.getElementById("status-server"), net ? (net.server || "Server unknown") : "No server");
  setText(document.getElementById("status-buffer"), activeBuf ? activeBuf.name : "No buffer");

  // 2. Sidebar Navigation
  const tSide0 = performance.now();
  updateSidebar();
  updateSwitchbar();
  const tSide1 = performance.now();

  // 3. Topic Bar
  setText(document.getElementById("topic-title"), activeBuf ? activeBuf.name : (net ? net.id : "No buffer"));
  const topicText = document.getElementById("topic-text");
  if (topicText) {
    const rawTopic = activeBuf && activeBuf.topic ? activeBuf.topic : (net ? `Server: ${net.server || net.id}` : "Connect to a server to start chatting.");
    if (rendered.topic !== rawTopic) {
      rendered.topic = rawTopic;
      topicText.innerHTML = parseIrcToHtml(rawTopic);
    }
  }

  // Update FiSH status badge. A new FiSH status line in this window (a key set by DH1080, or a key
  // removed) makes the badge check the key again, even when the window itself did not change.
  const fishLineId = activeBuf?._lastFishId ?? null;
  const newFishLine = fishLineId !== null && fishLineId !== state.lastFishLineId;
  state.lastFishLineId = fishLineId;
  checkActiveBufferFish(newFishLine);

  // 4. Message Log (preserves scroll position when reading scrollback)
  const tMsg0 = performance.now();
  const scrollLog = updateMessages(activeBuf, logWasAtBottom);
  const tMsg1 = performance.now();

  // 5. Input Field Placeholder (input element stays untouched!)
  const input = document.getElementById("chat-input");
  if (input) {
    const placeholder = activeBuf ? `Message ${activeBuf.name} (type /help, /query <nick>, /fish <key>)` : "Connect to a server to start chatting";
    if (input.placeholder !== placeholder) input.placeholder = placeholder;
    const disabled = !activeBuf;
    if (input.disabled !== disabled) input.disabled = disabled;
  }

  // 6. Nicklist
  const tNick0 = performance.now();
  updateNicklist(activeBuf);
  const tNick1 = performance.now();

  // All writes are done: restore scroll offsets and focus, then stick the log to the bottom.
  flushRestores();
  if (scrollLog && logEl) logEl.scrollTop = logEl.scrollHeight;

  const renderEnd = performance.now();
  frontendPerf.lastRenderTotalMs = renderEnd - renderStart;
  frontendPerf.lastSidebarMs = tSide1 - tSide0;
  frontendPerf.lastMessagesMs = tMsg1 - tMsg0;
  frontendPerf.lastNicklistMs = tNick1 - tNick0;
}

// Replace the children of a container only when the markup changed, keeping
// scroll offsets and the keyboard focus of a focused item across the swap.
function focusKey(el) {
  return `${el.className.split(" ")[0]}|${el.dataset.net || ""}|${el.dataset.buf || ""}|${el.dataset.toggleNet || ""}`;
}

function swapHtml(container, cacheKey, html, itemSelector) {
  if (rendered[cacheKey] === html) return false;
  rendered[cacheKey] = html;

  const snap = scrollSnap.get(container) || { top: container.scrollTop, left: container.scrollLeft };
  const focused = container.contains(document.activeElement) && document.activeElement !== container
    ? document.activeElement
    : null;
  const wanted = focused ? focusKey(focused) : null;

  container.innerHTML = html;

  // Restored at the end of render() (flushRestores), after every region has been written.
  pendingRestores.push(() => {
    if (snap.top) container.scrollTop = snap.top;
    if (snap.left) container.scrollLeft = snap.left;
    if (wanted && itemSelector) {
      for (const el of container.querySelectorAll(itemSelector)) {
        if (focusKey(el) === wanted) {
          el.focus({ preventScroll: true });
          break;
        }
      }
    }
  });
  return true;
}

// Networks in the server order, which is also the startup order. Networks the list does not name go last.
function sortedNetworks() {
  // A server switched off in the server manager is not listed at all.
  return Object.values(state.networks).filter((net) => net.enabled !== false).sort((a, b) => {
    const oa = a.order ?? Number.MAX_SAFE_INTEGER;
    const ob = b.order ?? Number.MAX_SAFE_INTEGER;
    return oa - ob || String(a.id).localeCompare(String(b.id));
  });
}

function updateSidebar() {
  const navSection = document.getElementById("nav-section");
  if (!navSection) return;

  const networks = sortedNetworks();
  const structureKey = networks.map((net) => {
    const isCollapsed = Boolean(state.collapsedNets[net.id]);
    const bufs = Object.values(net.buffers).map((buf) => `${buf.name}:${buf.is_channel ? 1 : 0}`).join(",");
    return `${net.id}:${isCollapsed ? 1 : 0}:${bufs}`;
  }).join(";");

  // In-place update: if networks, buffers and collapse states are unchanged, mutate DOM nodes in-place.
  // This preserves element identity so mouse hover highlights (:hover) never flicker during message activity.
  if (rendered.sidebarStructure === structureKey && navSection.firstElementChild) {
    const headerMap = new Map();
    for (const el of navSection.querySelectorAll(".network-header")) {
      headerMap.set(el.dataset.net, el);
    }
    const itemMap = new Map();
    for (const el of navSection.querySelectorAll(".channel-item")) {
      itemMap.set(`${el.dataset.net}/${(el.dataset.buf || "").toLowerCase()}`, el);
    }

    for (const net of networks) {
      const isCurNet = state.active_network === net.id;
      const head = headerMap.get(net.id);
      if (head) {
        head.classList.toggle("is-current", isCurNet);
        head.classList.toggle("is-status-active", isCurNet && net.active_buffer === "status");
        const netState = head.querySelector(".net-state");
        if (netState) {
          netState.classList.toggle("is-on", net.connected);
          netState.classList.toggle("is-off", !net.connected);
          netState.title = net.connected ? "Connected" : "Connecting / Disconnected";
        }
      }

      for (const buf of Object.values(net.buffers)) {
        const item = itemMap.get(`${net.id}/${buf.name.toLowerCase()}`);
        if (!item) continue;

        const isActive = isCurNet && net.active_buffer === buf.name.toLowerCase();
        const activity = buf.highlight_count > 0 ? "act-highlight" : (buf.unread_count > 0 ? "act-unread" : "act-idle");
        const attention = state.attention.has(`${net.id}/${buf.name.toLowerCase()}`);

        item.classList.toggle("active", isActive);
        item.classList.toggle("is-attention", attention);
        if (!item.classList.contains(activity)) {
          item.classList.remove("act-idle", "act-unread", "act-highlight");
          item.classList.add(activity);
        }

        const right = item.querySelector(".channel-right");
        if (right) {
          let badge = right.querySelector(".badge");
          if (buf.unread_count > 0) {
            const isHl = buf.highlight_count > 0;
            if (!badge) {
              badge = document.createElement("span");
              right.prepend(badge);
            }
            const badgeCls = `badge ${isHl ? "highlight" : ""}`;
            if (badge.className !== badgeCls) badge.className = badgeCls;
            const unreadStr = String(buf.unread_count);
            if (badge.textContent !== unreadStr) badge.textContent = unreadStr;
          } else if (badge) {
            badge.remove();
          }
        }
      }
    }
    return;
  }

  rendered.sidebarStructure = structureKey;
  const html = networks.map((net) => {
    const isCurNet = state.active_network === net.id;
    const isCollapsed = Boolean(state.collapsedNets[net.id]);
    // The network's status window is the network entry itself (as in the classic client), so it gets no row of its own.
    const rows = Object.values(net.buffers).filter((buf) => buf.name.toLowerCase() !== "status").map(buf => {
      const isActive = isCurNet && net.active_buffer === buf.name.toLowerCase();
      const isQuery = !buf.is_channel && buf.name.toLowerCase() !== "status";
      const iconClass = buf.is_channel ? "icon-chan" : (isQuery ? "icon-query" : "icon-status");
      const displayName = buf.is_channel ? buf.name.replace(/^#/, "") : buf.name;
      const activity = buf.highlight_count > 0 ? "act-highlight" : (buf.unread_count > 0 ? "act-unread" : "act-idle");
      const attention = state.attention.has(`${net.id}/${buf.name.toLowerCase()}`);

      return `
          <div class="channel-item ${activity} ${isActive ? "active" : ""} ${attention ? "is-attention" : ""}" tabindex="0" data-net="${escapeHtml(net.id)}" data-buf="${escapeHtml(buf.name)}">
            <span class="chan-label"><span class="chan-icon ${iconClass}" aria-hidden="true"></span><span class="chan-name">${escapeHtml(displayName)}</span></span>
            <div class="channel-right">
              ${buf.unread_count > 0 ? `
                <span class="badge ${buf.highlight_count > 0 ? "highlight" : ""}">${buf.unread_count}</span>
              ` : ""}
              ${isQuery ? `
                <span class="btn-close-buf" data-net="${escapeHtml(net.id)}" data-buf="${escapeHtml(buf.name)}" title="Close query">×</span>
              ` : ""}
            </div>
          </div>
        `;
    }).join("");

    return `
      <div class="tree-net ${isCollapsed ? "is-collapsed" : ""}">
        <div class="network-header ${isCurNet ? "is-current" : ""} ${isCurNet && net.active_buffer === "status" ? "is-status-active" : ""}" data-net="${escapeHtml(net.id)}">
          <button type="button" class="tree-box" data-toggle-net="${escapeHtml(net.id)}" aria-expanded="${isCollapsed ? "false" : "true"}" aria-label="${isCollapsed ? "Expand" : "Collapse"} ${escapeHtml(net.id)}"></button>
          <span class="network-name">${escapeHtml(net.label || net.id)}</span>
          <span class="net-state ${net.connected ? "is-on" : "is-off"}" title="${net.connected ? "Connected" : "Connecting / Disconnected"}"></span>
        </div>
        <div class="tree-children"${isCollapsed ? " hidden" : ""}>${rows}</div>
      </div>
    `;
  }).join("");

  swapHtml(navSection, "sidebar", html, ".channel-item, .tree-box");
}

function updateSwitchbar() {
  const switchbar = document.getElementById("switchbar");
  if (!switchbar || !state.showSwitchbar) return;

  const buffers = sortedNetworks().flatMap((net) =>
    Object.values(net.buffers).map((buf) => ({ net, buf }))
  );
  buffers.sort((a, b) => {
    if (a.net.id !== b.net.id) return a.net.id.localeCompare(b.net.id);
    if (a.buf.name.toLowerCase() === "status") return -1;
    if (b.buf.name.toLowerCase() === "status") return 1;
    return a.buf.name.localeCompare(b.buf.name);
  });

  const structureKey = buffers.map(({ net, buf }) => `${net.id}:${buf.name}`).join(";");

  // In-place update: if switchbar tabs have not changed structure, mutate DOM nodes in-place.
  if (rendered.switchbarStructure === structureKey && switchbar.firstElementChild && !switchbar.querySelector(".switchbar-empty")) {
    const tabMap = new Map();
    for (const el of switchbar.querySelectorAll(".switchbar-tab")) {
      tabMap.set(`${el.dataset.net}/${(el.dataset.buf || "").toLowerCase()}`, el);
    }

    for (const { net, buf } of buffers) {
      const tab = tabMap.get(`${net.id}/${buf.name.toLowerCase()}`);
      if (!tab) continue;

      const isActive = state.active_network === net.id && net.active_buffer === buf.name.toLowerCase();
      const attention = state.attention.has(`${net.id}/${buf.name.toLowerCase()}`);

      tab.classList.toggle("active", isActive);
      tab.classList.toggle("is-attention", attention);
      if (isActive) tab.setAttribute("aria-current", "page");
      else tab.removeAttribute("aria-current");

      const stateDot = tab.querySelector(".switchbar-state");
      if (stateDot) {
        stateDot.classList.toggle("connected", net.connected);
        stateDot.classList.toggle("disconnected", !net.connected);
      }

      let unreadEl = tab.querySelector(".switchbar-unread");
      if (buf.unread_count > 0) {
        if (!unreadEl) {
          unreadEl = document.createElement("span");
          unreadEl.className = "switchbar-unread";
          tab.appendChild(unreadEl);
        }
        const unreadStr = String(buf.unread_count);
        if (unreadEl.textContent !== unreadStr) unreadEl.textContent = unreadStr;
      } else if (unreadEl) {
        unreadEl.remove();
      }
    }
    return;
  }

  rendered.switchbarStructure = structureKey;
  const html = buffers.length === 0
    ? '<span class="switchbar-empty">No open windows</span>'
    : buffers.map(({ net, buf }) => {
      const isActive = state.active_network === net.id && net.active_buffer === buf.name.toLowerCase();
      const label = buf.name.toLowerCase() === "status" ? net.id : buf.name;
      const attention = state.attention.has(`${net.id}/${buf.name.toLowerCase()}`);
      return `
        <button class="switchbar-tab ${isActive ? "active" : ""} ${attention ? "is-attention" : ""}" type="button"
          data-net="${escapeHtml(net.id)}" data-buf="${escapeHtml(buf.name)}"
          title="${escapeHtml(net.id)} — ${escapeHtml(buf.name)}" ${isActive ? 'aria-current="page"' : ""}>
          <span class="switchbar-state ${net.connected ? "connected" : "disconnected"}" aria-hidden="true"></span>
          <span class="switchbar-label">${escapeHtml(label)}</span>
          ${buf.unread_count > 0 ? `<span class="switchbar-unread">${buf.unread_count}</span>` : ""}
        </button>
      `;
    }).join("");

  swapHtml(switchbar, "switchbar", html, ".switchbar-tab");
}

// The backend sends a window's lists only while it holds that window as shown, so it must hold the one
// the UI shows. Sets it there, then syncs at once so that window's lists arrive without a click.
function reportShownBuffer() {
  const networkId = state.active_network;
  const buffer = state.networks[networkId]?.active_buffer;
  if (!isTauri || !networkId || !buffer) return;
  // Kein .then(() => syncFromBackend()) mehr: Listen sind bereits aktuell im State.
  invoke("set_active_buffer", { network: networkId, buffer })
    .catch((e) => console.warn("set_active_buffer failed:", e));
}

function activateBuffer(networkId, bufferName) {
  const net = state.networks[networkId];
  if (!net || !bufferName) return;

  const key = bufferName.toLowerCase();
  const buffer = net.buffers[key];
  if (!buffer) return;

  const leftNet = state.networks[state.active_network];
  const leftBuf = leftNet ? leftNet.buffers[leftNet.active_buffer] : null;
  if (leftBuf && (leftNet !== net || leftNet.active_buffer !== key) && leftBuf.messages?.length > UI_TAIL_KEEP) {
    leftBuf.messages = leftBuf.messages.slice(-UI_TAIL_KEEP);
  }

  state.active_network = networkId;
  net.active_buffer = key;
  reportShownBuffer();
  void loadBufferHistory(networkId, key);
  state.attention.delete(`${networkId}/${key}`);
  readBaseline[`${networkId}/${key}`] = {
    u: buffer._rawUnread !== undefined ? buffer._rawUnread : 0,
    h: buffer._rawHighlight !== undefined ? buffer._rawHighlight : 0,
  };
  buffer.unread_count = 0;
  buffer.highlight_count = 0;

  paintActiveBuffer(networkId, key);
  // Sofort rendern (mit nur 120 DOM-Zeilen dauert das wenige Millisekunden)
  render();
  const input = document.getElementById("chat-input");
  if (input && !input.disabled) input.focus({ preventScroll: true });
}

function paintActiveBuffer(networkId, key) {
  for (const item of document.querySelectorAll("#nav-section .channel-item")) {
    const on = item.dataset.net === networkId && (item.dataset.buf || "").toLowerCase() === key;
    item.classList.toggle("active", on);
    if (on) {
      item.classList.remove("is-attention", "act-unread", "act-highlight");
      item.classList.add("act-idle");
      item.querySelector(".badge")?.remove();
    }
  }
  for (const head of document.querySelectorAll("#nav-section .network-header")) {
    head.classList.toggle("is-current", head.dataset.net === networkId);
  }
  for (const tab of document.querySelectorAll("#switchbar .switchbar-tab")) {
    const on = tab.dataset.net === networkId && (tab.dataset.buf || "").toLowerCase() === key;
    tab.classList.toggle("active", on);
    if (on) {
      tab.classList.remove("is-attention");
      tab.querySelector(".switchbar-unread")?.remove();
      tab.setAttribute("aria-current", "page");
    } else {
      tab.removeAttribute("aria-current");
    }
  }
  const buf = state.networks[networkId]?.buffers[key];
  if (buf) {
    setText(document.getElementById("topic-title"), buf.name);
    setText(document.getElementById("status-buffer"), buf.name);
  }
}

// Im Speicher bleiben bis zu 2000 Zeilen; im DOM liegen initial nur die neuesten Zeilen.
const INITIAL_DOM_ROWS = 120;
const PREPEND_DOM_STEP = 120;
const MAX_DOM_ROWS_AT_BOTTOM = 250;
const MAX_DOM_ROWS_SCROLLED = 1000;

function rowSig(msg) {
  return `${msg.id}|${msg.timestamp}|${msg.sender}|${msg.text}`;
}

// Lädt ältere Zeilen aus activeBuf.messages nach oben nach, wenn der Nutzer hochscrollt.
function prependOlderMessagesIfNeeded() {
  const log = document.getElementById("messages-log");
  const prev = rendered.log && rendered.log !== "empty" ? rendered.log : null;
  if (!log || !prev || prev.startIndex <= 0 || log.scrollTop > 250) return;

  const net = state.networks[state.active_network];
  const activeBuf = net ? net.buffers[net.active_buffer] : null;
  if (!activeBuf || !activeBuf.messages) return;

  const nextStart = Math.max(0, prev.startIndex - PREPEND_DOM_STEP);
  const slice = activeBuf.messages.slice(nextStart, prev.startIndex);
  if (slice.length === 0) return;

  const oldHeight = log.scrollHeight;
  const oldTop = log.scrollTop;
  log.insertAdjacentHTML("afterbegin", renderMessageRows(slice));
  log.scrollTop = oldTop + (log.scrollHeight - oldHeight);
  prev.startIndex = nextStart;
}

// Returns true when the caller should scroll the log to the bottom (after all other writes).
function updateMessages(activeBuf, wasAtBottom = true) {
  const log = document.getElementById("messages-log");
  if (!log) return false;

  if (!activeBuf || !activeBuf.messages) {
    if (rendered.log !== "empty") {
      log.innerHTML = '<div class="empty-state">No buffer selected</div>';
      rendered.log = "empty";
    }
    return false;
  }

  const bufKey = `${state.active_network}/${activeBuf.name}`;
  const msgs = activeBuf.messages;
  const count = msgs.length;
  const first = count > 0 ? rowSig(msgs[0]) : "";
  const last = count > 0 ? rowSig(msgs[count - 1]) : "";
  const prev = rendered.log && rendered.log !== "empty" ? rendered.log : null;

  // Keine Abhängigkeiten mehr von activeBuf.users (kein Komplett-Redraw bei JOIN/PART/QUIT)
  if (prev && prev.key === bufKey && prev.count === count && prev.first === first && prev.last === last) {
    return false;
  }

  const isAtBottom = wasAtBottom;
  const switched = !prev || prev.key !== bufKey;

  let appendFrom = -1;
  if (!switched && prev.count > 0) {
    for (let i = count - 1; i >= 0; i--) {
      if (rowSig(msgs[i]) === prev.last) {
        appendFrom = i + 1;
        break;
      }
    }
  }

  let startIndex = prev ? prev.startIndex : 0;

  if (appendFrom >= 0) {
    if (appendFrom < count) {
      log.insertAdjacentHTML("beforeend", renderMessageRows(msgs.slice(appendFrom)));
    }
    // Wenn vorne aus msgs abgeschnitten wurde, startIndex nachführen
    const removedFront = Math.max(0, prev.count - appendFrom);
    if (removedFront > 0) {
      startIndex = Math.max(0, startIndex - removedFront);
    }
    // DOM oben begrenzen: bei Leseposition unten auf 250, bei Scrollback auf 1000
    if (isAtBottom) {
      while (log.childElementCount > MAX_DOM_ROWS_AT_BOTTOM) {
        log.firstElementChild.remove();
        startIndex++;
      }
    } else {
      const oldHeight = log.scrollHeight;
      while (log.childElementCount > MAX_DOM_ROWS_SCROLLED) {
        log.firstElementChild.remove();
        startIndex++;
      }
      log.scrollTop = Math.max(0, log.scrollTop - (oldHeight - log.scrollHeight));
    }
  } else {
    startIndex = Math.max(0, count - INITIAL_DOM_ROWS);
    log.innerHTML = renderMessageRows(msgs.slice(startIndex));
  }

  rendered.log = { key: bufKey, count, first, last, startIndex };
  return isAtBottom || switched;
}

// Message timestamps are Unix milliseconds. Tokens: HH hh nn ss, and ms for milliseconds (3 digits).
function formatClock(timestamp) {
  if (typeof appOptions !== "undefined" && appOptions.timestampEvents === false) return "";
  const d = new Date(timestamp);
  const pad = (n) => String(n).padStart(2, "0");
  const fmt = (typeof appOptions !== "undefined" && appOptions.timestampFormat) ? appOptions.timestampFormat : "[HH:nn:ss:ms]";
  // A bare color reset (Ctrl+K without a number) directly before a token would read the token's digits
  // as a color number and drop them. A reset-all code after the reset keeps the digits as text.
  const reset = String.fromCharCode(3);
  return fmt
    .replace(new RegExp(reset + "(?!\\d)", "g"), reset + String.fromCharCode(15))
    .replace(/HH/g, pad(d.getHours()))
    .replace(/hh/g, pad((d.getHours() % 12) || 12))
    .replace(/nn/g, pad(d.getMinutes()))
    .replace(/ss/g, pad(d.getSeconds()))
    .replace(/ms/g, String(d.getMilliseconds()).padStart(3, "0"));
}

// The backend does not tag event types, so JOIN and QUIT are recognised from the
// wording of system lines (sender "*"). Everything else keeps its own kind.
function classifyMessage(msg) {
  const text = msg.text || "";
  if (msg.sender === "*") {
    if (/\b(has joined|joins|joined)\b/i.test(text)) return "join";
    if (/\b(has quit|quits|quit)\b/i.test(text)) return "quit";
    if (/\bhas left\b/i.test(text)) return "part";
    if (/\btopic\b/i.test(text)) return "topic";
    if (/\b(cannot|not (on|registered)|too many|no such|error)\b/i.test(text)) return "error";
    return "info";
  }
  if (msg.is_notice) return "notice";
  if (msg.is_action) return "action";
  return "chat";
}

// Rows are only hidden at render time; the messages stay in state.
function renderMessageRows(list) {
  const ignored = new Set(readIgnoreList());
  // Many rows share a sender: the ignore check runs once per sender in this render, not once per row.
  const hiddenBySender = new Map();
  return list.map((msg) => {
    const sender = String(msg.sender);
    let hidden = hiddenBySender.get(sender);
    if (hidden === undefined) {
      hidden = (ignored.size && ignored.has(sender.toLowerCase())) || isUserIgnored(msg.sender, state.active_network || "", senderHostFor(msg.sender));
      hiddenBySender.set(sender, Boolean(hidden));
    }
    if (hidden) {
      const kind = classifyMessage(msg);
      if (kind === "chat" || kind === "action" || kind === "notice") return "";
    }
    return renderMessageRow(msg);
  }).join("");
}

// Row markup is a pure function of the message, the sender's colour and mode prefix. Switching
// back to a buffer, or re-rendering after an ignore change, reuses it instead of re-parsing
// every IRC colour code. Bounded: dropped wholesale when full.
const rowHtmlCache = new Map();
const ROW_CACHE_MAX = 4000;

function renderMessageRow(msg) {
  const nickColor = senderColor(msg.sender);
  const modePrefix = senderUser(msg.sender)?.prefix || "";
  const fmtKey = typeof appOptions !== "undefined" ? `${appOptions.timestampEvents ? 1 : 0}:${appOptions.timestampFormat}:${appOptions.prefixShowModes ? 1 : 0}` : "";
  const cacheKey = `${msg.id}|${msg.timestamp}|${fmtKey}|${msg.sender}|${msg.is_notice ? 1 : 0}${msg.is_action ? 1 : 0}${msg.is_highlight ? 1 : 0}|${msg.spans ? msg.spans.length : 0}|${nickColor}|${modePrefix}|${state.nickColors ? 1 : 0}|${state.networks[state.active_network]?.nick || ""}|${state.networks[state.active_network]?.active_buffer || ""}`;
  const hit = rowHtmlCache.get(cacheKey);
  if (hit !== undefined) return hit;
  const html = buildMessageRow(msg, nickColor, modePrefix);
  // Oldest entry out first (Map keeps insertion order); the rest of the cache stays warm.
  if (rowHtmlCache.size >= ROW_CACHE_MAX) rowHtmlCache.delete(rowHtmlCache.keys().next().value);
  rowHtmlCache.set(cacheKey, html);
  return html;
}

function buildMessageRow(msg, nickColor, modePrefix) {
  const time = msg.clock !== undefined ? msg.clock : formatClock(msg.timestamp);
  const kind = classifyMessage(msg);
  const isEvent = kind === "join" || kind === "quit";

  let contentHtml;
  if (isEvent && !(msg.spans && msg.spans.length > 0)) {
    contentHtml = parseIrcToHtml((msg.text || "").replace(/^\s*\*+\s*/, ""));
  } else {
    contentHtml = msg.spans && msg.spans.length > 0
      ? renderSpansToHtml(msg.spans)
      : parseIrcToHtml(msg.text);
  }

  const net = state.networks[state.active_network];
  const ownNick = net ? (net.current_nick || net.nick) : "";
  const buf = net ? net.buffers[net.active_buffer] : null;
  const hlInfo = checkAddressBookHighlight(msg.text || "", msg.sender || "", buf?.name || "", ownNick);

  let extraClass = ` kind-${kind}`;
  if (msg.is_highlight || hlInfo.matched) extraClass += " highlight";
  if (msg.is_notice) extraClass += " msg-notice";
  if (msg.is_action) extraClass += " msg-action";

  let lead = "";
  if (kind === "join") lead = `<span class="msg-tag" style="color: var(--col-join)">(JOIN)</span> `;
  else if (kind === "quit") lead = `<span class="msg-tag" style="color: var(--col-quit)">(QUIT)</span> `;
  else if (kind === "notice") lead = `<span class="msg-tag" style="color: var(--col-notice)">(NOTICE)</span> from <span class="msg-tag-who" style="color: var(--col-notice)">(${escapeHtml(msg.sender)})</span> `;
  else if (kind === "action") lead = `<span class="msg-sender" data-nick="${escapeHtml(msg.sender)}" style="color: var(--col-action)">* ${escapeHtml(msg.sender)}</span> `;
  else if (kind === "chat") {
    const showModes = typeof appOptions === "undefined" || appOptions.prefixShowModes !== false;
    const prefix = showModes ? modePrefix : "";
    const isOwn = state.networks[state.active_network]?.nick?.toLowerCase() === (msg.sender || "").toLowerCase();
    const finalNickColor = isOwn ? "var(--col-own)" : (state.nickColors ? nickColor : "var(--col-normal)");
    lead = `<span class="msg-sender" data-nick="${escapeHtml(msg.sender)}" style="color: ${escapeHtml(finalNickColor)}">(${escapeHtml(prefix)}${escapeHtml(msg.sender)})</span> `;
  }

  const timeSpan = time ? `<span class="msg-time">${parseIrcToHtml(time)}</span> ` : "";
  // Part and topic lines take their saved colour from Options > Colors.
  const contentColour = kind === "part" ? "var(--col-part)" : kind === "topic" ? "var(--col-topic)" : "";
  const contentAttr = contentColour ? ` style="color: ${contentColour}"` : "";
  return `<div class="msg-row${extraClass}">${timeSpan}${lead}<span class="msg-content"${contentAttr}>${contentHtml}</span></div>`;
}

// Nick list selection: lowercased nicks of one buffer. `key` ties it to that buffer, so a
// buffer change clears it. The DOM only carries the is-selected class (applyNickSelection),
// so a click never rebuilds the rows and a double-click still lands on the same element.
const nickSelection = { key: "", anchor: "", nicks: new Set() };

function nickItemEls() {
  return Array.from(document.querySelectorAll("#nicklist-users .nick-item"));
}

function applyNickSelection() {
  for (const el of nickItemEls()) {
    el.classList.toggle("is-selected", nickSelection.nicks.has((el.dataset.nick || "").toLowerCase()));
  }
}

function selectOnlyNick(nick) {
  nickSelection.nicks = new Set([nick]);
  nickSelection.anchor = nick;
  applyNickSelection();
}

// Plain click: only this nick. Ctrl-click: toggle. Shift-click: range from the last clicked
// nick in list order. Empty space: clear.
function selectNickFromClick(e) {
  const el = e.target.closest(".nick-item");
  if (!el || !el.dataset.nick) {
    nickSelection.nicks = new Set();
    nickSelection.anchor = "";
    applyNickSelection();
    return;
  }
  const nick = el.dataset.nick.toLowerCase();
  if (e.ctrlKey || e.metaKey) {
    const next = new Set(nickSelection.nicks);
    if (!next.delete(nick)) next.add(nick);
    nickSelection.nicks = next;
    nickSelection.anchor = nick;
    applyNickSelection();
    return;
  }
  if (e.shiftKey && nickSelection.anchor) {
    const order = nickItemEls().map((item) => (item.dataset.nick || "").toLowerCase());
    const from = order.indexOf(nickSelection.anchor);
    const to = order.indexOf(nick);
    if (from >= 0 && to >= 0) {
      nickSelection.nicks = new Set(order.slice(Math.min(from, to), Math.max(from, to) + 1));
      nickSelection.anchor = nick;
      applyNickSelection();
      return;
    }
  }
  selectOnlyNick(nick);
}

function updateNicklist(activeBuf) {
  const nickUsersEl = document.getElementById("nicklist-users");
  const nickCountEl = document.getElementById("nicklist-count");
  if (!nickUsersEl || !nickCountEl) return;

  const selectionKey = `${state.active_network || ""}/${(activeBuf?.name || "").toLowerCase()}`;
  if (nickSelection.key !== selectionKey) {
    nickSelection.key = selectionKey;
    nickSelection.nicks = new Set();
    nickSelection.anchor = "";
  }

  if (!activeBuf || !activeBuf.users || Object.keys(activeBuf.users).length === 0) {
    nickSelection.nicks = new Set();
    setText(nickCountEl, "0");
    rendered.nickSrc = null;
    swapHtml(nickUsersEl, "nicklist", '<div class="nicklist-empty">No users</div>');
    return;
  }

  // The user table is replaced as a whole on every backend change, never edited in place.
  if (rendered.nickSrc === activeBuf.users && rendered.nickNet === state.active_network && rendered.nicklist !== null) {
    applyNickSelection();
    return;
  }
  rendered.nickSrc = activeBuf.users;
  rendered.nickNet = state.active_network;

  const usersList = Object.values(activeBuf.users).sort(compareUsers);
  setText(nickCountEl, String(usersList.length));

  // Nicks that left the channel drop out of the selection.
  if (nickSelection.nicks.size) {
    const present = new Set(usersList.map((user) => user.nick.toLowerCase()));
    nickSelection.nicks = new Set([...nickSelection.nicks].filter((nick) => present.has(nick)));
  }

  const html = usersList.map((user) => {
    const color = nickColorFor(user);
    return `
      <div class="nick-item" style="color: ${escapeHtml(color)}" data-net="${escapeHtml(state.active_network || "")}" data-nick="${escapeHtml(user.nick)}" title="Double-click to query ${escapeHtml(user.nick)}">
        <span class="nick-prefix">${escapeHtml(user.prefix || "")}</span><span class="nick-name">${escapeHtml(user.nick)}</span>
      </div>
    `;
  }).join("");

  swapHtml(nickUsersEl, "nicklist", html);
  applyNickSelection();
}

// Nick color rules after the Address Book "Colors" tab, first match wins. The
// colors are approximate values.
// The own nick follows its mode like any other nick. Users without a mode use the text
// color so they stay readable in both themes.
const NICK_COLOR_RULES = [
  { test: (user) => user.prefix === "~", color: "#ff00ff" },
  { test: (user) => user.prefix === "&", color: "#ff4040" },
  { test: (user) => user.prefix === "@", color: "#ff2020" },
  { test: (user) => user.prefix === "%", color: "#20a8a8" },
  { test: (user) => user.prefix === "+", color: "#33dd33" },
  { test: () => true, color: "var(--nicklist-text)" },
];

function nickColorFor(user) {
  if (!state.nickColors) return "var(--nicklist-text)";
  const net = state.networks[state.active_network];
  const ownNick = net ? (net.current_nick || net.nick) : "";
  const abColor = matchNickColor(user, ownNick);
  if (abColor) return abColor;
  return NICK_COLOR_RULES.find((rule) => rule.test(user))?.color || "var(--nicklist-text)";
}

// Chat names use the same rules as the nick list. Senders who are not in the active
// channel have no mode, so they get the text color.
function senderUser(sender) {
  const net = state.networks[state.active_network];
  const buf = net ? net.buffers[net.active_buffer] : null;
  return buf?.users?.[(sender || "").toLowerCase()] || null;
}

function senderColor(sender) {
  const user = senderUser(sender);
  if (!state.nickColors) return "var(--text)";
  const net = state.networks[state.active_network];
  const ownNick = net ? (net.current_nick || net.nick) : "";
  if (user) {
    const abColor = matchNickColor(user, ownNick);
    if (abColor) return abColor;
    if (user.prefix) return nickColorFor(user);
  } else {
    const abColor = matchNickColor({ nick: sender, prefix: "" }, ownNick);
    if (abColor) return abColor;
  }
  return "var(--text)";
}

function compareUsers(a, b) {
  const ranks = { "~": 5, "&": 4, "@": 3, "%": 2, "+": 1 };
  const rA = ranks[a.prefix] || 0;
  const rB = ranks[b.prefix] || 0;
  if (rA !== rB) return rB - rA;
  return a.nick.localeCompare(b.nick);
}

// Query management
async function openQuery(network, target) {
  if (!network || !target) return;
  const net = state.networks[network];
  if (!net) return;

  const key = target.toLowerCase();
  const pendingMark = `${network}/${key}`;
  if (!net.buffers[key]) {
    pendingBuffers.add(pendingMark);
    net.buffers[key] = {
      name: target,
      is_channel: false,
      topic: `Private conversation with ${target}`,
      users: {},
      messages: [],
      unread_count: 0,
      highlight_count: 0,
    };
  }
  net.active_buffer = key;
  state.attention.delete(pendingMark);
  render();

  if (isTauri) {
    try {
      await invoke("open_query", { network, target, autoKeyx: appOptions.queryAutoKeyx !== false });
    } catch (e) {
      console.error("open_query error:", e);
    }
  }
  pendingBuffers.delete(pendingMark);

  const input = document.getElementById("chat-input");
  if (input) input.focus();
}

async function closeBuffer(network, bufferName) {
  if (!network || !bufferName) return;
  const net = state.networks[network];
  if (!net) return;

  const key = bufferName.toLowerCase();
  if (key === "status") return;

  delete net.buffers[key];
  state.attention.delete(`${network}/${key}`);
  if (net.active_buffer === key) {
    net.active_buffer = "status";
  }
  render();

  if (isTauri) {
    try {
      await invoke("close_buffer", { network, buffer: bufferName });
    } catch (e) {
      console.error("close_buffer error:", e);
    }
  }
}

// Close on a network: QUIT, drop the backend client and state, then remove it from the tree.
async function closeNetwork(netId) {
  if (!netId || !state.networks[netId]) return;
  if (!window.confirm(`Close network ${netId}? The connection will be ended.`)) return;
  if (!(await dropNetwork(netId))) return;
  await syncFromBackend();
}

// Sends QUIT, drops the backend client and removes the network from the UI state.
// Returns false when the backend could not close it, so the window stays.
async function dropNetwork(netId) {
  if (isTauri) {
    try {
      await invoke("send_raw", { network: netId, raw: "QUIT :Closed" });
    } catch (e) {
      console.warn("QUIT not sent (already disconnected?):", e);
    }
    try {
      await invoke("close_network", { network: netId });
    } catch (e) {
      console.error("close_network error:", e);
      return false;
    }
  }

  delete state.networks[netId];
  delete state.collapsedNets[netId];
  const prefix = `${netId}/`;
  for (const key of Object.keys(readBaseline)) if (key.startsWith(prefix)) delete readBaseline[key];
  for (const key of [...pendingBuffers]) if (key.startsWith(prefix)) pendingBuffers.delete(key);
  for (const key of [...state.attention]) if (key.startsWith(prefix)) state.attention.delete(key);
  if (state.active_network === netId) {
    state.active_network = Object.keys(state.networks)[0] || null;
  }
  invalidateLog();
  render();
  return true;
}

// /server: syntax in serverCommand.js. The current server is disconnected first (QUIT), then the
// new one is connected. A host that is not in the server list is added to it. /server without
// arguments reconnects to the last server used.
let lastServerTarget = null;

async function runServerCommand(args) {
  if (!isTauri) return;
  const parsed = parseServerArgs(args);
  if (!parsed.ok) {
    notifyNoNetwork(parsed.error);
    return;
  }

  const current = state.networks[state.active_network] || null;
  const defaultNick = current?.current_nick || current?.nick || Object.values(state.networks)[0]?.nick || "ferric";
  let target = null;

  if (parsed.action === "last") {
    if (!lastServerTarget) {
      notifyNoNetwork("No server to connect to yet. Use /server <host> [port] [password].");
      return;
    }
    target = { ...lastServerTarget };
  } else if (parsed.action === "list") {
    let rows = [];
    try {
      rows = (await invoke("get_server_list")).rows;
    } catch (e) {
      notifyNoNetwork(`Server list could not be read: ${String(e?.message || e)}`);
      return;
    }
    const row = rows[parsed.index - 1];
    if (!row) {
      notifyNoNetwork(`The server list has no entry ${parsed.index}.`);
      return;
    }
    target = {
      host: row.host, port: row.port, tls: row.use_tls, password: row.password || null,
      nick: row.nick, altnick: row.altnick, username: row.username, realname: row.realname,
    };
  } else {
    target = buildServerTarget(parsed, defaultNick);
  }

  const proxy = proxyConnectArgs();
  if (proxy.error) {
    notifyNoNetwork(proxy.error);
    return;
  }

  // The current server goes first. Its window is replaced by the new one.
  if (current) {
    await dropNetwork(current.id);
  }
  // A network with the same name is the same server; it is replaced too.
  if (state.networks[target.host]) {
    await dropNetwork(target.host);
  }

  try {
    await invoke("connect_network", {
      ...proxy.args,
      config: {
        network: target.host,
        host: target.host,
        port: target.port,
        use_tls: target.tls,
        nick: target.nick,
        altnick: target.altnick,
        username: target.username,
        realname: target.realname,
        password: target.password,
        tls_fingerprint: null,
      },
    });
  } catch (e) {
    notifyNoNetwork(`Connection failed: ${String(e?.message || e)}`);
    await syncFromBackend();
    return;
  }

  lastServerTarget = target;
  if (parsed.action === "connect") {
    // The host is not in the list yet: add it, so /server N and the server manager know it.
    try {
      const view = await invoke("get_server_list");
      if (!view.rows.some((row) => row.host === target.host && row.port === target.port)) {
        await invoke("save_server_list", {
          rows: [...view.rows, {
            host: target.host, port: target.port, use_tls: target.tls, password: target.password,
            nick: target.nick, altnick: target.altnick, username: target.username, realname: target.realname,
          }],
          expectedMtime: view.mtime,
        });
      }
    } catch (e) {
      console.warn("server list not updated:", e);
    }
  }
  await syncFromBackend();
}

// A /server message goes to the active window when there is one. Without a network there is no
// window to write to, so the message is shown as an alert.
function notifyNoNetwork(text) {
  const net = state.networks[state.active_network];
  if (net) pushStatusNotice(net.id, text, net.active_buffer);
  else window.alert(text);
}

// User Command & Chat Handler
// The chat input grows with its lines, up to about six.
function resizeChatInput(el) {
  el.style.height = "auto";
  el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
}

// Ctrl+K in the chat input opens the color index. A click inserts the
// color code at the caret: the control character and the two-digit color number.
const CHAT_COLOR_ROWS = [
  [0, 1, 2, 3, 4, 5, 6, 7],
  [8, 9, 10, 11, 12, 13, 14, 15],
  [16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
  [28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39],
  [40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51],
  [52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63],
  [64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75],
  [76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87],
  [88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98],
];

function closeChatColorPopup() {
  document.getElementById("chat-color-popup")?.remove();
  document.removeEventListener("pointerdown", chatColorPopupOutside, true);
  document.removeEventListener("keydown", chatColorPopupEscape, true);
}

function chatColorPopupOutside(e) {
  if (!(e.target instanceof Element && e.target.closest("#chat-color-popup"))) closeChatColorPopup();
}

function chatColorPopupEscape(e) {
  if (e.key === "Escape") closeChatColorPopup();
}

function chatColorTextFor(hex) {
  const n = parseInt(hex.slice(1), 16);
  const lum = 0.299 * ((n >> 16) & 255) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255);
  return lum > 140 ? "#000000" : "#ffffff";
}

function openChatColorPopup(input) {
  closeChatColorPopup();
  // Ctrl+K inserts the color character at once. A color picked in the
  // index goes in right after it; closing the index without a pick leaves the bare character, so the
  // color number can be typed by hand.
  const at = input.selectionStart;
  input.setRangeText(String.fromCharCode(3), at, input.selectionEnd, "end");
  input.dispatchEvent(new Event("input", { bubbles: true }));
  const pop = document.createElement("div");
  pop.id = "chat-color-popup";
  pop.className = "chat-color-popup";
  for (const row of CHAT_COLOR_ROWS) {
    const rowEl = document.createElement("div");
    rowEl.className = "chat-color-row";
    for (const index of row) {
      const cell = document.createElement("button");
      cell.type = "button";
      cell.className = "chat-color-cell";
      cell.textContent = String(index);
      cell.title = "Color " + index;
      cell.style.background = IRC_COLORS[index];
      cell.style.color = chatColorTextFor(IRC_COLORS[index]);
      cell.addEventListener("click", () => {
        input.setRangeText(String(index).padStart(2, "0"), at + 1, at + 1, "end");
        closeChatColorPopup();
        input.focus();
        input.dispatchEvent(new Event("input", { bubbles: true }));
        input.focus();
      });
      rowEl.appendChild(cell);
    }
    pop.appendChild(rowEl);
  }
  document.body.appendChild(pop);
  const r = input.getBoundingClientRect();
  pop.style.left = Math.max(4, r.left) + "px";
  pop.style.bottom = Math.max(4, window.innerHeight - r.top + 6) + "px";
  document.addEventListener("pointerdown", chatColorPopupOutside, true);
  document.addEventListener("keydown", chatColorPopupEscape, true);
}

async function handleUserInput(text) {
  // /server works without a network too: it is how the first connection is made.
  if (/^\/server(\s|$)/i.test(text)) {
    await runServerCommand(text.trim().split(/\s+/).slice(1));
    return;
  }
  const net = state.networks[state.active_network];
  if (!net) return;

  if (text.startsWith("/")) {
    const parts = text.slice(1).split(" ");
    const cmd = parts[0].toUpperCase();
    const args = parts.slice(1);

    switch (cmd) {
      case "PERF": {
        const domCount = document.querySelectorAll("*").length;
        const msgElCount = document.getElementById("messages-log")?.childElementCount || 0;
        const mem = performance.memory;
        const heapMb = mem ? (mem.usedJSHeapSize / (1024 * 1024)).toFixed(1) : "n/a";
        let backendPerf = null;
        if (isTauri) {
          try {
            backendPerf = await invoke("get_backend_perf");
          } catch (e) {
            backendPerf = null;
          }
        }
        pushStatusNotice(net.id, "=== Ferric Performance Telemetry ===", net.active_buffer);
        if (backendPerf) {
          const gsSnapAvg = backendPerf.get_state_snapshot_avg_ms.toFixed(2);
          const gsAvg = backendPerf.get_state_avg_ms.toFixed(2);
          const gsLast = backendPerf.get_state_last_ms.toFixed(2);
          const gsMax = backendPerf.get_state_max_ms.toFixed(2);
          const gsLock = backendPerf.get_state_last_lock_wait_ms.toFixed(2);
          const gsKb = (backendPerf.get_state_last_payload_bytes / 1024).toFixed(1);
          const gsWarn = (backendPerf.get_state_last_ms > 5.0 || backendPerf.get_state_last_payload_bytes > 200 * 1024) ? " [WARN: high latency/payload]" : "";
          pushStatusNotice(net.id, `1. [Backend get_state] calls: ${backendPerf.get_state_calls} (snaps: ${backendPerf.get_state_snapshot_calls}), snap_avg: ${gsSnapAvg}ms (all_avg: ${gsAvg}ms), last: ${gsLast}ms, max: ${gsMax}ms, lock_wait: ${gsLock}ms, last_payload: ${gsKb} KB${gsWarn}`, net.active_buffer);

          const pyAvg = backendPerf.python_rpc_avg_ms.toFixed(2);
          const pyLast = backendPerf.python_rpc_last_ms.toFixed(2);
          const pyMax = backendPerf.python_rpc_max_ms.toFixed(2);
          const pyWarn = backendPerf.python_rpc_last_ms > 20.0 ? " [WARN: slow python RPC]" : "";
          pushStatusNotice(net.id, `5. [Backend Python RPC] calls: ${backendPerf.python_rpc_calls}, last: ${pyLast}ms, avg: ${pyAvg}ms, max: ${pyMax}ms${pyWarn}`, net.active_buffer);
        }
        const syncsPerMin = getFrontendSyncsPerMin();
        const syncsPerSec = (syncsPerMin / 60).toFixed(1);
        const snapIpc = frontendPerf.lastSnapshotIpcMs ? `${frontendPerf.lastSnapshotIpcMs.toFixed(2)}ms` : "n/a";
        const snapJs = frontendPerf.lastSnapshotJsMs ? `${frontendPerf.lastSnapshotJsMs.toFixed(2)}ms` : "n/a";
        const syncWarn = ((frontendPerf.lastSnapshotJsMs && frontendPerf.lastSnapshotIpcMs && frontendPerf.lastSnapshotJsMs + frontendPerf.lastSnapshotIpcMs > 16.0) || syncsPerMin > 180) ? " [WARN: frame drop / >3 syncs/sec]" : "";
        pushStatusNotice(net.id, `2. [Frontend Sync] syncs: ${frontendPerf.syncCount} (${syncsPerMin}/min, ~${syncsPerSec}/s), snapshot IPC: ${snapIpc}, JS: ${snapJs} (idle tick: ${frontendPerf.lastIpcMs.toFixed(2)}ms / ${frontendPerf.lastJsMs.toFixed(2)}ms)${syncWarn}`, net.active_buffer);

        const renderWarn = frontendPerf.lastRenderTotalMs > 16.0 ? " [WARN: render >16ms]" : "";
        pushStatusNotice(net.id, `3. [Frontend Render] total: ${frontendPerf.lastRenderTotalMs.toFixed(2)}ms (sidebar: ${frontendPerf.lastSidebarMs.toFixed(2)}ms, msgs: ${frontendPerf.lastMessagesMs.toFixed(2)}ms, nicks: ${frontendPerf.lastNicklistMs.toFixed(2)}ms)${renderWarn}`, net.active_buffer);

        const domWarn = domCount > 1500 ? " [WARN: DOM nodes > 1500]" : "";
        pushStatusNotice(net.id, `4. [Frontend DOM & Heap] DOM nodes: ${domCount}, log rows: ${msgElCount}, JS heap: ${heapMb} MB${domWarn}`, net.active_buffer);
        render();
        break;
      }
      case "COLOR":
      case "COLORS": {
        openColorsModal();
        break;
      }
      case "ABOOK":
      case "ADDRESS": {
        openAddressBookModal("users");
        break;
      }
      case "CTCP": {
        // /ctcp <nick> <command> [text]: a CTCP request. The address book buttons send this form.
        const ctcpTarget = args[0] || "";
        const ctcpCommand = (args[1] || "").toUpperCase();
        if (!validIrcToken(ctcpTarget) || !ctcpCommand) break;
        const ctcpText = args.slice(2).join(" ").replace(/\x01/g, "");
        if (isTauri) {
          await invoke("send_raw", {
            network: state.active_network,
            raw: `PRIVMSG ${ctcpTarget} :\x01${ctcpCommand}${ctcpText ? ` ${ctcpText}` : ""}\x01`,
          });
        }
        break;
      }
      case "WHOIS": {
        const targetNick = args[0] ? args[0].trim() : "";
        if (targetNick) {
          if (isTauri && net) {
            void invoke("send_raw", { network: state.active_network, raw: `WHOIS ${targetNick}` });
          }
          openAddressBookModal("whois", targetNick);
          break;
        }
        openAddressBookModal("whois");
        break;
      }
      case "NOTIFY": {
        openAddressBookModal("notify");
        break;
      }
      case "IGNORE": {
        openAddressBookModal("control");
        break;
      }
      case "HIGHLIGHT": {
        openAddressBookModal("highlight");
        break;
      }
      case "CNICK": {
        const setting = args.filter(Boolean).join(" ").toLowerCase();
        if (setting === "on" || setting === "off") {
          state.nickColors = setting === "on";
          saveViewFlag("nickColors");
          rendered.nickSrc = null;
          invalidateLog();
          render();
        } else {
          openAddressBookModal("colors");
        }
        break;
      }
      case "SCRIPTLOAD": {
        if (!isTauri) break;
        const name = args.join(" ").trim();
        if (!name) {
          pushStatusNotice(net.id, "Usage: /scriptload <filename.mrc|.py>", net.active_buffer);
          break;
        }
        try {
          const notices = await invoke("load_script_aliases", { name });
          pushStatusNotice(net.id, notices.length ? "Script load reported a problem:" : "Script loaded.", net.active_buffer);
          for (const notice of notices) pushStatusNotice(net.id, notice, net.active_buffer);
        } catch (e) {
          pushStatusNotice(net.id, `Script load failed: ${String(e?.message || e)}`, net.active_buffer);
        }
        break;
      }
      case "SCRIPTUNLOAD": {
        if (!isTauri) break;
        const name = args.join(" ").trim();
        if (!name) {
          pushStatusNotice(net.id, "Usage: /scriptunload <filename.mrc|.py>", net.active_buffer);
          break;
        }
        const unloaded = await invoke("unload_script_aliases", { name });
        pushStatusNotice(net.id, unloaded ? "Script unloaded." : "Script was not loaded; use the same filename as /scriptload.", net.active_buffer);
        break;
      }
      case "CHANNEL":
      case "CC": {
        const chan = args[0] && /^[#&+!]/.test(args[0]) ? args[0] : null;
        if (chan) {
          await openChannelCentralFor(net.id, chan);
        } else {
          await openChannelCentralForActive();
        }
        break;
      }
      case "JOIN":
      case "J": {
        // A name with a channel prefix (#, &, +, !) is kept as it is; a bare name gets #. The key is the second word.
        const chan = args[0] ? (/^[#&+!]/.test(args[0]) ? args[0] : "#" + args[0]) : null;
        if (chan && isTauri) {
          const key = args[1] ? ` ${args[1]}` : "";
          await invoke("send_raw", { network: net.id, raw: `JOIN ${chan}${key}` });
          await syncFromBackend();
        }
        break;
      }
      case "QUERY":
      case "Q": {
        const targetNick = args[0];
        if (targetNick) {
          await openQuery(net.id, targetNick);
          if (args.length > 1) {
            const initialMsg = args.slice(1).join(" ");
            await invoke("send_message", { network: net.id, target: targetNick, text: initialMsg });
            await syncFromBackend();
          }
        }
        break;
      }
      case "CLOSE": {
        const target = args[0] || net.active_buffer;
        await closeBuffer(net.id, target);
        break;
      }
      case "PART":
      case "P": {
        // /part [#chan] [reason]: a first word with a channel prefix is the channel, otherwise the active window.
        const named = args[0] && /^[#&+!]/.test(args[0]);
        const chan = named ? args[0] : net.active_buffer;
        const reason = (named ? args.slice(1) : args).join(" ");
        if (chan && isTauri) {
          await invoke("send_raw", { network: net.id, raw: reason ? `PART ${chan} :${reason}` : `PART ${chan}` });
          await closeBuffer(net.id, chan);
          await syncFromBackend();
        }
        break;
      }
      case "CONNECTD":
      case "LOAD": {
        if (isTauri) {
          try {
            await loadConnectdWithProxy();
            await syncFromBackend();
          } catch (e) {
            pushStatusNotice(net.id, String(e?.message || e));
          }
        }
        break;
      }
      case "MONITOR": {
        // /monitor + nick,nick | - nick,nick | l | s | c: the arguments go to the server as given.
        const monitorArgs = args.join(" ").trim();
        if (isTauri && net && monitorArgs) await invoke("send_raw", { network: net.id, raw: `MONITOR ${monitorArgs}` });
        break;
      }
      case "RAW": {
        const rawLine = args.join(" ");
        if (isTauri) await invoke("send_raw", { network: net.id, raw: rawLine });
        break;
      }
      case "CLEAR": {
        if (net) await clearBufferLines(net.id, net.active_buffer);
        break;
      }
      case "FISH":
      case "KEY":
      case "KEYS":
      case "BLOWFISH": {
        if (args.length === 0) {
          openFishModal();
          break;
        }
        let target = net.active_buffer;
        let key = args[0];
        if (args.length >= 2) {
          target = args[0];
          key = args[1];
        }
        if (target && key && isTauri) {
          await invoke("save_fish_key_entry", { network: net.id, target, key: formatFishCommandKey(key) });
          await checkActiveBufferFish(true);
          const buf = net.buffers[net.active_buffer];
          if (buf) {
            buf.messages.push({
              id: nextLocalId(), local: true,
              sender: "*",
              text: `FiSH key set & saved to blow.ini for ${target}`,
              spans: [],
              is_notice: true,
              is_action: false,
              is_encrypted: false,
              is_highlight: false,
              timestamp: Date.now(),
            });
            render();
          }
        }
        break;
      }
      case "DELKEY": {
        const target = args[0] || net.active_buffer;
        if (target && isTauri) {
          await invoke("delete_fish_key_entry", { section: fishSection(net.id, target) });
          await checkActiveBufferFish(true);
          const buf = net.buffers[net.active_buffer];
          if (buf) {
            buf.messages.push({
              id: nextLocalId(), local: true,
              sender: "*",
              text: `FiSH key deleted from blow.ini for ${target}`,
              spans: [],
              is_notice: true,
              is_action: false,
              is_encrypted: false,
              is_highlight: false,
              timestamp: Date.now(),
            });
            render();
          }
        }
        break;
      }
      case "KEYX":
      case "DH1080": {
        let nick = args[0] ? args[0].trim() : "";
        if (!nick) {
          const activeBuf = net.buffers[net.active_buffer];
          if (activeBuf && !activeBuf.is_channel && activeBuf.name.toLowerCase() !== "status") {
            nick = activeBuf.name;
          }
        }
        if (nick && isTauri && validIrcToken(nick)) {
          try {
            await invoke("start_dh1080", { network: net.id, nick, cbc: true });
          } catch (e) {
            pushStatusNotice(net.id, `*** FiSH: DH1080 key exchange with ${nick} failed: ${String(e?.message || e)}`, nick.toLowerCase());
          }
        } else {
          pushStatusNotice(net.id, "Usage: /keyx <nick> (or type /keyx inside a private query buffer)", net.active_buffer);
        }
        break;
      }
      case "SAY": {
        // /say <text>: plain text to the active channel or query, as if typed without the slash.
        const sayText = args.join(" ").replace(/[\r\n\x00]/g, " ");
        const sayBuf = net.buffers[net.active_buffer];
        if (!sayBuf || sayBuf.name.toLowerCase() === "status") {
          pushStatusNotice(net.id, "Not on a channel or query. Use /msg <nick> <text>.");
        } else if (sayText && isTauri) {
          try {
            await invoke("send_message", { network: net.id, target: sayBuf.name, text: sayText });
            await syncFromBackend();
          } catch {
            pushStatusNotice(net.id, "Message not sent. Check the connection and FiSH key.");
          }
        }
        break;
      }
      case "DESCRIBE": {
        // /describe <target> <text>: an action sent to a channel or nick, without changing the active window.
        const describeTarget = args[0] || "";
        const describeText = args.slice(1).join(" ").replace(/[\x01\r\n\x00]/g, " ");
        if (!validIrcToken(describeTarget) || !describeText) {
          pushStatusNotice(net.id, "Usage: /describe <target> <text>");
        } else if (isTauri) {
          try {
            await invoke("send_message", { network: net.id, target: describeTarget, text: `\x01ACTION ${describeText}\x01` });
            await syncFromBackend();
          } catch {
            pushStatusNotice(net.id, "Action not sent. Check the connection and FiSH key.");
          }
        }
        break;
      }
      case "ME": {
        // /me <text>: an action in the active channel or query. The FiSH key applies, as for a message.
        const actionText = args.join(" ").replace(/\x01/g, "");
        const actionBuf = net.buffers[net.active_buffer];
        if (!actionBuf || actionBuf.name.toLowerCase() === "status") {
          pushStatusNotice(net.id, "Not on a channel or query. Use /msg <nick> <text>.");
        } else if (actionText && isTauri) {
          try {
            await invoke("send_message", { network: net.id, target: actionBuf.name, text: `\x01ACTION ${actionText}\x01` });
            await syncFromBackend();
          } catch {
            pushStatusNotice(net.id, "Action not sent. Check the connection and FiSH key.");
          }
        }
        break;
      }
      case "MSG": {
        // /msg <nick> <text>: a private message. The query window opens as for any sent message.
        const msgTarget = args[0] || "";
        const msgText = args.slice(1).join(" ");
        if (!validIrcToken(msgTarget) || !msgText) {
          pushStatusNotice(net.id, "Use /msg <nick> <text>.");
        } else if (isTauri) {
          try {
            await invoke("send_message", { network: net.id, target: msgTarget, text: msgText });
            await syncFromBackend();
          } catch {
            pushStatusNotice(net.id, "Message not sent. Check the connection and FiSH key.");
          }
        }
        break;
      }
      case "NOTICE": {
        // /notice <target> <text>: a NOTICE line. It is not encrypted; the local line is shown in the target's window.
        const noticeTarget = args[0] || "";
        const noticeText = args.slice(1).join(" ").replace(/[\r\n\0]/g, "");
        if (!validIrcToken(noticeTarget) || !noticeText) {
          pushStatusNotice(net.id, "Use /notice <target> <text>.");
        } else if (isTauri) {
          await invoke("send_raw", { network: net.id, raw: `NOTICE ${noticeTarget} :${noticeText}` });
          echoSentNotice(net.id, noticeTarget, noticeText);
        }
        break;
      }
      default:
        if (isTauri) {
          const result = await invoke("run_script_alias", { name: parts[0], args: args.filter(Boolean), network: net.id, buffer: net.active_buffer });
          if (result) {
            await syncFromBackend();
          } else if (IRC_PASSTHROUGH_COMMANDS.has(cmd)) {
            await invoke("send_raw", { network: net.id, raw: text.slice(1) });
          } else {
            // Not sent: a typo would otherwise reach the server. /raw sends any line on purpose.
            pushStatusNotice(net.id, `Unknown command: /${parts[0]}. Use /raw to send a line to the server.`, net.active_buffer);
          }
        }
        break;
    }
  } else {
    // Normal Message (Channel or Query)
    const buf = net.buffers[net.active_buffer];
    if (buf && buf.name.toLowerCase() === "status") {
      // Plain text in the Status buffer is never sent to the server.
      pushStatusNotice(net.id, "Not on a channel or query. Use /msg <nick> <text>.");
    } else if (buf && isTauri) {
      try {
        await invoke("send_message", {
          network: net.id,
          target: buf.name,
          text,
        });
        await syncFromBackend();
      } catch {
        buf.messages.push({
          id: nextLocalId(), local: true,
          sender: "*",
          text: "Message not sent. Check the connection and FiSH key.",
          spans: [],
          is_notice: true,
          is_action: false,
          is_encrypted: false,
          is_highlight: false,
          timestamp: Date.now(),
        });
        invalidateLog();
        render();
      }
    }
  }
}

// Active Buffer FiSH Status Check
// Writes the badge only when its visible state changes, so frequent renders
// never rebuild it. The key lookup runs once per buffer (or when forced).
function setFishBadge(btn, on, title, mode = null) {
  const label = on ? mode.toUpperCase() : "Off";
  const sig = `${on ? 1 : 0}|${label}|${title}`;
  if (rendered.fishBadge === sig) return;
  rendered.fishBadge = sig;
  btn.className = `fish-status-badge ${on ? "badge-on" : "badge-off"}`;
  btn.innerHTML = `<span class="fish-icon">FiSH</span> <span class="fish-label">${label}</span>`;
  btn.title = title;
}

let fishPending = 0;

async function checkActiveBufferFish(force = false) {
  const btn = document.getElementById("btn-topic-fish");
  if (!btn) return;

  const net = state.networks[state.active_network];
  const activeBuf = net ? net.buffers[net.active_buffer] : null;

  if (!activeBuf || activeBuf.name.toLowerCase() === "status") {
    if (btn.style.display !== "none") btn.style.display = "none";
    state.currentBufferKey = null;
    state.fishStatusTarget = null;
    state.fishStatusRequest += 1;
    fishPending = 0;
    return;
  }

  if (btn.style.display !== "inline-flex") btn.style.display = "inline-flex";
  const targetKey = `${net.id}/${activeBuf.name.toLowerCase()}`;

  if (force || state.fishStatusTarget !== targetKey) {
    state.fishStatusTarget = targetKey;
    state.currentBufferKey = null;
    const requestId = ++state.fishStatusRequest;
    setFishBadge(btn, false, `Checking FiSH key for ${activeBuf.name}\u2026`);

    if (isTauri && net) {
      fishPending = requestId;
      try {
        const key = await invoke("get_buffer_fish_key", { network: net.id, target: activeBuf.name });
        const currentNet = state.networks[state.active_network];
        const currentBuffer = currentNet ? currentNet.buffers[currentNet.active_buffer] : null;
        if (requestId !== state.fishStatusRequest) return;
        if (!currentBuffer || `${currentNet.id}/${currentBuffer.name.toLowerCase()}` !== targetKey) {
          fishPending = 0;
          return;
        }
        state.currentBufferKey = key || null;
      } catch (e) {
        if (requestId !== state.fishStatusRequest) return;
      }
      if (fishPending === requestId) fishPending = 0;
    }
  }

  // A lookup for this buffer is still running: keep the "Checking" badge instead of flashing Off.
  if (fishPending) return;

  if (state.currentBufferKey) {
    const { mode } = parseFishKey(state.currentBufferKey);
    setFishBadge(btn, true, `FiSH Blowfish ${mode.toUpperCase()} active for ${activeBuf.name}. Click to view or edit key.`, mode);
  } else {
    setFishBadge(btn, false, `No FiSH key set for ${activeBuf.name}. Click to set key.`);
  }
}

function generateRandomKey(length = 16) {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  const arr = new Uint8Array(length);
  window.crypto.getRandomValues(arr);
  return Array.from(arr).map(x => chars[x % chars.length]).join("");
}

function initContextMenu() {
  const menu = document.getElementById("context-menu");
  if (!menu || contextMenuInitialized) return;
  contextMenuInitialized = true;

  window.addEventListener("contextmenu", (e) => {
    // The script editor keeps the native menu (cut, copy, paste) in its text.
    if (e.target.closest && e.target.closest("[data-se-root], input, textarea, [contenteditable=true]")) {
      hideContextMenu();
      return;
    }
    e.preventDefault();
    handleContextMenu(e);
  });

  window.addEventListener("click", (e) => {
    if (!e.target.closest("#context-menu")) {
      hideContextMenu();
    }
  });

  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      hideContextMenu();
    }
  });
}

function hideContextMenu() {
  const menu = document.getElementById("context-menu");
  if (menu) menu.style.display = "none";
  hideReferenceFlyout();
}

const REFERENCE_POPUPS = {
  status: [
    { heading: "Server" }, { label: "Lusers", action: "raw", command: "LUSERS" },
    { label: "Motd", action: "raw", command: "MOTD" }, { label: "Time", action: "raw", command: "TIME" },
    { heading: "Names" }, { label: "#ferric", action: "raw", command: "NAMES #ferric" },
    { label: "#irchelp", action: "raw", command: "NAMES #irchelp" }, { label: "Names…", action: "names" },
    { heading: "Join" }, { label: "#ferric", action: "raw", command: "JOIN #ferric" },
    { label: "#irchelp", action: "raw", command: "JOIN #irchelp" }, { label: "Join…", action: "join" },
    { heading: "Query" }, { label: "Query…", action: "query" },
    { heading: "Other" }, { label: "Query...", action: "query" }, { label: "Whois…", action: "whois" }, { label: "Change Nick…", action: "nick" },
    { heading: "Away" }, { label: "Set Away…", action: "away-prompt" }, { label: "Set Back", action: "raw", command: "AWAY" },
    { label: "List Channels", action: "raw", command: "LIST" }, { label: "FiSH Key Manager…", action: "fish" }, { separator: true },
    { label: "Edit Notes", action: "notes" }, { label: "Quit IRC", action: "raw", command: "QUIT :Leaving", danger: true },
  ],
  channel: [{ label: "Channel Modes", action: "channel-modes" }, { separator: true }, { label: "FiSH Key…", action: "fish" }],
  // Right-click on a network entry in the tree. No separate Disconnect: the app has no such action.
  network: [{ label: "Close", action: "close-network", danger: true }],
  menubar: [
    { heading: "Commands" }, { label: "Join Channel…", action: "join" }, { label: "Part Channel…", action: "part" },
    { label: "Query User…", action: "query" }, { label: "Send Notice…", action: "notice" }, { label: "Whois User…", action: "whois" },
    { heading: "CTCP" }, { label: "Ping…", action: "ctcp", command: "PING" }, { label: "Time…", action: "ctcp", command: "TIME" },
    { label: "Version…", action: "ctcp", command: "VERSION" }, { heading: "Set Away" },
    { label: "On…", action: "away-prompt" }, { label: "Off", action: "raw", command: "AWAY" },
    { label: "Invite User…", action: "invite" }, { label: "Ban User…", disabled: true }, { label: "Kick User…", action: "kick-prompt" },
    { label: "Ignore User…", disabled: true }, { label: "Unignore User…", disabled: true },
    { label: "Change Nick…", action: "nick" }, { label: "Quit IRC", action: "raw", command: "QUIT", danger: true },
  ],
  query: [
    { label: "Info", action: "whois" }, { label: "Whois", action: "whois" }, { label: "Query", action: "query" },
    { separator: true }, { label: "FiSH Key…", action: "fish" }, { label: "DH1080 keyXchange", action: "dh1080" }, { label: "Ignore", action: "ignore" }, { separator: true }, { heading: "CTCP" },
    { label: "Ping", action: "ctcp", command: "PING" }, { label: "Time", action: "ctcp", command: "TIME" },
    { label: "Version", action: "ctcp", command: "VERSION" },
  ],
  nicklist: [
    // Grouped like the client's nick menu: each group with a submenu opens to the right.
    { label: "Infos", children: [
      { label: "Info", action: "whois" }, { label: "Whois", action: "whois" }, { label: "Query", action: "query" },
    ] },
    // Status applies to every selected nick. OWNER and PROTECT need the letter in the network's PREFIX.
    { label: "Status", children: [
      { label: "OWNER", action: "status-mode", sign: "+", mode: "q", needsPrefix: true },
      { label: "deOWNER", action: "status-mode", sign: "-", mode: "q", needsPrefix: true },
      { label: "PROTECT", action: "status-mode", sign: "+", mode: "a", needsPrefix: true },
      { label: "dePROTECT", action: "status-mode", sign: "-", mode: "a", needsPrefix: true },
      { label: "OP", action: "status-mode", sign: "+", mode: "o" }, { label: "deOP", action: "status-mode", sign: "-", mode: "o" },
      { label: "halfOP", action: "status-mode", sign: "+", mode: "h" }, { label: "dehalfOP", action: "status-mode", sign: "-", mode: "h" },
      { label: "VOICE", action: "status-mode", sign: "+", mode: "v" }, { label: "deVOICE", action: "status-mode", sign: "-", mode: "v" },
    ] },
    { label: "Kicks and bans", children: [
      { label: "Kick", action: "kick" }, { label: "Kick (why)…", action: "kick-reason" }, { label: "Ban", action: "ban" },
      { label: "Ban, Kick", action: "ban-kick" }, { label: "Ban, Kick (why)…", action: "ban-kick-reason" },
    ] },
    { label: "Ignore", children: [{ label: "Ignore", action: "ignore" }, { label: "Unignore", action: "unignore" }] },
    { label: "Invite to my channel", action: "invite-here" },
    { separator: true },
    { label: "CTCP", children: [
      { label: "Ping", action: "ctcp", command: "PING" }, { label: "Time", action: "ctcp", command: "TIME" },
      { label: "Version", action: "ctcp", command: "VERSION" },
    ] },
    { label: "FiSH", children: [{ label: "FiSH Key…", action: "fish" }, { label: "DH1080 keyXchange", action: "dh1080" }] },
    { label: "Fun", children: [{ label: "Slap!", action: "slap" }] },
  ],
};

const FALLBACK_PREFIX_MODES = ["o", "v", "h", "q", "a"];
const STATUS_MODES_PER_LINE = 3;

// Mode letters of the network's PREFIX, from the backend. The fallback is used only when the
// backend sent no pairs for this network.
function prefixModesFor(networkId) {
  const pairs = state.networkPrefix?.[networkId];
  const modes = Array.isArray(pairs)
    ? pairs.map((pair) => (Array.isArray(pair) ? pair[0] : "")).filter((mode) => typeof mode === "string" && /^[A-Za-z]$/.test(mode))
    : [];
  return modes.length ? modes : FALLBACK_PREFIX_MODES;
}

// Standard IRC verbs sent to the server as typed when Ferric has no command of its own and no
// script alias matches. Any other unknown /word is shown as an error, not sent.
const IRC_PASSTHROUGH_COMMANDS = new Set([
  "MODE", "KICK", "TOPIC", "NICK", "AWAY", "INVITE", "QUIT", "LIST", "NAMES", "WHO", "WHOWAS",
  "OPER", "KILL", "PING", "PONG", "OP", "DEOP", "HOP", "DEHOP", "VOICE", "DEVOICE", "BAN", "UNBAN",
  "KICKBAN", "SETNAME", "STATS", "TIME", "VERSION", "ADMIN", "INFO", "LINKS", "MOTD", "LUSERS",
  "USERHOST", "ISON", "SILENCE", "SERVICE", "SQUIT", "WALLOPS", "CAP", "AUTHENTICATE",
]);

function validIrcToken(value) {
  return typeof value === "string" && value.length > 0 && value.length <= 200 && !/[\s,:\x00-\x1f\x7f]/.test(value);
}

// Verbs a Mouse-page command may use. Anything else is refused with a status line.
const MOUSE_DBLCLICK_VERBS = new Set(["query", "whois", "join", "part", "channel"]);

// Runs the stored command for a double-click target. $nick, $chan and $network are expanded
// per token after the command is split, so a value can never add tokens; each argument must
// then pass validIrcToken. The existing Ferric nick-list action is retained for unset settings.
async function runMouseDblclick(target, ctx) {
  const spec = MOUSE_DBLCLICK_TARGETS.find((t) => t.id === target);
  if (!spec || !spec.used || !ctx.network) return;
  let template = readMouseDblclick(target);
  try {
    if (target === "nicklist" && localStorage.getItem(MOUSE_DBLCLICK_PREFIX + target) === null) template = "query $nick";
  } catch { }
  if (template.includes("$$")) return;
  const tokens = template.trim().split(/\s+/).filter(Boolean);
  const verb = (tokens[0] || "").replace(/^\//, "").toLowerCase();
  if (!verb) return;
  if (!MOUSE_DBLCLICK_VERBS.has(verb)) {
    const net = state.networks[ctx.network];
    const status = net?.buffers?.status;
    if (status) {
      status.messages.push({
        id: nextLocalId(), local: true,
        sender: "*",
        text: `Command not supported: ${verb.replace(/[\x00-\x1f\x7f]/g, " ").slice(0, 40)}`,
        spans: [],
        is_notice: true,
        is_action: false,
        is_encrypted: false,
        is_highlight: false,
        timestamp: Date.now(),
      });
      invalidateLog();
      render();
    }
    return;
  }
  const args = tokens.slice(1).map((token) =>
    token.replace(/\$(nick|chan|network)(?![A-Za-z0-9_])/gi, (_, name) => ctx[name.toLowerCase()] || ""));
  if (verb === "channel") {
    const chan = args[0] || ctx.chan;
    if (chan) {
      await openChannelCentralFor(ctx.network, chan);
    } else {
      await openChannelCentralForActive();
    }
    return;
  }
  if (args.length !== 1 || !validIrcToken(args[0])) return;
  if (verb === "query") {
    await openQuery(ctx.network, args[0]);
    return;
  }
  if (!isTauri) return;
  const line = { whois: "WHOIS", join: "JOIN", part: "PART" }[verb] + " " + args[0];
  try {
    await invoke("send_raw", { network: ctx.network, raw: line });
  } catch (e) {
    console.error("send_raw error:", e);
  }
}

function validBanMask(mask) {
  return typeof mask === "string" && mask.length > 0 && mask.length <= 200 && !mask.startsWith(":") && !/[\s,\x00-\x1f\x7f]/.test(mask);
}

// Returns "*!*@host" for a nick in the context channel, or "" (with a chat notice) when the host is unknown.
function banMaskFor(context, nick) {
  const net = state.networks[context.network];
  const buf = net?.buffers?.[(context.channel || "").toLowerCase()];
  const host = buf?.users?.[(nick || "").toLowerCase()]?.host;
  const mask = host ? `*!*@${host}` : "";
  if (validBanMask(mask)) return mask;
  if (buf) {
    buf.messages.push({
      id: nextLocalId(), local: true,
      sender: "*",
      text: "Host not known yet. Wait for a message from this user.",
      spans: [],
      is_notice: true,
      is_action: false,
      is_encrypted: false,
      is_highlight: false,
      timestamp: Date.now(),
    });
    invalidateLog();
    render();
  }
  return "";
}

// A notice we sent shows locally as `-> -target- text`, in the target's window or the status window
//. A CTCP reply shows nothing.
function echoSentNotice(networkId, target, text) {
  if (text.startsWith("\u0001")) return;
  pushStatusNotice(networkId, `-> -${target}- ${text}`, target.replace(/^@/, "").toLowerCase());
}

// One notice line in the named buffer (lowercased key), or the status buffer when that one is not open.
function pushStatusNotice(networkId, text, bufferName = "status") {
  const buffers = state.networks[networkId]?.buffers;
  const status = buffers?.[bufferName] || buffers?.status;
  if (!status) return;
  status.messages.push({
    id: nextLocalId(), local: true,
    sender: "*",
    text,
    spans: [],
    is_notice: true,
    is_action: false,
    is_encrypted: false,
    is_highlight: false,
    timestamp: Date.now(),
  });
  invalidateLog();
  render();
}

async function runReferencePopupAction(item, context) {
  const ask = (message) => window.prompt(message) || "";
  const getNick = () => context.nick || ask("Enter nickname:").trim();
  const getChannel = () => context.channel || ask("Enter channel name:").trim();
  const raw = async (line) => {
    if (isTauri && context.network && line.length <= 510 && !/[\r\n\x00]/.test(line)) {
      await invoke("send_raw", { network: context.network, raw: line });
    }
  };
  switch (item.action) {
    case "raw": await raw(item.command); break;
    case "names": { const value = ask("Enter a channel name:").trim(); if (validIrcToken(value)) await raw(`NAMES ${value}`); break; }
    case "join": { const value = ask("Enter a channel to join:").trim(); if (validIrcToken(value)) await raw(`JOIN ${value}`); break; }
    case "part": { const channel = getChannel(); if (validIrcToken(channel)) await raw(`PART ${channel}`); break; }
    case "query": { const nick = getNick(); if (validIrcToken(nick) && context.network) await openQuery(context.network, nick); break; }
    case "channel-modes": { const channel = getChannel(); if (validIrcToken(channel)) await raw(`MODE ${channel}`); break; }
    case "whois": { const nick = getNick(); if (validIrcToken(nick)) await raw(`WHOIS ${nick}`); break; }
    case "nick": { const value = ask("Enter your new nickname:").trim(); if (validIrcToken(value)) await raw(`NICK ${value}`); break; }
    case "away-prompt": { const value = ask("Enter your away message:").replace(/[\r\n\x00]/g, " ").trim(); if (value.length <= 400) await raw(`AWAY :${value}`); break; }
    case "notice": { const to = ask("Enter nickname:").trim(); const text = ask("Enter notice:").replace(/[\r\n\x00]/g, " ").trim(); const line = `NOTICE ${to} :${text}`; if (validIrcToken(to) && text) { await raw(line); if (isTauri && context.network && line.length <= 510 && !/[\r\n\x00]/.test(line)) echoSentNotice(context.network, to, text); } break; }
    case "ctcp": { const nick = getNick(); if (validIrcToken(nick)) await raw(`PRIVMSG ${nick} :\x01${item.command}\x01`); break; }
    case "invite-here": { const nick = getNick(); const channel = getChannel(); if (validIrcToken(nick) && validIrcToken(channel)) await raw(`INVITE ${nick} ${channel}`); break; }
    case "invite": { const to = ask("Enter nickname:").trim(); const channel = getChannel(); if (validIrcToken(to) && validIrcToken(channel)) await raw(`INVITE ${to} ${channel}`); break; }
    case "kick": { const nick = getNick(); const channel = getChannel(); if (validIrcToken(nick) && validIrcToken(channel)) await raw(`KICK ${channel} ${nick}`); break; }
    case "kick-prompt": { const to = ask("Enter nickname:").trim(); const channel = getChannel(); if (validIrcToken(to) && validIrcToken(channel)) await raw(`KICK ${channel} ${to}`); break; }
    case "kick-reason": { const nick = getNick(); const channel = getChannel(); const reason = ask("Reason:").replace(/[\r\n\x00]/g, " ").trim(); if (validIrcToken(nick) && validIrcToken(channel)) await raw(`KICK ${channel} ${nick}${reason ? ` :${reason}` : ""}`); break; }
    case "ban": case "ban-kick": case "ban-kick-reason": {
      const nick = context.nick; const channel = context.channel;
      if (!validIrcToken(nick) || !validIrcToken(channel)) break;
      const mask = banMaskFor(context, nick);
      if (!mask) break;
      let reason = "Banned";
      if (item.action === "ban-kick-reason") {
        const answer = window.prompt("Reason:");
        if (answer === null) break;
        reason = answer.replace(/[\r\n\x00]/g, "").trim().slice(0, 200);
      }
      await raw(`MODE ${channel} +b ${mask}`);
      if (item.action !== "ban") await raw(`KICK ${channel} ${nick}${reason ? ` :${reason}` : ""}`);
      break;
    }
    case "status-mode": {
      if (!/^[+-]$/.test(item.sign || "") || !/^[A-Za-z]$/.test(item.mode || "")) break;
      const channel = context.channel;
      const wanted = context.nicks?.length ? context.nicks : [context.nick];
      const nicks = [];
      const skipped = [];
      for (const nick of wanted) {
        if (!validIrcToken(nick)) skipped.push(nick);
        else if (!nicks.some((seen) => seen.toLowerCase() === nick.toLowerCase())) nicks.push(nick);
      }
      if (!validIrcToken(channel)) {
        pushStatusNotice(context.network, "Mode change skipped: no valid channel");
        break;
      }
      if (skipped.length) {
        const shown = skipped.slice(0, 5).map((nick) => String(nick ?? "").replace(/[\x00-\x1f\x7f]/g, " ").slice(0, 30)).join(", ");
        pushStatusNotice(context.network, `Mode change skipped for ${skipped.length} invalid nick(s): ${shown}`);
      }
      for (let i = 0; i < nicks.length; i += STATUS_MODES_PER_LINE) {
        const batch = nicks.slice(i, i + STATUS_MODES_PER_LINE);
        await raw(`MODE ${channel} ${item.sign}${item.mode.repeat(batch.length)} ${batch.join(" ")}`);
      }
      break;
    }
    case "fish": { await openFishModal(context.fishTarget || null); break; }
    case "dh1080": { const nick = context.fishTarget; if (isTauri && context.network && validIrcToken(nick)) { try { await invoke("start_dh1080", { network: context.network, nick, cbc: true }); } catch { pushStatusNotice(context.network, `*** FiSH: DH1080 key exchange with ${nick} failed`, nick.toLowerCase()); } } break; }
    case "close-network": await closeNetwork(context.network); break;
    case "notes": await openNotesModal(); break;
    case "ignore": addIgnoredNick(context.fishTarget); invalidateLog(); render(); break;
    case "unignore": removeIgnoredNick(context.fishTarget); invalidateLog(); render(); break;
    case "slap": { const nick = getNick(); const channel = getChannel(); if (isTauri && context.network && validIrcToken(nick) && validIrcToken(channel)) { await invoke("send_message", { network: context.network, target: channel, text: `\x01ACTION slaps ${nick} around a bit with a large trout\x01` }); await syncFromBackend(); } break; }
  }
}

// The open submenus, outermost first. Each is a separate element, so the popup can scroll without
// clipping them. Submenus open on hover and nest to any depth.
const flyoutStack = [];

function closeFlyoutsFrom(depth) {
  while (flyoutStack.length > depth) flyoutStack.pop().el.remove();
}

function hideReferenceFlyout() {
  closeFlyoutsFrom(0);
}

// Drops the entries that need a PREFIX letter the network does not have, also inside submenus.
function filterPopupItems(list, modes) {
  return list
    .map((item) => (item.children ? { ...item, children: filterPopupItems(item.children, modes) } : item))
    .filter((item) => !item.needsPrefix || modes.includes(item.mode));
}

function showReferencePopup(menu, kind, context, x, y) {
  const allItems = REFERENCE_POPUPS[kind];
  if (!allItems) return false;
  // Hide OWNER/PROTECT when the network's PREFIX has no such letter.
  const modes = kind === "nicklist" ? prefixModesFor(context.network) : [];
  const items = filterPopupItems(allItems, modes);
  // Leaves are numbered as they are drawn, so a click finds its item whichever menu it is in.
  const leaves = [];
  const flyouts = new Map();
  const entryHtml = (item) => {
    if (item.separator) return '<div class="context-menu-separator"></div>';
    if (item.heading) return `<div class="context-menu-heading">${escapeHtml(item.heading)}</div>`;
    if (item.children) {
      const id = flyouts.size;
      flyouts.set(id, item.children.map(entryHtml).join(""));
      return `<div class="context-menu-item has-submenu" data-submenu="${id}">${escapeHtml(item.label)}</div>`;
    }
    leaves.push(item);
    return `<div class="context-menu-item${item.disabled ? " is-disabled" : ""}${item.danger ? " danger" : ""}" data-popup-index="${leaves.length - 1}"${item.disabled ? ' aria-disabled="true"' : ""}>${escapeHtml(item.label)}</div>`;
  };
  menu.innerHTML = `<div class="context-menu-header">${escapeHtml(context.nick || context.channel || (kind === "network" ? context.network : "") || kind)}</div>${items.map(entryHtml).join("")}`;
  menu.style.display = "block";
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(10, Math.min(x, window.innerWidth - rect.width - 10))}px`;
  menu.style.top = `${Math.max(10, Math.min(y, window.innerHeight - rect.height - 10))}px`;
  bindReferencePopup(menu, leaves, flyouts, context);
  void appendScriptMenuNodes(menu, kind, context, x, y);
  return true;
}

// Makes the entries of a popup run and opens its submenus on hover. `env` describes the entries of
// this kind of popup: the contents of each submenu by id, the attribute that marks a submenu entry,
// and how to bind the entries inside a root element.
function attachPopupEntries(root, depth, env) {
  env.bindLeaves(root);
  root.querySelectorAll(env.selector).forEach((anchor) => {
    anchor.addEventListener("mouseenter", () => openFlyout(anchor, env.idOf(anchor), depth, env));
  });
}

// Opens the flyout of a submenu entry beside it, at `depth`. Flyouts that were open at that depth
// or deeper close first.
function openFlyout(anchor, id, depth, env) {
  if (flyoutStack[depth]?.anchor === anchor) return;
  closeFlyoutsFrom(depth);
  const flyout = document.createElement("div");
  flyout.className = "context-menu reference-flyout";
  flyout.innerHTML = env.flyouts.get(id) || "";
  document.body.appendChild(flyout);
  flyoutStack.push({ el: flyout, anchor });
  attachPopupEntries(flyout, depth + 1, env);
  const r = anchor.getBoundingClientRect();
  const fr = flyout.getBoundingClientRect();
  // Beside the popup: to the right when it fits, otherwise to the left.
  const fitsRight = r.right + fr.width + 4 <= window.innerWidth - 10;
  const left = fitsRight ? r.right - 2 : r.left - fr.width + 2;
  flyout.style.left = `${Math.max(10, left)}px`;
  flyout.style.top = `${Math.max(10, Math.min(r.top - 3, window.innerHeight - fr.height - 10))}px`;
  // Leaving an entry or its flyout closes the flyouts, unless the pointer is in one of them.
  const maybeClose = () => setTimeout(() => {
    const menu = document.getElementById("context-menu");
    const hovered = [menu, ...flyoutStack.map((f) => f.el)].some((el) => el?.matches(":hover"));
    if (!hovered) closeFlyoutsFrom(0);
  }, 150);
  anchor.addEventListener("mouseleave", maybeClose);
  flyout.addEventListener("mouseleave", maybeClose);
}

// Clicks of a reference popup's entries run them. Its submenus use the same flyouts.
function bindReferencePopup(root, leaves, flyouts, context) {
  const bindLeaves = (el) => el.querySelectorAll("[data-popup-index]").forEach((leaf) => {
    leaf.addEventListener("click", async () => {
      const item = leaves[Number(leaf.dataset.popupIndex)];
      hideContextMenu();
      if (!item.disabled) await runReferencePopupAction(item, context);
    });
  });
  attachPopupEntries(root, 0, { flyouts, selector: "[data-submenu]", idOf: (a) => Number(a.dataset.submenu), bindLeaves });
}

let popupSerial = 0;

/**
 * Adds the entries of the loaded scripts below the reference entries of a popup, once the backend
 * has listed them. Nothing is added when the popup closed or was reopened in the meantime.
 */
async function appendScriptMenuNodes(menu, kind, context, x, y) {
  if (!isTauri) return;
  const serial = ++popupSerial;
  menu.dataset.popupSerial = String(serial);
  const nicks = context.nicks || [];
  const nodes = await fetchScriptMenuNodes(invoke, kind, {
    network: context.network,
    nick: context.nick || "",
    nicks,
    channel: context.channel || "",
    onError: (error) => pushStatusNotice(context.network, `*** Script menu: ${String(error?.message || error)}`),
  });
  if (!nodes.length || menu.style.display !== "block" || menu.dataset.popupSerial !== String(serial)) return;
  const { html, commands, flyouts } = renderScriptMenuNodes(nodes);
  menu.insertAdjacentHTML("beforeend", `<div class="context-menu-separator"></div>${html}`);
  const scriptOptions = {
    onClose: hideContextMenu,
    run: async (node) => {
      const buffer = state.networks[context.network]?.active_buffer || "status";
      try {
        await runScriptMenuNode(invoke, node, {
          network: context.network,
          buffer,
          nick: context.nick || "",
          nicks,
          channel: context.channel || "",
        });
      } catch (error) {
        pushStatusNotice(context.network, `*** Script: ${error}`, buffer);
      }
      syncFromBackend();
    },
  };
  // Submenus open a flyout beside them on hover, as in the reference popups.
  attachPopupEntries(menu, 0, {
    flyouts,
    selector: "[data-script-submenu]",
    idOf: (a) => Number(a.dataset.scriptSubmenu),
    bindLeaves: (el) => bindScriptMenuItems(el, commands, scriptOptions),
  });
  // The popup is taller now: keep it on screen.
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.max(10, Math.min(x, window.innerWidth - rect.width - 10))}px`;
  menu.style.top = `${Math.max(10, Math.min(y, window.innerHeight - rect.height - 10))}px`;
}

function handleContextMenu(e) {
  const menu = document.getElementById("context-menu");
  if (!menu) return;

  const net = state.networks[state.active_network];
  const activeBuf = net ? net.buffers[net.active_buffer] : null;

  const netHeader = e.target.closest(".network-header");
  if (netHeader && state.networks[netHeader.dataset.net]) {
    showReferencePopup(menu, "network", { network: netHeader.dataset.net }, e.clientX, e.clientY);
    return;
  }

  // The reference popup is for the window itself. A click on a nick in the chat, or a click that
  // ends a text selection, uses the menu below, which has the nick actions and Copy Selected Text.
  const hasSelection = Boolean(window.getSelection()?.toString());
  // With a text selection, only a click on a nick list entry or the menu bar still gets its popup.
  if (!e.target.closest(".channel-item") && !e.target.closest(".msg-sender") && (!hasSelection || e.target.closest(".nick-item, .app-menu-bar"))) {
    const nickItem = e.target.closest(".nick-item");
    const senderItem = e.target.closest(".msg-sender");
    const nick = nickItem ? nickItem.dataset.nick : senderItem ? senderItem.dataset.nick : "";
    const targetNetwork = nickItem?.dataset.net || state.active_network;
    const targetNet = state.networks[targetNetwork];
    const targetBuf = targetNet ? targetNet.buffers[targetNet.active_buffer] : null;
    const kind = e.target.closest(".app-menu-bar") ? "menubar" : nickItem ? "nicklist" : !targetBuf || targetBuf.name.toLowerCase() === "status" ? "status" : targetBuf.is_channel ? "channel" : "query";
    // Right-click on an unselected nick selects only it; on a selected nick the selection stays.
    if (nickItem && nickItem.dataset.nick && !nickSelection.nicks.has(nickItem.dataset.nick.toLowerCase())) {
      selectOnlyNick(nickItem.dataset.nick.toLowerCase());
    }
    const nicks = kind === "nicklist"
      ? nickItemEls().filter((el) => el.classList.contains("is-selected")).map((el) => el.dataset.nick)
      : [];
    if (showReferencePopup(menu, kind, {
      network: targetNetwork,
      channel: targetBuf?.is_channel ? targetBuf.name : "",
      // In a query window the partner is the nick, also on a click on the empty chat.
      nick: nick || (kind === "query" ? targetBuf?.name || "" : ""),
      nicks,
      fishTarget: kind === "nicklist" ? nick : kind === "status" || kind === "menubar" ? "" : targetBuf?.name || "",
    }, e.clientX, e.clientY)) return;
  }

  let itemsHtml = "";

  // 1. Channel or Query item in sidebar
  const chanItem = e.target.closest(".channel-item");
  if (chanItem) {
    const netId = chanItem.dataset.net;
    const bufName = chanItem.dataset.buf;
    const isChan = bufName.startsWith("#");

    itemsHtml = `
      <div class="context-menu-header">${escapeHtml(bufName)}</div>
      <div class="context-menu-item" data-action="fish" data-net="${escapeHtml(netId)}" data-target="${escapeHtml(bufName)}">
        <span>FiSH</span> <span>FiSH Key...</span>
      </div>
      <div class="context-menu-item" data-action="copy" data-text="${escapeHtml(bufName)}">
        Copy Name
      </div>
      <div class="context-menu-item" data-action="clear" data-net="${escapeHtml(netId)}" data-buf="${escapeHtml(bufName)}">
        Clear Buffer
      </div>
      <div class="context-menu-separator"></div>
      <div class="context-menu-item danger" data-action="close" data-net="${escapeHtml(netId)}" data-buf="${escapeHtml(bufName)}">
        ${isChan ? 'Part Channel' : 'Close Query'}
      </div>
    `;
  } else {
    // 2. Nick in nicklist or sender in chat message
    const nickItem = e.target.closest(".nick-item");
    const senderItem = e.target.closest(".msg-sender");
    const targetNick = nickItem ? nickItem.dataset.nick : (senderItem ? senderItem.dataset.nick : null);

    if (targetNick && targetNick !== "*" && targetNick !== "None") {
      const isChannel = activeBuf && activeBuf.is_channel;
      itemsHtml = `
        <div class="context-menu-header">${escapeHtml(targetNick)}</div>
        <div class="context-menu-item" data-action="query" data-nick="${escapeHtml(targetNick)}">
          Query (Private Chat)
        </div>
        <div class="context-menu-item" data-action="fish" data-target="${escapeHtml(targetNick)}">
          <span>FiSH</span> <span>FiSH Key for ${escapeHtml(targetNick)}...</span>
        </div>
        <div class="context-menu-item" data-action="whois" data-nick="${escapeHtml(targetNick)}">
          Whois ${escapeHtml(targetNick)}
        </div>
        <div class="context-menu-item" data-action="copy" data-text="${escapeHtml(targetNick)}">
          Copy Nick
        </div>
        ${isChannel ? `
          <div class="context-menu-separator"></div>
          <div class="context-menu-item" data-action="op" data-nick="${escapeHtml(targetNick)}">
            Give Op (+o)
          </div>
          <div class="context-menu-item" data-action="voice" data-nick="${escapeHtml(targetNick)}">
            Give Voice (+v)
          </div>
          <div class="context-menu-item" data-action="kick" data-nick="${escapeHtml(targetNick)}">
            Kick from Channel
          </div>
          <div class="context-menu-item" data-action="slap" data-nick="${escapeHtml(targetNick)}">
            Slap with Trout
          </div>
        ` : ''}
      `;
    } else {
      // 3. General chat log or window background
      const selectedText = window.getSelection() ? window.getSelection().toString() : "";
      const bufName = activeBuf ? activeBuf.name : null;

      itemsHtml = `
        <div class="context-menu-header">${escapeHtml(bufName || 'Ferric')}</div>
        ${selectedText ? `
          <div class="context-menu-item" data-action="copy" data-text="${escapeHtml(selectedText)}">
            Copy Selected Text
          </div>
          <div class="context-menu-separator"></div>
        ` : ''}
        ${bufName && bufName.toLowerCase() !== 'status' ? `
          <div class="context-menu-item" data-action="fish" data-target="${escapeHtml(bufName)}">
            <span>FiSH</span> <span>FiSH Key (${escapeHtml(bufName)})...</span>
          </div>
        ` : `
          <div class="context-menu-item" data-action="fish" data-target="">
            <span>FiSH</span> <span>FiSH Key Manager...</span>
          </div>
        `}
        <div class="context-menu-item" data-action="clear-active">
          Clear Window
        </div>
        <div class="context-menu-separator"></div>
        <div class="context-menu-item" data-action="connect-all">
          <span>Connect</span> <span>Connect all networks</span>
        </div>
      `;
    }
  }

  menu.innerHTML = itemsHtml;
  menu.style.display = "block";

  // Position within viewport boundaries
  const rect = menu.getBoundingClientRect();
  let x = e.clientX;
  let y = e.clientY;

  if (x + rect.width > window.innerWidth) {
    x = window.innerWidth - rect.width - 10;
  }
  if (y + rect.height > window.innerHeight) {
    y = window.innerHeight - rect.height - 10;
  }

  menu.style.left = `${Math.max(10, x)}px`;
  menu.style.top = `${Math.max(10, y)}px`;

  // Attach click handlers to context menu items
  menu.querySelectorAll(".context-menu-item").forEach(item => {
    item.addEventListener("click", async () => {
      const action = item.dataset.action;
      hideContextMenu();

      switch (action) {
        case "fish": {
          const tgt = item.dataset.target;
          openFishModal(tgt || null);
          break;
        }
        case "query": {
          const nick = item.dataset.nick;
          if (nick && net) {
            openQuery(net.id, nick);
          }
          break;
        }
        case "whois": {
          const nick = item.dataset.nick;
          if (nick && isTauri && net && /^[^\s,:\x00-\x1f\x7f]+$/.test(nick)) {
            await invoke("send_raw", { network: net.id, raw: `WHOIS ${nick}` });
          }
          break;
        }
        case "copy": {
          const text = item.dataset.text;
          if (text) {
            await navigator.clipboard.writeText(text);
          }
          break;
        }
        case "close": {
          const netId = item.dataset.net;
          const buf = item.dataset.buf;
          if (netId && buf) {
            if (buf.startsWith("#") && isTauri) {
              await invoke("send_raw", { network: netId, raw: `PART ${buf}` });
            }
            closeBuffer(netId, buf);
          }
          break;
        }
        case "clear": {
          await clearBufferLines(item.dataset.net, item.dataset.buf.toLowerCase());
          break;
        }
        case "clear-active": {
          if (net && activeBuf) await clearBufferLines(net.id, net.active_buffer);
          break;
        }
        case "op": {
          const nick = item.dataset.nick;
          if (nick && activeBuf && isTauri && net && /^[^\s,:\x00-\x1f\x7f]+$/.test(nick) && /^[^\s,\x00-\x1f\x7f]+$/.test(activeBuf.name)) {
            await invoke("send_raw", { network: net.id, raw: `MODE ${activeBuf.name} +o ${nick}` });
          }
          break;
        }
        case "voice": {
          const nick = item.dataset.nick;
          if (nick && activeBuf && isTauri && net && /^[^\s,:\x00-\x1f\x7f]+$/.test(nick) && /^[^\s,\x00-\x1f\x7f]+$/.test(activeBuf.name)) {
            await invoke("send_raw", { network: net.id, raw: `MODE ${activeBuf.name} +v ${nick}` });
          }
          break;
        }
        case "kick": {
          const nick = item.dataset.nick;
          if (nick && activeBuf && isTauri && net && /^[^\s,:\x00-\x1f\x7f]+$/.test(nick) && /^[^\s,\x00-\x1f\x7f]+$/.test(activeBuf.name)) {
            await invoke("send_raw", { network: net.id, raw: `KICK ${activeBuf.name} ${nick} :Requested` });
          }
          break;
        }
        case "slap": {
          const nick = item.dataset.nick;
          if (nick && activeBuf && isTauri && net) {
            await invoke("send_message", {
              network: net.id,
              target: activeBuf.name,
              text: `\x01ACTION slaps ${nick} around a bit with a large trout\x01`
            });
            await syncFromBackend();
          }
          break;
        }
        case "connect-all": {
          if (isTauri) {
            try {
              await loadConnectdWithProxy();
              await syncFromBackend();
            } catch (e) {
              pushStatusNotice(state.active_network, String(e?.message || e));
            }
          }
          break;
        }
      }
    });
  });
}

async function openFishModal(initialTarget = null) {
  dismissSlotDialogs();
  state.showFishModal = true;
  state.showConnectModal = false;
  state.showOptionsModal = false;
  state.showNotesModal = false;

  const net = state.networks[state.active_network];
  const activeBuf = net ? net.buffers[net.active_buffer] : null;

  state.quickNetwork = "";
  if (initialTarget) {
    state.quickTarget = initialTarget;
  } else if (activeBuf && activeBuf.name.toLowerCase() !== "status") {
    state.quickTarget = activeBuf.name;
  } else {
    state.quickTarget = "";
  }

  if (isTauri) {
    try {
      state.fishKeysList = await invoke("get_fish_keys", {});
    } catch (e) {
      console.error("Failed to load fish keys:", e);
    }
  }

  if (state.quickTarget) {
    const targetLower = state.quickTarget.toLowerCase();
    // Prefer the entry stored under the active network, so a channel kept under two networks shows its own mode.
    const networkSectionLower = `${state.active_network}:${targetLower}`.toLowerCase();
    const existing = state.fishKeysList.find(e =>
      e.section.toLowerCase() === networkSectionLower
    ) || state.fishKeysList.find(e =>
      e.target.toLowerCase() === targetLower || e.section.toLowerCase() === targetLower
    );
    const loadedKey = existing ? existing.key : (state.currentBufferKey || "");
    const parsedKey = parseFishKey(loadedKey);
    state.quickKey = parsedKey.key;
    state.quickFishMode = loadedKey ? parsedKey.mode : "ecb";
  } else {
    state.quickKey = "";
    state.quickFishMode = "ecb";
  }

  renderModal();
}

function closeFishModal() {
  state.showFishModal = false;
  renderModal();
}

// Tools > Server manager: takes the modal slot like the other dialogs.
function openServerManagerDialog() {
  closeSideDialogs();
  dismissSlotDialogs();
  openServerManager();
}

// Colors and the address book take the same modal slot, and renderModal draws one of them first. A
// dialog opened from elsewhere closes them, so their open flag cannot hide the new dialog.
function dismissSlotDialogs() {
  if (isServerManagerOpen()) closeServerManager();
  if (isColorsModalOpen()) closeColorsModal();
  if (isAddressBookModalOpen()) closeAddressBookModal();
  if (isScriptDialogOpen()) closeOpenScriptDialog(true);
}

// Opening Colors or the address book closes the other dialogs too: no open flag is left behind to hide
// the next dialog, and the other slot dialog is closed with its own cancel rules.
function closeSideDialogs() {
  if (isServerManagerOpen()) closeServerManager();
  state.showConnectModal = false;
  state.showFishModal = false;
  state.showOptionsModal = false;
  state.showNotesModal = false;
}

function openColorsModal() {
  closeSideDialogs();
  if (isAddressBookModalOpen()) closeAddressBookModal();
  if (isScriptDialogOpen()) closeOpenScriptDialog(true);
  openColorsDialog();
}

function openAddressBookModal(tab, nick) {
  closeSideDialogs();
  if (isColorsModalOpen()) closeColorsModal();
  if (isScriptDialogOpen()) closeOpenScriptDialog(true);
  openAddressBookDialog(tab, nick);
}

function renderModal() {
  if (isServerManagerOpen()) {
    renderServerManager();
  } else if (isAddressBookModalOpen()) {
    renderAddressBookModal();
  } else if (isColorsModalOpen()) {
    renderColorsModal();
  } else if (state.showFishModal) {
    renderFishModal();
  } else if (state.showConnectModal) {
    renderConnectModal();
  } else if (state.showOptionsModal) {
    renderOptionsModal();
  } else if (state.showNotesModal) {
    renderNotesModal();
  } else {
    const container = document.getElementById("modal-container");
    if (container) container.innerHTML = "";
  }
}

// Logging, identd and proxy settings. All off by default; each lives in one JSON object in
// localStorage. The proxy password is never stored: it stays in a variable until the app closes.
const LOGGING_KEY = "ferric-logging";
const IDENTD_KEY = "ferric-identd";
const PROXY_KEY = "ferric-proxy";
const OPTIONS_STORAGE_KEY = "ferric-options";
const IDENTD_DEFAULT_PORT = "113";

const DEFAULT_APP_OPTIONS = {
  // Connect > Options
  connectOnStartup: true,
  reconnectOnDisconnect: true,
  showConnectOnStartup: false,
  moveToTopOnConnect: false,
  checkConnectionTimeout: true,
  preserveNicknames: false,
  useInvisibleMode: false,
  connectDebug: true,
  defaultPort: "6667",
  performEnabled: true,
  performCommands: "",
  retryCount: "10",
  retryDelay: "30",
  sslRejectInvalid: false,
  sslAllowSelfSigned: true,
  pythonEnabled: true,
  pythonPath: "",

  // Connect > Identd
  identdEnabled: false,
  identdUserId: "IDENT1",
  identdSystem: "UNIX",
  identdPort: "113",
  identdShowRequests: true,
  identdOnlyWhenConnecting: true,
  identdUseEmailId: false,

  // Connect > Proxy
  proxyConnection: "None",
  proxyProtocol: "Socks4",
  proxyHostname: "",
  proxyUserId: "",
  proxyPassword: "",
  proxyPort: "1080",
  proxyExceptionMasks: "",

  // Connect > Local
  localOnConnectGetHost: true,
  localOnConnectGetIp: true,
  localLookupMethod: "server",
  localLookupWebsite: "icanhazip.com",
  localHostName: "",
  localIpAddress: "",

  // IRC
  prefixOwnMessages: true,
  prefixShowModes: true,
  queryMinimize: true,
  queryNotifyNicks: true,
  queryWhoisOnOpen: true,
  queryAutoKeyx: true,
  querySingleWindow: false,
  queryCopyMessages: true,
  channelAutoJoinOnInvite: true,
  channelRejoinWhenKicked: false,
  channelRejoinOnConnect: false,
  channelKeepOpenOnKickQuit: false,
  channelKeepOpenOnPart: false,
  showActiveAway: false,
  showActiveNotice: true,
  showActiveCtcp: true,
  showActiveQuery: false,
  showActiveInvite: true,
  showActiveWhois: false,

  // IRC > Options
  showShortJoinsParts: true,
  showUserAddresses: true,
  showNicknamesOnJoin: false,
  flashChannel: false,
  flashQuery: false,
  flashChat: false,
  skipMotdOnConnect: false,
  hidePingPong: true,
  cancelAwayOnActivity: false,
  sendTypingNotifications: true,
  hideAwayReminders: false,
  eventsSound: false,
  eventsFlash: false,

  // IRC > Messages
  timestampEvents: true,
  timestampFormat: "[HH:nn:ss:ms]",
  scrollbackLines: 2000,
  stripColor: false,
  stripOther: false,
  stripIfMoreThan: false,
  stripThreshold: "50",
  ctcpFingerReply: "*rrrrrrrrr*",
  quitMessage: "$read(quitmsg.txt)",
  splitLongMessages: true,
  utf8EncodeDecode: true,

  // IRC > Catcher
  catchUrls: true,
  catchEmails: false,
  chatLinkSupport: true,
  chatLinkConfirmRequests: true,
  unmarkedPlaceTop: true,
  unmarkedDeleteOnExit: true,
  onSendDescription: false,
  warnOpenUrl: false,

  // IRC > Logging
  logEnabled: false,
  logChannel: true,
  logQuery: true,
  logStatus: true,
  reloadLogsOnOpen: false,
  reloadLines: "100",
  logLockFiles: true,
  logStripCodes: true,
  logLineColors: false,
  logTrim: true,
  logTrimSize: "10000",
  logTimestamp: true,
  logTimestampFormat: "[dd.mm.yy|HH:nn:ss]",
  logDateFilenames: true,
  logDatePeriod: "By Month",
  logDateExceptStatus: true,
  logIncludeNetwork: true,
  logMakeFolder: true,
  logFolder: "logs\\",

  // IRC > Flood
  floodEnable: false,
  floodShowStatusActive: false,
  floodTriggerBytes: "300",
  floodBufferSize: "30",
  floodLinesPerUser: "10",
  floodIgnoreSeconds: "0",
  floodLimitCtcp: false,
  floodLimitWhois: false,
  floodLimitQuery: false,
  floodQueueOp: false,
  floodQueueOwn: false,

  // Display > RSS Feeds
  rssEnabled: true,
  rssUrls: "https://wid.cert-bund.de/content/public/securityAdvisory/rss\nhttps://www.heise.de/security/Alerts/feed.xml",
  rssCount: "3",
  rssMode: "fixed",
  rssInterval: "15",
};

function readJsonSetting(key) {
  try {
    const parsed = JSON.parse(localStorage.getItem(key) || "null");
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return parsed;
  } catch { }
  return {};
}

function writeJsonSetting(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch { }
}

// The proxy password is kept for the session only, so it never goes to storage.
function withoutSecrets(opts) {
  const { proxyPassword: _secret, ...rest } = opts;
  return rest;
}

const textSetting = (value, max, fallback = "") => (typeof value === "string" ? value.slice(0, max) : fallback);

function readAppOptions() {
  const saved = readJsonSetting(OPTIONS_STORAGE_KEY);
  const opts = { ...DEFAULT_APP_OPTIONS, ...saved };
  // Before version 2 "Connect on startup" was saved but never applied, so an old saved value was no
  // choice: the default applies once. From then on the saved value counts.
  if ((saved?.optionsVersion ?? 1) < 2) opts.connectOnStartup = DEFAULT_APP_OPTIONS.connectOnStartup;
  opts.optionsVersion = 2;
  // The old default without milliseconds moves to the new one.
  if (opts.timestampFormat === "[HH:nn:ss]") opts.timestampFormat = DEFAULT_APP_OPTIONS.timestampFormat;
  if (!opts.rssUrls || opts.rssUrls.includes("tagesschau.de") || opts.rssUrls.includes("nytimes.com")) {
    opts.rssUrls = DEFAULT_APP_OPTIONS.rssUrls;
  }
  // A password saved by an older build is removed from storage and not loaded.
  if (saved && typeof saved === "object" && "proxyPassword" in saved) {
    writeJsonSetting(OPTIONS_STORAGE_KEY, withoutSecrets(opts));
  }
  opts.proxyPassword = "";
  return opts;
}

let appOptions = readAppOptions();
let pendingOptions = { ...appOptions };
let optionsSubdialog = null;

const loggingSettings = {
  enabled: appOptions.logEnabled,
  channel: appOptions.logChannel,
  query: appOptions.logQuery,
  status: appOptions.logStatus,
};

const identdSettings = {
  enabled: appOptions.identdEnabled,
  port: textSetting(appOptions.identdPort, 5, IDENTD_DEFAULT_PORT),
  username: textSetting(appOptions.identdUserId, 32),
};

// The backend kind for the Protocol option. Socks4 means SOCKS4a, which sends host names to the proxy.
function proxyKindOf(protocol) {
  if (protocol === "Proxy") return "http";
  if (protocol === "Socks4") return "socks4";
  return "socks5";
}

const proxySettings = {
  enabled: appOptions.proxyConnection !== "None",
  kind: proxyKindOf(appOptions.proxyProtocol),
  host: textSetting(appOptions.proxyHostname, 255),
  port: textSetting(appOptions.proxyPort, 5),
  username: textSetting(appOptions.proxyUserId, 255),
};

let proxyPassword = appOptions.proxyPassword || "";
let identdError = "";

function syncSettingsFromAppOptions() {
  identdSettings.enabled = !!appOptions.identdEnabled;
  identdSettings.port = appOptions.identdPort || IDENTD_DEFAULT_PORT;
  identdSettings.username = appOptions.identdUserId || "";

  proxySettings.enabled = appOptions.proxyConnection !== "None";
  proxySettings.kind = proxyKindOf(appOptions.proxyProtocol);
  proxySettings.host = appOptions.proxyHostname || "";
  proxySettings.port = appOptions.proxyPort || "1080";
  proxySettings.username = appOptions.proxyUserId || "";
  proxyPassword = appOptions.proxyPassword || "";

  loggingSettings.enabled = !!appOptions.logEnabled;
  loggingSettings.channel = !!appOptions.logChannel;
  loggingSettings.query = !!appOptions.logQuery;
  loggingSettings.status = !!appOptions.logStatus;
}
syncSettingsFromAppOptions();

function saveAppOptions(opts) {
  appOptions = { ...appOptions, ...opts };
  writeJsonSetting(OPTIONS_STORAGE_KEY, withoutSecrets(appOptions));
  syncSettingsFromAppOptions();
  writeJsonSetting(IDENTD_KEY, identdSettings);
  writeJsonSetting(PROXY_KEY, proxySettings);
  writeJsonSetting(LOGGING_KEY, loggingSettings);
  void applyIdentd();
  void applyLogging();
  void applyConnectDebug();
  void applySelfSignedSetting();
  void applyPythonSettings();
  void applyQueryAutoKeyxSetting();
  rowHtmlCache.clear();
  render();
  if (typeof rssManager !== "undefined") {
    rssManager.render();
    rssManager.startSchedule();
    rssManager.fetchAll();
  }
}

// The Options setting "Allow self-signed certificates" reaches the backend, which applies it to every
// connect. Until this runs the backend uses its own default (on), which matches the Options default.
async function applySelfSignedSetting() {
  if (!isTauri) return;
  try {
    // "Reject invalid certificates" refuses self-signed ones, whatever the self-signed option says.
    await invoke("set_allow_self_signed", { value: appOptions.sslAllowSelfSigned !== false && !appOptions.sslRejectInvalid });
  } catch (err) {
    console.warn("set_allow_self_signed failed:", err);
  }
}

async function applyQueryAutoKeyxSetting() {
  if (!isTauri) return;
  try {
    await invoke("set_query_auto_keyx", { value: appOptions.queryAutoKeyx !== false });
  } catch (err) {
    console.warn("set_query_auto_keyx failed:", err);
  }
}

async function applyLogging() {
  if (!isTauri) return;
  try {
    await invoke("set_logging", { enabled: loggingSettings.enabled, channel: loggingSettings.channel, query: loggingSettings.query, status: loggingSettings.status });
  } catch (err) {
    console.warn("set_logging failed:", err);
  }
}

async function applyConnectDebug() {
  if (!isTauri) return;
  try {
    await invoke("set_connect_debug", { enabled: appOptions.connectDebug !== false });
  } catch (err) {
    console.warn("set_connect_debug failed:", err);
  }
}

async function applyPythonSettings() {
  if (!isTauri) return;
  try {
    const enabled = appOptions.pythonEnabled !== false;
    const path = (appOptions.pythonPath || "").trim();
    await invoke("set_python_settings", {
      enabled,
      path: path.length > 0 ? path : null,
    });
  } catch (err) {
    console.warn("set_python_settings failed:", err);
  }
}

function portInRange(text) {
  return /^\d{1,5}$/.test(text) && Number(text) >= 1 && Number(text) <= 65535;
}

// Same rules as set_identd in main.rs: port 1-65535, username up to 32 of letters, digits, - _ .
function identdProblem(settings) {
  if (!portInRange(settings.port)) return "Port must be a number from 1 to 65535.";
  if (settings.username.length > 32 || !/^[A-Za-z0-9._-]*$/.test(settings.username)) {
    return "Username: at most 32 characters, letters, digits, '-', '_' or '.'.";
  }
  return "";
}

function showIdentdError() {
  const el = document.getElementById("options-identd-error");
  if (!el) return;
  el.textContent = identdError;
  el.hidden = !identdError;
}

async function applyIdentd() {
  identdError = identdSettings.enabled ? identdProblem(identdSettings) : "";
  if (!identdError && isTauri) {
    const on = identdSettings.enabled;
    try {
      await invoke("set_identd", { enabled: on, port: on ? Number(identdSettings.port) : Number(IDENTD_DEFAULT_PORT), username: on ? identdSettings.username : "" });
    } catch (err) {
      identdError = String(err);
    }
  }
  showIdentdError();
}

// Proxy arguments for connect_network. An enabled proxy with a missing host or port is refused
// here: the backend would treat an empty host as "no proxy" and connect directly.
function proxyConnectArgs() {
  const p = proxySettings;
  if (!p.enabled) return { args: { proxyKind: "", proxyHost: "", proxyPort: null, proxyUsername: "", proxyPassword: "" } };
  const host = p.host.trim();
  if (!host || !portInRange(p.port.trim())) {
    return { error: "The proxy is on. Enter its host and a port from 1 to 65535, or turn it off in Options > Connect > Proxy." };
  }
  return { args: { proxyKind: p.kind, proxyHost: host, proxyPort: Number(p.port.trim()), proxyUsername: p.username, proxyPassword } };
}

// Exact blow.ini section for a key: "<network id>:<target>". A target that already has a
// network part is used as typed.
function fishSection(networkId, target) {
  return !networkId || target.includes(":") ? target : `${networkId}:${target}`;
}

// load_connectd with the proxy settings. An enabled but incomplete proxy refuses to connect
// (throws) instead of connecting directly.
async function loadConnectdWithProxy() {
  const proxy = proxyConnectArgs();
  if (proxy.error) throw new Error(proxy.error);
  return invoke("load_connectd", proxy.args);
}

// Notes (Edit Notes in the status popup): text lives in notes.txt next to the app, read and
// written by the backend.
let notesModal = { text: "", loading: false, loadFailed: false, saving: false, error: "" };

async function openNotesModal() {
  dismissSlotDialogs();
  state.showNotesModal = true;
  state.showConnectModal = false;
  state.showFishModal = false;
  state.showOptionsModal = false;
  const modal = { text: "", loading: isTauri, loadFailed: !isTauri, saving: false, error: isTauri ? "" : "Notes are available in the desktop app." };
  notesModal = modal;
  renderModal();
  if (!isTauri) return;
  try {
    modal.text = String(await invoke("read_notes", {}));
  } catch (err) {
    modal.loadFailed = true;
    modal.error = String(err);
  }
  modal.loading = false;
  if (state.showNotesModal && notesModal === modal) renderModal();
}

function closeNotesModal() {
  state.showNotesModal = false;
  renderModal();
}

function renderNotesModal() {
  const container = document.getElementById("modal-container");
  if (!container) return;
  const modal = notesModal;
  container.innerHTML = `
    <div class="modal-overlay" id="notes-modal-backdrop">
      <div class="modal-card options-dialog notes-dialog" role="dialog" aria-modal="true" aria-labelledby="notes-title">
        <h2 class="modal-title" id="notes-title">Notes</h2>
        <div class="notes-body">
          <div id="notes-error" class="form-error" role="alert" ${modal.error ? "" : "hidden"}>${escapeHtml(modal.error)}</div>
          <textarea id="notes-text" class="form-input notes-text" spellcheck="false" aria-label="Notes" ${modal.loading || modal.loadFailed ? "disabled" : ""}></textarea>
        </div>
        <div class="options-dialog-actions">
          <button id="notes-cancel" class="btn" type="button">Cancel</button>
          <button id="notes-save" class="btn btn-primary" type="button" ${modal.loading || modal.loadFailed || modal.saving ? "disabled" : ""}>Save</button>
        </div>
      </div>
    </div>
  `;
  const textarea = document.getElementById("notes-text");
  textarea.value = modal.text;
  textarea.addEventListener("input", () => { modal.text = textarea.value; });
  document.getElementById("notes-cancel").addEventListener("click", closeNotesModal);
  document.getElementById("notes-modal-backdrop").addEventListener("click", (event) => {
    if (event.target === event.currentTarget) closeNotesModal();
  });
  document.getElementById("notes-save").addEventListener("click", async () => {
    modal.text = textarea.value;
    modal.saving = true;
    modal.error = "";
    renderNotesModal();
    try {
      await invoke("write_notes", { text: modal.text });
      if (notesModal === modal && state.showNotesModal) closeNotesModal();
    } catch (err) {
      modal.error = String(err);
    }
    modal.saving = false;
    if (notesModal === modal && state.showNotesModal) renderNotesModal();
  });
  (textarea.disabled ? document.getElementById("notes-cancel") : textarea).focus();
}

function openExternalUrl(url) {
  if (!url) return;
  if (isTauri && invoke) {
    invoke("open_external_url", { url }).catch(() => {
      window.open(url, "_blank");
    });
  } else {
    window.open(url, "_blank");
  }
}

// Each set of headlines stays on screen this long, then the next set follows.
const RSS_SET_SECONDS = 30;
const RSS_SCROLL_PX_PER_SEC = 30; // 1 px every 2 frames at 60 Hz: calm and even

// Short source label for a feed: its first word, e.g. "BSI" or "heise" from the feed title.
const shortSource = (title) => title.match(/[\p{L}\p{N}]+/u)?.[0] || title;

const rssManager = {
  items: [],
  timer: null,
  rotateTimer: null,
  cursor: 0,
  isFetching: false,
  statusText: "",

  updateStatus(msg) {
    this.statusText = msg;
    const el = document.getElementById("rss-options-status");
    if (el) el.textContent = msg;
  },

  updateTickerVisibility() {
    const el = document.getElementById("toolbar-rss");
    if (!el) return;
    el.style.display = appOptions.rssEnabled ? "flex" : "none";
  },

  async fetchAll(force = false) {
    if (!appOptions.rssEnabled && !force) {
      this.updateTickerVisibility();
      return;
    }
    if (this.isFetching) return;
    this.isFetching = true;
    this.updateStatus("Loading RSS feeds...");

    const urls = (appOptions.rssUrls || "")
      .split("\n")
      .map((u) => u.trim())
      .filter((u) => u.startsWith("http://") || u.startsWith("https://"));

    if (urls.length === 0) {
      this.items = [];
      this.isFetching = false;
      this.render();
      this.updateStatus("No valid URLs entered.");
      return;
    }

    const fetchedItems = [];
    const parser = new DOMParser();

    for (const url of urls) {
      try {
        let xmlText = "";
        if (isTauri && invoke) {
          // Feeds go through the same proxy as the connections, or are refused when it cannot be used.
          const proxy = proxyConnectArgs();
          if (proxy.error) throw new Error(proxy.error);
          xmlText = await invoke("fetch_rss_feed", { url, ...proxy.args });
        } else {
          const resp = await fetch(url);
          xmlText = await resp.text();
        }

        const doc = parser.parseFromString(xmlText, "text/xml");
        if (doc.querySelector("parsererror")) {
          console.warn("RSS parse error for:", url);
          continue;
        }

        let feedTitle =
          doc.querySelector("channel > title")?.textContent?.trim() ||
          doc.querySelector("feed > title")?.textContent?.trim() ||
          "";
        if (!feedTitle) {
          try {
            feedTitle = new URL(url).hostname;
          } catch {
            feedTitle = "News";
          }
        }

        const rssItems = Array.from(doc.querySelectorAll("item"));
        if (rssItems.length > 0) {
          for (const item of rssItems.slice(0, 15)) {
            const rawTitle = item.querySelector("title")?.textContent || "";
            const cleanTitle = rawTitle.replace(/<[^>]*>/g, "").replace(/\s+/g, " ").trim();
            const link = item.querySelector("link")?.textContent?.trim() || "";
            if (cleanTitle) {
              fetchedItems.push({ title: cleanTitle, link, source: shortSource(feedTitle) });
            }
          }
        } else {
          const atomEntries = Array.from(doc.querySelectorAll("entry"));
          for (const entry of atomEntries.slice(0, 15)) {
            const rawTitle = entry.querySelector("title")?.textContent || "";
            const cleanTitle = rawTitle.replace(/<[^>]*>/g, "").replace(/\s+/g, " ").trim();
            const linkEl = entry.querySelector("link[href]") || entry.querySelector("link");
            const link = linkEl?.getAttribute("href") || linkEl?.textContent?.trim() || "";
            if (cleanTitle) {
              fetchedItems.push({ title: cleanTitle, link, source: shortSource(feedTitle) });
            }
          }
        }
      } catch (err) {
        console.warn("Could not load RSS feed:", url, err);
      }
    }

    this.items = fetchedItems;
    this.isFetching = false;
    this.updateStatus(this.items.length > 0 ? `${this.items.length} headlines loaded.` : "No headlines found.");
    this.render();
  },

  render() {
    this.updateTickerVisibility();
    const track = document.getElementById("rss-ticker-track");
    if (!track) return;
    clearTimeout(this.rotateTimer);

    if (!appOptions.rssEnabled) {
      track.innerHTML = '<span class="rss-empty-hint">RSS Ticker deaktiviert (Optionen > Display > RSS Feeds)</span>';
      return;
    }

    if (!this.items || this.items.length === 0) {
      track.innerHTML = `<span class="rss-empty-hint">${this.isFetching ? "Loading feeds..." : "No headlines loaded."}</span>`;
      return;
    }

    this.cursor = 0;
    if (appOptions.rssMode === "scroll") this.showScroll();
    else this.showPage();
  },

  itemHtml(item) {
    return `
        <a class="rss-ticker-item" href="${escapeHtml(item.link || "#")}" data-url="${escapeHtml(item.link || "")}" target="_blank" rel="noopener noreferrer" title="${escapeHtml(item.title)}">
          <span class="rss-source">[${escapeHtml(item.source)}]</span>
          <span class="rss-title">${escapeHtml(item.title)}</span>
          <span class="rss-separator">★</span>
        </a>
      `;
  },

  // Scrolling mode: all headlines run past in a loop. The list is written twice, so moving it by half its
  // width repeats it without a jump. Hovering pauses it. It moves at RSS_SCROLL_PX_PER_SEC in whole-pixel
  // steps (steps() in app.css), so the text never sits on a sub-pixel offset and does not smear.
  showScroll() {
    const track = document.getElementById("rss-ticker-track");
    if (!track) return;
    const run = this.items.map((item) => this.itemHtml(item)).join("");
    track.classList.add("is-scrolling");
    track.innerHTML = `<span class="rss-scroll-run">${run}</span><span class="rss-scroll-run" aria-hidden="true">${run}</span>`;
    const width = Math.round(track.firstElementChild.getBoundingClientRect().width) || 600;
    track.style.setProperty("--rss-shift", `-${width}px`);
    track.style.setProperty("--rss-steps", width);
    track.style.setProperty("--rss-duration", `${width / RSS_SCROLL_PX_PER_SEC}s`);
  },

  // Fixed mode: a set of headlines, then the next set after RSS_SET_SECONDS.
  showPage() {
    const track = document.getElementById("rss-ticker-track");
    if (!track) return;
    track.classList.remove("is-scrolling");
    const perSet = Math.min(Math.max(1, parseInt(appOptions.rssCount, 10) || 3), 10);
    const count = Math.min(perSet, this.items.length);
    let html = "";
    for (let i = 0; i < count; i++) {
      html += this.itemHtml(this.items[(this.cursor + i) % this.items.length]);
    }
    track.innerHTML = html;
    this.cursor = (this.cursor + count) % this.items.length;
    this.rotateTimer = setTimeout(() => this.showPage(), RSS_SET_SECONDS * 1000);
  },

  startSchedule() {
    if (this.timer) clearInterval(this.timer);
    const mins = Math.max(1, parseInt(appOptions.rssInterval || "15", 10) || 15);
    this.timer = setInterval(() => {
      this.fetchAll();
    }, mins * 60 * 1000);
  },

  init() {
    this.render();
    this.startSchedule();
    // In the app the first fetch waits for initTauri(): invoke is a stub until then.
    if (!isTauri) void this.fetchAll();
  }
};

// Category tree. Display, Mouse, Logging, Identd and Proxy have
// controls; the other pages say they are not available yet instead of showing switches that do nothing.
const OPTIONS_TREE = [
  {
    id: "connect", label: "Connect", children: [
      { id: "connect-options", label: "Options" }, { id: "connect-identd", label: "Identd" },
      { id: "connect-proxy", label: "Proxy" }, { id: "connect-local", label: "Local" },
    ]
  },
  {
    id: "irc", label: "IRC", page: true, children: [
      { id: "irc-options", label: "Options" }, { id: "irc-messages", label: "Messages" },
      { id: "irc-catcher", label: "Catcher" }, { id: "irc-logging", label: "Logging" },
      { id: "irc-flood", label: "Flood" },
    ]
  },
  {
    id: "sounds", label: "Sounds", children: [
      { id: "sounds-requests", label: "Requests" }, { id: "sounds-speech", label: "Speech" },
    ]
  },
  { id: "mouse", label: "Mouse", page: true, children: [{ id: "mouse-drop", label: "Drop" }] },
  {
    id: "dcc", label: "DCC", children: [
      { id: "dcc-options", label: "Options" }, { id: "dcc-folders", label: "Folders" },
      { id: "dcc-ignore", label: "Ignore" }, { id: "dcc-fserve", label: "Fserve" },
      { id: "dcc-server", label: "Server" },
    ]
  },
  {
    id: "display", label: "Display", page: true, children: [
      { id: "display-options", label: "General" },
      { id: "display-colors", label: "Colors" },
      { id: "display-rss", label: "RSS Feeds" },
    ]
  },
];

function optionsPageLabel(id) {
  for (const node of OPTIONS_TREE) {
    if (node.id === id) return node.label;
    const child = (node.children || []).find((c) => c.id === id);
    if (child) return `${node.label} > ${child.label}`;
  }
  return "";
}

function renderOptionsTree() {
  return OPTIONS_TREE.map((node) => {
    if (!node.children) {
      const selected = state.optionsPage === node.id ? " is-selected" : "";
      return `<button type="button" class="${selected.trim()}" data-page="${node.id}"><span class="tree-marker"></span>${escapeHtml(node.label)}</button>`;
    }
    const open = state.optionsOpen[node.id] !== false;
    const children = open
      ? node.children.map((child) => {
        const selected = state.optionsPage === child.id ? " is-selected" : "";
        return `<button type="button" class="tree-child${selected}" data-page="${child.id}">${escapeHtml(child.label)}</button>`;
      }).join("")
      : "";
    // A parent that is also a page: the label selects it, only the marker folds the children.
    if (node.page) {
      const selected = state.optionsPage === node.id ? ' class="is-selected"' : "";
      return `<button type="button"${selected} data-page="${node.id}"><span class="tree-marker" data-toggle="${node.id}">${open ? "⊟" : "⊞"}</span>${escapeHtml(node.label)}</button>${children}`;
    }
    return `<button type="button" data-toggle="${node.id}"><span class="tree-marker">${open ? "⊟" : "⊞"}</span>${escapeHtml(node.label)}</button>${children}`;
  }).join("");
}

function renderOptionsPage() {
  const p = pendingOptions;
  if (state.optionsPage === "connect-options") {
    return `
      <h3>Connect > Options</h3>
      <div class="options-group-plain">
        <label class="options-toggle"><input type="checkbox" data-opt="connectOnStartup" ${p.connectOnStartup ? "checked" : ""} /> Connect on startup</label>
        <label class="options-toggle"><input type="checkbox" data-opt="reconnectOnDisconnect" ${p.reconnectOnDisconnect ? "checked" : ""} /> Reconnect on disconnect</label>
        <label class="options-toggle"><input type="checkbox" data-opt="showConnectOnStartup" ${p.showConnectOnStartup ? "checked" : ""} /> Show connect on startup</label>
        <label class="options-toggle"><input type="checkbox" data-opt="moveToTopOnConnect" ${p.moveToTopOnConnect ? "checked" : ""} /> Move to top on connect</label>
        <label class="options-toggle"><input type="checkbox" data-opt="checkConnectionTimeout" ${p.checkConnectionTimeout ? "checked" : ""} /> Check connection time out</label>
        <label class="options-toggle"><input type="checkbox" data-opt="preserveNicknames" ${p.preserveNicknames ? "checked" : ""} /> Preserve nicknames</label>
        <label class="options-toggle"><input type="checkbox" data-opt="useInvisibleMode" ${p.useInvisibleMode ? "checked" : ""} /> Use invisible mode</label>
        <label class="options-toggle"><input type="checkbox" data-opt="connectDebug" ${p.connectDebug !== false ? "checked" : ""} /> Write connect debug log (logs/connect-debug.log)</label>
      </div>
      <div class="options-button-grid-2x2">
        <button type="button" class="btn btn-secondary" data-subdialog="Perform">Perform...</button>
        <button type="button" class="btn btn-secondary" data-subdialog="Retry">Retry...</button>
        <button type="button" class="btn btn-secondary" data-subdialog="SSL">SSL...</button>
        <button type="button" class="btn btn-secondary" data-subdialog="Ports">Ports...</button>
      </div>
      <div class="options-field-block">
        <label for="opt-defaultPort">Default Port:</label>
        <input id="opt-defaultPort" class="form-input form-input-sm" type="text" maxlength="5" data-opt="defaultPort" value="${escapeHtml(p.defaultPort)}" />
      </div>
      <div class="options-field-block" style="margin-top: 10px; border-top: 1px solid var(--line); padding-top: 8px;">
        <label class="options-toggle"><input type="checkbox" data-opt="pythonEnabled" ${p.pythonEnabled !== false ? "checked" : ""} /> Enable Python scripts (.py)</label>
        <div style="margin-top: 4px;">
          <label for="opt-pythonPath" style="display:block; margin-bottom: 2px; font-size: 11px;">Python Path (empty = auto-detect):</label>
          <input id="opt-pythonPath" class="form-input" style="width: 100%; box-sizing: border-box; margin-top: 2px;" type="text" placeholder="Auto (PATH or python/python.exe)" data-opt="pythonPath" value="${escapeHtml(p.pythonPath || "")}" />
        </div>
      </div>`;
  }
  if (state.optionsPage === "connect-identd") {
    return `
      <h3>Connect > Identd</h3>
      <div class="options-group-plain">
        <label class="options-toggle"><input type="checkbox" data-opt="identdEnabled" ${p.identdEnabled ? "checked" : ""} /> Enable Identd server</label>
      </div>
      <div class="options-form-grid">
        <label for="opt-identdUserId">User ID:</label>
        <input id="opt-identdUserId" class="form-input" type="text" maxlength="32" data-opt="identdUserId" value="${escapeHtml(p.identdUserId)}" />
        <label for="opt-identdSystem">System:</label>
        <input id="opt-identdSystem" class="form-input" type="text" maxlength="32" data-opt="identdSystem" value="${escapeHtml(p.identdSystem)}" />
        <label for="opt-identdPort">Port:</label>
        <input id="opt-identdPort" class="form-input form-input-sm" type="text" maxlength="5" data-opt="identdPort" value="${escapeHtml(p.identdPort)}" />
      </div>
      <div class="options-group-plain" style="margin-top: 14px;">
        <label class="options-toggle"><input type="checkbox" data-opt="identdShowRequests" ${p.identdShowRequests ? "checked" : ""} /> Show Identd requests</label>
        <label class="options-toggle"><input type="checkbox" data-opt="identdOnlyWhenConnecting" ${p.identdOnlyWhenConnecting ? "checked" : ""} /> Enable only when connecting</label>
        <label class="options-toggle"><input type="checkbox" data-opt="identdUseEmailId" ${p.identdUseEmailId ? "checked" : ""} /> Use ID from email address</label>
      </div>
      <p id="options-identd-error" class="form-error" role="alert" ${identdError ? "" : "hidden"}>${escapeHtml(identdError)}</p>`;
  }
  if (state.optionsPage === "connect-proxy") {
    return `
      <h3>Connect > Proxy</h3>
      <div class="options-form-grid">
        <label for="opt-proxyConnection">Connection:</label>
        <select id="opt-proxyConnection" class="form-input" data-opt="proxyConnection">
          <option value="None" ${p.proxyConnection === "None" ? "selected" : ""}>None</option>
          <option value="Server" ${p.proxyConnection === "Server" ? "selected" : ""}>Server</option>
          <option value="DCC" ${p.proxyConnection === "DCC" ? "selected" : ""}>DCC</option>
          <option value="Both" ${p.proxyConnection === "Both" ? "selected" : ""}>Both</option>
        </select>
        <label for="opt-proxyProtocol">Protocol:</label>
        <select id="opt-proxyProtocol" class="form-input" data-opt="proxyProtocol">
          <option value="Socks4" ${p.proxyProtocol === "Socks4" ? "selected" : ""}>Socks4</option>
          <option value="Socks5" ${p.proxyProtocol === "Socks5" ? "selected" : ""}>Socks5</option>
          <option value="Proxy" ${p.proxyProtocol === "Proxy" ? "selected" : ""}>Proxy</option>
        </select>
        <label for="opt-proxyHostname">Hostname:</label>
        <input id="opt-proxyHostname" class="form-input" type="text" maxlength="255" data-opt="proxyHostname" value="${escapeHtml(p.proxyHostname)}" />
        <label for="opt-proxyUserId">User ID:</label>
        <input id="opt-proxyUserId" class="form-input" type="text" maxlength="255" data-opt="proxyUserId" value="${escapeHtml(p.proxyUserId)}" />
        <label for="opt-proxyPassword">Password:</label>
        <div class="password-input-wrap">
          <input id="opt-proxyPassword" class="form-input" type="password" maxlength="255" data-opt="proxyPassword" value="${escapeHtml(p.proxyPassword)}" />
          <button type="button" class="btn-toggle-pw" id="btn-toggle-proxy-pw" title="Show/hide password">👁</button>
        </div>
        <label for="opt-proxyPort">Port:</label>
        <input id="opt-proxyPort" class="form-input form-input-sm" type="text" maxlength="5" data-opt="proxyPort" value="${escapeHtml(p.proxyPort)}" />
      </div>
      <div style="margin-top: 10px;">
        <label for="opt-proxyExceptionMasks" style="display:block; margin-bottom: 4px; font-size:12px;">Exception masks:</label>
        <textarea id="opt-proxyExceptionMasks" class="form-input" rows="5" style="width:100%; box-sizing:border-box; resize:vertical;" data-opt="proxyExceptionMasks">${escapeHtml(p.proxyExceptionMasks)}</textarea>
      </div>`;
  }
  if (state.optionsPage === "connect-local") {
    return `
      <h3>Connect > Local</h3>
      <fieldset class="options-section">
        <legend>On Connect:</legend>
        <label class="options-toggle"><input type="checkbox" data-opt="localOnConnectGetHost" ${p.localOnConnectGetHost ? "checked" : ""} /> Get host name</label>
        <label class="options-toggle"><input type="checkbox" data-opt="localOnConnectGetIp" ${p.localOnConnectGetIp ? "checked" : ""} /> Get IP address</label>
      </fieldset>
      <fieldset class="options-section">
        <legend>Lookup Method:</legend>
        <label class="options-toggle"><input type="radio" name="opt-localLookupMethod" value="normal" ${p.localLookupMethod === "normal" ? "checked" : ""} data-opt="localLookupMethod" /> Normal</label>
        <label class="options-toggle"><input type="radio" name="opt-localLookupMethod" value="server" ${p.localLookupMethod === "server" ? "checked" : ""} data-opt="localLookupMethod" /> Server</label>
        <label class="options-toggle"><input type="radio" name="opt-localLookupMethod" value="website" ${p.localLookupMethod === "website" ? "checked" : ""} data-opt="localLookupMethod" /> Website:</label>
        <input id="opt-localLookupWebsite" class="form-input" type="text" data-opt="localLookupWebsite" value="${escapeHtml(p.localLookupWebsite)}" style="margin-left: 22px; width: calc(100% - 22px); box-sizing: border-box;" />
      </fieldset>
      <div class="options-field-block" style="margin-top: 10px;">
        <label for="opt-localHostName">Host name:</label>
        <input id="opt-localHostName" class="form-input" type="text" data-opt="localHostName" value="${escapeHtml(p.localHostName)}" />
      </div>
      <div class="options-field-block" style="margin-top: 8px;">
        <label for="opt-localIpAddress">IP address:</label>
        <input id="opt-localIpAddress" class="form-input" type="text" data-opt="localIpAddress" value="${escapeHtml(p.localIpAddress)}" />
      </div>`;
  }
  if (state.optionsPage === "irc") {
    return `
      <h3>IRC</h3>
      <div class="options-scroll-box" style="height: 230px;">
        <div class="options-scroll-section">
          <div class="options-scroll-title">Prefix:</div>
          <label class="options-toggle"><input type="checkbox" data-opt="prefixOwnMessages" ${p.prefixOwnMessages ? "checked" : ""} /> Own messages</label>
          <label class="options-toggle"><input type="checkbox" data-opt="prefixShowModes" ${p.prefixShowModes ? "checked" : ""} /> Show modes</label>
        </div>
        <div class="options-scroll-section">
          <div class="options-scroll-title">Query:</div>
          <label class="options-toggle"><input type="checkbox" data-opt="queryMinimize" ${p.queryMinimize ? "checked" : ""} /> Minimize window</label>
          <label class="options-toggle"><input type="checkbox" data-opt="queryNotifyNicks" ${p.queryNotifyNicks ? "checked" : ""} /> Notify nicknames</label>
          <label class="options-toggle"><input type="checkbox" data-opt="queryWhoisOnOpen" ${p.queryWhoisOnOpen ? "checked" : ""} /> Whois on open</label>
          <label class="options-toggle"><input type="checkbox" data-opt="queryAutoKeyx" ${p.queryAutoKeyx !== false ? "checked" : ""} /> DH1080 key exchange on open</label>
          <label class="options-toggle"><input type="checkbox" data-opt="querySingleWindow" ${p.querySingleWindow ? "checked" : ""} /> Single message window</label>
          <label class="options-toggle"><input type="checkbox" data-opt="queryCopyMessages" ${p.queryCopyMessages ? "checked" : ""} /> Copy messages</label>
        </div>
        <div class="options-scroll-section">
          <div class="options-scroll-title">Channel:</div>
          <label class="options-toggle"><input type="checkbox" data-opt="channelAutoJoinOnInvite" ${p.channelAutoJoinOnInvite ? "checked" : ""} /> Auto-join on invite</label>
          <label class="options-toggle"><input type="checkbox" data-opt="channelRejoinWhenKicked" ${p.channelRejoinWhenKicked ? "checked" : ""} /> Rejoin when kicked</label>
          <label class="options-toggle"><input type="checkbox" data-opt="channelRejoinOnConnect" ${p.channelRejoinOnConnect ? "checked" : ""} /> Rejoin on connect</label>
          <label class="options-toggle"><input type="checkbox" data-opt="channelKeepOpenOnKickQuit" ${p.channelKeepOpenOnKickQuit ? "checked" : ""} /> Keep open on kick/quit</label>
          <label class="options-toggle"><input type="checkbox" data-opt="channelKeepOpenOnPart" ${p.channelKeepOpenOnPart ? "checked" : ""} /> Keep open on part</label>
        </div>
      </div>
      <fieldset class="options-section" style="margin-top: 10px;">
        <legend>Show in active:</legend>
        <div class="options-grid-2">
          <div>
            <label class="options-toggle"><input type="checkbox" data-opt="showActiveAway" ${p.showActiveAway ? "checked" : ""} /> Away</label>
            <label class="options-toggle"><input type="checkbox" data-opt="showActiveCtcp" ${p.showActiveCtcp ? "checked" : ""} /> CTCP</label>
            <label class="options-toggle"><input type="checkbox" data-opt="showActiveInvite" ${p.showActiveInvite ? "checked" : ""} /> Invite</label>
          </div>
          <div>
            <label class="options-toggle"><input type="checkbox" data-opt="showActiveNotice" ${p.showActiveNotice ? "checked" : ""} /> Notice</label>
            <label class="options-toggle"><input type="checkbox" data-opt="showActiveQuery" ${p.showActiveQuery ? "checked" : ""} /> Query</label>
            <label class="options-toggle"><input type="checkbox" data-opt="showActiveWhois" ${p.showActiveWhois ? "checked" : ""} /> Whois</label>
          </div>
        </div>
      </fieldset>`;
  }
  if (state.optionsPage === "irc-options") {
    return `
      <h3>IRC > Options</h3>
      <fieldset class="options-section">
        <legend>Show:</legend>
        <label class="options-toggle"><input type="checkbox" data-opt="showShortJoinsParts" ${p.showShortJoinsParts ? "checked" : ""} /> Short joins and parts</label>
        <label class="options-toggle"><input type="checkbox" data-opt="showUserAddresses" ${p.showUserAddresses ? "checked" : ""} /> User addresses</label>
        <label class="options-toggle"><input type="checkbox" data-opt="showNicknamesOnJoin" ${p.showNicknamesOnJoin ? "checked" : ""} /> Nicknames on join</label>
      </fieldset>
      <fieldset class="options-section">
        <legend>Flash on message:</legend>
        <div class="options-grid-2">
          <div>
            <label class="options-toggle"><input type="checkbox" data-opt="flashChannel" ${p.flashChannel ? "checked" : ""} /> Channel</label>
            <label class="options-toggle"><input type="checkbox" data-opt="flashChat" ${p.flashChat ? "checked" : ""} /> Chat</label>
          </div>
          <div>
            <label class="options-toggle"><input type="checkbox" data-opt="flashQuery" ${p.flashQuery ? "checked" : ""} /> Query</label>
          </div>
        </div>
      </fieldset>
      <fieldset class="options-section">
        <legend>Other:</legend>
        <label class="options-toggle"><input type="checkbox" data-opt="skipMotdOnConnect" ${p.skipMotdOnConnect ? "checked" : ""} /> Skip MOTD on connect</label>
        <label class="options-toggle"><input type="checkbox" data-opt="hidePingPong" ${p.hidePingPong ? "checked" : ""} /> Hide ping? pong! event</label>
        <label class="options-toggle"><input type="checkbox" data-opt="cancelAwayOnActivity" ${p.cancelAwayOnActivity ? "checked" : ""} /> Cancel away on activity</label>
        <label class="options-toggle"><input type="checkbox" data-opt="sendTypingNotifications" ${p.sendTypingNotifications ? "checked" : ""} /> Send typing notifications</label>
        <label class="options-toggle"><input type="checkbox" data-opt="hideAwayReminders" ${p.hideAwayReminders ? "checked" : ""} /> Hide away reminders</label>
      </fieldset>
      <div style="margin-top: 10px;">
        <button type="button" class="btn btn-secondary" data-subdialog="Events">Events...</button>
      </div>`;
  }
  if (state.optionsPage === "irc-messages") {
    return `
      <h3>IRC > Messages</h3>
      <div class="options-group-plain">
        <div class="options-inline-field">
          <label class="options-toggle"><input type="checkbox" data-opt="timestampEvents" ${p.timestampEvents ? "checked" : ""} /> Timestamp events:</label>
        </div>
        <input class="form-input" style="width: 100%; box-sizing: border-box; font-family: monospace;" type="text" data-opt="timestampFormat" value="${escapeHtml(p.timestampFormat)}" />
        <div style="font-size: 11px; color: var(--dim); margin-top: 3px;">Tokens: HH hh nn ss, and ms for milliseconds (3 digits), e.g. [HH:nn:ss.ms]</div>
        <div class="options-inline-field" style="margin-top: 8px;">
          <label for="opt-scrollbackLines">Scrollback lines (open buffer, 200 to 2000):</label>
          <input id="opt-scrollbackLines" class="form-input form-input-sm" type="number" min="200" max="2000" step="100" data-opt="scrollbackLines" value="${escapeHtml(String(p.scrollbackLines))}" style="width: 110px;" />
        </div>
      </div>
      <div class="options-group-plain" style="margin-top: 12px;">
        <div style="font-size: 12px; margin-bottom: 4px;">Strip codes from incoming messages:</div>
        <label class="options-toggle"><input type="checkbox" data-opt="stripColor" ${p.stripColor ? "checked" : ""} /> Color</label>
        <label class="options-toggle"><input type="checkbox" data-opt="stripOther" ${p.stripOther ? "checked" : ""} /> Other</label>
        <div class="options-inline-field">
          <label class="options-toggle"><input type="checkbox" data-opt="stripIfMoreThan" ${p.stripIfMoreThan ? "checked" : ""} /> If more than:</label>
          <input class="form-input form-input-sm" style="width: 60px; margin-left: 8px;" type="text" maxlength="4" data-opt="stripThreshold" value="${escapeHtml(p.stripThreshold)}" />
        </div>
      </div>
      <div class="options-field-block" style="margin-top: 12px;">
        <label for="opt-ctcpFingerReply">CTCP finger reply:</label>
        <input id="opt-ctcpFingerReply" class="form-input" type="text" data-opt="ctcpFingerReply" value="${escapeHtml(p.ctcpFingerReply)}" />
      </div>
      <div class="options-field-block" style="margin-top: 8px;">
        <label for="opt-quitMessage">Quit message:</label>
        <input id="opt-quitMessage" class="form-input" type="text" data-opt="quitMessage" value="${escapeHtml(p.quitMessage)}" />
      </div>
      <div class="options-group-plain" style="margin-top: 14px;">
        <label class="options-toggle"><input type="checkbox" data-opt="splitLongMessages" ${p.splitLongMessages ? "checked" : ""} /> Split long channel/query messages</label>
        <label class="options-toggle"><input type="checkbox" data-opt="utf8EncodeDecode" ${p.utf8EncodeDecode ? "checked" : ""} /> UTF-8 encode/decode messages</label>
      </div>`;
  }
  if (state.optionsPage === "irc-catcher") {
    return `
      <h3>IRC > Catcher</h3>
      <fieldset class="options-section">
        <legend>Enable catching for:</legend>
        <div class="options-grid-2">
          <label class="options-toggle"><input type="checkbox" data-opt="catchUrls" ${p.catchUrls ? "checked" : ""} /> URLs</label>
          <label class="options-toggle"><input type="checkbox" data-opt="catchEmails" ${p.catchEmails ? "checked" : ""} /> Emails</label>
        </div>
      </fieldset>
      <fieldset class="options-section">
        <legend>Chat links:</legend>
        <label class="options-toggle"><input type="checkbox" data-opt="chatLinkSupport" ${p.chatLinkSupport ? "checked" : ""} /> Enable chat link support</label>
        <label class="options-toggle"><input type="checkbox" data-opt="chatLinkConfirmRequests" ${p.chatLinkConfirmRequests ? "checked" : ""} /> Confirm requests</label>
      </fieldset>
      <fieldset class="options-section">
        <legend>Unmarked items:</legend>
        <label class="options-toggle"><input type="checkbox" data-opt="unmarkedPlaceTop" ${p.unmarkedPlaceTop ? "checked" : ""} /> Place at top of list</label>
        <label class="options-toggle"><input type="checkbox" data-opt="unmarkedDeleteOnExit" ${p.unmarkedDeleteOnExit ? "checked" : ""} /> Delete on exit</label>
      </fieldset>
      <fieldset class="options-section">
        <legend>On Send:</legend>
        <label class="options-toggle"><input type="checkbox" data-opt="onSendDescription" ${p.onSendDescription ? "checked" : ""} /> Send description</label>
      </fieldset>
      <div style="margin-top: 10px;">
        <label class="options-toggle"><input type="checkbox" data-opt="warnOpenUrl" ${p.warnOpenUrl ? "checked" : ""} /> Show warning when opening a URL</label>
      </div>`;
  }
  if (state.optionsPage === "irc-logging") {
    return `
      <h3>IRC > Logging</h3>
      <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
        <span style="font-size: 12px;">Automatic log/reload:</span>
        <button type="button" class="btn btn-secondary" data-subdialog="LoggingOptions">Options...</button>
      </div>
      <div class="options-group-plain">
        <label class="options-toggle"><input type="checkbox" data-opt="logEnabled" ${p.logEnabled ? "checked" : ""} /> Enable chat logging</label>
        <label class="options-toggle"><input type="checkbox" data-opt="logChannel" ${p.logChannel ? "checked" : ""} /> Log channels</label>
        <label class="options-toggle"><input type="checkbox" data-opt="logQuery" ${p.logQuery ? "checked" : ""} /> Log queries</label>
        <label class="options-toggle"><input type="checkbox" data-opt="logStatus" ${p.logStatus ? "checked" : ""} /> Log status</label>
        <label class="options-toggle"><input type="checkbox" data-opt="logLockFiles" ${p.logLockFiles ? "checked" : ""} /> Lock log files</label>
        <label class="options-toggle"><input type="checkbox" data-opt="logStripCodes" ${p.logStripCodes ? "checked" : ""} /> Strip codes</label>
        <label class="options-toggle"><input type="checkbox" data-opt="logLineColors" ${p.logLineColors ? "checked" : ""} /> Line colors</label>
      </div>
      <div class="options-group-plain" style="margin-top: 6px;">
        <div class="options-inline-field">
          <label class="options-toggle"><input type="checkbox" data-opt="logTrim" ${p.logTrim ? "checked" : ""} /> Trim log files to:</label>
          <input class="form-input form-input-sm" style="width: 100px; margin: 0 6px;" type="text" data-opt="logTrimSize" value="${escapeHtml(p.logTrimSize)}" />
          <span style="font-size: 12px;">KB</span>
        </div>
        <div class="options-inline-field">
          <label class="options-toggle"><input type="checkbox" data-opt="logTimestamp" ${p.logTimestamp ? "checked" : ""} /> Timestamp logs:</label>
          <input class="form-input" style="width: 170px; margin-left: 6px; font-family: monospace;" type="text" data-opt="logTimestampFormat" value="${escapeHtml(p.logTimestampFormat)}" />
        </div>
        <div class="options-inline-field">
          <label class="options-toggle"><input type="checkbox" data-opt="logDateFilenames" ${p.logDateFilenames ? "checked" : ""} /> Date filenames:</label>
          <select class="form-input" style="width: 170px; margin-left: 6px;" data-opt="logDatePeriod">
            <option value="None" ${p.logDatePeriod === "None" ? "selected" : ""}>None</option>
            <option value="By Day" ${p.logDatePeriod === "By Day" ? "selected" : ""}>By Day</option>
            <option value="By Week" ${p.logDatePeriod === "By Week" ? "selected" : ""}>By Week</option>
            <option value="By Month" ${p.logDatePeriod === "By Month" ? "selected" : ""}>By Month</option>
            <option value="By Year" ${p.logDatePeriod === "By Year" ? "selected" : ""}>By Year</option>
          </select>
        </div>
        <div class="options-sub-item">
          <label class="options-toggle"><input type="checkbox" data-opt="logDateExceptStatus" ${p.logDateExceptStatus ? "checked" : ""} /> Except status windows</label>
        </div>
        <div>
          <label class="options-toggle"><input type="checkbox" data-opt="logIncludeNetwork" ${p.logIncludeNetwork ? "checked" : ""} /> Include network</label>
          <div class="options-sub-item">
            <label class="options-toggle"><input type="checkbox" data-opt="logMakeFolder" ${p.logMakeFolder ? "checked" : ""} /> Make folder</label>
          </div>
        </div>
      </div>
      <div class="options-field-block" style="margin-top: 10px;">
        <label for="opt-logFolder">Logs folder:</label>
        <input id="opt-logFolder" class="form-input" type="text" data-opt="logFolder" value="${escapeHtml(p.logFolder)}" />
      </div>`;
  }
  if (state.optionsPage === "irc-flood") {
    return `
      <h3>IRC > Flood</h3>
      <div class="options-group-plain">
        <label class="options-toggle"><input type="checkbox" data-opt="floodEnable" ${p.floodEnable ? "checked" : ""} /> Enable flood protection</label>
        <label class="options-toggle"><input type="checkbox" data-opt="floodShowStatusActive" ${p.floodShowStatusActive ? "checked" : ""} /> Show status in active window</label>
      </div>
      <div class="options-form-grid" style="grid-template-columns: 140px 80px 1fr; align-items: center; margin-top: 10px;">
        <label>Trigger check after:</label>
        <input class="form-input form-input-sm" type="text" data-opt="floodTriggerBytes" value="${escapeHtml(p.floodTriggerBytes)}" />
        <span>Bytes</span>
        <label>Buffer size:</label>
        <input class="form-input form-input-sm" type="text" data-opt="floodBufferSize" value="${escapeHtml(p.floodBufferSize)}" />
        <span>Lines</span>
        <label>Lines per user:</label>
        <input class="form-input form-input-sm" type="text" data-opt="floodLinesPerUser" value="${escapeHtml(p.floodLinesPerUser)}" />
        <span>Lines</span>
        <label>Ignore user for:</label>
        <input class="form-input form-input-sm" type="text" data-opt="floodIgnoreSeconds" value="${escapeHtml(p.floodIgnoreSeconds)}" />
        <span>Seconds</span>
      </div>
      <div class="options-group-plain" style="margin-top: 12px;">
        <label class="options-toggle"><input type="checkbox" data-opt="floodLimitCtcp" ${p.floodLimitCtcp ? "checked" : ""} /> Limit CTCP replies</label>
        <label class="options-toggle"><input type="checkbox" data-opt="floodLimitWhois" ${p.floodLimitWhois ? "checked" : ""} /> Limit Whois requests</label>
        <label class="options-toggle"><input type="checkbox" data-opt="floodLimitQuery" ${p.floodLimitQuery ? "checked" : ""} /> Limit Query windows</label>
      </div>
      <div class="options-group-plain" style="margin-top: 10px;">
        <label class="options-toggle"><input type="checkbox" data-opt="floodQueueOp" ${p.floodQueueOp ? "checked" : ""} /> Queue Op commands</label>
        <label class="options-toggle"><input type="checkbox" data-opt="floodQueueOwn" ${p.floodQueueOwn ? "checked" : ""} /> Queue Own messages</label>
      </div>`;
  }
  if (state.optionsPage === "display" || state.optionsPage === "display-options") {
    return `
      <h3>Display</h3>
      <fieldset class="options-section">
        <label class="options-toggle"><input id="options-follow-system" type="checkbox" ${followSystemTheme ? "checked" : ""} /> Follow Windows dark mode</label>
        <label class="options-toggle"><input id="options-dark-mode" type="checkbox" ${activeTheme === "dark" ? "checked" : ""} ${followSystemTheme ? "disabled" : ""} /> Dark mode</label>
        <label class="options-toggle"><input id="options-show-nicklist" type="checkbox" ${state.showNicklist ? "checked" : ""} /> Show nick list</label>
        <label class="options-toggle"><input id="options-show-switchbar" type="checkbox" ${state.showSwitchbar ? "checked" : ""} /> Show switchbar</label>
      </fieldset>`;
  }
  if (state.optionsPage === "display-colors") {
    return `
      <h3>Display > Colors</h3>
      <fieldset class="options-section">
        <legend>Color Schemes</legend>
        <p style="font-size:12px; margin-bottom:12px;">Configure colors for chat messages, events, nickname list, treebar, and editbox.</p>
        <div style="display:flex; align-items:center; gap:10px; margin-bottom:14px;">
          <button type="button" class="btn btn-primary" id="btn-open-colors-modal">Open Colors Dialog (Alt+K)…</button>
        </div>
        <div style="font-size:11px; color:var(--dim);">Active Scheme: <strong id="lbl-active-scheme">${escapeHtml(getActiveSchemeName())}</strong></div>
      </fieldset>`;
  }
  if (state.optionsPage === "display-rss") {
    return `
      <h3>Display > RSS Feeds</h3>
      <fieldset class="options-section">
        <legend>Toolbar News Ticker</legend>
        <label class="options-toggle">
          <input type="checkbox" data-opt="rssEnabled" ${p.rssEnabled ? "checked" : ""} />
          Enable RSS news ticker in toolbar
        </label>
        <div style="margin: 10px 0 4px; font-size: 11px; color: var(--dim);">
          Feed URLs (RSS 2.0 or Atom, one URL per line):
        </div>
        <textarea data-opt="rssUrls" class="form-input" rows="5" spellcheck="false" style="width: 100%; box-sizing: border-box; font-family: var(--font-mono); font-size: 11px; resize: vertical;">${escapeHtml(p.rssUrls || "")}</textarea>
        <div class="options-form-grid" style="grid-template-columns: 120px 1fr; margin-top: 10px;">
          <label for="opt-rssCount">Headlines per set:</label>
          <select id="opt-rssCount" data-opt="rssCount" class="form-input form-input-sm" style="width: 140px;">
            ${[1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((n) => `<option value="${n}" ${String(p.rssCount || "3") === String(n) ? "selected" : ""}>${n}</option>`).join("")}
          </select>

          <label for="opt-rssMode">Ticker:</label>
          <select id="opt-rssMode" data-opt="rssMode" class="form-input form-input-sm" style="width: 140px;">
            <option value="fixed" ${p.rssMode !== "scroll" ? "selected" : ""}>Fixed set</option>
            <option value="scroll" ${p.rssMode === "scroll" ? "selected" : ""}>Scrolling</option>
          </select>

          <label for="opt-rssInterval">Refresh interval:</label>
          <select id="opt-rssInterval" data-opt="rssInterval" class="form-input form-input-sm" style="width: 140px;">
            <option value="5" ${p.rssInterval === "5" ? "selected" : ""}>5 Minutes</option>
            <option value="15" ${p.rssInterval === "15" || !p.rssInterval ? "selected" : ""}>15 Minutes</option>
            <option value="30" ${p.rssInterval === "30" ? "selected" : ""}>30 Minutes</option>
            <option value="60" ${p.rssInterval === "60" ? "selected" : ""}>60 Minutes</option>
          </select>
        </div>
        <div style="margin-top: 14px; display: flex; align-items: center; gap: 8px;">
          <button type="button" class="btn btn-secondary" id="btn-rss-fetch-now">Refresh Feeds Now</button>
          <span id="rss-options-status" style="font-size: 11px; color: var(--dim);">${escapeHtml(rssManager.statusText || "")}</span>
        </div>
      </fieldset>`;
  }
  if (state.optionsPage === "mouse") {
    const row = (spec) => `
        <label class="options-field">
          <span>${escapeHtml(spec.label)}:</span>
          <input class="form-input" type="text" maxlength="200" spellcheck="false" data-mouse-target="${spec.id}" value="${escapeHtml(readMouseDblclick(spec.id))}" />
          ${spec.used ? "" : '<span class="options-field-note">not used by Ferric yet</span>'}
        </label>`;
    const byId = (id) => MOUSE_DBLCLICK_TARGETS.find((t) => t.id === id);
    return `
      <h3>Mouse</h3>
      <fieldset class="options-section">
        <legend>Command to perform when double-clicking in a window:</legend>
        ${["status", "query", "channel", "message"].map((id) => row(byId(id))).join("")}
      </fieldset>
      <fieldset class="options-section">
        <legend>Command to perform when double-clicking on a nickname:</legend>
        ${["nicklist", "notify"].map((id) => row(byId(id))).join("")}
      </fieldset>
      <p class="options-page-empty">Ferric supports query, whois, join and part with one argument: $nick, $chan or $network. Script identifiers such as $$1 are not supported here yet.</p>`;
  }
  if (state.optionsPage === "mouse-drop") {
    return `
      <h3>Drop</h3>
      <fieldset class="options-section">
        <legend>Drop command rules</legend>
        <label class="options-toggle"><input type="radio" name="drop-mode" value="normal" ${selectedDropRules === "normal" ? "checked" : ""} /> No modifier</label>
        <label class="options-toggle"><input type="radio" name="drop-mode" value="shift" ${selectedDropRules === "shift" ? "checked" : ""} /> Shift</label>
        <textarea id="options-drop-rules" class="form-input" rows="6" spellcheck="false" aria-label="Drop command rules">${escapeHtml(readDropRules(selectedDropRules))}</textarea>
      </fieldset>
      <p class="options-page-empty">Rules are saved, but file-drop commands are not supported yet.</p>`;
  }
  return `<h3>${escapeHtml(optionsPageLabel(state.optionsPage))}</h3><p class="options-page-empty">Not available in Ferric yet.</p>`;
}

function openOptionsModal() {
  if (isServerManagerOpen()) closeServerManager();
  pendingOptions = { ...appOptions };
  optionsSubdialog = null;
  if (!state.optionsPage) {
    state.optionsPage = "connect-options";
  }
  dismissSlotDialogs();
  state.showOptionsModal = true;
  state.showNotesModal = false;
  state.showConnectModal = false;
  state.showFishModal = false;
  renderModal();
}

function closeOptionsModal() {
  state.showOptionsModal = false;
  optionsSubdialog = null;
  renderModal();
}

function renderOptionsSubmodal() {
  if (!optionsSubdialog) return "";
  const p = pendingOptions;
  if (optionsSubdialog === "Perform") {
    return `
      <div class="options-submodal-overlay" id="options-submodal-backdrop">
        <div class="options-submodal-card" role="dialog" aria-modal="true" aria-labelledby="submodal-title">
          <div class="options-submodal-header" id="submodal-title">Perform On Connect</div>
          <div class="options-submodal-body">
            <div style="margin-bottom:8px;">
              <label class="options-toggle"><input type="checkbox" id="sub-performEnabled" ${p.performEnabled !== false ? "checked" : ""} /> Enable perform on connect</label>
            </div>
            <div style="margin-bottom:4px; font-size:11px; color:var(--dim);">Commands to execute when connected:</div>
            <textarea id="sub-performCommands" class="form-input" rows="6" style="width:100%; box-sizing:border-box;">${escapeHtml(p.performCommands || "")}</textarea>
          </div>
          <div class="options-submodal-actions">
            <button type="button" class="btn btn-primary" id="submodal-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="submodal-cancel">Cancel</button>
          </div>
        </div>
      </div>`;
  }
  if (optionsSubdialog === "Retry") {
    return `
      <div class="options-submodal-overlay" id="options-submodal-backdrop">
        <div class="options-submodal-card" role="dialog" aria-modal="true" aria-labelledby="submodal-title">
          <div class="options-submodal-header" id="submodal-title">Connection Retry</div>
          <div class="options-submodal-body">
            <div class="options-form-grid" style="grid-template-columns: 100px 1fr;">
              <label for="sub-retryCount">Retry count:</label>
              <input id="sub-retryCount" class="form-input form-input-sm" type="text" value="${escapeHtml(p.retryCount || "10")}" />
              <label for="sub-retryDelay">Delay (seconds):</label>
              <input id="sub-retryDelay" class="form-input form-input-sm" type="text" value="${escapeHtml(p.retryDelay || "30")}" />
            </div>
          </div>
          <div class="options-submodal-actions">
            <button type="button" class="btn btn-primary" id="submodal-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="submodal-cancel">Cancel</button>
          </div>
        </div>
      </div>`;
  }
  if (optionsSubdialog === "SSL") {
    return `
      <div class="options-submodal-overlay" id="options-submodal-backdrop">
        <div class="options-submodal-card" role="dialog" aria-modal="true" aria-labelledby="submodal-title">
          <div class="options-submodal-header" id="submodal-title">SSL Configuration</div>
          <div class="options-submodal-body">
            <div class="options-group-plain">
              <label class="options-toggle"><input type="checkbox" id="sub-sslRejectInvalid" ${p.sslRejectInvalid ? "checked" : ""} /> Reject invalid certificates</label>
              <label class="options-toggle"><input type="checkbox" id="sub-sslAllowSelfSigned" ${p.sslAllowSelfSigned !== false ? "checked" : ""} /> Allow self-signed certificates</label><div style="font-size:12px; opacity:0.7; margin-top:4px;">On: the certificate is still checked (validity period, host name, structure), but a self-signed one is accepted instead of requiring a public authority. Its SHA-256 is shown in the status window; pin the fingerprint in the connect dialog to refuse any other certificate. Expired certificates are still refused. Off: only a pinned fingerprint is accepted, or a certificate from a public authority.</div>
            </div>
          </div>
          <div class="options-submodal-actions">
            <button type="button" class="btn btn-primary" id="submodal-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="submodal-cancel">Cancel</button>
          </div>
        </div>
      </div>`;
  }
  if (optionsSubdialog === "Ports") {
    return `
      <div class="options-submodal-overlay" id="options-submodal-backdrop">
        <div class="options-submodal-card" role="dialog" aria-modal="true" aria-labelledby="submodal-title">
          <div class="options-submodal-header" id="submodal-title">Port Configuration</div>
          <div class="options-submodal-body">
            <div style="font-size:12px; margin-bottom:4px;">Default Port:</div>
            <input id="sub-defaultPort" class="form-input" type="text" value="${escapeHtml(p.defaultPort || "6667")}" />
            <div style="font-size:11px; color:var(--dim); margin-top:6px;">Standard ports: 6667 (plain text), 6697 (SSL).</div>
          </div>
          <div class="options-submodal-actions">
            <button type="button" class="btn btn-primary" id="submodal-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="submodal-cancel">Cancel</button>
          </div>
        </div>
      </div>`;
  }
  if (optionsSubdialog === "Events") {
    return `
      <div class="options-submodal-overlay" id="options-submodal-backdrop">
        <div class="options-submodal-card" role="dialog" aria-modal="true" aria-labelledby="submodal-title">
          <div class="options-submodal-header" id="submodal-title">IRC Events</div>
          <div class="options-submodal-body">
            <div class="options-group-plain">
              <label class="options-toggle"><input type="checkbox" id="sub-eventsSound" ${p.eventsSound ? "checked" : ""} /> Play sound for query messages</label>
              <label class="options-toggle"><input type="checkbox" id="sub-eventsFlash" ${p.eventsFlash ? "checked" : ""} /> Flash icon on activity</label>
            </div>
          </div>
          <div class="options-submodal-actions">
            <button type="button" class="btn btn-primary" id="submodal-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="submodal-cancel">Cancel</button>
          </div>
        </div>
      </div>`;
  }
  if (optionsSubdialog === "LoggingOptions") {
    return `
      <div class="options-submodal-overlay" id="options-submodal-backdrop">
        <div class="options-submodal-card" role="dialog" aria-modal="true" aria-labelledby="submodal-title">
          <div class="options-submodal-header" id="submodal-title">Automatic Log / Reload Options</div>
          <div class="options-submodal-body">
            <div style="font-weight:600; margin-bottom:6px;">Log:</div>
            <div class="options-group-plain">
              <label class="options-toggle"><input type="checkbox" id="sub-logChannel" ${p.logChannel !== false ? "checked" : ""} /> Channel</label>
              <label class="options-toggle"><input type="checkbox" id="sub-logQuery" ${p.logQuery !== false ? "checked" : ""} /> Chat (private messages)</label>
              <label class="options-toggle"><input type="checkbox" id="sub-logStatus" ${p.logStatus !== false ? "checked" : ""} /> Status</label>
            </div>
            <div style="font-weight:600; margin: 10px 0 6px;">Reload:</div>
            <label class="options-toggle"><input type="checkbox" id="sub-reloadLogsOnOpen" ${p.reloadLogsOnOpen ? "checked" : ""} /> Reload log files on buffer open</label>
            <div class="options-inline-field" style="margin-top:6px;">
              <span>Lines to reload:</span>
              <input id="sub-reloadLines" class="form-input form-input-sm" type="text" value="${escapeHtml(p.reloadLines || "100")}" />
            </div>
          </div>
          <div class="options-submodal-actions">
            <button type="button" class="btn btn-primary" id="submodal-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="submodal-cancel">Cancel</button>
          </div>
        </div>
      </div>`;
  }
  return "";
}

function renderOptionsModal() {
  const container = document.getElementById("modal-container");
  if (!container) return;

  container.innerHTML = `
    <div class="modal-overlay" id="options-modal-backdrop">
      <div class="modal-card options-dialog" role="dialog" aria-modal="true" aria-labelledby="options-title">
        <div class="modal-title" id="options-title">Options</div>
        <div class="options-body">
          <aside class="options-sidebar">
            <div class="options-tree-heading">Category:</div>
            <nav class="options-tree" aria-label="Option categories">${renderOptionsTree()}</nav>
          </aside>
          <div class="options-page">${renderOptionsPage()}</div>
        </div>
        <div class="options-dialog-actions">
          <button id="options-ok" class="btn btn-primary" type="button">OK</button>
          <button id="options-cancel" class="btn btn-secondary" type="button">Cancel</button>
          <button id="options-help" class="btn btn-secondary" type="button">Help</button>
        </div>
      </div>
      ${renderOptionsSubmodal()}
    </div>
  `;

  // Bind subdialog buttons
  container.querySelectorAll("[data-subdialog]").forEach((btn) => {
    btn.addEventListener("click", () => {
      optionsSubdialog = btn.dataset.subdialog;
      renderOptionsModal();
    });
  });

  // Bind submodal actions
  const subBackdrop = document.getElementById("options-submodal-backdrop");
  if (subBackdrop) {
    subBackdrop.addEventListener("click", (e) => {
      if (e.target === subBackdrop) {
        optionsSubdialog = null;
        renderOptionsModal();
      }
    });
  }
  document.getElementById("submodal-cancel")?.addEventListener("click", () => {
    optionsSubdialog = null;
    renderOptionsModal();
  });
  document.getElementById("submodal-ok")?.addEventListener("click", () => {
    if (optionsSubdialog === "Perform") {
      pendingOptions.performEnabled = document.getElementById("sub-performEnabled")?.checked ?? true;
      pendingOptions.performCommands = document.getElementById("sub-performCommands")?.value ?? "";
    } else if (optionsSubdialog === "Retry") {
      pendingOptions.retryCount = document.getElementById("sub-retryCount")?.value ?? "10";
      pendingOptions.retryDelay = document.getElementById("sub-retryDelay")?.value ?? "30";
    } else if (optionsSubdialog === "SSL") {
      pendingOptions.sslRejectInvalid = document.getElementById("sub-sslRejectInvalid")?.checked ?? false;
      pendingOptions.sslAllowSelfSigned = document.getElementById("sub-sslAllowSelfSigned")?.checked ?? true;
    } else if (optionsSubdialog === "Ports") {
      pendingOptions.defaultPort = document.getElementById("sub-defaultPort")?.value ?? "6667";
    } else if (optionsSubdialog === "Events") {
      pendingOptions.eventsSound = document.getElementById("sub-eventsSound")?.checked ?? false;
      pendingOptions.eventsFlash = document.getElementById("sub-eventsFlash")?.checked ?? false;
    } else if (optionsSubdialog === "LoggingOptions") {
      pendingOptions.logChannel = document.getElementById("sub-logChannel")?.checked ?? true;
      pendingOptions.logQuery = document.getElementById("sub-logQuery")?.checked ?? true;
      pendingOptions.logStatus = document.getElementById("sub-logStatus")?.checked ?? true;
      pendingOptions.reloadLogsOnOpen = document.getElementById("sub-reloadLogsOnOpen")?.checked ?? false;
      pendingOptions.reloadLines = document.getElementById("sub-reloadLines")?.value ?? "100";
    }
    optionsSubdialog = null;
    renderOptionsModal();
  });

  // Bind OK, Cancel, Help
  document.getElementById("options-ok")?.addEventListener("click", () => {
    saveAppOptions(pendingOptions);
    closeOptionsModal();
  });
  document.getElementById("options-cancel")?.addEventListener("click", () => {
    pendingOptions = { ...appOptions };
    closeOptionsModal();
  });
  document.getElementById("options-help")?.addEventListener("click", () => {
    // Show brief help note in the dialog or status
    const pageName = optionsPageLabel(state.optionsPage);
    window.alert(`Options: ${pageName}\nSettings configured here match the built-in defaults and persist across sessions.`);
  });

  // Bind all data-opt inputs to pendingOptions
  container.querySelectorAll("[data-opt]").forEach((el) => {
    const key = el.dataset.opt;
    const update = () => {
      if (el.type === "checkbox") {
        pendingOptions[key] = el.checked;
      } else if (el.type === "radio") {
        if (el.checked) pendingOptions[key] = el.value;
      } else {
        pendingOptions[key] = el.value;
      }
    };
    el.addEventListener("input", update);
    el.addEventListener("change", update);
    // The timestamp format takes color codes too, so Ctrl+K opens the color index here as well.
    if (key === "timestampFormat") {
      el.addEventListener("keydown", (e) => {
        if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.code === "KeyK") {
          e.preventDefault();
          openChatColorPopup(el);
        }
      });
    }
  });

  // Password visibility toggle
  const pwToggle = container.querySelector("#btn-toggle-proxy-pw");
  const pwInput = container.querySelector("#opt-proxyPassword");
  if (pwToggle && pwInput) {
    pwToggle.addEventListener("click", () => {
      pwInput.type = pwInput.type === "password" ? "text" : "password";
    });
  }

  // Display page hooks
  container.querySelector("#btn-open-colors-modal")?.addEventListener("click", () => {
    closeOptionsModal();
    openColorsModal();
  });
  document.getElementById("options-dark-mode")?.addEventListener("change", (event) => {
    applyTheme(event.target.checked ? "dark" : "light");
  });
  document.getElementById("options-follow-system")?.addEventListener("change", (event) => {
    followSystemTheme = event.target.checked;
    try {
      localStorage.setItem(FOLLOW_SYSTEM_THEME_STORAGE_KEY, followSystemTheme ? "on" : "off");
    } catch { }
    applyTheme(followSystemTheme ? systemThemeName() : readThemePreference(), false);
    syncDarkModeCheckbox();
  });
  document.getElementById("options-show-nicklist")?.addEventListener("change", (event) => {
    state.showNicklist = event.target.checked;
    render();
  });
  document.getElementById("options-show-switchbar")?.addEventListener("change", (event) => {
    state.showSwitchbar = event.target.checked;
    try {
      localStorage.setItem(SWITCHBAR_STORAGE_KEY, state.showSwitchbar ? "on" : "off");
    } catch { }
    render();
  });

  // Mouse double-click targets
  document.querySelectorAll("[data-mouse-target]").forEach((el) => {
    el.addEventListener("input", () => saveMouseDblclick(el.dataset.mouseTarget, el.value));
  });

  // Drop rules
  const dropRules = document.getElementById("options-drop-rules");
  if (dropRules) {
    dropRules.addEventListener("input", () => {
      try { localStorage.setItem(DROP_RULES_PREFIX + selectedDropRules, dropRules.value); } catch { }
    });
    document.querySelectorAll('input[name="drop-mode"]').forEach((el) => {
      el.addEventListener("change", () => {
        selectedDropRules = el.value;
        dropRules.value = readDropRules(selectedDropRules);
      });
    });
  }

  // RSS manual refresh button
  const btnRssNow = container.querySelector("#btn-rss-fetch-now");
  if (btnRssNow) {
    btnRssNow.addEventListener("click", async () => {
      container.querySelectorAll("[data-opt]").forEach((el) => {
        const key = el.dataset.opt;
        if (el.type === "checkbox") pendingOptions[key] = el.checked;
        else if (el.type === "radio") { if (el.checked) pendingOptions[key] = el.value; }
        else pendingOptions[key] = el.value;
      });
      appOptions.rssUrls = pendingOptions.rssUrls;
      appOptions.rssEnabled = pendingOptions.rssEnabled;
      appOptions.rssCount = pendingOptions.rssCount;
      appOptions.rssMode = pendingOptions.rssMode;
      appOptions.rssInterval = pendingOptions.rssInterval;
      writeJsonSetting(OPTIONS_STORAGE_KEY, withoutSecrets(appOptions));
      await rssManager.fetchAll(true);
    });
  }

  // Tree category click & toggle handlers
  document.querySelectorAll(".options-tree [data-page]").forEach((el) => {
    el.addEventListener("click", () => {
      state.optionsPage = el.dataset.page;
      renderOptionsModal();
    });
  });
  document.querySelectorAll(".options-tree [data-toggle]").forEach((el) => {
    el.addEventListener("click", (event) => {
      event.stopPropagation();
      const id = el.dataset.toggle;
      state.optionsOpen[id] = state.optionsOpen[id] === false;
      renderOptionsModal();
    });
  });

  // Backdrop click to close
  document.getElementById("options-modal-backdrop")?.addEventListener("click", (event) => {
    if (event.target === event.currentTarget && !optionsSubdialog) {
      pendingOptions = { ...appOptions };
      closeOptionsModal();
    }
  });
}

function renderFishModal() {
  const container = document.getElementById("modal-container");
  if (!container) return;

  const q = (state.fishSearchQuery || "").toLowerCase().trim();
  const filtered = state.fishKeysList.filter(item => {
    if (!q) return true;
    return item.target.toLowerCase().includes(q) || item.section.toLowerCase().includes(q);
  });

  const totalCount = state.fishKeysList.length;

  container.innerHTML = `
    <div class="modal-overlay" id="fish-modal-backdrop">
      <div class="fish-modal-card">
        <div class="fish-modal-header">
          <div style="display:flex; align-items:center; gap:10px;">
            <span class="fish-modal-mark">FiSH</span>
            <div>
              <div class="fish-modal-title">FiSH Blowfish Key Manager</div>
              <div class="fish-modal-path">
                Loaded from the app's portable or user configuration folder.
              </div>
            </div>
          </div>
          <button id="btn-fish-close" class="btn-icon-sm" style="font-size:16px; width:28px; height:28px; display:flex; align-items:center; justify-content:center;">✕</button>
        </div>

        <div class="fish-modal-body">
          <!-- Quick Set Section -->
          <div class="fish-section">
            <div class="fish-section-title">
              <span>Set / Update Channel or User Key</span>
              <span id="fish-quick-status" style="font-size:11px; font-weight:600; text-transform:none; color:${state.quickKey ? 'var(--accent)' : 'var(--text-muted)'};">
                ${state.quickTarget ? (state.quickKey ? `Key configured · ${state.quickFishMode.toUpperCase()}` : 'No key set') : ''}
              </span>
            </div>

            <div class="fish-quick-form">
              <div>
                <label class="form-label">Target (#channel or nick)</label>
                <input id="quick-target-input" class="form-input" style="font-family:var(--font-mono);" placeholder="#channel or nick" value="${escapeHtml(state.quickTarget)}" />
              </div>

              <div>
                <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:6px;">
                  <label class="form-label" style="margin-bottom:0;">Blowfish Key</label>
                  <div style="display:flex; gap:6px;">
                    <button id="btn-toggle-key-vis" class="btn-icon-sm" title="Show or hide key">Show</button>
                    <button id="btn-gen-key" class="btn-icon-sm" title="Generate random 16-character key">Random</button>
                  </div>
                </div>
                <input id="quick-key-input" type="password" class="form-input" style="font-family:var(--font-mono);" placeholder="Enter secret key..." value="${escapeHtml(state.quickKey)}" />
                <label class="form-label" for="quick-fish-mode" style="margin-top:10px;">Encryption mode</label>
                <select id="quick-fish-mode" class="form-input">
                  <option value="cbc" ${state.quickFishMode === "cbc" ? "selected" : ""}>CBC — required by some bots</option>
                  <option value="ecb" ${state.quickFishMode === "ecb" ? "selected" : ""}>ECB — legacy</option>
                </select>
              </div>

              <div style="display:flex; gap:6px;">
                <button id="btn-save-quick-key" class="btn btn-primary" style="white-space:nowrap; padding:8px 14px;">Save key</button>
                ${state.quickKey ? `<button id="btn-del-quick-key" class="btn btn-danger" style="white-space:nowrap; padding:8px 10px;" title="Delete key">Delete</button>` : ''}
              </div>
            </div>
          </div>

          <!-- All Keys Browser -->
          <div class="fish-section" style="flex:1; display:flex; flex-direction:column; min-height:0;">
            <div class="fish-section-title">
              <span>All Configured Keys (${totalCount} in blow.ini)</span>
              <span style="font-size:11px; color:var(--text-muted); text-transform:none;">
                Showing ${filtered.length} of ${totalCount}
              </span>
            </div>

            <div style="margin-bottom: 10px;">
              <input id="fish-search-input" class="form-input" placeholder="Search keys by #channel, network or nick..." value="${escapeHtml(state.fishSearchQuery)}" style="font-family:var(--font-mono); font-size:12px;" />
            </div>

            <div class="fish-table-container">
              <table class="fish-table">
                <thead>
                  <tr>
                    <th style="width:38%;">Section / Target</th>
                    <th style="width:36%;">Decrypted Key</th>
                    <th style="width:14%;">Date</th>
                    <th style="width:12%; text-align:right;">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  ${filtered.length === 0 ? `
                    <tr><td colspan="4" style="text-align:center; padding:20px; color:var(--text-dim);">No keys match search filter</td></tr>
                  ` : filtered.map(item => `
                    <tr data-target="${escapeHtml(item.target)}" data-section="${escapeHtml(item.section)}">
                      <td>
                        <span style="color:var(--accent); font-weight:700;">${escapeHtml(item.section)}</span>
                      </td>
                      <td>
                        <span class="key-code" data-key="${escapeHtml(item.key)}" title="Click to reveal / copy">••••••••••••</span>
                      </td>
                      <td style="color:var(--text-muted); font-size:11px;">${escapeHtml(item.date || "-")}</td>
                      <td style="text-align:right; white-space:nowrap;">
                        <button class="btn-icon-sm btn-edit-entry" data-target="${escapeHtml(item.target)}" data-key="${escapeHtml(item.key)}" title="Edit in form">Edit</button>
                        <button class="btn-icon-sm btn-copy-entry" data-key="${escapeHtml(item.key)}" title="Copy key">Copy</button>
                        <button class="btn-icon-sm btn-icon-danger btn-del-entry" data-target="${escapeHtml(item.section)}" title="Delete key">Delete</button>
                      </td>
                    </tr>
                  `).join("")}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </div>
    </div>
  `;

  attachFishModalHandlers();
}

function attachFishModalHandlers() {
  const container = document.getElementById("modal-container");
  if (!container) return;

  // Close handlers
  const btnClose = document.getElementById("btn-fish-close");
  if (btnClose) btnClose.addEventListener("click", closeFishModal);

  const backdrop = document.getElementById("fish-modal-backdrop");
  if (backdrop) {
    backdrop.addEventListener("click", (e) => {
      if (e.target === backdrop) closeFishModal();
    });
  }

  // Quick inputs
  const targetInput = document.getElementById("quick-target-input");
  const keyInput = document.getElementById("quick-key-input");
  const modeInput = document.getElementById("quick-fish-mode");

  if (targetInput) {
    targetInput.addEventListener("input", (e) => {
      state.quickTarget = e.target.value.trim();
      state.quickNetwork = "";
    });
  }

  if (keyInput) {
    keyInput.addEventListener("input", (e) => {
      state.quickKey = e.target.value.trim();
    });
  }

  if (modeInput) {
    modeInput.addEventListener("change", (e) => {
      state.quickFishMode = e.target.value === "ecb" ? "ecb" : "cbc";
      const statusEl = document.getElementById("fish-quick-status");
      if (statusEl && state.quickTarget && state.quickKey) {
        statusEl.textContent = `Key configured · ${state.quickFishMode.toUpperCase()}`;
      }
    });
  }

  // Toggle key visibility
  const btnToggleVis = document.getElementById("btn-toggle-key-vis");
  if (btnToggleVis && keyInput) {
    btnToggleVis.addEventListener("click", () => {
      if (keyInput.type === "password") {
        keyInput.type = "text";
        btnToggleVis.textContent = "Hide";
      } else {
        keyInput.type = "password";
        btnToggleVis.textContent = "Show";
      }
    });
  }

  // Generate random key
  const btnGenKey = document.getElementById("btn-gen-key");
  if (btnGenKey && keyInput) {
    btnGenKey.addEventListener("click", () => {
      const generated = generateRandomKey(16);
      keyInput.value = generated;
      keyInput.type = "text";
      state.quickKey = generated;
      if (btnToggleVis) btnToggleVis.textContent = "Hide";
    });
  }

  // Save quick key
  const btnSave = document.getElementById("btn-save-quick-key");
  if (btnSave && targetInput && keyInput) {
    btnSave.addEventListener("click", async () => {
      const target = targetInput.value.trim();
      const key = keyInput.value.trim();
      if (!target || !key) return;
      const storedKey = formatFishKey(key, state.quickFishMode);

      btnSave.textContent = "Saving...";
      if (isTauri) {
        try {
          await invoke("save_fish_key_entry", { network: state.quickNetwork || state.active_network, target, key: storedKey });
          state.quickNetwork = "";
          state.fishKeysList = await invoke("get_fish_keys", {});
          await checkActiveBufferFish(true);
        } catch (e) {
          console.error("Save fish key error:", e);
        }
      }
      state.quickTarget = target;
      state.quickKey = key;
      renderFishModal();
    });
  }

  // Delete quick key
  const btnDel = document.getElementById("btn-del-quick-key");
  if (btnDel && targetInput) {
    btnDel.addEventListener("click", async () => {
      const target = targetInput.value.trim();
      if (!target) return;

      if (isTauri) {
        try {
          await invoke("delete_fish_key_entry", { section: fishSection(state.quickNetwork || state.active_network, target) });
          state.fishKeysList = await invoke("get_fish_keys", {});
          await checkActiveBufferFish(true);
        } catch (e) {
          console.error("Delete fish key error:", e);
        }
      }
      state.quickKey = "";
      renderFishModal();
    });
  }

  // Search filter
  const searchInput = document.getElementById("fish-search-input");
  if (searchInput) {
    searchInput.addEventListener("input", (e) => {
      state.fishSearchQuery = e.target.value;
      // Re-filter table body without re-rendering whole modal to preserve input focus
      const q = state.fishSearchQuery.toLowerCase().trim();
      const filtered = state.fishKeysList.filter(item => {
        if (!q) return true;
        return item.target.toLowerCase().includes(q) || item.section.toLowerCase().includes(q);
      });

      const tbody = container.querySelector(".fish-table tbody");
      if (tbody) {
        tbody.innerHTML = filtered.length === 0 ? `
          <tr><td colspan="4" style="text-align:center; padding:20px; color:var(--text-dim);">No keys match search filter</td></tr>
        ` : filtered.map(item => `
          <tr data-target="${escapeHtml(item.target)}" data-section="${escapeHtml(item.section)}">
            <td>
              <span style="color:var(--accent); font-weight:700;">${escapeHtml(item.section)}</span>
            </td>
            <td>
              <span class="key-code" data-key="${escapeHtml(item.key)}" title="Click to reveal / copy">••••••••••••</span>
            </td>
            <td style="color:var(--text-muted); font-size:11px;">${escapeHtml(item.date || "-")}</td>
            <td style="text-align:right; white-space:nowrap;">
              <button class="btn-icon-sm btn-edit-entry" data-target="${escapeHtml(item.target)}" data-key="${escapeHtml(item.key)}" title="Edit in form">Edit</button>
              <button class="btn-icon-sm btn-copy-entry" data-key="${escapeHtml(item.key)}" title="Copy key">Copy</button>
              <button class="btn-icon-sm btn-icon-danger btn-del-entry" data-target="${escapeHtml(item.section)}" title="Delete key">Delete</button>
            </td>
          </tr>
        `).join("");
        attachTableEntryHandlers();
      }
    });
  }

  attachTableEntryHandlers();
}

function attachTableEntryHandlers() {
  const container = document.getElementById("modal-container");
  if (!container) return;

  // Edit button in table
  container.querySelectorAll(".btn-edit-entry").forEach(btn => {
    btn.addEventListener("click", () => {
      const tgt = btn.dataset.target;
      const k = btn.dataset.key;
      const parsedKey = parseFishKey(k);
      const section = btn.closest("tr")?.dataset.section || "";
      const sepIdx = section.indexOf(":");
      state.quickNetwork = sepIdx >= 0 ? section.slice(0, sepIdx) : "";
      state.quickTarget = tgt;
      state.quickKey = parsedKey.key;
      state.quickFishMode = parsedKey.mode;
      const targetInput = document.getElementById("quick-target-input");
      const keyInput = document.getElementById("quick-key-input");
      const modeInput = document.getElementById("quick-fish-mode");
      if (targetInput) targetInput.value = tgt;
      if (keyInput) {
        keyInput.value = parsedKey.key;
        keyInput.type = "text";
        const btnToggleVis = document.getElementById("btn-toggle-key-vis");
        if (btnToggleVis) btnToggleVis.textContent = "Hide";
        keyInput.focus();
      }
      if (modeInput) modeInput.value = parsedKey.mode;
      const statusEl = document.getElementById("fish-quick-status");
      if (statusEl) {
        statusEl.textContent = `Key loaded · ${parsedKey.mode.toUpperCase()}`;
        statusEl.style.color = "var(--accent)";
      }
    });
  });

  // Copy button
  container.querySelectorAll(".btn-copy-entry").forEach(btn => {
    btn.addEventListener("click", async () => {
      const k = btn.dataset.key;
      try {
        await navigator.clipboard.writeText(k);
        const prev = btn.textContent;
        btn.textContent = "✓";
        setTimeout(() => { btn.textContent = prev; }, 1200);
      } catch (err) {
        console.error("Copy failed:", err);
      }
    });
  });

  // Key code click to reveal / copy
  container.querySelectorAll(".key-code").forEach(span => {
    span.addEventListener("click", async () => {
      const k = span.dataset.key;
      if (span.textContent === "••••••••••••") {
        span.textContent = k;
      } else {
        try {
          await navigator.clipboard.writeText(k);
          const orig = span.textContent;
          span.textContent = "COPIED!";
          setTimeout(() => { span.textContent = orig; }, 1000);
        } catch (e) {
          span.textContent = "••••••••••••";
        }
      }
    });
  });

  // Delete button in table
  container.querySelectorAll(".btn-del-entry").forEach(btn => {
    btn.addEventListener("click", async () => {
      const section = btn.dataset.target;
      if (!section) return;

      if (isTauri) {
        try {
          await invoke("delete_fish_key_entry", { section });
          state.fishKeysList = await invoke("get_fish_keys", {});
          await checkActiveBufferFish(true);
        } catch (e) {
          console.error("Delete fish key error:", e);
        }
      }
      renderFishModal();
    });
  });
}

function renderConnectModal() {
  const container = document.getElementById("modal-container");
  if (!container) return;

  container.innerHTML = `
    <div class="modal-overlay">
      <div class="modal-card">
        <h2 class="modal-title">Connect to IRC Network</h2>
        <div id="modal-connect-error" class="form-error" role="alert" hidden></div>
        <div class="form-group">
          <label class="form-label">Network Name</label>
          <input id="modal-network" class="form-input" value="net-one" />
        </div>
        <div class="form-group">
          <label class="form-label">Server Host</label>
          <input id="modal-host" class="form-input" value="irc.example.invalid" />
        </div>
        <div class="form-group">
          <label class="form-label">Port</label>
          <input id="modal-port" type="number" class="form-input" value="59000" />
        </div>
        <div class="form-group">
          <label class="form-label">Nickname</label>
          <input id="modal-nick" class="form-input" value="Ferric" />
        </div>
        <div class="form-group">
          <label class="form-label">Password</label>
          <input id="modal-pass" type="password" class="form-input" value="" />
        </div>
        <div class="form-group" style="display:flex; align-items:center; gap:8px;">
          <input id="modal-tls" type="checkbox" checked />
          <label for="modal-tls" class="form-label" style="margin:0;">Use TLS / SSL</label>
        </div>
        <div class="form-group">
          <label class="form-label" for="modal-tls-fp">Certificate fingerprint (only for a self-signed certificate)</label>
          <div style="display:flex; gap:8px;">
            <input id="modal-tls-fp" type="text" class="form-input" placeholder="SHA-256, 64 hex digits" autocomplete="off" spellcheck="false" />
            <button id="modal-tls-fetch" type="button" class="btn btn-secondary">Fetch</button>
          </div>
          <div style="font-size:12px; opacity:0.7; margin-top:4px;">Fetch reads the fingerprint the server shows. Compare it with the value from the server admin before you connect: a fetched value is not verified.</div>
        </div>
        <div style="display:flex; justify-content:flex-end; gap:8px; margin-top:20px;">
          <button id="modal-cancel" class="btn btn-secondary">Cancel</button>
          <button id="modal-connect" class="btn btn-primary">Connect</button>
        </div>
      </div>
    </div>
  `;

  document.getElementById("modal-cancel").addEventListener("click", () => {
    state.showConnectModal = false;
    renderModal();
  });

  document.getElementById("modal-tls-fetch").addEventListener("click", async () => {
    const error = document.getElementById("modal-connect-error");
    if (!isTauri) {
      error.textContent = "Certificate fingerprints are available in the desktop app.";
      error.hidden = false;
      return;
    }
    const host = document.getElementById("modal-host").value.trim();
    const port = parseInt(document.getElementById("modal-port").value, 10);
    if (!host || !Number.isInteger(port) || port < 1 || port > 65535) {
      error.textContent = "Enter a host and a port first.";
      error.hidden = false;
      return;
    }
    error.hidden = true;
    try {
      // Through the proxy the connection uses; an incomplete proxy setting is refused, not bypassed.
      const proxy = proxyConnectArgs();
      if (proxy.error) throw new Error(proxy.error);
      const fingerprint = await invoke("tls_fingerprint", { host, port, ...proxy.args });
      document.getElementById("modal-tls-fp").value = fingerprint;
    } catch (err) {
      error.textContent = `Could not read the certificate: ${String(err)}`;
      error.hidden = false;
    }
  });

  document.getElementById("modal-connect").addEventListener("click", async () => {
    const netName = document.getElementById("modal-network").value.trim();
    const host = document.getElementById("modal-host").value.trim();
    const port = parseInt(document.getElementById("modal-port").value, 10);
    const nick = document.getElementById("modal-nick").value.trim();
    const pass = document.getElementById("modal-pass").value.trim();
    const useTls = document.getElementById("modal-tls").checked;
    const tlsFingerprint = document.getElementById("modal-tls-fp").value.trim();
    const error = document.getElementById("modal-connect-error");
    const connectButton = document.getElementById("modal-connect");

    if (!netName || !host || !nick || !Number.isInteger(port) || port < 1 || port > 65535) {
      error.textContent = "Enter a network name, server, nickname, and a port from 1 to 65535.";
      error.hidden = false;
      return;
    }

    if (!isTauri) {
      error.textContent = "Server connections are available in the desktop app.";
      error.hidden = false;
      return;
    }

    const proxy = proxyConnectArgs();
    if (proxy.error) {
      error.textContent = proxy.error;
      error.hidden = false;
      return;
    }

    error.hidden = true;
    connectButton.disabled = true;
    connectButton.textContent = "Connecting…";
    try {
      await invoke("connect_network", {
        ...proxy.args,
        config: {
          network: netName,
          host,
          port,
          use_tls: useTls,
          nick,
          username: nick,
          realname: nick,
          password: pass.length > 0 ? pass : null,
          tls_fingerprint: tlsFingerprint.length > 0 ? tlsFingerprint : null,
        },
      });
      state.showConnectModal = false;
      renderModal();
      await syncFromBackend();
    } catch (err) {
      error.textContent = `Connection failed: ${String(err)}`;
      error.hidden = false;
    } finally {
      if (connectButton.isConnected) {
        connectButton.disabled = false;
        connectButton.textContent = "Connect";
      }
    }
  });
}

// App Initialization
mountAppShell();

async function initTauri() {
  if (isTauri) {
    try {
      const tauriCore = await import("@tauri-apps/api/core");
      const tauriEvent = await import("@tauri-apps/api/event");
      invoke = tauriCore.invoke;
      listen = tauriEvent.listen;
      initScriptDialog({
        onBeforeOpen: () => {
          closeSideDialogs();
          if (isColorsModalOpen()) closeColorsModal();
          if (isAddressBookModalOpen()) closeAddressBookModal();
          closeChannelCentral();
        },
      });
      void rssManager.fetchAll();

      // Saved logging and identd settings reach the backend before any network connects.
      await applyLogging();
      await applyConnectDebug();
      await applyIdentd();
      await applySelfSignedSetting();
      await applyPythonSettings();
      await applyQueryAutoKeyxSetting();

      // Periodically sync backend state (500ms) without replacing input or disrupting scroll
      // Started once. A tick is skipped while a sync is still running (it would only queue
      // another full copy of the state) and while the window is hidden; irc-event syncs still run.
      if (!syncTimer) {
        syncTimer = setInterval(() => {
          if (!syncRunning && !document.hidden) void syncFromBackend();
        }, 500);
      }
      try {
        await listen("irc-event", (event) => {
          const ircEvent = event.payload;
          const payload = ircEvent?.payload;
          const net = payload && state.networks[payload.network];
          if (ircEvent?.type === "Message" && typeof payload?.sender === "string" && payload.sender_host) {
            rememberSenderHost(payload.sender, payload.sender_host);
          }
          const queryKey = typeof payload?.sender === "string" ? payload.sender.toLowerCase() : "";
          const isQueryMessage = Boolean(ircEvent?.type === "Message"
            && net
            && !payload.is_notice
            && !isIgnoredNick(payload.sender)
            && !isUserIgnored(payload.sender, payload.network, payload.sender_host || "")
            && typeof payload.target === "string"
            && payload.target.toLowerCase() === net.nick.toLowerCase()
            && queryKey);
          // A brand-new query opens at once; a reply in an existing query only flags it. That work
          // runs after the sync that follows, once per window (see runQueryFollowUps).
          const openNewQuery = isQueryMessage && !net.buffers[queryKey];
          const replyToOpenQuery = isQueryMessage && Boolean(net.buffers[queryKey]);
          if (openNewQuery || replyToOpenQuery) {
            pendingQueryFollowUps.set(`${payload.network}/${queryKey}`, {
              network: payload.network,
              key: queryKey,
              sender: payload.sender,
              opened: openNewQuery,
            });
          }
          void syncFromBackend();
        });
      } catch (err) {
        console.warn("IRC event listener setup failed:", err);
      }

      // connectd.mrc is loaded on startup only when "Connect on startup" is set (off by default).
      try {
        if (appOptions.connectOnStartup) await loadConnectdWithProxy();
        await syncFromBackend();
        loadActiveHistory();
      } catch (err) {
        console.warn("Auto load_connectd error:", err);
        pushStatusNotice(state.active_network, String(err?.message || err));
      }
    } catch (e) {
      console.warn("Tauri API init failed:", e);
    }
  } else {
    // Demo fallback state when outside Tauri
    state.networks = {
      "net-one": {
        id: "net-one",
        server: "irc.example.invalid:+59000",
        nick: "Ferric",
        connected: true,
        active_buffer: "#demo",
        buffers: {
          "status": {
            name: "status",
            is_channel: false,
            topic: "Server messages",
            users: {},
            messages: [
              {
                id: 1,
                sender: "*",
                text: "*** Connected to irc.example.invalid:+59000 using TLS",
                spans: [],
                is_notice: true,
                is_action: false,
                is_encrypted: false,
                is_highlight: false,
                timestamp: Date.now(),
              },
            ],
            unread_count: 0,
            highlight_count: 0,
          },
          "#demo": {
            name: "#Demo",
            is_channel: true,
            topic: "Welcome — please stay on topic.",
            users: {
              "ferric": { nick: "Ferric", prefix: "@" },
              "nightowl": { nick: "NightOwl", prefix: "+" },
              "kaffeekeks": { nick: "KaffeeKeks", prefix: null },
              "opsbot": { nick: "OpsBot", prefix: "@" },
              "zeta": { nick: "Zeta", prefix: null },
              "morgen": { nick: "Morgenstern", prefix: "+" },
            },
            messages: [
              {
                id: 20,
                sender: "*",
                text: "*** NightOwl (nightowl@example.invalid) has joined #Demo",
                spans: [],
                is_notice: false,
                is_action: false,
                is_encrypted: false,
                is_highlight: false,
                timestamp: Date.now() - 95 * 1000,
              },
              {
                id: 21,
                sender: "*",
                text: "*** Zebra (zebra@example.invalid) has quit (Ping timeout: 240 seconds)",
                spans: [],
                is_notice: false,
                is_action: false,
                is_encrypted: false,
                is_highlight: false,
                timestamp: Date.now() - 70 * 1000,
              },
              {
                id: 22,
                sender: "ServBot",
                text: "Channel registration is active for #Demo.",
                spans: [],
                is_notice: true,
                is_action: false,
                is_encrypted: false,
                is_highlight: false,
                timestamp: Date.now() - 55 * 1000,
              },
              {
                id: 2,
                sender: "NightOwl",
                text: "Hey! Anyone else in the channel?",
                spans: [],
                is_notice: false,
                is_action: false,
                is_encrypted: true,
                is_highlight: false,
                timestamp: Date.now() - 30 * 1000,
              },
              {
                id: 3,
                sender: "KaffeeKeks",
                text: "Just connected. Chat is active.",
                spans: [],
                is_notice: false,
                is_action: false,
                is_encrypted: true,
                is_highlight: false,
                timestamp: Date.now() - 10 * 1000,
              },
              {
                id: 4,
                sender: "NightOwl",
                text: "Ferric: could you send the key again?",
                spans: [],
                is_notice: false,
                is_action: false,
                is_encrypted: true,
                is_highlight: true,
                timestamp: Date.now() - 5 * 1000,
              },
            ],
            unread_count: 0,
            highlight_count: 0,
          },
          "#dev": {
            name: "#dev",
            is_channel: true,
            topic: "Build notes",
            users: {},
            messages: [],
            unread_count: 3,
            highlight_count: 0,
          },
          "#lounge": {
            name: "#lounge",
            is_channel: true,
            topic: "",
            users: {},
            messages: [],
            unread_count: 0,
            highlight_count: 0,
          },
          "nightowl": {
            name: "NightOwl",
            is_channel: false,
            topic: "Private conversation with NightOwl",
            users: {},
            messages: [],
            unread_count: 1,
            highlight_count: 1,
          },
        },
      },
      "testnet": {
        id: "testnet",
        server: "irc.example.invalid:6697",
        nick: "Ferric",
        connected: false,
        active_buffer: "status",
        buffers: {
          "status": {
            name: "status",
            is_channel: false,
            topic: "Server messages",
            users: {},
            messages: [],
            unread_count: 0,
            highlight_count: 0,
          },
          "#sandbox": {
            name: "#sandbox",
            is_channel: true,
            topic: "",
            users: {},
            messages: [],
            unread_count: 0,
            highlight_count: 0,
          },
        },
      },
    };
    // A fuller conversation for the demo channel: fake nicks, color codes, an action and a highlight.
    const demoAt = (secondsAgo) => Date.now() - secondsAgo * 1000;
    const demoLine = (id, secondsAgo, sender, text, extra = {}) => ({
      id, sender, text, spans: [], is_notice: false, is_action: false,
      is_encrypted: false, is_highlight: false, timestamp: demoAt(secondsAgo), ...extra,
    });
    const demoChannel = state.networks["net-one"].buffers["#demo"];
    demoChannel.topic = "Ferric demo channel · fake data only · be nice";
    demoChannel.users = {
      ferric: { nick: "Ferric", prefix: "@" },
      opsbot: { nick: "OpsBot", prefix: "~" },
      morgen: { nick: "Morgenstern", prefix: "&" },
      nightowl: { nick: "NightOwl", prefix: "%" },
      zeta: { nick: "Zeta", prefix: "+" },
      quasar: { nick: "Quasar", prefix: "+" },
      kaffeekeks: { nick: "KaffeeKeks", prefix: null },
      pixel: { nick: "Pixel", prefix: null },
      tinsel: { nick: "Tinsel", prefix: null },
    };
    demoChannel.messages = [
      demoLine(10, 1500, "*", "*** Topic set by OpsBot: Ferric demo channel · fake data only · be nice", { is_notice: true }),
      demoLine(11, 1420, "OpsBot", "Welcome to #Demo. Logs are off, FiSH is on.", { is_notice: true }),
      demoLine(12, 1300, "Pixel", "who is up for a quick test of the new \u000306purple\u0003 theme?"),
      demoLine(13, 1210, "Morgenstern", "me! \u0002bold\u0002, \u001funderline\u001f and \u000308yellow\u0003 all render fine here"),
      demoLine(14, 1100, "Tinsel", "the switchbar can be toggled in the View menu"),
      demoLine(15, 980, "Zeta", "and the topic bar opens the channel dialog on double-click"),
      demoLine(16, 900, "*", "*** Pixel sets mode: +o Morgenstern"),
      demoLine(17, 800, "Quasar", "does the Server manager keep my comments in the file?"),
      demoLine(18, 740, "Ferric", "yes, only the server lines change, everything else stays as it is"),
      demoLine(19, 650, "KaffeeKeks", "great. the key manager handles DH1080 for queries too"),
      demoLine(20, 560, "*", "*** NightOwl (nightowl@example.invalid) has joined #Demo"),
      demoLine(21, 540, "*", "*** Quasar (quasar@example.invalid) has joined #Demo"),
      demoLine(22, 520, "OpsBot", "Backup finished: 3 jobs, 0 errors", { is_notice: true }),
      demoLine(23, 470, "NightOwl", "Morning! The new build is \u000309green\u0003 on Windows and Linux."),
      demoLine(24, 430, "KaffeeKeks", "nice. do the \u0002FiSH\u0002 keys survive the update?", { is_encrypted: true }),
      demoLine(25, 400, "Ferric", "yep, ECB and CBC both work. keys stay in \u001fblow.ini\u001f", { is_encrypted: true }),
      demoLine(26, 330, "Morgenstern", "waves at everyone", { is_action: true }),
      demoLine(27, 280, "Zeta", "anyone tried the \u000312blue\u0003 / \u000304red\u0003 / \u000307orange\u0003 nick colors yet?"),
      demoLine(28, 230, "Quasar", "dark theme looks great. the scripts editor has line numbers too"),
      demoLine(29, 170, "*", "*** Tinsel (tinsel@example.invalid) has joined #Demo"),
      demoLine(30, 120, "*", "*** Zebra (zebra@example.invalid) has quit (Ping timeout: 240 seconds)"),
      demoLine(31, 70, "Pixel", "server manager is neat, I switched two networks off with one click"),
      demoLine(32, 20, "NightOwl", "Ferric: could you send the key again?", { is_encrypted: true, is_highlight: true }),
    ];
    state.active_network = "net-one";
    render();
  }
}

initTauri();
