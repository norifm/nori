// Inter and Literata from the Android app into web/src/fonts as WOFF2, each in two files: Latin, which
// nearly every page needs, and the rest (Greek, Cyrillic, symbols), which a browser fetches only for a
// character in it. Writes the @font-face rules beside them. The variable axes and features are kept.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import subsetFont from "subset-font";

const here = dirname(fileURLToPath(import.meta.url));
const source = join(here, "../app/src/main/res/font");
const out = join(here, "src/fonts");

// What a browser turns on itself, and the tabular digits the times use.
const FEATURES = ["rvrn", "rclt", "rlig", "ccmp", "locl", "mark", "mkmk", "kern", "liga", "clig", "calt", "tnum"];
const FONTS = [
  { family: "Inter", file: "inter", weight: "100 900" },
  { family: "Literata", file: "literata", weight: "200 900" },
];
// Latin with Extended-A, its marks, punctuation and the signs a title may hold.
const LATIN = [
  [0x0000, 0x017f],
  [0x0300, 0x036f],
  [0x2000, 0x206f],
  [0x20a0, 0x20bf],
  [0x2122, 0x2122],
  [0x2190, 0x2193],
  [0x2212, 0x2212],
  [0x2215, 0x2215],
  [0xfeff, 0xfeff],
  [0xfffd, 0xfffd],
];
const REST = [];
for (let i = 0; i <= LATIN.length; i++) {
  const from = i ? LATIN[i - 1][1] + 1 : 0;
  const to = i < LATIN.length ? LATIN[i][0] - 1 : 0x10ffff;
  if (from <= to) REST.push([from, to]);
}

const text = (ranges) => ranges.flatMap(([from, to]) => Array.from({ length: to - from + 1 }, (_, i) => from + i)).filter((c) => c < 0xd800 || c > 0xdfff).map((c) => String.fromCodePoint(c)).join("");
const css = (ranges) => ranges.map(([from, to]) => `U+${from.toString(16)}-${to.toString(16)}`).join(",");

mkdirSync(out, { recursive: true });
let rules = "";
for (const { family, file, weight } of FONTS) {
  const font = readFileSync(join(source, `${file}.ttf`));
  for (const [part, ranges] of [["latin", LATIN], ["rest", REST]]) {
    writeFileSync(join(out, `${file}-${part}.woff2`), await subsetFont(font, text(ranges), { targetFormat: "woff2", keepFeatures: FEATURES }));
    rules += `@font-face {\n  font-family: "${family}";\n  src: url("./${file}-${part}.woff2") format("woff2");\n  font-weight: ${weight};\n  font-display: swap;\n  unicode-range: ${css(ranges)};\n}\n\n`;
  }
}
writeFileSync(join(out, "fonts.css"), rules);
