# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

- Open-sourced the CLI repository

## [0.10.0] - 2026-03-17

### Added

- Track aside subagent (`/btw`) conversations separately with `is_aside` and `aside_count` fields
- Track subagent type breakdown per prompt and session (`subagents` map with type counts)
- Filter `<task-notification` messages from real user prompt detection
