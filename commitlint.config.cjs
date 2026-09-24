module.exports = {
  extends: ["@commitlint/config-conventional"],
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
      ],
    ],
    "subject-case": [0],       // allow any case in subject
    "body-max-line-length": [0], // disable for semantic-release changelog commits, if/when adopted
  },
};
