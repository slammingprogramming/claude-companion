# Translations

Claude Companion's app speaks 27 languages. Each follows the system language unless you pick one
under Settings › General › Language.

| Status | Languages |
| --- | --- |
| Carried over from cc-same (written by people) | English, 简体中文, 繁體中文, 日本語, 한국어, Français, Deutsch, Español, Português (Brasil), Русский |
| Machine-translated for this fork | Italiano, Nederlands, Polski, Türkçe, Українська, Svenska, Dansk, Norsk bokmål, Suomi, Čeština, Magyar, Română, Ελληνικά, Bahasa Indonesia, Tiếng Việt, ไทย, हिन्दी |

The machine translations were written by an AI model from the English text, with the app's
placeholders and plural forms checked by a test. They read plausibly but have not been reviewed by
native speakers, so some wording will be stiff, too literal or wrong for the context. Corrections are
very welcome and need no programming:

1. Open `crates/app/locales/app.yml`. Each message is a block with one line per language, such as
   `  nl: "Je accounts worden gelezen…"`.
2. Change the line for your language and keep every `%{name}` placeholder exactly as it is.
3. Open a pull request, or an issue with the old and new text. Say you are a native speaker.

## Not covered yet

- **Right-to-left languages** (Arabic, Hebrew, Persian): the interface toolkit does not lay out
  right-to-left text yet, so they are left out rather than shown mirrored wrongly.
- Languages whose plural rules the app does not know are not offered; add the rule to
  `plural_form` in `crates/app/src/i18n.rs` with a test, then the messages.
- The command-line tool and the notes in `CHANGELOG.md` are English only.

## Adding a language

1. Add the translated lines to `crates/app/locales/app.yml` for every message, including every
   plural form the language uses (`.one`, `.few`, `.many` or `.other`).
2. Add the language to `LANGUAGES` in `crates/app/src/i18n.rs` (its code and its own name), map any
   regional tags in `matching`, and add its plural rule to `plural_form`.
3. Run `cargo app-test`: a test fails if a message is missing or its placeholders differ from the
   English text.
