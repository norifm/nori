# Habits that make docs bad

Readers call a text "slop" mostly because it is irrelevant, thin or in the wrong tone, not because of a
word. So the content habits come first. A page with none of the words below can still be empty.

Sources: Wikipedia, "Signs of AI writing"; Shaib et al., "Measuring AI 'Slop' in Text" (2025); Kobak et al.,
"Delving into ChatGPT usage in academic writing" (2024); Fabrizio Ferri-Benedetti (passo.uno); V. Winland,
"AI wrote your docs"; the Google and GitLab style guides.

## Content habits

| Habit | Example | Fix |
|---|---|---|
| Even coverage | Every option gets two lines, errors get none | Spend words on what goes wrong, what it costs, what it cannot do |
| Restating the obvious | "The **Save** button saves your changes." | Delete it |
| Restating the code | A paragraph per function, in order | Describe the design and the invariants. The code says the rest |
| Invented facts | A command, a flag or a default that sounds right | Check every one in the code. Remove what you cannot check |
| Context before action | Three paragraphs of background before step 1 | Steps first, background after or on its own page |
| Repeated facts | The same prerequisite in the intro, the setup and troubleshooting | One home, links elsewhere |
| Sections that only work in order | "As mentioned above, ..." | Each section stands on its own |
| Length as quality | A page twice as long as the feature needs | Cut until each sentence carries a fact |
| Implementation for users | "The engine fills a 10 s AudioTrack ring" on a user page | Say the effect ("nori decodes ahead and lets the phone sleep"). Link to "How it works" |
| Telegraphic compression | "Kept per output. Off on USB. Grayed out while AutoMix on." | Full sentences that say how facts relate: "nori keeps this setting for each output." |
| Private jargon | "the door", "a burst", "its seal paid" with no definition | Plain words, or define the term once and use the glossary's name |

## Wording habits

| Habit | Example | Fix |
|---|---|---|
| Inflated significance | "plays a crucial role", "a pivotal component", "ensuring seamless playback" | State what it does: "decodes the song" |
| Fake analysis with -ing | "..., highlighting nori's focus on battery." | Delete the clause, or make it a fact with a number |
| Avoiding "is" and "has" | "serves as", "boasts", "features", "stands as" | "is", "has" |
| Straw-man contrast | "It's not just a player, it's a ..." / "X rather than Y" when nobody said Y | State X |
| Padded triads | "fast, reliable and efficient" | Keep the one that is true and measured |
| Hedging | "It's important to note that this may potentially..." | State the claim, or cut it. Keep a real "might" |
| Summaries | "In summary, ..." / "Overall, ..." at the end of a section | Delete. The headings are the summary |
| Intros that restate the title | "This page describes AutoMix." under "AutoMix" | Start with what AutoMix does |
| Marketing words | seamless, robust, powerful, effortless, blazing-fast, cutting-edge | Delete, or give the measurement |
| Filler words | simply, just, easily, please, note that, basically, actually | Delete |
| AI vocabulary | delve, tapestry, landscape, realm, leverage, utilize, underscore, showcase, foster, embark | The plain word: look at, use, show |
| Bold-label lists | "- **Speed:** The speed of ..." for every item | A plain list, or a table if items share attributes |
| Emoji and decoration | ✅ ⚡ 🚀 in headings or lists | Delete. Use "Yes"/"No" in tables |
| Title Case Headings | "Connecting To Your Server" | Sentence case: "Connect to your server" |
| Synonym rotation | "profile", "account", "connection" for one thing | One glossary term |
| Punctuation swaps | Every dash replaced by a period, giving rows of tiny sentences | Write natural sentences. Join two short ones that make one idea |

## When you edit someone's draft

- Compare it against the code. List every sentence that conflicts with the code or that the code does not
  support.
- Ask for the hard parts if they are missing: errors, edge cases, what the reader cannot do.
- Edit by hand a draft that is nearly right. A rewrite from scratch loses the facts that were right.
