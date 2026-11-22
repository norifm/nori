# Simplified Technical English, for nori's docs

ASD-STE100 (Issue 9, 2025) is a controlled language from aircraft maintenance manuals. Its aim is that a
reader who cannot ask the author still reads one meaning. That is the reader of docs too. This page keeps
the rules that help software docs and drops the ones that hurt them. It follows the rule summary in
[danyuchn/asd-ste100-skill](https://github.com/danyuchn/asd-ste100-skill) (MIT), in its "STE-flavored"
mode: structural rules in full, word rules as a direction.

## Rules we keep

| Rule | Do | Do not |
|---|---|---|
| Short sentences | At most 20 words in a step, 25 in a description | One sentence holding a condition, an action and an exception |
| One instruction per sentence | "Open **Settings**. Tap **Sound**." | "Open Settings and tap Sound, then check that..." |
| Active voice | "nori sends your plays to the server." | "Your plays are sent to the server." |
| Simple tenses | "You added a server." | "You have added a server." |
| No semicolons | Two sentences | `;` anywhere |
| Verbs, not nouns | "nori measures the song." | "nori performs a measurement of the song." |
| No phrasal verbs | "remove", "start", "contact", "read" | "take out", "spin up", "reach out", "dive into" |
| Short noun stacks | "the stream cache size" (3 words) | "the stream cache size limit setting value" |
| No dropped words | "Songs that you did not download are lost." | "Songs not downloaded lost." |
| One topic per paragraph | At most about 6 sentences | A paragraph about two settings |
| Lists for sequences | Numbered steps for 3 or more actions | A sequence hidden in a sentence |
| Keep the hedge | "The server might not send lyrics." stays "might" | Turning "might" into a fact, or a fact into "might" |
| Warnings first | "Caution: this deletes your downloads. Tap **Clear**." | The warning after the action |

## Rules we relax

| STE rule | Why we relax it | What we do instead |
|---|---|---|
| Only words from the approved dictionary | It lacks software words (sign in, sync, stream, download) | Plain, common words. The glossary fixes nori's own terms |
| One word for every action, everywhere | Readers search with their own words | One name for each nori concept. General verbs can vary in explanation pages |
| Flat, uniform tone everywhere | "How it works" pages need some flow to explain an idea | Strict on steps and reference. Natural on explanation, still short |
| No present perfect, ever | Status can need it ("The download has finished" means it is done now) | Simple past by default. Keep present perfect when "now" is the point |

## Examples from nori's docs

| Before | After | Rule |
|---|---|---|
| "Each profile tells its server what you play (scrobbles, now playing and the saved queue) unless you turn that off in the profile's **Scrobbling** part; whether the server passes plays on is the server's setting." | "nori tells your server what you play. To stop it, turn off **Scrobbling** in the profile. Your server decides whether to pass your plays on." | One idea per sentence, no semicolon |
| "A profile goes out once at a time, and how a new one was asked for comes back only with its acceptance." | "nori sends one new profile at a time. It learns how the profile was added only when the server accepts it." | Full sentences, no private shorthand |
| "Songs above it are converted down by a factor of two where they can be." | "nori halves the sample rate of songs above the limit when it can: 96 kHz becomes 48 kHz." | Active voice, a concrete example |
| "Grayed out while AutoMix is on." | "You cannot change it while AutoMix is on." | Say what it means for the reader |
