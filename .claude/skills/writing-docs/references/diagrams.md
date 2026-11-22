# Diagrams that read cleanly

These rules come from graph-readability research (Purchase; Huang; Ghoniem), perception (Larkin and Simon;
Ware), the C4 model, the Google style guide, and the diagrams people point to as clear: Tailscale's "How
Tailscale works" and Julia Evans' drawings. The sources are in `docs/research/writing-docs.md` and its
notes.

## Before you draw

- **Ask whether the reader needs it.** On a user page, a diagram earns its place only when a listener
  would wonder about the thing it shows (where their data goes, what plays next) and gets it faster from
  the picture. Mechanism is for "How it works" pages and the developer docs. A diagram that is only
  decoration, or that repeats a two-line paragraph, is left out.

- **Name the one idea.** "Who nori talks to", "What plays next". If you need "and", draw two diagrams.
- **Choose the kind by the idea:**

  | The idea | Kind | D2 |
  |---|---|---|
  | Parts and what moves between them | Boxes and labeled arrows | plain shapes and `->` |
  | An order, or layers | A stack of boxes, no arrows | `grid-rows: N` |
  | A fallback, or a decision | Boxes with a labeled arrow per outcome | `->` with labels, `direction: down` |
  | A request through several parts, in time | Sequence | `shape: sequence_diagram` |
  | Something that repeats | Two boxes and a loop | `a -> b` and `b -> a` |
  | Places things live in | Groups | a container: `home: Your network { phone; laptop }` |

- **Write the text first.** The diagram shows the shape of what the text says. It holds no fact the text
  lacks.

## Make it read

- **Crossings are the worst.** Lines that cross cost more than any other flaw. Reorder the shapes, drop an
  arrow the text already states, or split the diagram.
- **Small.** About eight boxes. Past twenty, people stop reading a diagram as a picture. A sequence has at
  most five or six parties and about twelve messages.
- **Short labels.** A box says what the part is in one to four words: "Your server", "nori-engine thread".
  An arrow says what moves or what happens, in one to four words: "the song", "samples, through the
  ring", "no timed lyrics". Never "uses" or "calls".
- **Arrows point one way**, in the direction the thing moves. A reply in a sequence is dashed:
  `b -> a: answer {style.stroke-dash: 4}`.
- **One emphasis.** `class: you` puts the accent on one shape: the reader's device, or what they see or
  hear. Everything else stays ink. Do not color shapes by kind.
- **Same words as the text.** A box named "The device playing" is called that in the text too.
- **Caption as a claim or a name.** On the site, the first line `# Who nori talks to`. In the developer
  docs, the `alt` of the `<img>` says what the diagram shows, in one sentence.

## Shape the layout

- Leave `direction` out on the site: it draws left to right on wide screens and top to bottom on phones.
  Set `direction: down` when the order is top to bottom by nature (a fallback, layers).
- The layout engine is TALA on the site and ELK in the developer docs. When a site diagram comes out
  tangled, try ELK: `vars: { d2-config: { layout-engine: elk } }`.
- A phone shows about 360 px. A row of three boxes with long labels scrolls there. Keep rows to two or
  three short boxes.
- Look at it. Build and open the page (or the SVG) in light and dark, wide and narrow, before you commit.

## Examples

A fan-out, from the privacy page:

```d2
# Who nori talks to
nori: nori on your device {class: you}
server: Your server
services: Services you turned on
checks: Update and plugin checks
nori -> server: library, music, plays
nori -> services: lyrics, scrobbles, plugins
nori -> checks: once a day
```

An order, from the queue page:

```d2
# The queue, from what plays now to what plays last
grid-rows: 5
vertical-gap: 10
playing: The song playing {class: you}
next: Play next, newest first
list: The rest of the list you played
added: Add to queue, in order
auto: Autofill
```

A sequence, from the developer docs:

```d2
shape: sequence_diagram
app: A client {class: you}
worker: Support service
stripe: Stripe
app -> worker: POST /start
worker -> stripe: start the payment
stripe -> worker: client secret {style.stroke-dash: 4}
worker -> app: login and secret {style.stroke-dash: 4}
```

## Developer docs on GitHub

GitHub renders only Mermaid, and Mermaid's colors cannot follow nori's look there. So developer diagrams
are D2 files drawn into SVG:

1. Write `docs/developers/diagrams/<name>.d2`.
2. Run `npm install` once, then `npm run draw`, in `tools/diagrams`. It writes `<name>.svg` (Day) and
   `<name>-dark.svg` (Night).
3. Show it in the page:

   ```html
   <picture>
     <source media="(prefers-color-scheme: dark)" srcset="diagrams/<name>-dark.svg">
     <img alt="What the diagram shows, in one sentence" src="diagrams/<name>.svg">
   </picture>
   ```

4. Commit the `.d2` and both SVGs together. `npm run check` fails when an SVG is older than its source.
