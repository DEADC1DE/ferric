import { escapeHtml } from "./utils/ircColors.js";

const STORAGE_USERS_KEY = "ferric-abook-users";
const STORAGE_NOTIFY_KEY = "ferric-abook-notify";
const STORAGE_CONTROL_KEY = "ferric-abook-control";
const STORAGE_COLORS_KEY = "ferric-abook-colors";
const STORAGE_HIGHLIGHT_KEY = "ferric-abook-highlight";

// Default nickname color rules matching the default color rules
export const DEFAULT_NICK_COLOR_RULES = [
  { id: "me", label: "$me", pattern: "$me", mode: "", color: "#ffffff", isDefault: true },
  { id: "q", label: "Any nickname (~!)", pattern: "*", mode: "~!", color: "#ff00ff", isDefault: true },
  { id: "a", label: "Any nickname (&*)", pattern: "*", mode: "&*", color: "#ff0000", isDefault: true },
  { id: "o", label: "Any nickname (@)", pattern: "*", mode: "@", color: "#ff0000", isDefault: true },
  { id: "h", label: "Any nickname (%)", pattern: "*", mode: "%", color: "#00b5b5", isDefault: true },
  { id: "v", label: "Any nickname (+)", pattern: "*", mode: "+", color: "#00fc00", isDefault: true },
  { id: "none", label: "Any nickname (No mode)", pattern: "*", mode: "none", color: "#ffffff", isDefault: true },
];

export const DEFAULT_HIGHLIGHT_RULES = [
  {
    id: "me",
    words: "$me",
    targets: "",
    matchOn: "Any",
    caseSensitive: false,
    regex: false,
    color: "#ff00ff",
    sound: "No Sound",
    flash: false,
    flashTimes: "--",
    tip: false,
    tipSeconds: "3",
    message: ""
  }
];

let state = {
  isOpen: false,
  activeTab: "users", // "users" | "whois" | "notify" | "control" | "colors" | "highlight"
  selectedUserIdx: 0,
  users: [],
  whois: {
    nick: "",
    name: "",
    address: "",
    channels: "",
    idle: "",
    away: "",
    server: "",
    status: "",
    ctcp: ""
  },
  notify: {
    enabled: true,
    entries: []
  },
  control: {
    enabled: true,
    type: "Ignore", // "Ignore" | "Op" | "Voice" | "Protect"
    randomDelay: false,
    entries: [
          ]
  },
  colors: {
    enabled: true,
    rules: [...DEFAULT_NICK_COLOR_RULES]
  },
  highlight: {
    enabled: true,
    rules: [...DEFAULT_HIGHLIGHT_RULES]
  },
  selectedColorRuleIdx: 0,
  selectedHighlightRuleIdx: 0,
  selectedControlIdx: 0,
  selectedNotifyIdx: 0,
  editingHighlight: null, // Subdialog Edit Highlight
  editingRule: null // Subdialog Edit Color Rule
};

let sendCommandCallback = null;
// The address book data as it was when the dialog opened. Cancel, Escape and a click outside put it back.
let snapshotAddressBook = null;
let onAddressBookChangedCallback = null;

export function setAddressBookCallbacks(sendCmd, onChanged) {
  sendCommandCallback = sendCmd;
  onAddressBookChangedCallback = onChanged;
}

export function initAddressBook() {
  try {
    const savedUsers = localStorage.getItem(STORAGE_USERS_KEY);
    if (savedUsers) state.users = JSON.parse(savedUsers);

    const savedNotify = localStorage.getItem(STORAGE_NOTIFY_KEY);
    if (savedNotify) state.notify = JSON.parse(savedNotify);

    const savedControl = localStorage.getItem(STORAGE_CONTROL_KEY);
    if (savedControl) state.control = JSON.parse(savedControl);

    const savedColors = localStorage.getItem(STORAGE_COLORS_KEY);
    if (savedColors) state.colors = JSON.parse(savedColors);

    const savedHl = localStorage.getItem(STORAGE_HIGHLIGHT_KEY);
    if (savedHl) state.highlight = JSON.parse(savedHl);
  } catch { }
}

function saveAddressBook() {
  try {
    localStorage.setItem(STORAGE_USERS_KEY, JSON.stringify(state.users));
    localStorage.setItem(STORAGE_NOTIFY_KEY, JSON.stringify(state.notify));
    localStorage.setItem(STORAGE_CONTROL_KEY, JSON.stringify(state.control));
    localStorage.setItem(STORAGE_COLORS_KEY, JSON.stringify(state.colors));
    localStorage.setItem(STORAGE_HIGHLIGHT_KEY, JSON.stringify(state.highlight));
  } catch { }
  if (onAddressBookChangedCallback) onAddressBookChangedCallback();
}

export function isAddressBookModalOpen() {
  return state.isOpen;
}

export function openAddressBookModal(tab = "users", initialNick = "") {
  // Reopening while open (a Whois click, say) keeps the snapshot of the first open, so Cancel still
  // undoes everything since then.
  if (!state.isOpen) {
    const copy = (value) => JSON.parse(JSON.stringify(value));
    snapshotAddressBook = {
      users: copy(state.users),
      notify: copy(state.notify),
      control: copy(state.control),
      colors: copy(state.colors),
      highlight: copy(state.highlight),
    };
  }
  state.isOpen = true;
  state.activeTab = tab;
  state.editingHighlight = null;
  state.editingRule = null;
  if (initialNick && tab === "whois") {
    // A new lookup starts from empty fields, so the answers of the last lookup do not stay behind.
    state.whois = { nick: initialNick, name: "", address: "", channels: "", idle: "", away: "", server: "", status: "", ctcp: "" };
  }
  renderAddressBookModal();
}

// Closing without OK puts back the data from the snapshot: Add, Delete, Up, Down and Sort were
// already written to storage as they happened, so the restore writes it again.
export function closeAddressBookModal({ keepChanges = false } = {}) {
  if (!keepChanges && snapshotAddressBook) {
    Object.assign(state, snapshotAddressBook);
    // The selections may point past the restored lists.
    const clamp = (idx, len) => Math.max(0, Math.min(idx || 0, len - 1));
    state.selectedUserIdx = clamp(state.selectedUserIdx, state.users.length);
    state.selectedNotifyIdx = clamp(state.selectedNotifyIdx, state.notify.entries.length);
    state.selectedControlIdx = clamp(state.selectedControlIdx, state.control.entries.length);
    state.selectedColorRuleIdx = clamp(state.selectedColorRuleIdx, state.colors.rules.length);
    state.selectedHighlightRuleIdx = clamp(state.selectedHighlightRuleIdx, state.highlight.rules.length);
    saveAddressBook();
  }
  snapshotAddressBook = null;
  state.isOpen = false;
  state.editingHighlight = null;
  state.editingRule = null;
  const container = document.getElementById("modal-container");
  // The container is shared with the other dialogs, so this handler does not stay behind.
  if (container) {
    container.onchange = null;
    container.innerHTML = "";
  }
}

export function getAddressBookColors() {
  return state.colors;
}

export function getAddressBookHighlight() {
  return state.highlight;
}

export function getAddressBookControl() {
  return state.control;
}

export function getAddressBookNotify() {
  return state.notify;
}

export function setWhoisInfo(info) {
  state.whois = { ...state.whois, ...info };
  if (state.isOpen && state.activeTab === "whois") {
    renderAddressBookModal();
  }
}

export function renderAddressBookModal() {
  const container = document.getElementById("modal-container");
  if (!container || !state.isOpen) return;

  const tabs = [
    { id: "users", label: "Users" },
    { id: "whois", label: "Whois" },
    { id: "notify", label: "Notify" },
    { id: "control", label: "Control" },
    { id: "colors", label: "Colors" },
    { id: "highlight", label: "Highlight" },
  ];

  const tabHeaders = tabs.map((t) => {
    const active = state.activeTab === t.id ? " is-active" : "";
    return `<button type="button" class="abook-tab-btn${active}" data-tab="${t.id}">${escapeHtml(t.label)}</button>`;
  }).join("");

  let tabBodyHtml = "";
  if (state.activeTab === "users") tabBodyHtml = renderUsersTab();
  else if (state.activeTab === "whois") tabBodyHtml = renderWhoisTab();
  else if (state.activeTab === "notify") tabBodyHtml = renderNotifyTab();
  else if (state.activeTab === "control") tabBodyHtml = renderControlTab();
  else if (state.activeTab === "colors") tabBodyHtml = renderColorsTab();
  else if (state.activeTab === "highlight") tabBodyHtml = renderHighlightTab();

  container.innerHTML = `
    <div class="modal-overlay" id="abook-modal-backdrop">
      <div class="modal-card abook-dialog" role="dialog" aria-modal="true" aria-labelledby="abook-title">
        <div class="abook-header">
          <div class="abook-title" id="abook-title">Address Book</div>
          <button type="button" class="btn-icon-sm" id="btn-abook-close" aria-label="Close">✕</button>
        </div>

        <nav class="abook-tab-bar" aria-label="Address Book Tabs">
          ${tabHeaders}
        </nav>

        <div class="abook-tab-content">
          ${tabBodyHtml}
        </div>

        <div class="abook-dialog-actions">
          <button type="button" class="btn btn-primary" id="btn-abook-ok">OK</button>
          <button type="button" class="btn btn-secondary" id="btn-abook-cancel">Cancel</button>
          <button type="button" class="btn btn-secondary" id="btn-abook-help">Help</button>
        </div>
      </div>
      ${renderSubdialog()}
    </div>
  `;

  attachAddressBookEvents(container);
}

function renderUsersTab() {
  const user = state.users[state.selectedUserIdx] || {
    nick: "",
    name: "",
    email: "",
    website: "",
    address: "",
    notes: "",
    picture: ""
  };

  const nickOptions = state.users.map((u, i) => {
    const sel = i === state.selectedUserIdx ? "selected" : "";
    return `<option value="${i}" ${sel}>${escapeHtml(u.nick || "New User")}</option>`;
  }).join("");

  return `
    <div class="abook-tab-body abook-users-grid">
      <div class="abook-form-fields">
        <div class="abook-row">
          <label for="ab-nick">Nickname:</label>
          <div style="display:flex; gap:6px; flex:1;">
            <input type="text" id="ab-nick" class="form-input form-input-sm" value="${escapeHtml(user.nick)}" placeholder="Nickname" style="flex:1;" />
            ${state.users.length > 0 ? `<select id="ab-nick-select" class="form-input form-input-sm" style="width:110px;">${nickOptions}</select>` : ""}
          </div>
        </div>
        <div class="abook-row">
          <label for="ab-name">Name:</label>
          <input type="text" id="ab-name" class="form-input form-input-sm" value="${escapeHtml(user.name)}" />
        </div>
        <div class="abook-row">
          <label for="ab-email">Email:</label>
          <input type="text" id="ab-email" class="form-input form-input-sm" value="${escapeHtml(user.email)}" />
        </div>
        <div class="abook-row">
          <label for="ab-website">Website:</label>
          <input type="text" id="ab-website" class="form-input form-input-sm" value="${escapeHtml(user.website)}" />
        </div>
        <div class="abook-row">
          <label for="ab-address">Address:</label>
          <input type="text" id="ab-address" class="form-input form-input-sm" value="${escapeHtml(user.address)}" />
        </div>
        <div class="abook-row" style="align-items:flex-start; flex:1;">
          <label for="ab-notes" style="margin-top:4px;">Notes:</label>
          <textarea id="ab-notes" class="form-input" rows="6" style="flex:1; resize:none;">${escapeHtml(user.notes)}</textarea>
        </div>
      </div>

      <div class="abook-actions-col">
        <button type="button" class="btn btn-secondary" id="btn-ab-add">Add</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-delete" ${state.users.length === 0 ? "disabled" : ""}>Delete</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-email" ${!user.email ? "disabled" : ""}>Email</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-visit" ${!user.website ? "disabled" : ""}>Visit</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-chat" ${!user.nick ? "disabled" : ""}>Chat</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-whois" ${!user.nick ? "disabled" : ""}>Whois</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-notify" ${!user.nick ? "disabled" : ""}>Notify</button>
        <div class="abook-picture-box" id="ab-picture" title="Click to select a picture">
          ${user.picture ? `<img src="${escapeHtml(user.picture)}" alt="User picture" class="abook-avatar-img"/>` : "Click to select a picture"}
        </div>
      </div>
    </div>
  `;
}

function renderWhoisTab() {
  const w = state.whois;
  return `
    <div class="abook-tab-body abook-whois-grid">
      <div class="abook-whois-fields">
        <div class="abook-row">
          <label for="ab-whois-nick">Nickname:</label>
          <input type="text" id="ab-whois-nick" class="form-input form-input-sm" value="${escapeHtml(w.nick)}" style="flex:1;" />
        </div>
        <div class="abook-row-info"><span class="abook-info-label">Name:</span> <span class="abook-info-val">${escapeHtml(w.name || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">Address:</span> <span class="abook-info-val">${escapeHtml(w.address || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">Channels:</span> <span class="abook-info-val">${escapeHtml(w.channels || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">Idle time:</span> <span class="abook-info-val">${escapeHtml(w.idle || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">Away:</span> <span class="abook-info-val">${escapeHtml(w.away || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">Server:</span> <span class="abook-info-val">${escapeHtml(w.server || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">Status:</span> <span class="abook-info-val">${escapeHtml(w.status || "")}</span></div>
        <div class="abook-row-info"><span class="abook-info-label">CTCP:</span> <span class="abook-info-val">${escapeHtml(w.ctcp || "")}</span></div>

        <div class="abook-ctcp-buttons">
          <button type="button" class="btn btn-secondary" id="btn-ab-ping">Ping</button>
          <button type="button" class="btn btn-secondary" id="btn-ab-time">Time</button>
          <button type="button" class="btn btn-secondary" id="btn-ab-version">Version</button>
          <button type="button" class="btn btn-secondary" id="btn-ab-finger">Finger</button>
        </div>
      </div>

      <div class="abook-actions-col">
        <button type="button" class="btn btn-secondary" id="btn-ab-whois-query">Whois</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-whois-add">Add</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-whois-find" disabled title="Not available yet">Find</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-whois-copy">Copy</button>
        <div style="flex:1;"></div>
        <button type="button" class="btn btn-secondary" id="btn-ab-whois-connect" disabled title="Not available yet">Connect</button>
      </div>
    </div>
  `;
}

function renderNotifyTab() {
  const n = state.notify;
  const rows = n.entries.map((entry, idx) => {
    const sel = idx === state.selectedNotifyIdx ? " is-selected" : "";
    return `
      <tr class="abook-table-row${sel}" data-notify-idx="${idx}">
        <td>${escapeHtml(entry.nick)}</td>
        <td>${escapeHtml(entry.network || "*")}</td>
        <td>${escapeHtml(entry.note || "")}</td>
      </tr>
    `;
  }).join("");

  return `
    <div class="abook-tab-body abook-list-grid">
      <div class="abook-list-left">
        <label class="options-toggle" style="margin-bottom:8px;">
          <input type="checkbox" id="ab-notify-enable" ${n.enabled ? "checked" : ""} />
          Enable notify
        </label>
        <div class="abook-table-wrap">
          <table class="abook-table">
            <thead>
              <tr>
                <th style="width:30%;">Nickname</th>
                <th style="width:30%;">Network</th>
                <th style="width:40%;">Note</th>
              </tr>
            </thead>
            <tbody>
              ${rows.length > 0 ? rows : '<tr><td colspan="3" class="abook-empty-cell">No notify entries configured.</td></tr>'}
            </tbody>
          </table>
        </div>
      </div>

      <div class="abook-actions-col">
        <button type="button" class="btn btn-secondary" id="btn-ab-notify-add">Add...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-notify-edit" disabled title="Not available yet">Edit...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-notify-delete" ${n.entries.length === 0 ? "disabled" : ""}>Delete</button>
        <div style="flex:1;"></div>
        <button type="button" class="btn btn-secondary" id="btn-ab-notify-options" disabled title="Not available yet">Options...</button>
      </div>
    </div>
  `;
}

function renderControlTab() {
  const c = state.control;
  // Only the entries of the chosen control type; the indices are those of the full list.
  const ofType = c.entries.map((entry, idx) => ({ entry, idx })).filter(({ entry }) => (entry.type || "Ignore") === c.type);
  // Position of the selected entry among the shown ones (-1 when none of this type is selected).
  const shownPos = ofType.findIndex(({ idx }) => idx === state.selectedControlIdx);
  const items = ofType.map(({ entry, idx }) => {
    const sel = idx === state.selectedControlIdx ? " is-selected" : "";
    const label = `${entry.mask}${entry.flags ? ` (${entry.flags})` : ""}${entry.network ? ` - ${entry.network}` : ""}`;
    return `<div class="abook-list-item${sel}" data-control-idx="${idx}">${escapeHtml(label)}</div>`;
  }).join("");

  return `
    <div class="abook-tab-body abook-list-grid">
      <div class="abook-list-left">
        <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:8px;">
          <label class="options-toggle">
            <input type="checkbox" id="ab-control-enable" ${c.enabled ? "checked" : ""} />
            Enable
          </label>
          <select id="ab-control-type" class="form-input form-input-sm" style="width:110px;">
            <option value="Ignore" ${c.type === "Ignore" ? "selected" : ""}>Ignore</option>
            <option value="Op" ${c.type === "Op" ? "selected" : ""}>Op</option>
            <option value="Voice" ${c.type === "Voice" ? "selected" : ""}>Voice</option>
            <option value="Protect" ${c.type === "Protect" ? "selected" : ""}>Protect</option>
          </select>
        </div>
        <div class="abook-listbox" id="ab-control-list">
          ${items.length > 0 ? items : '<div class="abook-empty-cell">No entries configured.</div>'}
        </div>
        <label class="options-toggle" style="margin-top:8px;">
          <input type="checkbox" id="ab-control-random" ${c.randomDelay ? "checked" : ""} />
          Random delay op/voice
        </label>
      </div>

      <div class="abook-actions-col">
        <button type="button" class="btn btn-secondary" id="btn-ab-control-add">Add...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-control-edit" disabled title="Not available yet">Edit...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-control-delete" ${shownPos < 0 ? "disabled" : ""}>Delete</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-control-up" ${shownPos <= 0 ? "disabled" : ""}>Up</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-control-down" ${shownPos < 0 || shownPos >= ofType.length - 1 ? "disabled" : ""}>Down</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-control-sort" ${ofType.length <= 1 ? "disabled" : ""}>Sort</button>
      </div>
    </div>
  `;
}

function renderColorsTab() {
  const col = state.colors;
  const items = col.rules.map((rule, idx) => {
    const sel = idx === state.selectedColorRuleIdx ? " is-selected" : "";
    return `
      <div class="abook-list-item${sel}" data-rule-idx="${idx}" style="color:${escapeHtml(rule.color)};">
        ${escapeHtml(rule.label || rule.pattern)}
      </div>
    `;
  }).join("");

  return `
    <div class="abook-tab-body abook-list-grid">
      <div class="abook-list-left">
        <label class="options-toggle" style="margin-bottom:8px;">
          <input type="checkbox" id="ab-colors-enable" ${col.enabled ? "checked" : ""} />
          Enable nickname colors
        </label>
        <div class="abook-listbox" id="ab-colors-list">
          ${items}
        </div>
      </div>

      <div class="abook-actions-col">
        <button type="button" class="btn btn-secondary" id="btn-ab-colors-add">Add...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-colors-edit">Edit...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-colors-delete">Delete</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-colors-up">Up</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-colors-down">Down</button>
      </div>
    </div>
  `;
}

function renderHighlightTab() {
  const hl = state.highlight;
  const items = hl.rules.map((rule, idx) => {
    const sel = idx === state.selectedHighlightRuleIdx ? " is-selected" : "";
    return `
      <div class="abook-list-item${sel}" data-hl-idx="${idx}" style="color:${escapeHtml(rule.color)};">
        ${escapeHtml(rule.words)}
      </div>
    `;
  }).join("");

  return `
    <div class="abook-tab-body abook-list-grid">
      <div class="abook-list-left">
        <label class="options-toggle" style="margin-bottom:8px;">
          <input type="checkbox" id="ab-highlight-enable" ${hl.enabled ? "checked" : ""} />
          Enable highlight
        </label>
        <div class="abook-listbox" id="ab-highlight-list">
          ${items}
        </div>
      </div>

      <div class="abook-actions-col">
        <button type="button" class="btn btn-secondary" id="btn-ab-hl-add">Add...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-hl-edit" ${hl.rules.length === 0 ? "disabled" : ""}>Edit...</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-hl-delete" ${hl.rules.length === 0 ? "disabled" : ""}>Delete</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-hl-up" ${hl.rules.length <= 1 ? "disabled" : ""}>Up</button>
        <button type="button" class="btn btn-secondary" id="btn-ab-hl-down" ${hl.rules.length <= 1 ? "disabled" : ""}>Down</button>
      </div>
    </div>
  `;
}

function renderSubdialog() {
  if (state.editingHighlight) {
    const h = state.editingHighlight;
    return `
      <div class="options-submodal-overlay" id="submodal-backdrop">
        <div class="modal-card abook-subdialog" role="dialog" aria-modal="true" aria-labelledby="sub-title">
          <div class="abook-header">
            <div class="abook-title" id="sub-title">Edit Highlight</div>
            <button type="button" class="btn-icon-sm" id="btn-sub-close">✕</button>
          </div>

          <div class="abook-sub-body">
            <label style="font-size:12px; font-weight:600; margin-bottom:4px; display:block;">Highlight lines that contain these words:</label>
            <input type="text" id="sub-hl-words" class="form-input form-input-sm" value="${escapeHtml(h.words || "$me")}" style="width:100%; box-sizing:border-box; margin-bottom:10px;" />

            <label style="font-size:12px; margin-bottom:4px; display:block;">From these channels/nicknames:</label>
            <input type="text" id="sub-hl-targets" class="form-input form-input-sm" value="${escapeHtml(h.targets || "")}" style="width:100%; box-sizing:border-box; margin-bottom:10px;" />

            <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:10px;">
              <div style="display:flex; align-items:center; gap:8px;">
                <label style="font-size:12px;">Match on:</label>
                <select id="sub-hl-match" class="form-input form-input-sm" style="width:110px;">
                  <option value="Any" ${h.matchOn === "Any" ? "selected" : ""}>Any</option>
                  <option value="Nickname" ${h.matchOn === "Nickname" ? "selected" : ""}>Nickname</option>
                  <option value="Message" ${h.matchOn === "Message" ? "selected" : ""}>Message</option>
                </select>
              </div>
              <div style="display:flex; flex-direction:column; gap:4px;">
                <label class="options-toggle"><input type="checkbox" id="sub-hl-case" ${h.caseSensitive ? "checked" : ""} /> Case sensitive</label>
                <label class="options-toggle"><input type="checkbox" id="sub-hl-regex" ${h.regex ? "checked" : ""} /> Regex</label>
              </div>
            </div>

            <div style="display:flex; align-items:center; gap:10px; margin-bottom:10px;">
              <label style="font-size:12px;">Color:</label>
              <input type="color" id="sub-hl-color" value="${h.color || "#ff00ff"}" style="width:60px; height:24px; padding:0; border:1px solid var(--line-dark); cursor:pointer;" />
              <span id="sub-hl-color-preview" style="font-size:11px; color:${h.color || "#ff00ff"}; font-weight:600;">Sample Text</span>
            </div>

            <div style="margin-bottom:10px;">
              <label style="font-size:12px; display:block; margin-bottom:4px;">Play sound:</label>
              <div style="display:flex; gap:6px;">
                <input type="text" id="sub-hl-sound" class="form-input form-input-sm" value="${escapeHtml(h.sound || "No Sound")}" style="flex:1;" />
                <button type="button" class="btn btn-secondary" id="btn-sub-sound-test" disabled title="Not available yet" style="width:28px;">▶</button>
              </div>
            </div>

            <div style="display:flex; justify-content:space-between; margin-bottom:10px;">
              <div style="display:flex; align-items:center; gap:6px;">
                <label class="options-toggle"><input type="checkbox" id="sub-hl-flash" ${h.flash ? "checked" : ""} /> Flash message</label>
                <select id="sub-hl-flash-times" class="form-input form-input-sm" style="width:50px;">
                  ${["--", "1", "3", "5"].map((v) => `<option value="${v}" ${String(h.flashTimes ?? "--") === v ? "selected" : ""}>${v}</option>`).join("")}
                </select>
                <span style="font-size:11px; color:var(--dim);">times</span>
              </div>
              <div style="display:flex; align-items:center; gap:6px;">
                <label class="options-toggle"><input type="checkbox" id="sub-hl-tip" ${h.tip ? "checked" : ""} /> Tip message:</label>
                <select id="sub-hl-tip-secs" class="form-input form-input-sm" style="width:50px;">
                  ${["1", "3", "5", "10"].map((v) => `<option value="${v}" ${String(h.tipSeconds ?? "3") === v ? "selected" : ""}>${v}</option>`).join("")}
                </select>
                <span style="font-size:11px; color:var(--dim);">seconds</span>
              </div>
            </div>

            <label style="font-size:12px; margin-bottom:4px; display:block;">Message:</label>
            <input type="text" id="sub-hl-msg" class="form-input form-input-sm" value="${escapeHtml(h.message || "")}" style="width:100%; box-sizing:border-box;" />
          </div>

          <div class="abook-dialog-actions" style="margin-top:14px;">
            <button type="button" class="btn btn-primary" id="btn-sub-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="btn-sub-cancel">Cancel</button>
          </div>
        </div>
      </div>
    `;
  }

  if (state.editingRule) {
    const r = state.editingRule;
    return `
      <div class="options-submodal-overlay" id="submodal-backdrop">
        <div class="modal-card abook-subdialog" role="dialog" aria-modal="true" aria-labelledby="sub-title">
          <div class="abook-header">
            <div class="abook-title" id="sub-title">Edit Nickname Color Rule</div>
            <button type="button" class="btn-icon-sm" id="btn-sub-close">✕</button>
          </div>

          <div class="abook-sub-body">
            <label style="font-size:12px; font-weight:600; margin-bottom:4px; display:block;">Nickname or pattern:</label>
            <input type="text" id="sub-rule-pattern" class="form-input form-input-sm" value="${escapeHtml(r.pattern || "*")}" style="width:100%; box-sizing:border-box; margin-bottom:10px;" />

            <label style="font-size:12px; margin-bottom:4px; display:block;">Channel mode criteria (e.g. @, +, ~, %, none, or empty):</label>
            <input type="text" id="sub-rule-mode" class="form-input form-input-sm" value="${escapeHtml(r.mode || "")}" style="width:100%; box-sizing:border-box; margin-bottom:10px;" />

            <div style="display:flex; align-items:center; gap:10px; margin-bottom:10px;">
              <label style="font-size:12px;">Color:</label>
              <input type="color" id="sub-rule-color" value="${r.color || "#00fc00"}" style="width:60px; height:24px; padding:0; border:1px solid var(--line-dark); cursor:pointer;" />
              <span id="sub-rule-color-preview" style="font-size:11px; color:${r.color || "#00fc00"}; font-weight:600;">Sample Nickname</span>
            </div>
          </div>

          <div class="abook-dialog-actions" style="margin-top:14px;">
            <button type="button" class="btn btn-primary" id="btn-sub-rule-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="btn-sub-cancel">Cancel</button>
          </div>
        </div>
      </div>
    `;
  }

  return "";
}

function attachAddressBookEvents(container) {
  // Close / Backdrop
  const backdrop = container.querySelector("#abook-modal-backdrop");
  backdrop?.addEventListener("click", (e) => {
    if (e.target === backdrop) closeAddressBookModal();
  });
  container.querySelector("#btn-abook-close")?.addEventListener("click", closeAddressBookModal);
  container.querySelector("#btn-abook-cancel")?.addEventListener("click", closeAddressBookModal);

  // Tab switching
  container.querySelectorAll(".abook-tab-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      // Keep what was typed on the tab being left before the next one is drawn.
      syncActiveTabInputs(container);
      state.activeTab = btn.dataset.tab;
      renderAddressBookModal();
    });
  });

  // OK Button: save all
  container.querySelector("#btn-abook-ok")?.addEventListener("click", () => {
    syncActiveTabInputs(container);
    saveAddressBook();
    closeAddressBookModal({ keepChanges: true });
  });

  // Help Button
  container.querySelector("#btn-abook-help")?.addEventListener("click", () => {
    window.alert("Address Book Reference\n\nManage users, whois information, notify watchlist, ignore control filters, nickname colors, and highlight keywords.\n\nShortcut: Alt+B or /abook.");
  });

  // Subdialog close / cancel
  container.querySelector("#btn-sub-close")?.addEventListener("click", () => {
    state.editingHighlight = null;
    state.editingRule = null;
    renderAddressBookModal();
  });
  container.querySelector("#btn-sub-cancel")?.addEventListener("click", () => {
    state.editingHighlight = null;
    state.editingRule = null;
    renderAddressBookModal();
  });

  // A changed field is kept at once, so a later click on a list entry does not drop it.
  container.onchange = () => syncActiveTabInputs(container);

  // Tab-specific bindings
  if (state.activeTab === "users") bindUsersTabEvents(container);
  else if (state.activeTab === "whois") bindWhoisTabEvents(container);
  else if (state.activeTab === "notify") bindNotifyTabEvents(container);
  else if (state.activeTab === "control") bindControlTabEvents(container);
  else if (state.activeTab === "colors") bindColorsTabEvents(container);
  else if (state.activeTab === "highlight") bindHighlightTabEvents(container);

  // Subdialog OK bindings
  container.querySelector("#btn-sub-ok")?.addEventListener("click", () => {
    if (state.editingHighlight) {
      const h = state.editingHighlight;
      h.words = container.querySelector("#sub-hl-words")?.value || "$me";
      h.targets = container.querySelector("#sub-hl-targets")?.value || "";
      h.matchOn = container.querySelector("#sub-hl-match")?.value || "Any";
      h.caseSensitive = container.querySelector("#sub-hl-case")?.checked || false;
      h.regex = container.querySelector("#sub-hl-regex")?.checked || false;
      h.color = container.querySelector("#sub-hl-color")?.value || "#ff00ff";
      h.sound = container.querySelector("#sub-hl-sound")?.value || "No Sound";
      h.flash = container.querySelector("#sub-hl-flash")?.checked || false;
      h.flashTimes = container.querySelector("#sub-hl-flash-times")?.value || "--";
      h.tip = container.querySelector("#sub-hl-tip")?.checked || false;
      h.tipSeconds = container.querySelector("#sub-hl-tip-secs")?.value || "3";
      h.message = container.querySelector("#sub-hl-msg")?.value || "";

      if (h.isNew) {
        state.highlight.rules.push(h);
      } else {
        state.highlight.rules[state.selectedHighlightRuleIdx] = h;
      }
      state.editingHighlight = null;
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#sub-hl-color")?.addEventListener("input", (e) => {
    const preview = container.querySelector("#sub-hl-color-preview");
    if (preview) preview.style.color = e.target.value;
  });

  container.querySelector("#btn-sub-rule-ok")?.addEventListener("click", () => {
    if (state.editingRule) {
      const r = state.editingRule;
      r.pattern = container.querySelector("#sub-rule-pattern")?.value || "*";
      r.mode = container.querySelector("#sub-rule-mode")?.value || "";
      r.color = container.querySelector("#sub-rule-color")?.value || "#00fc00";
      r.label = r.pattern + (r.mode ? ` (${r.mode})` : "");

      if (r.isNew) {
        state.colors.rules.push(r);
      } else {
        state.colors.rules[state.selectedColorRuleIdx] = r;
      }
      state.editingRule = null;
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#sub-rule-color")?.addEventListener("input", (e) => {
    const preview = container.querySelector("#sub-rule-color-preview");
    if (preview) preview.style.color = e.target.value;
  });
}

function syncActiveTabInputs(container) {
  if (state.activeTab === "whois") {
    const whoisNick = container.querySelector("#ab-whois-nick")?.value.trim();
    if (whoisNick) state.whois.nick = whoisNick;
  } else if (state.activeTab === "users") {
    const nick = container.querySelector("#ab-nick")?.value.trim() || "";
    if (nick) {
      const u = {
        nick,
        name: container.querySelector("#ab-name")?.value || "",
        email: container.querySelector("#ab-email")?.value || "",
        website: container.querySelector("#ab-website")?.value || "",
        address: container.querySelector("#ab-address")?.value || "",
        notes: container.querySelector("#ab-notes")?.value || "",
        picture: state.users[state.selectedUserIdx]?.picture || ""
      };
      if (state.users[state.selectedUserIdx]) {
        state.users[state.selectedUserIdx] = u;
      } else {
        state.users.push(u);
      }
    }
  } else if (state.activeTab === "notify") {
    state.notify.enabled = container.querySelector("#ab-notify-enable")?.checked ?? true;
  } else if (state.activeTab === "control") {
    state.control.enabled = container.querySelector("#ab-control-enable")?.checked ?? true;
    state.control.randomDelay = container.querySelector("#ab-control-random")?.checked ?? false;
    state.control.type = container.querySelector("#ab-control-type")?.value || "Ignore";
  } else if (state.activeTab === "colors") {
    state.colors.enabled = container.querySelector("#ab-colors-enable")?.checked ?? true;
  } else if (state.activeTab === "highlight") {
    state.highlight.enabled = container.querySelector("#ab-highlight-enable")?.checked ?? true;
  }
}

function bindUsersTabEvents(container) {
  container.querySelector("#ab-nick-select")?.addEventListener("change", (e) => {
    syncActiveTabInputs(container);
    state.selectedUserIdx = parseInt(e.target.value, 10);
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-add")?.addEventListener("click", () => {
    const nick = container.querySelector("#ab-nick")?.value.trim();
    if (!nick) {
      window.alert("Please enter a nickname.");
      return;
    }
    const newUser = {
      nick,
      name: container.querySelector("#ab-name")?.value || "",
      email: container.querySelector("#ab-email")?.value || "",
      website: container.querySelector("#ab-website")?.value || "",
      address: container.querySelector("#ab-address")?.value || "",
      notes: container.querySelector("#ab-notes")?.value || "",
      picture: ""
    };
    const existing = state.users[state.selectedUserIdx];
    if (existing && existing.nick === newUser.nick && existing.name === newUser.name && existing.email === newUser.email && existing.website === newUser.website && existing.address === newUser.address && existing.notes === newUser.notes) {
      // onchange on blur already created this exact entry when the list was empty.
      saveAddressBook();
      renderAddressBookModal();
      return;
    }
    state.users.push(newUser);
    state.selectedUserIdx = state.users.length - 1;
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-delete")?.addEventListener("click", () => {
    if (state.users.length === 0) return;
    state.users.splice(state.selectedUserIdx, 1);
    if (state.selectedUserIdx >= state.users.length) state.selectedUserIdx = Math.max(0, state.users.length - 1);
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-email")?.addEventListener("click", () => {
    const email = container.querySelector("#ab-email")?.value;
    if (email) window.open(`mailto:${email}`);
  });

  container.querySelector("#btn-ab-visit")?.addEventListener("click", () => {
    let url = container.querySelector("#ab-website")?.value || "";
    if (url && !url.startsWith("http://") && !url.startsWith("https://")) url = `https://${url}`;
    if (url) window.open(url, "_blank");
  });

  container.querySelector("#btn-ab-chat")?.addEventListener("click", () => {
    const nick = container.querySelector("#ab-nick")?.value.trim();
    if (nick && sendCommandCallback) {
      // The contact's edits are kept: store them, then close without the cancel rollback.
      syncActiveTabInputs(container);
      saveAddressBook();
      closeAddressBookModal({ keepChanges: true });
      sendCommandCallback(`/query ${nick}`);
    }
  });

  container.querySelector("#btn-ab-whois")?.addEventListener("click", () => {
    syncActiveTabInputs(container);
    const nick = container.querySelector("#ab-nick")?.value.trim();
    if (nick && sendCommandCallback) {
      state.activeTab = "whois";
      state.whois.nick = nick;
      sendCommandCallback(`/whois ${nick}`);
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-notify")?.addEventListener("click", () => {
    const nick = container.querySelector("#ab-nick")?.value.trim();
    if (nick) {
      state.notify.entries.push({ nick, network: "*", note: "From Address Book" });
      saveAddressBook();
      state.activeTab = "notify";
      renderAddressBookModal();
    }
  });

  container.querySelector("#ab-picture")?.addEventListener("click", () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.onchange = (e) => {
      const file = e.target.files?.[0];
      if (file) {
        const reader = new FileReader();
        reader.onload = (ev) => {
          if (state.users[state.selectedUserIdx]) {
            state.users[state.selectedUserIdx].picture = ev.target.result;
            saveAddressBook();
            renderAddressBookModal();
          }
        };
        reader.readAsDataURL(file);
      }
    };
    input.click();
  });
}

function bindWhoisTabEvents(container) {
  const doWhois = () => {
    const nick = container.querySelector("#ab-whois-nick")?.value.trim();
    if (nick && sendCommandCallback) {
      state.whois.nick = nick;
      sendCommandCallback(`/whois ${nick}`);
    }
  };

  container.querySelector("#btn-ab-whois-query")?.addEventListener("click", doWhois);

  container.querySelector("#btn-ab-whois-add")?.addEventListener("click", () => {
    if (state.whois.nick) {
      state.users.push({
        nick: state.whois.nick,
        name: state.whois.name || "",
        email: "",
        website: "",
        address: state.whois.address || "",
        notes: `Server: ${state.whois.server || ""}\nChannels: ${state.whois.channels || ""}`,
        picture: ""
      });
      state.selectedUserIdx = state.users.length - 1;
      state.activeTab = "users";
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-whois-copy")?.addEventListener("click", () => {
    const w = state.whois;
    const text = `Nickname: ${w.nick}\nName: ${w.name}\nAddress: ${w.address}\nChannels: ${w.channels}\nServer: ${w.server}\nIdle: ${w.idle}`;
    navigator.clipboard.writeText(text);
    window.alert("Whois info copied to clipboard.");
  });

  container.querySelector("#btn-ab-ping")?.addEventListener("click", () => {
    if (state.whois.nick && sendCommandCallback) sendCommandCallback(`/ctcp ${state.whois.nick} PING`);
  });
  container.querySelector("#btn-ab-time")?.addEventListener("click", () => {
    if (state.whois.nick && sendCommandCallback) sendCommandCallback(`/ctcp ${state.whois.nick} TIME`);
  });
  container.querySelector("#btn-ab-version")?.addEventListener("click", () => {
    if (state.whois.nick && sendCommandCallback) sendCommandCallback(`/ctcp ${state.whois.nick} VERSION`);
  });
  container.querySelector("#btn-ab-finger")?.addEventListener("click", () => {
    if (state.whois.nick && sendCommandCallback) sendCommandCallback(`/ctcp ${state.whois.nick} FINGER`);
  });
}

function bindNotifyTabEvents(container) {
  container.querySelectorAll(".abook-table-row").forEach((row) => {
    row.addEventListener("click", () => {
      state.selectedNotifyIdx = parseInt(row.dataset.notifyIdx, 10);
      renderAddressBookModal();
    });
  });

  container.querySelector("#btn-ab-notify-add")?.addEventListener("click", () => {
    const nick = window.prompt("Enter nickname to add to Notify list:");
    if (nick && nick.trim()) {
      state.notify.entries.push({ nick: nick.trim(), network: "*", note: "" });
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-notify-delete")?.addEventListener("click", () => {
    if (state.notify.entries.length === 0) return;
    state.notify.entries.splice(state.selectedNotifyIdx, 1);
    if (state.selectedNotifyIdx >= state.notify.entries.length) {
      state.selectedNotifyIdx = Math.max(0, state.notify.entries.length - 1);
    }
    saveAddressBook();
    renderAddressBookModal();
  });
}

// Indices in the full list of the entries of the chosen control type, in list order. Selection and
// the Up, Down, Delete and Sort actions work on these only.
function controlTypeIndices() {
  const c = state.control;
  return c.entries.reduce((acc, entry, idx) => {
    if ((entry.type || "Ignore") === c.type) acc.push(idx);
    return acc;
  }, []);
}

function bindControlTabEvents(container) {
  // The dropdown picks the list to show; a change re-renders at once.
  container.querySelector("#ab-control-type")?.addEventListener("change", (e) => {
    state.control.type = e.target.value;
    state.selectedControlIdx = -1;
    renderAddressBookModal();
  });

  container.querySelectorAll(".abook-list-item").forEach((item) => {
    item.addEventListener("click", () => {
      state.selectedControlIdx = parseInt(item.dataset.controlIdx, 10);
      renderAddressBookModal();
    });
  });

  container.querySelector("#btn-ab-control-add")?.addEventListener("click", () => {
    const mask = window.prompt("Enter hostmask or nickname to ignore (e.g. *!*@*.badhost.com):");
    if (mask && mask.trim()) {
      // An entry belongs to the control type it was added under; the dropdown only picks which list shows.
      state.control.entries.push({ mask: mask.trim(), network: "*", flags: "pcntdikshfy", type: state.control.type || "Ignore" });
      state.selectedControlIdx = state.control.entries.length - 1;
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-control-delete")?.addEventListener("click", () => {
    // Only a selected entry of the shown type can be deleted.
    if (!controlTypeIndices().includes(state.selectedControlIdx)) return;
    state.control.entries.splice(state.selectedControlIdx, 1);
    state.selectedControlIdx = -1;
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-control-up")?.addEventListener("click", () => {
    // Swaps with the previous entry of the shown type; other types stay where they are.
    const indices = controlTypeIndices();
    const pos = indices.indexOf(state.selectedControlIdx);
    if (pos <= 0) return;
    const list = state.control.entries;
    const [a, b] = [indices[pos - 1], indices[pos]];
    [list[a], list[b]] = [list[b], list[a]];
    state.selectedControlIdx = a;
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-control-down")?.addEventListener("click", () => {
    // Swaps with the next entry of the shown type; other types stay where they are.
    const indices = controlTypeIndices();
    const pos = indices.indexOf(state.selectedControlIdx);
    if (pos < 0 || pos >= indices.length - 1) return;
    const list = state.control.entries;
    const [a, b] = [indices[pos], indices[pos + 1]];
    [list[a], list[b]] = [list[b], list[a]];
    state.selectedControlIdx = b;
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-control-sort")?.addEventListener("click", () => {
    // Sorts the shown type only, into the same slots; other types stay where they are.
    const indices = controlTypeIndices();
    const list = state.control.entries;
    const sorted = indices.map((i) => list[i]).sort((a, b) => a.mask.localeCompare(b.mask));
    indices.forEach((idx, k) => { list[idx] = sorted[k]; });
    state.selectedControlIdx = -1;
    saveAddressBook();
    renderAddressBookModal();
  });
}

function bindColorsTabEvents(container) {
  container.querySelectorAll("#ab-colors-list .abook-list-item").forEach((item) => {
    item.addEventListener("click", () => {
      state.selectedColorRuleIdx = parseInt(item.dataset.ruleIdx, 10);
      renderAddressBookModal();
    });
  });

  container.querySelector("#btn-ab-colors-add")?.addEventListener("click", () => {
    state.editingRule = {
      isNew: true,
      pattern: "*",
      mode: "",
      color: "#00fc00"
    };
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-colors-edit")?.addEventListener("click", () => {
    const rule = state.colors.rules[state.selectedColorRuleIdx];
    if (rule) {
      state.editingRule = { ...rule, isNew: false };
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-colors-delete")?.addEventListener("click", () => {
    if (state.colors.rules.length <= 1) return;
    state.colors.rules.splice(state.selectedColorRuleIdx, 1);
    if (state.selectedColorRuleIdx >= state.colors.rules.length) {
      state.selectedColorRuleIdx = Math.max(0, state.colors.rules.length - 1);
    }
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-colors-up")?.addEventListener("click", () => {
    const idx = state.selectedColorRuleIdx;
    if (idx > 0) {
      const temp = state.colors.rules[idx];
      state.colors.rules[idx] = state.colors.rules[idx - 1];
      state.colors.rules[idx - 1] = temp;
      state.selectedColorRuleIdx = idx - 1;
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-colors-down")?.addEventListener("click", () => {
    const idx = state.selectedColorRuleIdx;
    if (idx < state.colors.rules.length - 1) {
      const temp = state.colors.rules[idx];
      state.colors.rules[idx] = state.colors.rules[idx + 1];
      state.colors.rules[idx + 1] = temp;
      state.selectedColorRuleIdx = idx + 1;
      saveAddressBook();
      renderAddressBookModal();
    }
  });
}

function bindHighlightTabEvents(container) {
  container.querySelectorAll("#ab-highlight-list .abook-list-item").forEach((item) => {
    item.addEventListener("click", () => {
      state.selectedHighlightRuleIdx = parseInt(item.dataset.hlIdx, 10);
      renderAddressBookModal();
    });
  });

  container.querySelector("#btn-ab-hl-add")?.addEventListener("click", () => {
    state.editingHighlight = {
      isNew: true,
      words: "",
      targets: "",
      matchOn: "Any",
      caseSensitive: false,
      regex: false,
      color: "#ff00ff",
      sound: "No Sound",
      flash: false,
      flashTimes: "--",
      tip: false,
      tipSeconds: "3",
      message: ""
    };
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-hl-edit")?.addEventListener("click", () => {
    const rule = state.highlight.rules[state.selectedHighlightRuleIdx];
    if (rule) {
      state.editingHighlight = { ...rule, isNew: false };
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-hl-delete")?.addEventListener("click", () => {
    if (state.highlight.rules.length <= 1) return;
    state.highlight.rules.splice(state.selectedHighlightRuleIdx, 1);
    if (state.selectedHighlightRuleIdx >= state.highlight.rules.length) {
      state.selectedHighlightRuleIdx = Math.max(0, state.highlight.rules.length - 1);
    }
    saveAddressBook();
    renderAddressBookModal();
  });

  container.querySelector("#btn-ab-hl-up")?.addEventListener("click", () => {
    const idx = state.selectedHighlightRuleIdx;
    if (idx > 0) {
      const temp = state.highlight.rules[idx];
      state.highlight.rules[idx] = state.highlight.rules[idx - 1];
      state.highlight.rules[idx - 1] = temp;
      state.selectedHighlightRuleIdx = idx - 1;
      saveAddressBook();
      renderAddressBookModal();
    }
  });

  container.querySelector("#btn-ab-hl-down")?.addEventListener("click", () => {
    const idx = state.selectedHighlightRuleIdx;
    if (idx < state.highlight.rules.length - 1) {
      const temp = state.highlight.rules[idx];
      state.highlight.rules[idx] = state.highlight.rules[idx + 1];
      state.highlight.rules[idx + 1] = temp;
      state.selectedHighlightRuleIdx = idx + 1;
      saveAddressBook();
      renderAddressBookModal();
    }
  });
}

const patternRegexCache = new Map();
function getCachedRegex(pattern, flags = "i") {
  const key = `${flags}|${pattern}`;
  let re = patternRegexCache.get(key);
  if (!re) {
    if (patternRegexCache.size >= 500) patternRegexCache.clear();
    re = new RegExp(pattern, flags);
    patternRegexCache.set(key, re);
  }
  return re;
}

export function matchNickColor(user, ownNick = "") {
  if (!state.colors || !state.colors.enabled || !state.colors.rules) return null;
  const nick = user.nick || "";
  const prefix = user.prefix || "";
  const isMe = ownNick && nick.toLowerCase() === ownNick.toLowerCase();

  for (const rule of state.colors.rules) {
    if (rule.pattern === "$me" || rule.id === "me" || rule.label === "$me") {
      if (isMe) return rule.color;
      continue;
    }
    // Check mode
    if (rule.mode) {
      if (rule.mode === "none" || rule.mode === "No mode") {
        if (prefix) continue; // has a mode
      } else {
        let match = false;
        for (const ch of rule.mode) {
          if (ch === prefix) {
            match = true;
            break;
          }
        }
        if (!match) continue;
      }
    }
    // Check pattern wildcard if specified
    if (rule.pattern && rule.pattern !== "*") {
      const regexStr = "^" + rule.pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$";
      try {
        if (!getCachedRegex(regexStr, "i").test(nick)) continue;
      } catch {
        continue;
      }
    }
    return rule.color;
  }
  return null;
}

export function checkAddressBookHighlight(text, sender = "", target = "", ownNick = "") {
  if (!state.highlight || !state.highlight.enabled || !state.highlight.rules) {
    return { matched: false, color: null };
  }
  const isMe = ownNick && sender.toLowerCase() === ownNick.toLowerCase();
  if (isMe) return { matched: false, color: null };

  for (const rule of state.highlight.rules) {
    if (rule.targets && rule.targets.trim()) {
      const allowed = rule.targets.split(",").map(t => t.trim().toLowerCase());
      if (target && !allowed.includes(target.toLowerCase())) continue;
    }

    const words = (rule.words || "").trim();
    if (!words) continue;

    let targetText = "";
    if (rule.matchOn === "Nickname") targetText = sender;
    else if (rule.matchOn === "Message") targetText = text;
    else targetText = `${sender} ${text}`;

    let matched = false;
    if (rule.regex) {
      try {
        const flags = rule.caseSensitive ? "" : "i";
        matched = getCachedRegex(words, flags).test(targetText);
      } catch { }
    } else {
      const searchWords = words.split(" ").filter(Boolean);
      for (let w of searchWords) {
        if (w === "$me" || w === "$nick") {
          if (!ownNick) continue;
          w = ownNick;
        }
        if (rule.caseSensitive) {
          if (targetText.includes(w)) { matched = true; break; }
        } else {
          if (targetText.toLowerCase().includes(w.toLowerCase())) { matched = true; break; }
        }
      }
    }

    if (matched) {
      return {
        matched: true,
        color: rule.color || "#ff00ff",
        sound: rule.sound,
        flash: rule.flash,
        tip: rule.tip,
        message: rule.message
      };
    }
  }
  return { matched: false, color: null };
}

const ignoreRegexCache = new Map();
// `host` is the sender's host when the channel list knows it; without it the user part is `*`.
export function isUserIgnored(sender, network = "", host = "") {
  if (!state.control || !state.control.enabled) return false;
  if (!sender) return false;
  const s = sender.toLowerCase();
  for (const entry of (state.control.entries || [])) {
    // Only Ignore entries ignore; "*" means every network, not a network named "*".
    if (entry.type && entry.type !== "Ignore") continue;
    if (entry.network && entry.network !== "*" && network && entry.network.toLowerCase() !== network.toLowerCase()) continue;
    const mask = (entry.mask || "").toLowerCase();
    const regexStr = "^" + mask.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$";
    try {
      // Every message row asks this, so each mask is compiled once, not once per row.
      let re = ignoreRegexCache.get(regexStr);
      if (!re) {
        if (ignoreRegexCache.size >= 500) ignoreRegexCache.clear();
        re = new RegExp(regexStr);
        ignoreRegexCache.set(regexStr, re);
      }
      if (re.test(s) || re.test(`${s}!*@${host || "*"}`)) return true;
    } catch { }
  }
  return false;
}
