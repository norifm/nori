// Draws each docs/developers/diagrams/<name>.d2 as <name>.svg and <name>-dark.svg, in nori's Day and Night
// colors; a page shows the pair with <picture>, so GitHub picks the one its theme needs. `--check` draws
// them in memory and fails when a committed SVG differs from its source.
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { D2 } from '@d2lang/d2';

const folder = new URL('../../docs/developers/diagrams/', import.meta.url);

// Nori Seasons' roles (crates/look/src/theme.rs, Spring) as D2's color codes. N7 is the page behind arrow
// labels: GitHub's own background, so a label sits on the page it is read on.
const themes = {
  light: {
    ink: '#26262A', quiet: '#6E6D71', line: '#C4C3C8', surface: '#F6F5F8', group: '#EEEDF2',
    page: '#FFFFFF', accent: '#B14C66', you: '#F7E9ED',
  },
  dark: {
    ink: '#ECEBEF', quiet: '#9F9EA3', line: '#4A4950', surface: '#1B1B20', group: '#16161B',
    page: '#0D1117', accent: '#E9A0B1', you: '#2E1F25',
  },
};

function preamble(t) {
  return `vars: {
  d2-config: {
    theme-overrides: {
      N1: "${t.ink}"; N2: "${t.quiet}"; N3: "${t.quiet}"; N4: "${t.line}"; N5: "${t.line}"; N6: "${t.surface}"
      N7: "${t.page}"; B1: "${t.quiet}"; B2: "${t.quiet}"; B3: "${t.line}"; B4: "${t.group}"; B5: "${t.group}"
      B6: "${t.surface}"; AA2: "${t.quiet}"; AA4: "${t.group}"; AA5: "${t.surface}"; AB4: "${t.group}"
      AB5: "${t.surface}"
    }
  }
}
classes: {
  you: { style: { stroke: "${t.accent}"; stroke-width: 2; fill: "${t.you}" } }
}
**.style.border-radius: 8
`;
}

const d2 = new D2();

async function draw(source, theme) {
  const layout = source.match(/layout-engine\s*:\s*(\w+)/)?.[1] ?? 'elk';
  const { diagram, renderOptions } = await d2.compile(preamble(themes[theme]) + source, { layout });
  const svg = await d2.render(diagram, { ...renderOptions, pad: 16 });
  // A width and height of its own, so a page shows a small diagram at its size rather than stretched.
  const [, width, height] = svg.match(/viewBox="[\d.-]+ [\d.-]+ ([\d.]+) ([\d.]+)"/);
  return svg.replace('<svg ', `<svg width="${width}" height="${height}" `);
}

const check = process.argv.includes('--check');
const stale = [];
for (const file of readdirSync(folder).filter((name) => name.endsWith('.d2')).sort()) {
  const source = readFileSync(new URL(file, folder), 'utf8');
  for (const [theme, suffix] of [['light', ''], ['dark', '-dark']]) {
    const out = new URL(file.replace(/\.d2$/, `${suffix}.svg`), folder);
    const svg = await draw(source, theme);
    if (!check) writeFileSync(out, svg);
    else if (readFileSync(out, 'utf8') !== svg) stale.push(out.pathname);
  }
}
await d2.dispose();
if (stale.length) {
  console.error(`Drawn from an older source; run npm run draw in tools/diagrams:\n  ${stale.join('\n  ')}`);
  process.exit(1);
}
