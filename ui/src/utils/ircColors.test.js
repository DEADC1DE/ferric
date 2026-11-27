import test from "node:test";
import assert from "node:assert/strict";
import { sliceRawIrcText, sliceSpansToRawIrc } from "./ircColors.js";

test("sliceRawIrcText preserves active color and bold across partial selection", () => {
  const raw = "hi \x02\x0304,01hello world\x0F bye";
  // Visible: "hi hello world bye" -> slice "world" (9..14)
  assert.equal(sliceRawIrcText(raw, 9, 14), "\x02\x0304,01world\x0F");
});

test("sliceRawIrcText keeps inline formatting transitions", () => {
  const raw = "\x0303green \x1Ditalic\x0F plain";
  assert.equal(sliceRawIrcText(raw, 0, 18), "\x0303green \x1Ditalic\x0F plain");
});

test("sliceSpansToRawIrc reconstructs control codes from spans", () => {
  const spans = [
    { text: "plain " },
    { text: "red", fg: 4, bold: true },
    { text: " end" },
  ];
  assert.equal(sliceSpansToRawIrc(spans, 0, 13), "plain \x02\x0304red\x0F end");
  assert.equal(sliceSpansToRawIrc(spans, 6, 9), "\x02\x0304red\x0F");
});
