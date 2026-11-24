import { escapeHtml } from "./utils/ircColors.js";

const STORAGE_SCHEMES_KEY = "ferric-color-schemes";
const STORAGE_ACTIVE_SCHEME_KEY = "ferric-active-color-scheme";

export const DEFAULT_SCHEME_4400 = {
  name: "4400",
  colors: {
    action: "#00fc00",
    ctcp: "#00ffff",
    highlight: "#00ffff",
    info: "#a0a0a0",
    info2: "#8c8c8c",
    invite: "#a0a0a0",
    join: "#00fc00",
    kick: "#ff0000",
    mode: "#00b5b5",
    nickname: "#00b5b5",
    normal: "#ffffff",
    notice: "#a0a0a0",
    notify: "#a0a0a0",
    other: "#d2d2d2",
    own: "#ffffff",
    part: "#7f7f7f",
    quit: "#ff0000",
    topic: "#7f7f7f",
    wallops: "#7f7f7f",
    whois: "#7f7f7f",
    listbox: "#d2d2d2",
    listbox_bg: "#000000",
    gray: "#7f7f7f",
    title: "#7f7f7f",
    treebar: "#d2d2d2",
    treebar_bg: "#000000",
    mdi: "#000000",
    mdi_bg: "#dcdcdc",
    editbox: "#ffffff",
    editbox_bg: "#000000",
    inactive: "#ffffff",
    inactive_bg: "#000000",
    background: "#000000",
  }
};

export const DEFAULT_SCHEME_CLASSIC = {
  name: "Classic",
  colors: {
    action: "#7f0000",
    ctcp: "#00007f",
    highlight: "#ff0000",
    info: "#00007f",
    info2: "#00007f",
    invite: "#00007f",
    join: "#009300",
    kick: "#ff0000",
    mode: "#7f0000",
    nickname: "#00007f",
    normal: "#000000",
    notice: "#7f0000",
    notify: "#7f0000",
    other: "#00007f",
    own: "#000000",
    part: "#7f0000",
    quit: "#00007f",
    topic: "#00007f",
    wallops: "#7f0000",
    whois: "#00007f",
    listbox: "#000000",
    listbox_bg: "#ffffff",
    gray: "#7f7f7f",
    title: "#00007f",
    treebar: "#000000",
    treebar_bg: "#ffffff",
    mdi: "#000000",
    mdi_bg: "#808080",
    editbox: "#000000",
    editbox_bg: "#ffffff",
    inactive: "#7f7f7f",
    inactive_bg: "#ffffff",
    background: "#ffffff",
  }
};

export const DEFAULT_SCHEME_DARK = {
  name: "Dark",
  colors: {
    ...DEFAULT_SCHEME_4400.colors,
    action: "#33e033",
    join: "#33e033",
    quit: "#ff3b3b",
    kick: "#ff3b3b",
    highlight: "#6db3f2",
  }
};

const STANDARD_16_COLORS = [
  "#ffffff", "#000000", "#00007f", "#009300", "#ff0000", "#7f0000", "#9c009c", "#fc7f00",
  "#ffff00", "#00fc00", "#009393", "#00ffff", "#0000fc", "#ff00ff", "#7f7f7f", "#d2d2d2"
];

function generateMatrixSwatches() {
  const hues = [0, 30, 60, 90, 120, 160, 190, 220, 240, 270, 300, 330];
  const lightnessLevels = [15, 26, 38, 50, 64, 78, 90];
  const swatches = [];

  for (const l of lightnessLevels) {
    for (const h of hues) {
      swatches.push(hslToHex(h, 95, l));
    }
  }

  // Row 8: 12 Grayscale shades
  const grayLevels = [0, 9, 18, 27, 36, 45, 55, 64, 73, 82, 91, 100];
  for (const g of grayLevels) {
    const val = Math.round((g * 255) / 100).toString(16).padStart(2, "0");
    swatches.push(`#${val}${val}${val}`);
  }

  return swatches;
}

function hslToHex(h, s, l) {
  l /= 100;
  const a = (s * Math.min(l, 1 - l)) / 100;
  const f = (n) => {
    const k = (n + h / 30) % 12;
    const color = l - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
    return Math.round(255 * color).toString(16).padStart(2, "0");
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

const MATRIX_96_COLORS = generateMatrixSwatches();

let colorSchemes = {};
// The schemes as they were when the dialog opened. Cancel, Escape and closing put them back.
let snapshotSchemes = null;
let snapshotActiveScheme = null;
let activeSchemeName = "4400";
let selectedTarget = "action";
let draftColors = {};
let snapshotColors = {};
let isOpen = false;
let onColorsChangedCallback = null;

export function setOnColorsChanged(cb) {
  onColorsChangedCallback = cb;
}

export function initColorSchemes() {
  try {
    const saved = localStorage.getItem(STORAGE_SCHEMES_KEY);
    if (saved) {
      colorSchemes = JSON.parse(saved);
    }
  } catch { }

  if (!colorSchemes || Object.keys(colorSchemes).length === 0) {
    colorSchemes = {
      "4400": { ...DEFAULT_SCHEME_4400 },
      "Classic": { ...DEFAULT_SCHEME_CLASSIC },
      "Dark": { ...DEFAULT_SCHEME_DARK },
    };
  }

  try {
    const storedActive = localStorage.getItem(STORAGE_ACTIVE_SCHEME_KEY);
    const savedActive = storedActive;
    if (savedActive && colorSchemes[savedActive]) {
      activeSchemeName = savedActive;
    }
  } catch { }


  applyActiveColorScheme();
}

export function getActiveSchemeName() {
  return activeSchemeName;
}

export function getActiveColorScheme() {
  return colorSchemes[activeSchemeName] || DEFAULT_SCHEME_4400;
}

export function applyActiveColorScheme() {
  const scheme = getActiveColorScheme();
  if (!scheme || !scheme.colors) return;
  const root = document.documentElement;
  for (const [key, val] of Object.entries(scheme.colors)) {
    root.style.setProperty(`--col-${key.replace(/_/g, "-")}`, val);
  }
  if (onColorsChangedCallback) onColorsChangedCallback();
}

export function isColorsModalOpen() {
  return isOpen;
}

export function openColorsModal() {
  isOpen = true;
  snapshotSchemes = JSON.parse(JSON.stringify(colorSchemes));
  snapshotActiveScheme = activeSchemeName;
  selectedTarget = "action";
  const current = getActiveColorScheme();
  draftColors = { ...current.colors };
  snapshotColors = { ...current.colors };
  renderColorsModal();
}

// Closing without OK drops the scheme changes made in this dialog (Add, Delete, scheme choice).
export function closeColorsModal({ keepChanges = false } = {}) {
  if (!keepChanges && snapshotSchemes) {
    colorSchemes = snapshotSchemes;
    activeSchemeName = snapshotActiveScheme;
  }
  snapshotSchemes = null;
  isOpen = false;
  const container = document.getElementById("modal-container");
  if (container) container.innerHTML = "";
}

export function renderColorsModal() {
  const container = document.getElementById("modal-container");
  if (!container || !isOpen) return;

  const schemeOptions = Object.keys(colorSchemes).map((name) => {
    const sel = name === activeSchemeName ? "selected" : "";
    return `<option value="${escapeHtml(name)}" ${sel}>${escapeHtml(name)}</option>`;
  }).join("");

  const c = draftColors;
  const targetClass = (id) => `color-target-item${selectedTarget === id ? " is-selected" : ""}`;
  // Clicking the empty area of a preview box selects its background, like the client.
  const boxClass = (base, bgKey) => `color-preview-box ${base}${selectedTarget === bgKey ? " is-selected" : ""}`;

  container.innerHTML = `
    <div class="modal-overlay" id="colors-modal-backdrop">
      <div class="modal-card colors-dialog" role="dialog" aria-modal="true" aria-labelledby="colors-title">
        <div class="colors-header">
          <div class="colors-title" id="colors-title">Colors</div>
          <button type="button" class="btn-icon-sm" id="btn-colors-close" aria-label="Close">✕</button>
        </div>

        <div class="colors-top-bar">
          <div class="colors-scheme-row">
            <label for="colors-scheme-select" style="font-size:12px; font-weight:600;">Scheme:</label>
            <select id="colors-scheme-select" class="form-input form-input-sm" style="flex:1 1 auto; height:24px;">
              ${schemeOptions}
            </select>
          </div>
          <div class="colors-scheme-buttons">
            <button type="button" class="btn btn-secondary" id="btn-color-scheme-add">Add...</button>
            <button type="button" class="btn btn-secondary" id="btn-color-scheme-delete" ${Object.keys(colorSchemes).length <= 1 ? "disabled" : ""}>Delete</button>
          </div>
        </div>

        <div class="color-preview-container">
          <!-- Chat Window Box -->
          <div class="${boxClass("color-box-chat", "background")}" id="preview-box-chat" data-bg-target="background" style="background:${c.background || '#000000'};">
            <div class="color-preview-col">
              <div class="${targetClass("action")}" data-target="action" style="color:${c.action};">Action</div>
              <div class="${targetClass("ctcp")}" data-target="ctcp" style="color:${c.ctcp};">CTCP</div>
              <div class="${targetClass("highlight")}" data-target="highlight" style="color:${c.highlight};">Highlight</div>
              <div class="${targetClass("info")}" data-target="info" style="color:${c.info};">Info</div>
              <div class="${targetClass("info2")}" data-target="info2" style="color:${c.info2};">Info2</div>
              <div class="${targetClass("invite")}" data-target="invite" style="color:${c.invite};">Invite</div>
              <div class="${targetClass("join")}" data-target="join" style="color:${c.join};">Join</div>
              <div class="${targetClass("kick")}" data-target="kick" style="color:${c.kick};">Kick</div>
              <div class="${targetClass("mode")}" data-target="mode" style="color:${c.mode};">Mode</div>
              <div class="${targetClass("nickname")}" data-target="nickname" style="color:${c.nickname};">Nickname</div>
              <div class="${targetClass("normal")}" data-target="normal" style="color:${c.normal};">Normal</div>
              <div class="${targetClass("notice")}" data-target="notice" style="color:${c.notice};">Notice</div>
              <div class="${targetClass("notify")}" data-target="notify" style="color:${c.notify};">Notify</div>
            </div>
            <div class="color-preview-col">
              <div class="${targetClass("other")}" data-target="other" style="color:${c.other};">Other</div>
              <div class="${targetClass("own")}" data-target="own" style="color:${c.own};">Own</div>
              <div class="${targetClass("part")}" data-target="part" style="color:${c.part};">Part</div>
              <div class="${targetClass("quit")}" data-target="quit" style="color:${c.quit};">Quit</div>
              <div class="${targetClass("topic")}" data-target="topic" style="color:${c.topic};">Topic</div>
              <div class="${targetClass("wallops")}" data-target="wallops" style="color:${c.wallops};">Wallops</div>
              <div class="${targetClass("whois")}" data-target="whois" style="color:${c.whois};">Whois</div>
            </div>
          </div>

          <!-- Right side boxes -->
          <div class="color-preview-right">
            <div class="${boxClass("color-box-listbox", "listbox_bg")}" data-bg-target="listbox_bg" style="background:${c.listbox_bg || '#000000'};">
              <div class="${targetClass("listbox")}" data-target="listbox" style="color:${c.listbox};">Listbox</div>
              <div class="${targetClass("gray")}" data-target="gray" style="color:${c.gray};">Gray</div>
              <div class="${targetClass("title")}" data-target="title" style="color:${c.title};">Title</div>
            </div>
            <div class="${boxClass("color-box-treebar", "treebar_bg")}" data-bg-target="treebar_bg" style="background:${c.treebar_bg || '#000000'};">
              <div class="${targetClass("treebar")}" data-target="treebar" style="color:${c.treebar};">Treebar</div>
            </div>
            <div class="${boxClass("color-box-mdi", "mdi_bg")}" data-bg-target="mdi_bg" style="background:${c.mdi_bg || '#dcdcdc'};">
              <div class="${targetClass("mdi")}" data-target="mdi" style="color:${c.mdi};">MDI</div>
            </div>
          </div>
        </div>

        <!-- Bottom row boxes -->
        <div class="color-preview-bottom">
          <div class="${boxClass("color-box-editbox", "editbox_bg")}" data-bg-target="editbox_bg" style="background:${c.editbox_bg || '#000000'};">
            <div class="${targetClass("editbox")}" data-target="editbox" style="color:${c.editbox};">Editbox</div>
          </div>
          <div class="${boxClass("color-box-inactive", "inactive_bg")}" data-bg-target="inactive_bg" style="background:${c.inactive_bg || '#000000'};">
            <div class="${targetClass("inactive")}" data-target="inactive" style="color:${c.inactive};">Inactive</div>
          </div>
        </div>

        <!-- Palette & Actions Section -->
        <div class="colors-palette-section">
          <div class="colors-palette-left">
            <!-- 16 Standard Colors (2x8) -->
            <div class="colors-standard-grid">
              ${STANDARD_16_COLORS.map((col, idx) => `
                <div class="color-swatch" data-color="${col}" style="background-color:${col};" title="Standard Color ${idx}: ${col}"></div>
              `).join("")}
            </div>

            <!-- 96 Matrix Colors (8x12) -->
            <div class="colors-matrix-grid">
              ${MATRIX_96_COLORS.map((col) => `
                <div class="color-swatch" data-color="${col}" style="background-color:${col};" title="${col}"></div>
              `).join("")}
            </div>
          </div>

          <div class="colors-actions-right">
            <button type="button" class="btn btn-secondary" id="btn-colors-default">Default</button>
            <button type="button" class="btn btn-secondary" id="btn-colors-undo">Undo</button>
            <div style="flex:1 1 auto;"></div>
            <button type="button" class="btn btn-primary" id="btn-colors-ok">OK</button>
            <button type="button" class="btn btn-secondary" id="btn-colors-cancel">Cancel</button>
            <button type="button" class="btn btn-secondary" id="btn-colors-help">Help</button>
          </div>
        </div>
      </div>
    </div>
  `;

  attachModalEvents(container);
}

function attachModalEvents(container) {
  // Close / Backdrop
  const backdrop = container.querySelector("#colors-modal-backdrop");
  backdrop?.addEventListener("click", (e) => {
    if (e.target === backdrop) closeColorsModal();
  });
  container.querySelector("#btn-colors-close")?.addEventListener("click", closeColorsModal);
  container.querySelector("#btn-colors-cancel")?.addEventListener("click", closeColorsModal);

  // Target item selection
  container.querySelectorAll(".color-target-item").forEach((el) => {
    el.addEventListener("click", () => {
      selectedTarget = el.dataset.target;
      renderColorsModal();
    });
  });

  // Empty box area selects the box background.
  container.querySelectorAll("[data-bg-target]").forEach((box) => {
    box.addEventListener("click", (e) => {
      if (e.target.closest(".color-target-item")) return;
      selectedTarget = box.dataset.bgTarget;
      renderColorsModal();
    });
  });

  // Swatch click
  container.querySelectorAll(".color-swatch").forEach((el) => {
    el.addEventListener("click", () => {
      const color = el.dataset.color;
      if (selectedTarget && color) {
        draftColors[selectedTarget] = color;
        // If clicking MDI or Listbox or Editbox, also set appropriate matching text/bg if desired
        renderColorsModal();
      }
    });
  });

  // Scheme selector
  const schemeSelect = container.querySelector("#colors-scheme-select");
  schemeSelect?.addEventListener("change", (e) => {
    const name = e.target.value;
    if (colorSchemes[name]) {
      activeSchemeName = name;
      draftColors = { ...colorSchemes[name].colors };
      // Undo returns to the colors the scheme had when it was chosen, not those of the previous one.
      snapshotColors = { ...draftColors };
      renderColorsModal();
    }
  });

  // Add Scheme
  container.querySelector("#btn-color-scheme-add")?.addEventListener("click", () => {
    const name = window.prompt("Enter new color scheme name:", `Scheme ${Object.keys(colorSchemes).length + 1}`);
    if (name && name.trim()) {
      const cleanName = name.trim();
      colorSchemes[cleanName] = {
        name: cleanName,
        colors: { ...draftColors },
      };
      activeSchemeName = cleanName;
      renderColorsModal();
    }
  });

  // Delete Scheme
  container.querySelector("#btn-color-scheme-delete")?.addEventListener("click", () => {
    if (Object.keys(colorSchemes).length <= 1) return;
    if (window.confirm(`Delete color scheme "${activeSchemeName}"?`)) {
      delete colorSchemes[activeSchemeName];
      activeSchemeName = Object.keys(colorSchemes)[0] || "4400";
      draftColors = { ...colorSchemes[activeSchemeName].colors };
      snapshotColors = { ...draftColors };
      renderColorsModal();
    }
  });

  // Default button
  container.querySelector("#btn-colors-default")?.addEventListener("click", () => {
    draftColors = { ...DEFAULT_SCHEME_4400.colors };
    renderColorsModal();
  });

  // Undo button
  container.querySelector("#btn-colors-undo")?.addEventListener("click", () => {
    draftColors = { ...snapshotColors };
    renderColorsModal();
  });

  // OK button
  container.querySelector("#btn-colors-ok")?.addEventListener("click", () => {
    if (!colorSchemes[activeSchemeName]) {
      colorSchemes[activeSchemeName] = { name: activeSchemeName, colors: {} };
    }
    colorSchemes[activeSchemeName].colors = { ...draftColors };
    try {
      localStorage.setItem(STORAGE_SCHEMES_KEY, JSON.stringify(colorSchemes));
      localStorage.setItem(STORAGE_ACTIVE_SCHEME_KEY, activeSchemeName);
    } catch { }

    applyActiveColorScheme();
    closeColorsModal({ keepChanges: true });
  });

  // Help button
  container.querySelector("#btn-colors-help")?.addEventListener("click", () => {
    window.alert("Colors\n\nClick any item in the preview boxes above to select it, then click a color swatch in the palette below to assign its color.\n\nClick the empty area of a preview box (e.g. the chat window) to select its background instead.\n\nUse 'Add...' to save custom color schemes. Click 'OK' to apply changes.");
  });
}
