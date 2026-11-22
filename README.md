# The Noble Quran — native (egui)

A native reader for the 15-line Madani mushaf, written in Rust with
[egui](https://github.com/emilk/egui). It is a port of
[quran-dx](https://github.com/qatoqat/quran-dx) (Dioxus + webview) without the webview: one small
binary that draws everything itself.

Data comes from the public [Quran.com API v4](https://api-docs.quran.com)
(`https://api.quran.com/api/v4`); fonts are fetched from quran.com's font
servers and kept in `~/.cache/quran-eg/fonts/`.

## Features

- **15-line Madani mushaf everywhere** (Uthman Taha's calligraphy): surah, verse
  range, juz and page views all show real mushaf pages, every word on its printed
  line, drawn with the King Fahd (KFGQPC) QCF per-page fonts — tajweed colours
  (QCF V4) by default, or plain (QCF V2). A full line is justified edge to edge; a
  line is never wrapped or cut. If lines don't fit the window, everything is scaled
  down together, and all pages share one scale and one line width (the widest
  line's).
- **Surah banner** as in the printed mushaf: an ornamental frame one line tall, the
  calligraphic name in its cartouche, the place of revelation and the number of
  verses in its roundels; then the Bismillah line.
- **Word by word**: the English meaning and/or the transliteration under each word,
  or in a tooltip on hover.
- **Verse translations** under each page (optional), with footnotes and a copy
  action; any of the API's English translations.
- **Header**, kept to the width of the content: the view's title (`Surah
  Al-Baqarah`, `Juz 2`, `Page 22`), a memorize button, a search button, a Surah | Juz | Page toggle and
  a settings button. The toggle's current button flips between that mode's list and
  reading; the other two open the line being read in their view; each says where it
  leads. There is no list of pages.
- **Reading position**: the word that starts the top line. It is saved (with the
  view) in `~/.config/quran-eg/location`, e.g. `/juz/2#2:282:45`, and the app starts
  there; it stays put when the window is resized or settings change.
- **Settings** in a panel floating over the right edge (the page behind it is the
  live preview): theme (auto, light, dark, sepia, or custom from a background and an
  accent), mushaf font, word-by-word options, translations, text sizes, spacing
  (between words, above the meanings, under each line), content width, a reading
  pointer, and a font source. Saved in `~/.config/quran-eg/settings.json`, in the
  same shape quran-dx uses.
- **Reading pointer** (optional): a dot or ring in place of the mouse cursor below
  the header, with colour, size, opacity, a soft edge, a smudge along the path it has
  just travelled (a motion blur, with a strength and a distance), shrinking with
  speed, and a glow that makes the tajweed colours under it more vivid.
- **Recording and voice pointer** (desktop): the microphone button records your
  recitation with a note of the word it started at (`~/.local/share/quran-eg/recordings/`),
  for adapting quran-voice's model to your voice (its `qv enroll`). **Listen** follows the recitation with
  quran-voice's listener and puts a dot on the letter being read, inside an accuracy
  range (a rounded rectangle over the words it could be on, wider when unsure), and
  keeps the line in view. Settings: the dot's look (as the reading pointer), the
  range's colour, height, width, opacity, roundness, outline and edge blur, and the
  motion (an easing curve and glide time each for the pointer and for either edge of
  the range, when to fade over instead of moving back or jumping ahead, how far it
  may run ahead at your pace, how long it waits for you while the range reaches back
  to where you are heard).
- **Memorizing (hifz)**: the header's lines button hides the mushaf's words; the ayahs' ends, the signs
  between the words (pause marks, the rub el hizb ۞, the sajdah sign ۩: the page font's own glyphs, or from
  the word's text with the fallback font where the font draws a sign with the letters), the surah banners and
  the Bismillah line stay, and the meanings and translations go (the Bismillah's too). Only what is drawn
  changes: everything keeps its place, nothing is loaded or laid out again, and the words fade out and in
  (0.2 s), as does a word clicked, which shows until it is clicked again; the words shown are forgotten when
  memorizing ends or the view changes. The voice pointer follows over the hidden words as over the shown
  ones. Saved in the settings.
- **Search**: suggestions under the search box (shortcuts such as `2:255`,
  `juz 30`, `page 3`, `kahf 10`, and matching verses), and a results view with
  highlighted matches and paging.

Right to left, as the text reads: "next" is on the left, "previous" on the right.

## Run

Rust 1.100 (pinned in `rust-toolchain.toml`).

```bash
cargo run -p quran-eg              # development build
cargo build --release -p quran-eg  # target/release/quran-eg
```

It needs OpenGL (glow backend; `--features wgpu` builds the other one, which has
not been tried), and Wayland or X11. Under KWin the window gets the desktop's own
frame (it asks for server-side decoration).

`QURAN_EG_STATS=1` prints once a second what a view holds in memory (pages laid
out, sprite sheets, fonts).

### Voice pointer

Listen runs quran-voice's native listener (`qv listen`: the microphone, the model
and the tracker, all Rust) in its own process, so the reader stays light, and
follows it over `ws://127.0.0.1:8765`. Install it once from a quran-voice checkout
with `scripts/install-listener.sh` (into `~/.local/share/quran-voice`; another
folder can be set in the settings). A listener that is already running (`qv listen`
started by hand, or for the quran.com extension) is joined instead of starting
another. `QURAN_EG_LISTEN=1` starts listening when the app opens, and
`QURAN_EG_VOICE_PORT` picks another port than 8765 (for tests with a stand-in).

Settings › Voice pointer › Listen with picks the model the listener runs: its own `model/` (the default) or a
folder of its `models/` (`models/<name>/` with `model.bpk`, `config.json`, `norm.json` and a `README.txt` whose
first line names it), each shown by that name and the next lines on hover. A listener this app started starts
again at once with the model chosen (`qv listen --run models/<name>`); one started elsewhere keeps its own, and
the choice applies when Listen starts one.

**Replay** (the triangle left of Record) plays the newest recording through the speakers while the installed
listener hears the same file at its real pace (`qv listen --file <wav> --cue` on a spare port, its output in
`replay.log`), so the pointer follows it as it follows you; the view goes to where the recording started.
Live listening stops meanwhile and comes back after. The listener holds the file until the app says when its
first sample is heard (the output's latency, from cpal) and starts its feed then. The button stops the replay,
or calls it off while it starts; Record or Listen end it too. It needs a listener built with `--cue`.

While it listens, the Listen button shows the microphone: its disc fills from the
bottom as high as the listener hears you (empty below −54 dBFS, full at −6), turns
red for a moment when the input clips, and is struck through when no audio arrives
or capture fails. Its tooltip says which, with the level and the microphone's name;
the title bar says so too. A listener that does not report its microphone leaves
the button as it was.

The isti'adha and, away from al-Fatiha, the basmala are heard but not pointed at:
the pointer waits for the surah to start. Closing with ṣadaqa-llāhu-l-ʿaẓīm stops
listening (Settings › Voice). With "Fade out when you pause at an ayah's end" (on),
a pause at an ayah's end longer than the set delay (1 s) fades the pointer out, and
it fades in wherever you go on instead of gliding over the ayah's mark. The
listener's output goes to `listen.log` in its folder; the previous sessions' logs
are `listen.1.log` … `listen.5.log`.

With "Follow tajweed timing" the app asks the listener for its plan (how the next half
second of the reading goes) and the voice's level, and the pointer moves letter by letter
at your pace, creeps through madd and ghunnah and waits at a word's end until the next
word is heard; it stays on the line you are heard on until you are heard on the next,
rests where you stop, can step back to the word's end when you stop past it, and holds
while you are quiet: after a second of silence nothing the listener hears in the room
moves it on. "Hold while you are quiet" does that last part without the plan, and "Dim
while you are quiet" dims the pointer in a silence as long as the fade's delay. Off (as
they are by default), the pointer moves as before; an older listener sends neither plan
nor level, and Voice › Motion says so.

A place heard far from where the recitation was being followed (another surah, or
more than about a page away) counts as lost for a while (5 s by default): the
pointer hides and the page stays, in case it was a slip or a mishearing, and the
title bar says so. The reader goes there only if the recitation keeps being followed
there, reading on, for that long; reading on into the next page or surah is followed
as before.

While the voice pointer's settings are in view (their heading above it), a preview
sits at the foot of the settings panel: a scripted reading of 1:6–1:7 in the page's
own glyphs, heard by a scripted listener and drawn by the same pointer code as the
page, at the page's pace. Its timeline has six scenes (all three lines read with a
breath, an ayah's end, a held letter, slips, a mishearing, reading on elsewhere); the
chosen one plays over and over, a changed setting goes to the scene where it acts, a
pause or a hold lasts as long as its setting needs to be seen, and the line under the
text says what the pointer decided. In a lane under each line the ticks are where the
listener heard you and the caret where you are; "Even" hears every step alike, "As
heard" in bursts as quran-voice does, so a stutter can be told apart from the curves.
It is a miniature (its scale is shown): on the page the pointer moves further in the
same time. It plays only while in view and unfolded, at a few frames a second while
nothing in it moves.

### Install on Arch Linux

```bash
paru -Bi --rebuild packaging/arch   # or: cd packaging/arch && makepkg -sif
```

This builds the working tree into a `quran-eg` package: the `quran-eg` command, a
menu entry and an icon. Run it again after changes (`--rebuild` / `-f` matters: the
version doesn't change, so the last package file would be reused otherwise);
`pacman -R quran-eg` removes it.

## Web

The same app runs in a browser (WebAssembly, WebGL): <https://qatoqat.com/quran-eg/>.

```bash
cargo install wasm-bindgen-cli --locked --version "$(scripts/build-web.sh --bindgen-version)"
scripts/build-web.sh               # dist/, to be served from the root of a site
scripts/build-web.sh /quran-eg/    # ... or under a base path, as on GitHub Pages
```

The script builds the `web` profile for `wasm32-unknown-unknown`, runs
`wasm-bindgen` (which must be the very version in `Cargo.lock`) and puts the page,
the wasm and a font in `dist/`. `dist/404.html` is a copy of the page: a static
host that serves it for unknown paths makes every address of the app loadable.
A push to `main` builds and deploys the site (`.github/workflows/pages.yml`).

What differs from the native app:

- **The address is the location**, as in quran-dx: the path is the view (`/`,
  `/juz`, `/2`, `/2/255`, `/juz/30`, `/page/3`, `/search?q=…`), the hash the reading
  position (`/2#2:142`). Going somewhere adds a history entry, scrolling updates the
  current one, back and forward work, and the tab is named after the view.
- **No threads**: what the native app gives to worker threads (drawing a page's
  sprite sheet, shaping) runs between frames, a few milliseconds at a time; requests
  use the browser's `fetch`.
- **Settings** are kept in the browser's `localStorage`; fonts are cached by the
  browser, not by the app. A font source that is a path is a path on the site.
- **Letters the built-in font lacks** (ḥ, ṣ, ʿ) come from Noto Sans, a file of the
  site (`app/web/fonts/`, SIL Open Font License), where the native app asks the system.

## Tests

```bash
sh crates/qcf-glyphs/tests/fixtures/fetch.sh   # once: two page-1 fonts the glyph tests draw
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p quran-core -- --ignored     # live tests against api.quran.com
cargo test -p quran-eg --release page_check -- --ignored --nocapture   # every page, see below
```

The font files are King Fahd Complex fonts served by quran.com; like all fonts the app
uses, they are fetched, not kept in this repository.

The app's own tests run reader views without a window or a network (real egui
frames on made-up verses): every word gets a place on its line, pages far from the
viewport hold nothing, and the top line stays the top line across changes.

## Nothing goes missing

- **Data**: each verse must have contiguous word positions ending in its ayah
  marker, every word must have text, and no verse of a view may be skipped. Tables
  compiled into the app (`crates/quran-core/src/reference/`: verses per surah,
  words per verse, words on each line of each page, the verses printed on each page)
  catch a response that lost a word, a verse or part of a line although it looks
  consistent. Problems are named in a notice above the pages; a verse with broken
  word data is shown from its verse text instead.
- **Whole pages**: a page view shows exactly the words printed on that page. The
  API lists some verses under the neighbouring page (83:34 under 588, though it is
  printed on line 1 of 589), so a page is loaded from every listing that holds one
  of its verses, and each of its lines must have as many words as printed.
- **Drawing**: a word is drawn as its glyph only if all of it draws; otherwise as
  shaped text (the KFGQPC Hafs face), and a word whose page font should have drawn
  it but could not is marked, with a line on its page saying so. Until any font is
  there, a word is a marked box with its meaning — never nothing. A page says so if
  it could not place every one of its words.
- Fonts or requests that fail are named in a notice with a retry; a background
  job that dies is a failure like any other, not an endless "Loading…".

## How it draws

Each page near the viewport gets one **sprite sheet**: all its word glyphs
(rasterised by `crates/qcf-glyphs` from the page's font), or shaped fallback text,
packed into one texture on a worker thread, and drawn as one mesh. Sheets, laid-out
pages and page fonts are freed when their page is a few screens away.

egui lays text out left to right only, so Arabic outside the page fonts (fallback
words, surah names, search results) is shaped with HarfRust and drawn from sheets
too. The banner's frame and names are two ligature fonts from QUL, the Quranic
Universal Library (`static-cdn.tarteel.ai`); QUL lists no licence for them, so they
are fetched at run time, not bundled, and until they are there the banner is a
plain framed line with the same content.

## Decisions

**The surah banner stays on the first line of its surah.** In the printed mushaf, 18
surahs (4, 10, 22, 23, 24, 26, 27, 32, 33, 37, 38, 45, 47, 53, 60, 64, 65, 80) begin on
line 2 of a page with their banner printed as the last line of the page before. Here, as
in quran-dx, the banner is always drawn directly above its surah's first line, on the
surah's own page. Those pages therefore show one row more than the print and the page
before one row less; the text lines themselves are as printed. A banner at the bottom of
a page that holds none of its surah was judged worse than this difference from the print.

## Differences from quran-dx

- No webview, no async runtime: blocking HTTP on a few worker threads. Requests
  try IPv4 first (a network that announces IPv6 without routing it would otherwise
  stall every request).
- The tajweed fonts' own palettes are used for light, dark and sepia (quran-dx
  needs separate OpenType-SVG fonts for WebKit).
- Fallback text is always the Hafs face (quran-dx uses Amiri Quran for plain
  Uthmani text).
- A custom font source that is an absolute path is a folder on this computer.
- Search results are not part of the saved location: the app starts in the view
  that was open under them.
- The pointer smudges along its path instead of leaving a trail of copies; the
  smudge is at most 70 % of the pointer's opacity (at strength 10), and its glow acts
  on the tajweed colours only, not on the page behind them.
- Right-to-left translation languages are laid out left to right (the picker
  offers English translations only).

## Layout

```
crates/quran-core/   everything without a GUI: API client, verse references, views
                     and locations, verse feeds, integrity checks and reference
                     tables, mushaf layout, search shortcuts, settings
crates/qcf-glyphs/   rasteriser for the QCF page fonts (V2 mono, V4 tajweed)
app/src/             the egui app: reader (layout, scrolling, drawing), header,
                     lists, search, settings panel, pointer, fonts, sprite sheets
app/web/             the web build's page and font
scripts/             build-web.sh
packaging/           Arch Linux package, desktop entry and icon
```

## License

GPL-3.0-or-later, like quran-dx and the original quran.com frontend. See [LICENSE](LICENSE).
