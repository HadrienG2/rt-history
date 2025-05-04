# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

_No unreleased changes in the pipeline at the moment


## [4.0.0] - 2025-05-04

### Changed

- Bumped MRSV to 1.74 owing to new dependency requirements.
- Update dependencies


## [3.0.0] - 2023-07-31

### Changed

- Bumped MSRV to 1.64 per new criterion requirements.

### Fixed

- Strengthened bound to `T: NoUninit` from bytemuck, since the former `T: Copy`
  bound was unsound and is thus not accepted by `atomic` anymore.


## [2.0.0] - 2023-02-16

### Changed

- Bumped MSRV to 1.60 as we can't easily test on earlier releases since some of
  our dev dependencies require 1.60.


## [1.0.0] - 2021-11-17

### Added

- Initial release



[Unreleased]: https://github.com/HadrienG2/rt-history/compare/v4.0.0...HEAD
[4.0.0]: https://github.com/HadrienG2/rt-history/compare/v3.0.0...v4.0.0
[3.0.0]: https://github.com/HadrienG2/rt-history/compare/v2.0.0...v3.0.0
[2.0.0]: https://github.com/HadrienG2/rt-history/compare/v1.0.0...v2.0.0
[1.0.0]: https://github.com/HadrienG2/rt-history/releases/tag/v1.0.0
