// Commit conventions (BL-TV-08, IP-01 T01.12). Commits follow Conventional Commits: `type(scope): subject`.
//
// Release mapping (the automation that acts on it is IP-23, decision D01-9; until then it is documentation):
//   feat                                         -> minor release
//   fix, perf, revert, refactor                  -> patch release
//   docs, style, test, build, ci, chore          -> no release
//   a breaking change (a `BREAKING CHANGE:` footer, or `!` after the type or scope) -> major release;
//   IP-23 chooses the exact preset.
//
// An unknown type is an error. An unknown scope is only a warning. Merge commits are ignored.
module.exports = {
  extends: ["@commitlint/config-conventional"],
  // Merge commits ("Merge pull request #2 from ...", "Merge branch ...") are not linted: commitlint's own default ignores.
  defaultIgnores: true,
  rules: {
    "type-enum": [
      2,
      "always",
      [
        "feat",     // New feature -> minor bump
        "fix",      // Bug fix -> patch bump
        "perf",     // Performance improvement -> patch bump
        "revert",   // Revert a previous commit -> patch bump
        "refactor", // Code refactoring -> patch bump
        "docs",     // Documentation only -> no bump
        "style",    // Formatting, no logic change -> no bump
        "test",     // Tests only -> no bump
        "build",    // Build system changes -> no bump
        "ci",       // CI configuration -> no bump
        "chore",    // Other changes -> no bump
      ],
    ],
    "scope-enum": [
      1, // warn (not error) — scope is optional but standardised
      "always",
      [
        "model",      // the Model crate: plain types mirroring the schema
        "io",         // the Mapping/IO layer: HDF5 <-> Model translation, renames, gap-handling
        "processing", // derived computation over Model data: decimation, corrections, spot size
        "facade",     // the public entry point orchestrating the other layers
        "ci",         // pipeline/workflow changes
        "docs",       // standalone docs/ content, not inline rustdoc (that's scoped to its own crate)
    "schema",     // the language-neutral schema in schema/ (IP-18)
    "adapter",    // a version adapter: the V16, V17 and V18 readers and writers
    "tools",      // scripts under tools/
    "fixtures",   // fixture handling: manifest, parity, fixture documents
    "testdata",   // the committed miniatures in testdata/
      ],
    ],
    "subject-case": [0],       // allow any case in subject
    "body-max-line-length": [0], // disable for semantic-release changelog commits, if/when adopted
  },
};
