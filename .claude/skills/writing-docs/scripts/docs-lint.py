#!/usr/bin/env python3
"""Mechanical checks for nori's docs (Markdown).

Flags what a rule can find without judgment: filler and AI words, long sentences, semicolons, vague link
text, title-case headings, bold-label lists, stacked notices, wordy table cells, and (as advice) passive
voice and present perfect. It cannot tell whether a page is useful: see SKILL.md for that.

Usage:
    docs-lint.py FILE_OR_DIR [...]          # exit 1 when a rule (not advice) fires
    docs-lint.py --advice FILE              # also print passive voice and present perfect
    docs-lint.py --disable long-sentence FILE

The sentence checks follow the structural rules of ASD-STE100 as summarized by
github.com/danyuchn/asd-ste100-skill (MIT).
"""
import argparse
import pathlib
import re
import sys

STEP_WORDS = 20
DESCRIPTION_WORDS = 25
CELL_WORDS = 16

WORDS = {
    "filler": r"simply|just|easily|basically|actually|really|please|note that|in order to|it is worth noting"
    r"|it's worth noting|it is important to note|it's important to note|needless to say|of course",
    "marketing": r"seamless(?:ly)?|robust|powerful|effortless(?:ly)?|blazing[- ]fast|cutting[- ]edge"
    r"|state[- ]of[- ]the[- ]art|world[- ]class|game[- ]chang\w+|best[- ]in[- ]class|lightning[- ]fast",
    "ai-vocabulary": r"delv\w*|tapestry|landscape|realm|leverag\w+|utiliz\w+|underscor\w+|showcas\w+"
    r"|foster\w*|embark\w*|pivotal|crucial|vital|navigat(?:e|es|ing) the|elevat\w+|unlock\w*|empower\w*"
    r"|serves as|stands as|boasts|a testament|plays a (?:key|crucial|vital|pivotal) role",
    "time-words": r"currently|as of now|no longer|from now on|coming soon|in the future|we added|recently",
    "latin": r"e\.g\.|i\.e\.|etc\.|via",
    "summary": r"in summary|in conclusion|overall,|to sum up|in short,",
    "direction": r"see above|see below|as (?:mentioned|described|shown) (?:above|below|earlier)"
    r"|the (?:above|following) (?:section|table|list)",
    "click-on": r"click on|tap on",
}
WORD_PATTERNS = {rule: re.compile(rf"(?<![\w-])(?:{words})(?![\w-])", re.I) for rule, words in WORDS.items()}

PHRASAL = re.compile(
    r"\b(spin(?:s|ning)? up|spun up|reach(?:es|ing|ed)? out|div(?:e|es|ing|ed) into|kick(?:s|ing|ed)? off"
    r"|carr(?:y|ies|ied) out|figur(?:e|es|ed) out|find(?:s)? out|turn(?:s|ed)? out to)\b",
    re.I,
)
VAGUE_LINK = re.compile(r"\[(here|this|this page|link|read more|learn more|more|click here|see more)\]\(", re.I)
BARE_URL = re.compile(r"(?<![(<\[`])\bhttps?://\S+")
PASSIVE = re.compile(r"\b(?:is|are|was|were|be|been|being)\s+(?:\w+ly\s+)?(\w+ed|shown|known|kept|made|sent|"
                     r"set|done|given|taken|found|built|written|read|held|run)\b", re.I)
PERFECT = re.compile(r"(?<!may )(?<!might )(?<!could )(?<!would )(?<!should )(?<!must )\b(?:has|have)\s+"
                     r"(?:been\s+)?(?:\w+ed|shown|known|kept|made|sent|done|given|taken|found|built|written|"
                     r"gone|seen|begun|become|run)\b", re.I)
EMOJI = re.compile("[\U0001F300-\U0001FAFF☀-➿⭐✅]")
BOLD_LABEL = re.compile(r"^\s*[-*]\s+\*\*[^*]+?(?::\*\*|\*\*:)")
NOTICE = re.compile(r"^\s*(?::::\s*(?:note|tip|caution|danger)|>\s*\*\*(?:note|tip|warning|caution)\b|>\s*\[!)", re.I)

# Capitalized words that are names, not title case.
NAMES = set("""nori Android AutoMix AutoEQ YouTube Music SoundCloud Navidrome Subsonic OpenSubsonic octo-fiesta
Bluetooth iPod iOS Linux macOS Windows Last.fm Libre.fm ListenBrainz ReplayGain Rust Kotlin Swift Slint
ratatui Cargo Gradle Bazel GitHub Stripe Spring Summer Autumn Winter Day Night Midnight Auto Wi-Fi USB DAC
TV Google Apple Sony Soundcore Chromecast MPRIS JNI FFI API WIT WASI Mermaid Starlight Xvfb Docker I
Smooth DJ Settings Home Library Search Queue Downloads Jam Jams Play Next Radio Mixes Profiles Sound
Playback Lyrics Appearance Output Equalizer Beat This""".split())


def prose_lines(text):
    """Yields (line number, line) for lines outside frontmatter and fenced code, with inline code removed."""
    in_fence = False
    in_front = text.startswith("---\n")
    for number, line in enumerate(text.split("\n"), 1):
        if in_front:
            if number > 1 and line.strip() == "---":
                in_front = False
            continue
        if re.match(r"^\s*(```|~~~)", line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        yield number, re.sub(r"`[^`]*`", "code", line)


def plain(line):
    """The words a reader sees: links keep their text, markup goes."""
    line = re.sub(r"!\[[^\]]*\]\([^)]*\)", "", line)
    line = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", line)
    line = re.sub(r"<[^>]+>", "", line)
    return re.sub(r"[*_#>|]", " ", line)


def sentences(paragraph):
    # A sentence can start with "nori" or with code, both lowercase.
    return [s for s in re.split(r"(?<=[.!?])\s+(?=[A-Z0-9\"'(`]|nori\b|code\b)", paragraph) if s.strip()]


def title_case(heading):
    words = re.findall(r"[A-Za-z][\w'.-]*", heading)[1:]
    caps = [w for w in words if w[0].isupper() and w not in NAMES and not w.isupper()]
    return len(caps) >= 2


def lint(path, advice, disabled):
    text = path.read_text(encoding="utf-8")
    findings = []

    def add(number, rule, message, excerpt=""):
        if rule in disabled:
            return
        findings.append((number, rule, message, excerpt.strip()[:90]))

    lines = list(prose_lines(text))
    paragraph, start, in_list_step = [], 0, False
    bold_run, last_notice = 0, -10

    def flush():
        joined = " ".join(paragraph)
        limit = STEP_WORDS if in_list_step else DESCRIPTION_WORDS
        for sentence in sentences(plain(joined)):
            count = len(sentence.split())
            if count > limit:
                add(start, "long-sentence", f"{count} words (limit {limit}); split it if it holds two ideas",
                    sentence)

    for number, line in lines:
        stripped = line.strip()
        is_table = stripped.startswith("|")
        is_heading = stripped.startswith("#")
        item = re.match(r"^\s*(?:[-*]|(\d+)\.)\s+", line)

        if paragraph and (not stripped or is_table or is_heading or item or NOTICE.match(line)):
            flush()
            paragraph = []
        if stripped and not is_table and not is_heading and not stripped.startswith(":::"):
            if not paragraph:
                start, in_list_step = number, bool(item and item.group(1))
            paragraph.append(stripped)

        seen = plain(line)
        for rule, pattern in WORD_PATTERNS.items():
            for match in pattern.finditer(seen):
                add(number, rule, f'"{match.group(0)}"', stripped)
        for match in PHRASAL.finditer(seen):
            add(number, "phrasal-verb", f'"{match.group(0)}": use one verb', stripped)
        if ";" in seen and not stripped.startswith("<"):
            add(number, "semicolon", "write two sentences", stripped)
        for match in VAGUE_LINK.finditer(line):
            add(number, "vague-link", f'link text "{match.group(1)}" does not name the page', stripped)
        if BARE_URL.search(re.sub(r"\]\([^)]*\)", "]", line)) and not is_table:
            add(number, "bare-url", "give the link words that name the page", stripped)
        if EMOJI.search(line):
            add(number, "emoji", "remove the emoji", stripped)

        if is_heading:
            heading = stripped.lstrip("#").strip()
            if title_case(heading):
                add(number, "title-case", "headings are sentence case", heading)
            if re.search(r"\bcosts?\b", heading, re.I):
                add(number, "cost-heading", 'reads as a price to a glancing reader; say "Battery use" or "Data use"',
                    heading)
            if heading.endswith((".", ":")):
                add(number, "heading-punctuation", "no end punctuation", heading)

        bold_run = bold_run + 1 if BOLD_LABEL.match(line) else (bold_run if not stripped else 0)
        if bold_run == 3:
            add(number, "bold-labels", "three or more '**Label:**' items; use a plain list or a table", stripped)

        if NOTICE.match(line):
            if number - last_notice <= 4:
                add(number, "stacked-notices", "two notices in a row; fold one into the text", stripped)
            last_notice = number
        elif stripped.startswith(":::") and stripped != ":::":
            pass
        elif stripped == ":::":
            last_notice = number

        if is_table and not re.match(r"^\|[\s:|-]+\|$", stripped):
            for cell in stripped.strip("|").split("|"):
                words = plain(cell).split()
                if len(words) > CELL_WORDS or len(sentences(plain(cell))) > 1:
                    add(number, "wordy-cell", f"{len(words)} words in a cell; move detail to prose", cell)

        if advice and not is_table:
            for match in PASSIVE.finditer(seen):
                add(number, "advice:passive", f'"{match.group(0)}": name who does it', stripped)
            for match in PERFECT.finditer(seen):
                add(number, "advice:present-perfect", f'"{match.group(0)}": simple past', stripped)

    if paragraph:
        flush()
    return findings


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("paths", nargs="+")
    parser.add_argument("--advice", action="store_true", help="also report passive voice and present perfect")
    parser.add_argument("--disable", default="", help="comma-separated rules to skip")
    args = parser.parse_args()
    disabled = set(filter(None, args.disable.split(",")))

    files = []
    for name in args.paths:
        path = pathlib.Path(name)
        files += sorted(path.rglob("*.md")) + sorted(path.rglob("*.mdx")) if path.is_dir() else [path]

    failed = 0
    for path in files:
        for number, rule, message, excerpt in lint(path, args.advice, disabled):
            print(f"{path}:{number}: {rule}: {message}" + (f"\n    {excerpt}" if excerpt else ""))
            failed += not rule.startswith("advice:")
    if failed:
        print(f"\n{failed} finding(s).", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
