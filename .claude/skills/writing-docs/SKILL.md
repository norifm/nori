---
name: writing-docs
description: Write or rewrite nori's documentation so people understand it on the first read. Use for any page in the user docs (norifm.com, src/content/docs/), the developer docs (docs/developers/), the plugin docs, READMEs and plugin READMEs, and for any docs review. Covers page types, page shapes, wording (Simplified Technical English, adapted), tables, diagrams, links, the nori glossary, the habits that make AI-written docs bad, and a lint script.
---

# Writing nori's docs

A doc is good when a reader finds what they came for in seconds and believes it. Readers scan: most read a
fifth of a page's words, jump from heading to heading, and see the first two words of a line. They arrive
from search, on any page. Write every page for that reader.

Most bad docs fail on content, not on wording. They cover the easy parts evenly, skip the hard parts (what
fails, what it needs, what the feature does not do), repeat facts, drift between names for one thing, and
state things nobody checked. The opposite failure is as common: a page that says everything the code does.
A reader needs what changes what they do or what they understand, and nothing else. Fix the content first.
The wording rules come second, and the lint comes last.

## The process

Do these steps in order, for every page.

1. **Decide.** Write down, for yourself:
   - the reader: a listener, a curious listener, a contributor, or a plugin author;
   - the one question or task that brings them to this page;
   - the page type (see [Page types](#page-types)).
2. **Gather.** Read the code before you write. List every fact the page will state (a setting's label and
   default, a limit, a path, a command, an error, which clients have it) with the file it came from. The
   code is the source. The current docs are only a lead: they can be stale.
   Collect the hard parts on purpose, but only the ones a reader will meet: what commonly goes wrong and
   how to recover, what it needs (a server, an account), its battery or data use when it is noticeable,
   what it does not do.
3. **Write** the page in its shape ([references/page-shapes.md](references/page-shapes.md)), with the
   wording rules below.
4. **Cut.** For each sentence, ask: what does the reader lose if I delete it? If the answer is "nothing",
   delete it. Apply [What to leave out](#what-to-leave-out). Then remove the habits in
   [references/ai-tells.md](references/ai-tells.md). Expect to cut a third of a first draft.
5. **Check.** Run the lint, then re-read the page against your fact list:

   ```sh
   python3 .claude/skills/writing-docs/scripts/docs-lint.py <files>
   ```

   Every label, default, number, path and command must match the code. Remove what you cannot check, or
   mark it `TODO(verify)` and say so in your report.

## Page types

Every page has one type. A sentence that belongs to another type becomes a link to the page where it
belongs.

| Type | The reader | It holds | It does not hold | Title |
|---|---|---|---|---|
| Feature page (user) | wants to use a feature | what it does for you, how to turn it on, options by effect, costs, limits | how it is built | the feature's name: "AutoMix" |
| How-to (user or dev) | has a goal | prerequisites, numbered steps, the result, recovery | teaching, every option | a task: "Connect a server", "Add a setting" |
| How it works (curious user) | wants the idea | the model in plain words, one diagram, why it was built so | steps, code names, every option | "How AutoMix works" |
| Reference | looks something up | complete, neutral lists and tables | instructions, opinions | the thing: "Settings", "Manifest" |
| Concept (dev) | needs the design | the parts, the data path, invariants, where the code is | step-by-step procedures | the system: "Architecture", "The plugin host" |
| Tutorial (dev) | learns by doing | one path with visible results at every step | options, alternatives, theory | an outcome: "Build your first plugin" |

User pages never explain the implementation. They link one level down, to a "How it works" page, with
link text that names it ("[How AutoMix picks a transition](...)"). The "How it works" page may link to the
developer docs. Never more than two levels.

## What to leave out

A fact belongs on a page only if it changes what the reader does or what they understand. True is not
enough. Leave out:

- **What the screen already says.** A message that explains itself, a confirmation dialog's text, an
  **Undo** after every action, a list's sort order.
- **Trivia.** How two devices with one name are told apart, which list a row appears in on one client,
  what happens in a case few readers will meet.
- **Every client's path, in prose.** When the steps differ per client, give one small table (app, how),
  or name the most common path and put the others in the client pages.
- **Label differences between clients.** One line on the client's page, or the settings reference. Not
  on every feature page.
- **Mechanism on a user page.** Why it works goes to "How it works". The feature page says what you get.
- **Restated defaults and limits that do not matter.** Keep a default when a reader would change it.
  Keep a limit when a reader will hit it.
- **Reassurance and asides.** "You can change this later", "This is optional", "nori handles this for
  you".
- **On developer pages:** anything the code states better (each field, each branch, each test name).
  State each part's job, its rules, where it lives and how the parts connect. Everything a contributor
  needs to change the design safely, nothing they would find by reading one function.

Rough lengths, as a check, not a target: a feature page 200 to 450 words, a "How it works" page 400 to
600, a how-to as long as its steps. A page much longer than that holds two topics, or trivia.

## Wording

These rules adapt ASD-STE100, Simplified Technical English, to software docs. The details and examples
are in [references/ste.md](references/ste.md).

- **Lead with the point.** The first sentence of a page says what the thing is or what the page lets
  you do. The first words of a heading, list item, table row and link carry its meaning. Siblings do
  not start with the same words.
- **One idea per sentence.** A step has at most 20 words. A description has at most 25. Split longer
  sentences when they hold two ideas.
- **Active voice, present tense, "you".** "nori saves the queue", not "the queue is saved" or "nori will
  save the queue". Simple past, not present perfect: "you added", not "you have added".
- **Full sentences.** Keep articles, verbs and connectives. Telegraphic text ("Kept per output. Grayed
  out while on.") is the opposite failure of padding, and it is just as hard to read. Each sentence must
  read on its own.
- **One name for one thing.** Use the terms in [references/glossary.md](references/glossary.md) and the
  exact labels from the app. Do not rotate synonyms. Do not coin terms: say what the thing is, or define
  the term once.
- **One home for each fact.** State a fact on one page. Other pages link to it.
- **Plain words.** "use", not "utilize" or "leverage"; "start", not "spin up"; "about", not "approximately".
  No phrasal verbs where one verb exists ("remove", not "take out").
- **Words with fixed meanings.** "must" (required), "can" (possible), "might" (possibly true). Never
  "simply", "just", "easy", "please", "note that", "currently", "new", "in order to", "e.g.", "i.e.".
- **No semicolons.** Write two sentences.
- **No history and no future.** Describe the product as it is. No "now", "no longer", "we added", "soon".
  Versions go in the changelog.

### UI words

- Labels in bold, exactly as the app shows them: tap **Save**.
- Paths with `›`: **Settings › Playback › Between songs**.
- "Tap" on Android and the iPod, "click" on the desktop app, "select" or "choose" when it covers both, "press"
  for keys. Never "click on".
- Name clients as the glossary does: Android, the desktop app, the terminal app, the iPod app.

## Structure and formatting

- **Headings** are sentence case, with no end punctuation, and at most three levels (`##`, `###`).
  A heading names what is under it: "When AutoMix does not mix", not "Limitations". A reader who only
  glances reads the headings: never "What it costs" for battery or data, which reads as a price. Write
  "Battery use" or "Battery and data use".
- **Steps** are a numbered list. Introduce them with a sentence. One action per step, location first:
  "In **Settings › Sound**, tap **Equalizer**." Put the result after the action. If a step can fail, the
  next line says how to recover.
- **Lists** are for three or more parallel items. Do not start every item with a bold label and a colon.
- **Bold** is for UI labels and, rarely, the one word a reader must not miss. Not for emphasis in running
  prose.
- **Notices** (`:::note`, `:::caution`, `> **Note**`) are rare. Never two in a row. Never put a step or a
  prerequisite in one. A caution is only for data loss, money, security or an action you cannot undo.
- **Code** formatting is for code, paths, commands, keys and values a user types. Not for feature names.
- **Screenshots** only where position or look cannot be put into words.

### Tables

Use a table only when several items share three or more attributes that a reader compares. Otherwise use
a list or prose.

- The left column names the row. Rows have a logical order, or alphabetical order.
- A cell holds a value, a short phrase or one short sentence. If a cell needs two sentences, the content
  belongs in prose under a heading.
- Headers are specific nouns in sentence case: "Default", not "Value".
- No empty cells and no lone dashes: write "None" or "No".
- Introduce the table with a sentence.
- Right-align numbers that readers compare (`---:`).

Bad: `| Missing lyrics | The lyrics services you have on, and Apple's iTunes Search for PaxSenix to find the song | Artist, song and album name, the song's length; your key, to a service you gave one | ...`

Good: one row per request, each cell a short value, and the detail in a sentence under the table or on
the page it belongs to.

### Diagrams

Draw a diagram when the layout carries information: parts and what flows between them, layers, an order
that matters (what plays next), a fallback (what nori tries first, then next), states, or messages over
time. Look for one on every page that has such a structure: readers who glance see a diagram before any
text. A list of steps is a numbered list, not a flowchart. A diagram never holds a fact the text does not
state: the text around it must make sense without it.

Every diagram is D2, drawn at build time into SVG in nori's colors. How to write one that reads cleanly,
with examples, is [references/diagrams.md](references/diagrams.md). In short:

- **On user pages, only what a listener wonders about**: where their data goes, what plays next, why
  the phone sleeps. At most five boxes, in a line or with one fork. Never a diagram of nori's internals,
  and never one that needs the text to be understood. If a listener would not miss it, leave it out.
- **One idea per diagram**, at most about eight boxes, or five or six parties in a sequence. Split a
  bigger one into steps, each adding one part to the same picture.
- **No crossing lines.** If the layout crosses, remove an arrow, reorder, or split the diagram.
- **Short words.** Box labels of one to four words, arrow labels of one to four words, naming what moves.
- **One accent:** `class: you` marks the reader's own part (their device, what they hear), once.
- **User docs (norifm.com):** a ` ```d2 ` block in the page; its first line `# Caption` is its title.
  The site draws it left to right on wide screens and top to bottom on phones.
- **Developer docs (GitHub):** a `docs/developers/diagrams/<name>.d2` file, drawn by
  `cd tools/diagrams && npm run draw` into `<name>.svg` and `<name>-dark.svg`, shown with `<picture>`.

## Linking

- Link text names the page it opens: "see [Offload](...)". Never "here", "this page", "read more" or a
  bare URL.
- User docs link to each other as `.md` files (`../sound/automix.md`). The site turns them into pages and
  fails the build on a broken link.
- Developer docs link to code by path in backticks (`crates/engine/src/hls.rs`), not with links that rot.
- A user page that has a "How it works" page ends its main part with one line linking to it.

## Facts and scope

- The code is the truth. A fact in two places is a bug: link instead.
- Each page says which clients it covers, in its first lines, when that is not every client.
  The docs index says which nori version the docs describe.
- User docs never name crates, types or files. Developer docs name them, in backticks, so a reader can
  search for them.
- The monorepo's `TODO.md` and `docs/research/` are never published.
- User-visible changes update the user docs in the website repository, `norifm/norifm.com`, as a commit
  there. Developer docs change in the same commit as the code.

## Before you finish

- [ ] The first sentence says what the page is about, for this page's reader.
- [ ] Every fact was checked in the code. Labels match the app exactly.
- [ ] The hard parts are there: failure, cost, limits.
- [ ] No sentence repeats another page. No sentence restates its heading.
- [ ] Tables hold short values. Diagrams carry structure the text also states.
- [ ] `docs-lint.py` reports nothing you did not decide to keep.
- [ ] Read it once as the reader. Cut what they would skip.

The research behind these rules, with sources: `docs/research/writing-docs.md`.
