import { escapeHtml, parseIrcToHtml } from "./utils/ircColors.js";

// Channel Central for one channel, opened by double-clicking the topic bar or an empty part of
// the chat. Tabs: Topic, Channel modes, and three list tabs (Bans, Excepts, Invites). Statistics
// come later. main.js passes in fresh data after every sync; the dialog only redraws when what it
// shows has changed, so clicks are not lost to a redraw.

const FLAGS = [
  ["i", "Invite only"],
  ["m", "Moderated"],
  ["n", "No outside messages"],
  ["p", "Private"],
  ["s", "Secret"],
  ["t", "Topic locked to operators"],
];

// Each list tab: the mode letter and its texts.
const LIST_TABS = {
  bans: {
    letter: "b",
    title: "Ban list",
    empty: "No bans listed yet.",
    placeholder: "*!*@host",
    note: "Use masks like *!*@host. Only operators can change the list.",
  },
  excepts: {
    letter: "e",
    title: "Exception list",
    empty: "No exceptions listed yet.",
    placeholder: "*!*@host",
    note: "Exceptions let matching users in past bans. Only operators can change the list.",
  },
  invites: {
    letter: "I",
    title: "Invite list",
    empty: "No invite exceptions listed yet.",
    placeholder: "nick!*@*",
    note: "Invite exceptions let matching users in while the channel is +i. Only operators can change the list.",
  },
};

const BACKDROP_ID = "channel-central-backdrop";
let isOpen = false;
let view = null;
let activeTab = "topic";
// Values the user typed or ticked but not applied yet. Server state shows where there is none.
let drafts = {};
// What the dialog currently shows, to skip redraws that would change nothing.
let renderedSignature = "";
let listenersAttached = false;
let pressStartedOnBackdrop = false;

export function isChannelCentralOpen() {
  return isOpen;
}

// The network and channel the dialog is showing, or null.
export function channelCentralTarget() {
  return isOpen && view ? { network: view.network, channel: view.channel } : null;
}

// view: { network, channel, topic, setBy, history, flags, key, limit, bans, excepts, invites,
//         canOp, onSetTopic(text), send(line), requestLists() }
export function openChannelCentral(next) {
  view = next;
  isOpen = true;
  activeTab = "topic";
  drafts = {};
  attachListeners();
  render();
  view.requestLists?.();
}

// Called after every sync. Redraws only when the shown data changed.
export function refreshChannelCentral(next) {
  if (!isOpen || !view || !next || next.network !== view.network || next.channel !== view.channel) return;
  // Another dialog took the shared container: our backdrop is gone. This one yields and closes;
  // the container is not cleared, so the other dialog stays.
  if (!document.getElementById(BACKDROP_ID)) {
    isOpen = false;
    view = null;
    drafts = {};
    renderedSignature = "";
    return;
  }
  view = next;
  if (signatureOf(next) === renderedSignature) return;
  render();
}

export function closeChannelCentral() {
  isOpen = false;
  view = null;
  drafts = {};
  renderedSignature = "";
  const container = document.getElementById("modal-container");
  if (container) container.innerHTML = "";
}

function signatureOf(v) {
  return JSON.stringify([v.topic, v.setBy, v.history, v.flags, v.key, v.limit, v.bans, v.excepts, v.invites, v.canOp]);
}

function formatWhen(seconds) {
  if (!seconds) return "";
  return new Date(seconds * 1000).toLocaleString();
}

// One MODE line from a list of changes: [{ sign: "+"|"-", letter, param? }].
function buildModeLine(channel, changes) {
  let letters = "";
  const params = [];
  let lastSign = "";
  for (const c of changes) {
    if (c.sign !== lastSign) {
      letters += c.sign;
      lastSign = c.sign;
    }
    letters += c.letter;
    if (c.param !== undefined) params.push(c.param);
  }
  return `MODE ${channel} ${letters}${params.length ? " " + params.join(" ") : ""}`;
}

function currentFlagSet() {
  return new Set([...(view.flags || "")]);
}

function renderTopic() {
  const current = view.topic ? parseIrcToHtml(view.topic) : "<em>No topic set.</em>";
  const setBy = view.setBy ? `Set by ${escapeHtml(view.setBy)}` : "";
  const draft = drafts.topic !== undefined ? drafts.topic : (view.topic || "");
  const history = [...(view.history || [])].reverse();
  const rows = history.length
    ? history.map((change) => `
        <li>
          <div class="cc-when">${escapeHtml(formatWhen(change.at))} <span class="cc-by">${escapeHtml(change.by || "unknown")}</span></div>
          <div class="cc-text">${parseIrcToHtml(change.topic)}</div>
        </li>`).join("")
    : `<li class="cc-empty">No topic changes recorded yet.</li>`;
  return `
    <section class="cc-section">
      <div class="cc-label">Current topic</div>
      <div class="cc-current">${current}</div>
      <div class="cc-meta">${setBy}</div>
      <label class="cc-label" for="cc-topic-input">New topic</label>
      <div class="cc-set-row">
        <input id="cc-topic-input" class="form-input" value="${escapeHtml(draft)}" spellcheck="false" />
        <button type="button" class="btn btn-secondary" id="cc-topic-set">Set</button>
      </div>
      <div class="cc-note">The server decides whether you may change it (operator status when the channel is +t).</div>
    </section>
    <section class="cc-section">
      <div class="cc-label">Topic history</div>
      <ol class="cc-history">${rows}</ol>
    </section>`;
}

function renderModes() {
  const set = currentFlagSet();
  const flagRows = FLAGS.map(([f, name]) => {
    const on = drafts.flags ? Boolean(drafts.flags[f]) : set.has(f);
    return `<label class="cc-check"><input type="checkbox" data-flag="${f}" ${on ? "checked" : ""} /> <span class="cc-flag-letter">${f}</span> ${escapeHtml(name)}</label>`;
  }).join("");
  const keyOn = drafts.keyOn !== undefined ? drafts.keyOn : Boolean(view.key);
  const keyText = drafts.key !== undefined ? drafts.key : (view.key || "");
  const limitOn = drafts.limitOn !== undefined ? drafts.limitOn : Boolean(view.limit);
  const limitText = drafts.limit !== undefined ? drafts.limit : (view.limit || "");
  const current = `+${view.flags || ""}${view.key ? "k" : ""}${view.limit ? "l" : ""}`;
  return `
    <section class="cc-section">
      <div class="cc-label">Flags</div>
      <div class="cc-flags">${flagRows}</div>
      <div class="cc-label">Key and limit</div>
      <div class="cc-mode-row">
        <label class="cc-check"><input type="checkbox" id="cc-key-on" ${keyOn ? "checked" : ""} /> Key (+k)</label>
        <input id="cc-mode-key" class="form-input" value="${escapeHtml(keyText)}" spellcheck="false" />
      </div>
      <div class="cc-mode-row">
        <label class="cc-check"><input type="checkbox" id="cc-limit-on" ${limitOn ? "checked" : ""} /> Limit (+l)</label>
        <input id="cc-mode-limit" class="form-input" type="number" min="1" value="${escapeHtml(limitText)}" />
      </div>
      <div class="cc-set-row">
        <button type="button" class="btn btn-secondary" id="cc-modes-apply">Apply</button>
        <span class="cc-meta">Current: ${escapeHtml(current)}</span>
      </div>
      <div class="cc-note">Only operators can change modes. Changes are sent as one MODE line.</div>
    </section>`;
}

// Bans, excepts or invites: the same view for each, driven by LIST_TABS.
function renderList(tab) {
  const spec = LIST_TABS[tab];
  const entries = view[tab] || [];
  const rows = entries.length
    ? entries.map((entry) => `
        <li>
          <div class="cc-entry">
            <span class="cc-mask">${escapeHtml(entry.mask)}</span>
            <button type="button" class="btn btn-secondary cc-remove" data-remove-entry="${escapeHtml(entry.mask)}" title="Remove this entry">Remove</button>
          </div>
          <div class="cc-when">${escapeHtml(formatWhen(entry.at))} <span class="cc-by">${escapeHtml(entry.by || "unknown")}</span></div>
        </li>`).join("")
    : `<li class="cc-empty">${spec.empty}</li>`;
  const draft = drafts.list?.[tab] !== undefined ? drafts.list[tab] : "";
  return `
    <section class="cc-section">
      <div class="cc-label">${spec.title} (${entries.length})</div>
      <ul class="cc-history">${rows}</ul>
      <label class="cc-label" for="cc-list-input">Add entry</label>
      <div class="cc-set-row">
        <input id="cc-list-input" class="form-input" value="${escapeHtml(draft)}" placeholder="${escapeHtml(spec.placeholder)}" spellcheck="false" />
        <button type="button" class="btn btn-secondary" id="cc-list-add">Add</button>
      </div>
      <div class="cc-note">${spec.note}</div>
    </section>`;
}

function render() {
  const container = document.getElementById("modal-container");
  if (!container || !isOpen || !view) return;
  renderedSignature = signatureOf(view);

  const tabs = [["topic", "Topic"], ["modes", "Channel modes"], ["bans", "Bans"], ["excepts", "Excepts"], ["invites", "Invites"]];
  const tabBar = tabs.map(([id, label]) =>
    `<button type="button" class="cc-tab${id === activeTab ? " is-active" : ""}" data-tab="${id}">${label}</button>`
  ).join("");
  const body = activeTab === "modes" ? renderModes() : LIST_TABS[activeTab] ? renderList(activeTab) : renderTopic();

  container.innerHTML = `
    <div class="modal-overlay" id="${BACKDROP_ID}">
      <div class="modal-card cc-dialog" role="dialog" aria-modal="true" aria-labelledby="cc-title">
        <div class="cc-header">
          <div class="cc-title" id="cc-title">Channel Central: ${escapeHtml(view.channel)}</div>
          <button type="button" class="btn btn-secondary" id="cc-close">Close</button>
        </div>
        <div class="cc-tabs">${tabBar}</div>
        ${body}
      </div>
    </div>`;
}

// Listeners sit on the container, which stays in the page; redraws replace only its content, so
// a click cannot land on an element that is gone by the time the mouse button comes up.
function attachListeners() {
  if (listenersAttached) return;
  const container = document.getElementById("modal-container");
  if (!container) return;
  listenersAttached = true;
  container.addEventListener("mousedown", (e) => {
    pressStartedOnBackdrop = isOpen && e.target.id === BACKDROP_ID;
  });
  container.addEventListener("click", onClick);
  container.addEventListener("input", onInput);
  container.addEventListener("change", onInput);
}

function inDialog(target) {
  return isOpen && view && target && target.closest && target.closest(`#${BACKDROP_ID}`);
}

// Stores what the user changed, so a later redraw keeps it. Nothing is stored for server updates.
function onInput(e) {
  if (!inDialog(e.target)) return;
  const t = e.target;
  if (t.id === "cc-topic-input") drafts.topic = t.value;
  else if (t.id === "cc-list-input" && LIST_TABS[activeTab]) drafts.list = { ...(drafts.list || {}), [activeTab]: t.value };
  else if (t.dataset.flag || t.id === "cc-key-on" || t.id === "cc-mode-key" || t.id === "cc-limit-on" || t.id === "cc-mode-limit") {
    captureModesDraft();
  }
}

function captureModesDraft() {
  const root = document.getElementById(BACKDROP_ID);
  if (!root) return;
  const checked = (sel) => Boolean(root.querySelector(sel)?.checked);
  const val = (sel) => root.querySelector(sel)?.value ?? "";
  drafts.flags = Object.fromEntries(FLAGS.map(([f]) => [f, checked(`[data-flag="${f}"]`)]));
  drafts.keyOn = checked("#cc-key-on");
  drafts.key = val("#cc-mode-key");
  drafts.limitOn = checked("#cc-limit-on");
  drafts.limit = val("#cc-mode-limit");
}

function onClick(e) {
  if (!inDialog(e.target)) return;
  // A click only closes when the press also started on the backdrop (not on a drag out of the text).
  if (e.target.id === BACKDROP_ID) {
    if (pressStartedOnBackdrop) closeChannelCentral();
    return;
  }
  const el = e.target.closest("button");
  if (!el) return;
  if (el.id === "cc-close") return closeChannelCentral();
  if (el.dataset.tab) {
    activeTab = el.dataset.tab;
    render();
    return;
  }
  if (el.id === "cc-topic-set") {
    const text = (document.getElementById("cc-topic-input")?.value || "").trim();
    if (!text || !view.onSetTopic) return;
    view.onSetTopic(text);
    closeChannelCentral();
    return;
  }
  if (el.id === "cc-modes-apply") return applyModes();
  if (el.id === "cc-list-add") {
    const mask = (document.getElementById("cc-list-input")?.value || "").trim();
    if (!mask || !LIST_TABS[activeTab]) return;
    view.send(`MODE ${view.channel} +${LIST_TABS[activeTab].letter} ${mask}`);
    drafts.list = { ...(drafts.list || {}), [activeTab]: "" };
    render();
    return;
  }
  if (el.dataset.removeEntry !== undefined && LIST_TABS[activeTab]) {
    view.send(`MODE ${view.channel} -${LIST_TABS[activeTab].letter} ${el.dataset.removeEntry}`);
  }
}

// Works out the changes between the checkboxes and the channel, then sends one MODE line. The
// checkboxes go back to the channel's state right away; the server's echo updates them.
function applyModes() {
  const root = document.getElementById(BACKDROP_ID);
  if (!root) return;
  const changes = [];
  const had = currentFlagSet();
  for (const [f] of FLAGS) {
    const want = Boolean(root.querySelector(`[data-flag="${f}"]`)?.checked);
    if (want && !had.has(f)) changes.push({ sign: "+", letter: f });
    if (!want && had.has(f)) changes.push({ sign: "-", letter: f });
  }
  const keyOn = Boolean(root.querySelector("#cc-key-on")?.checked);
  const keyText = (root.querySelector("#cc-mode-key")?.value || "").trim();
  if (view.key && (!keyOn || keyText !== view.key)) changes.push({ sign: "-", letter: "k", param: view.key });
  if (keyOn && keyText && keyText !== view.key) changes.push({ sign: "+", letter: "k", param: keyText });
  const limitOn = Boolean(root.querySelector("#cc-limit-on")?.checked);
  const limitNum = parseInt(root.querySelector("#cc-mode-limit")?.value || "", 10);
  if (view.limit && (!limitOn || String(limitNum) !== String(view.limit))) changes.push({ sign: "-", letter: "l" });
  if (limitOn && limitNum > 0 && String(limitNum) !== String(view.limit)) changes.push({ sign: "+", letter: "l", param: String(limitNum) });
  drafts = {};
  if (!changes.length) {
    render();
    return;
  }
  // A key must be removed before another one is set.
  changes.sort((a, b) => (a.sign === "-" ? 0 : 1) - (b.sign === "-" ? 0 : 1));
  view.send(buildModeLine(view.channel, changes));
  render();
}

document.addEventListener("keydown", (e) => {
  if (isOpen && e.key === "Escape") closeChannelCentral();
});
