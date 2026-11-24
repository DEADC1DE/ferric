// Toolbar icons: flat 18x18 inline SVG, 1.5 stroke, square caps. Each icon mixes two or three hues.
// Colours come from the --ic-* tokens in app.css (one set per theme) through the i-* classes:
//   i-g   green fill + pine outline          i-gd  solid pine fill
//   i-b   blue fill + navy outline           i-bl  navy outline only     i-bd  solid navy fill
//   i-r / i-o / i-p / i-t   rose, orange, violet, teal: fill + dark outline
//   i-rl / i-ol / i-pl / i-tl   bright outline only     i-rd / i-od / i-pd / i-td   solid dark fill
//   i-sl  slate outline only     i-sd  solid slate fill  (neutral frames and dots)
// The yellow tokens (--ic-yellow, --ic-amber) are reserved for a Help icon; no Help action exists yet.
const svg = (body) =>
  `<svg class="tb-icon" viewBox="0 0 18 18" width="18" height="18" aria-hidden="true" focusable="false">${body}</svg>`;

export const icons = {
  // Plug: green body, rose prongs, teal cable.
  connect: svg(
    '<path class="i-rl" d="M7 2v4M11 2v4"/>' +
      '<path class="i-g" d="M4.75 6.75h8.5v3.5l-2 2.5h-4.5l-2-2.5z"/>' +
      '<path class="i-tl" d="M9 12.75v3.25"/>',
  ),
  // Server stack: green slabs with rose lamps.
  connectAll: svg(
    '<rect class="i-g" x="2.75" y="2.75" width="12.5" height="3.5"/>' +
      '<rect class="i-g" x="2.75" y="7.25" width="12.5" height="3.5"/>' +
      '<rect class="i-g" x="2.75" y="11.75" width="12.5" height="3.5"/>' +
      '<rect class="i-rd" x="4.5" y="4" width="1.5" height="1"/>' +
      '<rect class="i-rd" x="4.5" y="8.5" width="1.5" height="1"/>' +
      '<rect class="i-rd" x="4.5" y="13" width="1.5" height="1"/>',
  ),
  // Key: orange head, violet bit, orange shaft.
  fishKeys: svg(
    '<rect class="i-o" x="2.75" y="5.75" width="6" height="6"/>' +
      '<rect class="i-pd" x="5" y="8" width="1.5" height="1.5"/>' +
      '<path class="i-ol" d="M8.75 8.75h6.5M12.5 8.75v3M15.25 8.75v2"/>',
  ),
  // Channel tile with a hash sign (blue, navy).
  join: svg(
    '<rect class="i-b" x="2.75" y="2.75" width="12.5" height="12.5"/>' +
      '<path class="i-bl" d="M7.25 5.25v7.5M10.75 5.25v7.5M5.25 7.5h7.5M5.25 10.5h7.5"/>',
  ),
  // Channel list: blue rows with teal bullets.
  channelList: svg(
    '<rect class="i-b" x="6.75" y="2.75" width="8.5" height="3"/>' +
      '<rect class="i-b" x="6.75" y="7.5" width="8.5" height="3"/>' +
      '<rect class="i-b" x="6.75" y="12.25" width="8.5" height="3"/>' +
      '<rect class="i-td" x="2.75" y="3.25" width="2" height="2"/>' +
      '<rect class="i-td" x="2.75" y="8" width="2" height="2"/>' +
      '<rect class="i-td" x="2.75" y="12.75" width="2" height="2"/>',
  ),
  // Mixer sliders: slate tracks with rose, orange and teal knobs.
  options: svg(
    '<path class="i-sl" d="M5 2.75v12.5M9 2.75v12.5M13 2.75v12.5"/>' +
      '<rect class="i-r" x="3.25" y="4.75" width="3.5" height="2.5"/>' +
      '<rect class="i-o" x="7.25" y="10.75" width="3.5" height="2.5"/>' +
      '<rect class="i-t" x="11.25" y="6.75" width="3.5" height="2.5"/>',
  ),
  // Script page: violet sheet, teal text lines, slate fold.
  scripts: svg(
    '<path class="i-p" d="M4.75 2.75h6.5l2.5 2.5v10h-9z"/>' +
      '<path class="i-sl" d="M11.25 2.75v2.5h2.5"/>' +
      '<path class="i-tl" d="M7 8.75h4M7 11.75h4"/>',
  ),
  // Capital A: rose strokes with an orange crossbar.
  font: svg(
    '<path class="i-rl" d="M3.75 15.25L9 2.75l5.25 12.5"/>' +
      '<rect class="i-od" x="5.5" y="10.25" width="7" height="1.5"/>',
  ),
  // Painter palette with colour spots: slate outline, green, navy and rose dots.
  colors: svg(
    '<path class="i-sl" d="M9 2.5a6.5 6.5 0 1 0 6.5 6.5c0-.83-.67-1.5-1.5-1.5h-1.5a2 2 0 0 1-2-2V4a1.5 1.5 0 0 0-1.5-1.5z"/>' +
      '<circle class="i-gd" cx="6" cy="6" r="1"/>' +
      '<circle class="i-bd" cx="6" cy="11" r="1"/>' +
      '<circle class="i-rd" cx="11" cy="11" r="1"/>',
  ),
  // Address Book: teal booklet, violet spine, rose 'A'.
  addressBook: svg(
    '<rect class="i-t" x="3.25" y="2.75" width="11.5" height="12.5" rx="1"/>' +
      '<path class="i-pl" d="M6 2.75v12.5"/>' +
      '<path class="i-rl" d="M8.5 10.5l2-5.5 2 5.5M9.2 8.5h2.6"/>',
  ),
};
