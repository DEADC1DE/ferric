// Full standard 99-color IRC palette (0 to 98)
export const IRC_COLORS = [
  "#ffffff", "#000000", "#00007f", "#009300", "#ff0000", "#7f0000", "#9c009c", "#fc7f00",
  "#ffff00", "#00fc00", "#009393", "#00ffff", "#0000fc", "#ff00ff", "#7f7f7f", "#d2d2d2",
  "#470000", "#472100", "#474700", "#324700", "#004700", "#00472c", "#004747", "#002c47",
  "#000047", "#2c0047", "#470047", "#47002c", "#740000", "#743a00", "#747400", "#517400",
  "#007400", "#007449", "#007474", "#004974", "#000074", "#490074", "#740074", "#740049",
  "#b50000", "#b55a00", "#b5b500", "#7eb500", "#00b500", "#00b571", "#00b5b5", "#0071b5",
  "#0000b5", "#7100b5", "#b500b5", "#b50071", "#ff0000", "#ff7f00", "#ffff00", "#b2ff00",
  "#00ff00", "#00ffa1", "#00ffff", "#00a1ff", "#0000ff", "#a100ff", "#ff00ff", "#ff00a1",
  "#ff4f4f", "#ff9f4f", "#ffff4f", "#c8ff4f", "#4fff4f", "#4fffa6", "#4fffff", "#4fa6ff",
  "#4f4fff", "#a64fff", "#ff4fff", "#ff4fa6", "#ff8989", "#ffc589", "#ffff89", "#dbff89",
  "#89ff89", "#89ffc7", "#89ffff", "#89c7ff", "#8989ff", "#c789ff", "#ff89ff", "#ff89c7",
  "#c6c6c6", "#e1e1e1", "#141414", "#282828", "#3c3c3c", "#505050", "#646464", "#787878",
  "#8c8c8c", "#a0a0a0", "#b4b4b4"
];

export function renderSpansToHtml(spans) {
  if (!spans || spans.length === 0) return "";

  return spans
    .map((span) => {
      let style = [];
      let curFg = span.fg;
      let curBg = span.bg;

      if (span.reverse) {
        const tmp = curFg;
        curFg = curBg !== null && curBg !== undefined ? curBg : 1;
        curBg = tmp !== null && tmp !== undefined ? tmp : 0;
      }

      if (curFg !== null && curFg !== undefined) {
        const color = IRC_COLORS[curFg % IRC_COLORS.length];
        style.push(`color: ${color}`);
        if (curFg === 1 && (curBg === null || curBg === undefined)) {
          style.push("background-color: rgba(255,255,255,0.2); border-radius: 2px; padding: 0 2px");
        }
      }
      if (curBg !== null && curBg !== undefined) {
        const color = IRC_COLORS[curBg % IRC_COLORS.length];
        style.push(`background-color: ${color}`);
      }
      if (span.bold) style.push("font-weight: 700");
      if (span.italic) style.push("font-style: italic");
      if (span.underline && span.strikethrough) {
        style.push("text-decoration: underline line-through");
      } else if (span.underline) {
        style.push("text-decoration: underline");
      } else if (span.strikethrough) {
        style.push("text-decoration: line-through");
      }

      const escaped = escapeHtml(span.text);
      const styleAttr = style.length > 0 ? ` style="${style.join("; ")}"` : "";
      return `<span${styleAttr}>${escaped}</span>`;
    })
    .join("");
}

// Most styled pieces per line. Past this, text is kept but no more <span>s are written, so one line of
// color codes cannot make hundreds of nodes. The Rust side caps the same way.
const MAX_STYLED_PIECES = 64;

export function parseIrcToHtml(raw) {
  if (!raw) return "";

  let fg = null;
  let bg = null;
  let bold = false;
  let italic = false;
  let underline = false;
  let strikethrough = false;
  let reverse = false;

  let out = "";
  let styledPieces = 0;
  let i = 0;
  const len = raw.length;

  function buildStyle() {
    let styles = [];
    let curFg = fg;
    let curBg = bg;

    if (reverse) {
      const tmp = curFg;
      curFg = curBg !== null ? curBg : 1;
      curBg = tmp !== null ? tmp : 0;
    }

    if (curFg !== null) {
      const color = IRC_COLORS[curFg % IRC_COLORS.length];
      styles.push(`color: ${color}`);
      if (curFg === 1 && curBg === null) {
        styles.push("background-color: rgba(255,255,255,0.2); border-radius: 2px; padding: 0 2px");
      }
    }
    if (curBg !== null) {
      const color = IRC_COLORS[curBg % IRC_COLORS.length];
      styles.push(`background-color: ${color}`);
    }
    if (bold) styles.push("font-weight: 700");
    if (italic) styles.push("font-style: italic");
    if (underline && strikethrough) {
      styles.push("text-decoration: underline line-through");
    } else if (underline) {
      styles.push("text-decoration: underline");
    } else if (strikethrough) {
      styles.push("text-decoration: line-through");
    }

    return styles.length > 0 ? ` style="${styles.join("; ")}"` : "";
  }

  while (i < len) {
    const ch = raw[i];

    if (ch === "\x03") {
      i++;
      let fgStr = "";
      while (i < len && raw[i] >= "0" && raw[i] <= "9" && fgStr.length < 2) {
        fgStr += raw[i];
        i++;
      }

      if (fgStr.length > 0) {
        fg = parseInt(fgStr, 10);
        if (i < len && raw[i] === ",") {
          if (i + 1 < len && raw[i + 1] >= "0" && raw[i + 1] <= "9") {
            i++;
            let bgStr = "";
            while (i < len && raw[i] >= "0" && raw[i] <= "9" && bgStr.length < 2) {
              bgStr += raw[i];
              i++;
            }
            if (bgStr.length > 0) {
              bg = parseInt(bgStr, 10);
            }
          }
        }
      } else {
        fg = null;
        bg = null;
      }
      continue;
    }

    if (ch === "\x02") {
      bold = !bold;
      i++;
      continue;
    }
    if (ch === "\x1D") {
      italic = !italic;
      i++;
      continue;
    }
    if (ch === "\x1F") {
      underline = !underline;
      i++;
      continue;
    }
    if (ch === "\x1E") {
      strikethrough = !strikethrough;
      i++;
      continue;
    }
    if (ch === "\x16") {
      reverse = !reverse;
      i++;
      continue;
    }
    if (ch === "\x0F") {
      fg = null;
      bg = null;
      bold = false;
      italic = false;
      underline = false;
      strikethrough = false;
      reverse = false;
      i++;
      continue;
    }

    let textChunk = "";
    while (i < len) {
      const c = raw[i];
      if (c === "\x03" || c === "\x02" || c === "\x1D" || c === "\x1F" || c === "\x1E" || c === "\x16" || c === "\x0F") {
        break;
      }
      textChunk += c;
      i++;
    }

    if (textChunk.length > 0) {
      const style = buildStyle();
      const escaped = escapeHtml(textChunk);
      if (style && styledPieces < MAX_STYLED_PIECES) {
        styledPieces++;
        out += `<span${style}>${escaped}</span>`;
      } else {
        out += escaped;
      }
    }
  }

  return out;
}

function hasActiveFormatting(fmt) {
  return (
    fmt.fg !== null && fmt.fg !== undefined ||
    fmt.bg !== null && fmt.bg !== undefined ||
    fmt.bold ||
    fmt.italic ||
    fmt.underline ||
    fmt.strikethrough ||
    fmt.reverse
  );
}

function formatIrcPrefix(fmt) {
  let out = "";
  if (fmt.bold) out += "\x02";
  if (fmt.italic) out += "\x1D";
  if (fmt.underline) out += "\x1F";
  if (fmt.strikethrough) out += "\x1E";
  if (fmt.reverse) out += "\x16";
  if (fmt.fg !== null && fmt.fg !== undefined) {
    out += `\x03${String(fmt.fg).padStart(2, "0")}`;
    if (fmt.bg !== null && fmt.bg !== undefined) {
      out += `,${String(fmt.bg).padStart(2, "0")}`;
    }
  } else if (fmt.bg !== null && fmt.bg !== undefined) {
    out += `\x03,${String(fmt.bg).padStart(2, "0")}`;
  }
  return out;
}

export function sliceRawIrcText(raw, visStart, visEnd) {
  if (!raw || visEnd <= visStart) return "";

  const fmt = {
    fg: null,
    bg: null,
    bold: false,
    italic: false,
    underline: false,
    strikethrough: false,
    reverse: false,
  };
  let out = "";
  let started = false;
  let visIdx = 0;
  let i = 0;
  const len = raw.length;

  while (i < len && visIdx < visEnd) {
    const ch = raw[i];

    if (ch === "\x03") {
      const codeStart = i;
      i++;
      let fgStr = "";
      while (i < len && raw[i] >= "0" && raw[i] <= "9" && fgStr.length < 2) {
        fgStr += raw[i];
        i++;
      }
      if (fgStr.length > 0) {
        fmt.fg = parseInt(fgStr, 10);
        if (i < len && raw[i] === "," && i + 1 < len && raw[i + 1] >= "0" && raw[i + 1] <= "9") {
          i++;
          let bgStr = "";
          while (i < len && raw[i] >= "0" && raw[i] <= "9" && bgStr.length < 2) {
            bgStr += raw[i];
            i++;
          }
          if (bgStr.length > 0) fmt.bg = parseInt(bgStr, 10);
        }
      } else {
        fmt.fg = null;
        fmt.bg = null;
      }
      if (started) out += raw.slice(codeStart, i);
      continue;
    }

    if (ch === "\x02" || ch === "\x1D" || ch === "\x1F" || ch === "\x1E" || ch === "\x16" || ch === "\x0F") {
      if (ch === "\x02") fmt.bold = !fmt.bold;
      else if (ch === "\x1D") fmt.italic = !fmt.italic;
      else if (ch === "\x1F") fmt.underline = !fmt.underline;
      else if (ch === "\x1E") fmt.strikethrough = !fmt.strikethrough;
      else if (ch === "\x16") fmt.reverse = !fmt.reverse;
      else {
        fmt.fg = null;
        fmt.bg = null;
        fmt.bold = false;
        fmt.italic = false;
        fmt.underline = false;
        fmt.strikethrough = false;
        fmt.reverse = false;
      }
      if (started) out += ch;
      i++;
      continue;
    }

    if (visIdx >= visStart) {
      if (!started) {
        started = true;
        out += formatIrcPrefix(fmt);
      }
      out += ch;
    }
    visIdx++;
    i++;
  }

  if (started && hasActiveFormatting(fmt) && !out.endsWith("\x0F")) {
    out += "\x0F";
  }
  return out;
}

export function sliceSpansToRawIrc(spans, visStart, visEnd) {
  if (!spans || spans.length === 0 || visEnd <= visStart) return "";

  let out = "";
  let visIdx = 0;
  let active = {
    fg: null,
    bg: null,
    bold: false,
    italic: false,
    underline: false,
    strikethrough: false,
    reverse: false,
  };

  for (const span of spans) {
    const text = span.text || "";
    const spanStart = visIdx;
    const spanEnd = spanStart + text.length;
    visIdx = spanEnd;
    if (spanEnd <= visStart || spanStart >= visEnd) continue;

    const sliceFrom = Math.max(0, visStart - spanStart);
    const sliceTo = Math.min(text.length, visEnd - spanStart);
    const piece = text.slice(sliceFrom, sliceTo);
    if (!piece) continue;

    const next = {
      fg: span.fg ?? null,
      bg: span.bg ?? null,
      bold: Boolean(span.bold),
      italic: Boolean(span.italic),
      underline: Boolean(span.underline),
      strikethrough: Boolean(span.strikethrough),
      reverse: Boolean(span.reverse),
    };

    const same =
      active.fg === next.fg &&
      active.bg === next.bg &&
      active.bold === next.bold &&
      active.italic === next.italic &&
      active.underline === next.underline &&
      active.strikethrough === next.strikethrough &&
      active.reverse === next.reverse;

    if (!same) {
      if (hasActiveFormatting(active)) out += "\x0F";
      out += formatIrcPrefix(next);
      active = next;
    }
    out += piece;
  }

  if (hasActiveFormatting(active)) out += "\x0F";
  return out;
}

export function escapeHtml(str) {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}
