Changed

- **Unreleased notes live in `changelog.d/`.** Feature pulls add one
  fragment there and leave `CHANGELOG.md` alone. `release-cut.sh`
  folds the fragments into `## [Unreleased]` and deletes them before
  cutting the version section.
