# Contributing to Vibenalytics

Thanks for your interest in contributing!

## Development setup

1. Install Rust 2021 edition via [rustup](https://rustup.rs/)
2. Clone the repo and build:
   ```bash
   cargo build --release
   ```
3. Symlink for local testing:
   ```bash
   ln -sf "$(pwd)/target/release/vibenalytics" ~/.local/bin/vibenalytics
   ```

For development builds, use a separate data directory to avoid colliding with production:

```bash
APP_NAME=vibenalytics-dev cargo build --release
ln -sf "$(pwd)/target/release/vibenalytics" ~/.local/bin/vibenalytics-dev
```

## Making changes

1. Fork the repo and create a branch from `main`
2. Make your changes
3. Run `cargo build --release` to verify it compiles
4. Open a pull request with a clear description of what you changed and why

## Bug reports and feature requests

Open an issue on GitHub. Include steps to reproduce for bugs.

## Code style

- Follow standard Rust conventions (`cargo fmt`)
- Keep dependencies minimal - this is a single-binary CLI
- No content from user codebases or conversations should ever be collected or transmitted

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
