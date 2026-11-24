// Generates consistent, high-contrast, legible nick colors on dark backgrounds
const PALETTE = [
  "#38bdf8", // Sky
  "#34d399", // Emerald
  "#a78bfa", // Violet
  "#f472b6", // Pink
  "#fbbf24", // Amber
  "#4ade80", // Green
  "#2dd4bf", // Teal
  "#f87171", // Red
  "#c084fc", // Purple
  "#60a5fa", // Blue
  "#fb923c", // Orange
  "#22d3ee", // Cyan
  "#e879f9", // Fuchsia
  "#a3e635", // Lime
];

export function getNickColor(nick) {
  if (!nick) return "#94a3b8";
  const normalized = nick.toLowerCase();
  let hash = 0;
  for (let i = 0; i < normalized.length; i++) {
    hash = (hash << 5) - hash + normalized.charCodeAt(i);
    hash |= 0;
  }
  const index = Math.abs(hash) % PALETTE.length;
  return PALETTE[index];
}
