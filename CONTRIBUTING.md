# Contributing to ClotoCore

Thank you for your interest in contributing to ClotoCore.

## Getting Started

1. Fork the repository
2. Create a feature branch from `master`
3. Make your changes
4. Run tests: `cargo test`
5. Submit a pull request

## Development Setup

```bash
git clone https://github.com/Cloto-dev/ClotoCore.git
cd ClotoCore
cp .env.example .env
cargo build
cargo test
```

For faster development builds (skips icon embedding):

```bash
export CLOTO_SKIP_ICON_EMBED=1
cargo build
```

## Code Style

- Follow standard Rust conventions (`cargo fmt`, `cargo clippy`)
- Write comments in English
- Add tests for new functionality
- Keep commits focused and descriptive

## Pull Requests

- Keep PRs small and focused on a single change
- Include a clear description of what changed and why
- Make sure `cargo test` passes before submitting
- Reference any related issues in the PR description

## Reporting Issues

Use [GitHub Issues](https://github.com/Cloto-dev/ClotoCore/issues) to report bugs or request features. Include:

- Steps to reproduce (for bugs)
- Expected vs actual behavior
- ClotoCore version and OS

## License

By contributing, you agree that your contributions will be licensed under the same [BSL 1.1 license](LICENSE) as the project.
