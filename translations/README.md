# Translations

Hora's source texts are English. Each language has one gettext file:

```
translations/<code>/LC_MESSAGES/hora-app.po
```

The Slint UI bundles these files at build time (`@tr(...)` in `crates/app/ui`),
and the Rust code reads the same files through `crates/i18n` (`tr!`, `trn!`,
`trc!`). `hora-app.pot` is the template with all source texts.

| Code | Language   | Plural forms |
|------|------------|--------------|
| de   | Deutsch    | 2            |
| fr   | Français   | 2 (0 and 1 are singular) |
| es   | Español    | 2            |
| it   | Italiano   | 2            |
| nl   | Nederlands | 2            |
| pl   | Polski     | 3 (1 · 2–4 · 5+) |

## Workflow

```
python tools/i18n.py update   # after changing texts in the code
python tools/i18n.py check    # every language complete? (also runs in CI)
python tools/i18n.py stats    # progress per language
```

`update` adds new texts with an empty translation to every `.po` file and
moves texts that are no longer used to the end as obsolete (`#~`). Empty or
`fuzzy` translations fall back to English.

## Placeholders

- `{}` – arguments in order, `{0}`, `{1}` – arguments by position (use these
  when a language needs a different order)
- `{n}` – the count in plural forms
- `{{` and `}}` – literal braces

A translation may only use placeholders that appear in the English text.

## Adding a language

1. Add the language to `Lang` in `crates/i18n/src/lib.rs` (code, native name,
   plural rule) and its weekday and month names and date patterns to
   `crates/i18n/src/dates.rs`.
2. Add its `Plural-Forms` to `PLURAL_FORMS` in `tools/i18n.py`.
3. Create `translations/<code>/LC_MESSAGES/hora-app.po` (copy the header of
   another language), run `python tools/i18n.py update` and translate.

Quick add (natural language input such as "Lunch tomorrow 1pm") understands
English and German only, in every UI language.
