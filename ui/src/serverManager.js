import { escapeHtml } from "./utils/ircColors.js";
import "./serverManager.css";

// Server manager (Tools menu): edits the `server` lines of connectd.mrc as a list, so nobody has to
// write mrc text. Every other line of the file is kept by the backend.

const isTauri = window.__TAURI_INTERNALS__ !== undefined;

const view = {
  open: false,
  file: "connectd.mrc",
  rows: [],
  // Startup order and network names, from ferric-servers.txt next to connectd.mrc.
  order: [],
  mtime: null,
  // While the entry form is shown: index of the edited row (null = new) and the values typed.
  form: null,
  error: "",
  notice: "",
  busy: false,
};

async function call(cmd, args = {}) {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke(cmd, args);
}

export function isServerManagerOpen() {
  return view.open;
}

export async function openServerManager() {
  view.open = true;
  view.form = null;
  view.error = "";
  view.notice = "";
  renderServerManager();
  await reload();
}

export function closeServerManager() {
  view.open = false;
  view.form = null;
  const container = document.getElementById("modal-container");
  if (container) container.innerHTML = "";
}

function applyList(result) {
  view.file = result.file;
  view.rows = result.rows;
  view.mtime = result.mtime;
}

async function reload() {
  view.form = null;
  if (!isTauri) {
    view.error = "The server manager is available in the desktop app.";
  } else {
    try {
      applyList(await call("get_server_list"));
      view.order = await call("get_server_order");
      view.error = "";
    } catch (e) {
      view.error = String(e);
    }
  }
  renderServerManager();
}

// Writes the whole list. The backend refuses it when the file changed on disk since the last read.
async function saveRows(rows, notice) {
  view.busy = true;
  renderServerManager();
  try {
    applyList(await call("save_server_list", { rows, expectedMtime: view.mtime }));
    view.form = null;
    view.error = "";
    view.notice = notice;
  } catch (e) {
    view.error = String(e);
    view.notice = "";
  } finally {
    view.busy = false;
    renderServerManager();
  }
}

function blankRow() {
  return { host: "", port: 6667, use_tls: false, password: null, nick: "", altnick: "", username: "", realname: "" };
}

// Matches a row to its saved order entry: host, port and ident, as the backend matches them.
function rowKey(row) {
  return `${row.host.toLowerCase()}:${row.port}:${(row.username || "").toLowerCase()}`;
}

function metaKey(entry) {
  return `${entry.host.toLowerCase()}:${entry.port}:${(entry.ident || "").toLowerCase()}`;
}

function nameOf(row) {
  const key = rowKey(row);
  return view.order.find((entry) => metaKey(entry) === key)?.name ?? "";
}

// The network name the backend uses when none is set: the first host label and the ident.
function defaultNetworkName(row) {
  return `${row.host.split(".")[0]}-${row.username || row.nick}`;
}

// Rows in startup order: the saved places first, then rows the order does not know, in file order.
function orderedRows() {
  const rank = new Map(view.order.map((entry, i) => [metaKey(entry), i]));
  const place = (item) => rank.get(rowKey(item.row)) ?? Number.MAX_SAFE_INTEGER;
  return view.rows
    .map((row, index) => ({ row, index }))
    .sort((a, b) => place(a) - place(b) || a.index - b.index);
}

// Whether Ferric connects this server at startup. A server the order does not know yet is on.
function enabledOf(row) {
  const key = rowKey(row);
  const entry = view.order.find((item) => metaKey(item) === key);
  return entry ? entry.enabled !== false : true;
}

// Saves the full order with its names and start flags. `rows` is the complete list in the wanted order.
async function saveOrder(rows, names = new Map(), starts = new Map()) {
  const entries = rows.map((row) => ({
    host: row.host,
    port: row.port,
    ident: row.username || "",
    name: names.has(rowKey(row)) ? names.get(rowKey(row)) : nameOf(row),
    enabled: starts.has(rowKey(row)) ? starts.get(rowKey(row)) : enabledOf(row),
  }));
  await call("save_server_order", { entries });
  view.order = entries;
}

// Switches a server on or off for the next start. Other rows keep their settings.
async function toggleStart(position) {
  const rows = orderedRows().map((item) => item.row);
  const row = rows[position];
  if (!row) return;
  view.busy = true;
  renderServerManager();
  try {
    await saveOrder(rows, new Map(), new Map([[rowKey(row), !enabledOf(row)]]));
    view.error = "";
    view.notice = "Start setting saved. It applies at the next start.";
  } catch (e) {
    view.error = String(e);
  } finally {
    view.busy = false;
    renderServerManager();
  }
}

async function moveRow(position, direction) {
  const rows = orderedRows().map((item) => item.row);
  const target = position + direction;
  if (target < 0 || target >= rows.length) return;
  [rows[position], rows[target]] = [rows[target], rows[position]];
  view.busy = true;
  renderServerManager();
  try {
    await saveOrder(rows);
    view.error = "";
    view.notice = "Order saved. Ferric connects in this order at the next start.";
  } catch (e) {
    view.error = String(e);
  } finally {
    view.busy = false;
    renderServerManager();
  }
}

function readFormRow() {
  const val = (id) => document.getElementById(id)?.value ?? "";
  const host = val("srv-host").trim();
  const port = Number(val("srv-port"));
  const nick = val("srv-nick").trim();
  if (!host) return { error: "Host is required." };
  if (!Number.isInteger(port) || port < 1 || port > 65535) return { error: "Port must be between 1 and 65535." };
  if (!nick) return { error: "Nick is required." };
  const password = val("srv-password");
  return {
    row: {
      host,
      port,
      use_tls: document.getElementById("srv-tls")?.checked === true,
      password: password === "" ? null : password,
      nick,
      altnick: val("srv-altnick").trim() || `${nick}_`,
      username: val("srv-ident").trim() || nick,
      realname: val("srv-realname").trim() || nick,
    },
  };
}

// What the form shows right now, unchecked: kept when a value is refused, so typing is not lost.
function formDraft() {
  const val = (id) => document.getElementById(id)?.value ?? "";
  const password = val("srv-password");
  return {
    host: val("srv-host"),
    port: val("srv-port"),
    use_tls: document.getElementById("srv-tls")?.checked === true,
    password: password === "" ? null : password,
    nick: val("srv-nick"),
    altnick: val("srv-altnick"),
    username: val("srv-ident"),
    realname: val("srv-realname"),
  };
}

async function submitForm() {
  const form = view.form;
  if (!form) return;
  const network = (document.getElementById("srv-network")?.value ?? "").trim();
  const { row, error } = readFormRow();
  if (error) {
    form.row = formDraft();
    form.network = network;
    view.error = error;
    renderServerManager();
    return;
  }
  // The typed values stay in the form if the save is refused.
  const oldRow = form.index !== null ? view.rows[form.index] : null;
  const oldKey = oldRow ? rowKey(oldRow) : null;
  const oldEnabled = oldRow ? enabledOf(oldRow) : true;
  const oldPosition = oldRow ? orderedRows().findIndex((item) => item.index === form.index) : -1;
  form.row = row;
  form.network = network;
  const rows = view.rows.slice();
  if (form.index === null) rows.push(row);
  else rows[form.index] = row;
  await saveRows(rows, `Saved ${row.host}:${row.port}.`);
  if (view.error) return;
  try {
    // Preserve startup order and enabled flag when host/port/ident changed on an edited row.
    const ordered = orderedRows().map((item) => item.row);
    if (oldKey && oldKey !== rowKey(row) && oldPosition >= 0) {
      const curPos = ordered.indexOf(view.rows[form.index]);
      if (curPos >= 0 && curPos !== oldPosition) {
        const [moved] = ordered.splice(curPos, 1);
        ordered.splice(oldPosition, 0, moved);
      }
    }
    await saveOrder(ordered, new Map([[rowKey(row), network]]), new Map([[rowKey(row), oldEnabled]]));
  } catch (e) {
    view.error = String(e);
    renderServerManager();
  }
}

async function onAction(action, index) {
  if (action === "reload") return reload();
  if (action === "add") {
    view.form = { index: null, row: blankRow(), network: "" };
    view.error = "";
    view.notice = "";
    return renderServerManager();
  }
  const i = Number(index);
  const row = view.rows[i];
  if (!row) return;
  if (action === "edit") {
    view.form = { index: i, row: { ...row }, network: nameOf(row) };
    view.error = "";
    view.notice = "";
    return renderServerManager();
  }
  if (action === "delete") {
    if (!window.confirm(`Delete ${row.host}:${row.port} from ${view.file}?`)) return;
    view.form = null;
    return saveRows(view.rows.filter((_, k) => k !== i), "Server deleted.");
  }
}

function renderForm(form) {
  const r = form.row;
  const disabled = view.busy ? "disabled" : "";
  return `
    <form class="server-form" id="server-form" autocomplete="off">
      <div class="server-form-title">${form.index === null ? "New server" : "Edit server"}</div>
      <div class="server-form-grid">
        <label>Host
          <input id="srv-host" class="form-input" value="${escapeHtml(r.host)}" placeholder="irc.example.org" />
        </label>
        <label>Port
          <input id="srv-port" class="form-input" type="number" min="1" max="65535" value="${escapeHtml(String(r.port))}" />
        </label>
        <label class="server-check">
          <input id="srv-tls" type="checkbox" ${r.use_tls ? "checked" : ""} /> Use TLS
        </label>
        <label>Password (optional)
          <span class="server-password">
            <input id="srv-password" class="form-input" type="password" value="${escapeHtml(r.password ?? "")}" />
            <button type="button" class="btn-icon-sm" id="btn-srv-show">Show</button>
          </span>
        </label>
        <label>Nick
          <input id="srv-nick" class="form-input" value="${escapeHtml(r.nick)}" />
        </label>
        <label>Alt nick
          <input id="srv-altnick" class="form-input" value="${escapeHtml(r.altnick)}" placeholder="nick_ if empty" />
        </label>
        <label>Ident
          <input id="srv-ident" class="form-input" value="${escapeHtml(r.username)}" placeholder="nick if empty" />
        </label>
        <label>Network name (optional)
          <input id="srv-network" class="form-input" value="${escapeHtml(form.network || "")}" placeholder="${escapeHtml(r.host ? defaultNetworkName(r) : "")}" />
        </label>
        <label>Real name (optional)
          <input id="srv-realname" class="form-input" value="${escapeHtml(r.realname)}" />
        </label>
      </div>
      <div class="server-form-buttons">
        <button type="button" class="btn" id="btn-srv-cancel" ${disabled}>Cancel</button>
        <button type="submit" class="btn btn-primary" ${disabled}>Save server</button>
      </div>
    </form>`;
}

export function renderServerManager() {
  const container = document.getElementById("modal-container");
  if (!container || !view.open) return;
  const disabled = view.busy ? "disabled" : "";
  const count = view.rows.length;
  const ordered = orderedRows();
  const rowsHtml = count === 0
    ? `<tr><td colspan="8" class="server-empty">No servers in ${escapeHtml(view.file)} yet.</td></tr>`
    : ordered.map(({ row, index }, position) => {
        const name = nameOf(row);
        const networkCell = name
          ? escapeHtml(name)
          : `<span class="server-default-name">${escapeHtml(defaultNetworkName(row))}</span>`;
        return `
        <tr>
          <td class="server-order">
            <button class="btn-icon-sm" data-move="-1" data-position="${position}" ${disabled || position === 0 ? "disabled" : ""}>Up</button>
            <button class="btn-icon-sm" data-move="1" data-position="${position}" ${disabled || position === count - 1 ? "disabled" : ""}>Down</button>
          </td>
          <td class="server-start">
            <input type="checkbox" data-start="${position}" title="Connect on start" ${enabledOf(row) ? "checked" : ""} ${disabled} />
          </td>
          <td class="server-host">${escapeHtml(row.host)}:${row.use_tls ? "+" : ""}${row.port}</td>
          <td class="server-network">${networkCell}</td>
          <td>${row.use_tls ? "TLS" : "Plain"}</td>
          <td>${escapeHtml(row.nick)}</td>
          <td>${escapeHtml(row.username)}</td>
          <td class="server-actions">
            <button class="btn-icon-sm" data-action="edit" data-index="${index}" ${disabled}>Edit</button>
            <button class="btn-icon-sm btn-icon-danger" data-action="delete" data-index="${index}" ${disabled}>Delete</button>
          </td>
        </tr>`;
      }).join("");

  container.innerHTML = `
    <div class="modal-overlay" id="server-modal-backdrop">
      <div class="server-modal-card">
        <div class="server-modal-header">
          <div>
            <div class="server-modal-title">Server manager</div>
            <div class="server-modal-path">${escapeHtml(view.file)} · only server lines change; comments and other lines stay</div>
          </div>
          <button id="btn-server-close" class="btn-icon-sm" title="Close">✕</button>
        </div>
        <div class="server-modal-body">
          <div class="server-toolbar">
            <span>${count} server${count === 1 ? "" : "s"}</span>
            <span class="server-toolbar-buttons">
              <button class="btn" data-action="reload" ${disabled}>Reload</button>
              <button class="btn btn-primary" data-action="add" ${disabled || (view.form ? "disabled" : "")}>Add server</button>
            </span>
          </div>
          ${view.error ? `<div class="server-message server-error" role="alert">${escapeHtml(view.error)}</div>` : ""}
          ${!view.error && view.notice ? `<div class="server-message">${escapeHtml(view.notice)}</div>` : ""}
          <div class="server-table-wrap">
            <table class="server-table">
              <thead>
                <tr><th>Order</th><th>Start</th><th>Server</th><th>Network name</th><th>Connection</th><th>Nick</th><th>Ident</th><th></th></tr>
              </thead>
              <tbody>${rowsHtml}</tbody>
            </table>
          </div>
          ${view.form ? renderForm(view.form) : ""}
        </div>
      </div>
    </div>`;
  attachServerManagerHandlers(container);
}

function attachServerManagerHandlers(container) {
  document.getElementById("btn-server-close")?.addEventListener("click", closeServerManager);
  document.getElementById("server-modal-backdrop")?.addEventListener("click", (e) => {
    if (e.target.id === "server-modal-backdrop") closeServerManager();
  });
  container.querySelectorAll("[data-action]").forEach((button) => {
    button.addEventListener("click", () => onAction(button.dataset.action, button.dataset.index));
  });
  container.querySelectorAll("[data-start]").forEach((input) => {
    input.addEventListener("change", () => toggleStart(Number(input.dataset.start)));
  });
  container.querySelectorAll("[data-move]").forEach((button) => {
    button.addEventListener("click", () => moveRow(Number(button.dataset.position), Number(button.dataset.move)));
  });
  document.getElementById("btn-srv-cancel")?.addEventListener("click", () => {
    view.form = null;
    view.error = "";
    renderServerManager();
  });
  document.getElementById("btn-srv-show")?.addEventListener("click", (e) => {
    const input = document.getElementById("srv-password");
    const shown = input.type === "text";
    input.type = shown ? "password" : "text";
    e.currentTarget.textContent = shown ? "Show" : "Hide";
  });
  document.getElementById("server-form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    submitForm();
  });
}
