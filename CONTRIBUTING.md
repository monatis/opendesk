# Contributing to opendesk

Thanks for your interest in contributing!

---

## Ways to contribute

- Report bugs via [GitHub Issues](https://github.com/vitalops/opendesk/issues)
- Suggest features or improvements
- Submit pull requests for bug fixes or new features
- Improve documentation

---

## Getting started

### Python SDK

```bash
git clone https://github.com/vitalops/opendesk
cd opendesk/python
pip install -e '.[core,mcp,remote]'
```

Run tests:

```bash
pytest
```

### JavaScript / TypeScript SDK

```bash
cd opendesk/js
npm install
npm run build
```

---

## Pull request guidelines

- Keep PRs focused — one fix or feature per PR
- Add or update tests for any code changes
- Run existing tests before submitting
- Use clear commit messages that describe what and why

---

## Reporting bugs

Please include:
- OS and version
- Python or Node.js version
- Steps to reproduce
- What you expected vs what happened

---

## Code of Conduct

This project follows our [Code of Conduct](CODE_OF_CONDUCT.md). Be respectful and constructive.
