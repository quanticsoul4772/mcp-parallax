# Research: Screen research pages for prompt injection

## D1: Matching without a regex engine

- **Decision**: plain matching on a normalized copy of the text: lowercased,
  with every run of whitespace collapsed to one space. Phrases are substring
  checks on that copy; hidden text is a scan of the original chars.
- **Rationale**: deterministic, no dependency, linear in the text length
  (pages are capped by the extraction character limit), and simple to test.
- **Alternatives**: `fancy-regex` (already a dependency, but backtracking is
  unnecessary for fixed phrases); a model classifier (rejected by the user,
  2026-11-23).

## D2: Pattern families and names

- `instruction_override`: phrasings such as "ignore previous instructions",
  "ignore all prior instructions", "disregard the above", "forget your
  instructions", "you are now a", "you are now an", "new instructions:",
  "system prompt:", "reveal your system prompt". ("you are now" and "system
  prompt" alone were narrowed to these forms because they occur in ordinary
  prose; user choice, 2026-11-23.)
- `role_marker`: chat-format markers and special tokens such as
  `<|im_start|>`, `<|im_end|>`, `<|system|>`, `[inst]`, `<<sys>>`,
  `<|endoftext|>`.
- `hidden_text`: zero-width characters (U+200B–U+200D, U+2060, U+FEFF) and
  bidirectional controls (U+202A–U+202E, U+2066–U+2069).
- **Rationale**: the three families in the spec (FR-002). The phrase list is
  short and conservative to limit false positives; it is versioned in code.

## D3: What is screened

- **Decision**: the exact text `extract_source` sends to extraction (trimmed,
  capped). Pages that never reach extraction are not screened.
- **Rationale**: the screen describes what the model actually read (spec edge
  cases).

## D4: Where the result shows it

- **Decision**: `SourceRef.injection_patterns: Vec<String>` (always present,
  empty when clean) and `Stats.pages_flagged: u32`; both required in the
  published contract.
- **Rationale**: the caller reads sources and stats; no record column (out of
  scope).
