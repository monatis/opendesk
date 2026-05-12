<div align="center">

# opendesk

**Give any AI agent eyes and hands on your desktop.**

Opendesk is a computer use framework that lets AI agents navigate your computer just like a human would — screenshots, mouse, keyboard, UI interaction, OCR, workflow recording, and scheduling.

**macOS · Linux · Windows**

[![PyPI](https://img.shields.io/pypi/v/opendesk)](https://pypi.org/project/opendesk/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

---

![opendesk demo](docs/opendesk_demo.gif)

---

## SDKs

| Language | Location | Package | Install |
|----------|----------|---------|---------|
| Python | [`python/`](python/) | `opendesk` (PyPI) | `pip install 'opendesk[core,mcp]'` |
| JavaScript / TypeScript | [`js/`](js/) | `@vitalops/opendesk-sdk` (npm) | `npm install @vitalops/opendesk-sdk` |

More SDKs can be added to this repo following the same pattern.

---

## Quick start

### Python

```bash
pip install 'opendesk[core,mcp]'
opendesk install
```

Start a Claude Code conversation and try:

```
Take a screenshot of my screen
Click the Chrome icon
Open Spotify and play lo-fi beats
```

### JavaScript / TypeScript

```bash
cd js
npm install @vitalops/opendesk-sdk
npx opendesk-js install
```

```typescript
import { OpenDeskClient } from "@vitalops/opendesk-sdk";

const client = new OpenDeskClient();
await client.screenshot({ marks: true });
await client.ui({ action: "click", app: "Safari", title: "Go" });
```

---

## Architecture

opendesk is built in three layers — each independently importable:

```
┌─────────────────────────────────────────────────────────┐
│  Integrations  (MCP, Anthropic, OpenAI, LangChain)      │
├─────────────────────────────────────────────────────────┤
│  Tools   (screenshot · mouse · keyboard · ui            │
│           clipboard · ocr · learn · schedule · audit)   │
├─────────────────────────────────────────────────────────┤
│  Computer  (capture · Set-of-Marks · OCR · sandbox)     │
└─────────────────────────────────────────────────────────┘
```

| Layer | What it does |
|-------|-------------|
| **Computer** | Low-level screen capture, SoM element detection, OCR, per-session audit log |
| **Tools** | One class per capability. Pydantic schema auto-shared with every integration |
| **Integrations** | Thin adapters for MCP, Anthropic, OpenAI, LangChain — add one tool, get all four |
| **Automation** | `learn` + `schedule` backed by pynput recording, JSON storage, APScheduler daemon |

Full details → [docs/architecture.md](docs/architecture.md)

---

## Tools

| Tool | What it does |
|------|-------------|
| `screenshot` | Capture the screen with numbered boxes on every clickable element (Set-of-Marks) |
| `ui` | Click and type by element name — no coordinates needed |
| `mouse` | Pixel-level mouse control for anything `ui` can't reach |
| `keyboard` | Type text, press keys, send hotkeys |
| `app` | Open, close, and focus applications |
| `clipboard` | Read and write the system clipboard |
| `ocr` | Extract text from any region of the screen |
| `learn` | Record a workflow once, replay it anytime |
| `schedule` | Run any task or learned procedure on a timer |

Full reference → [docs/tools.md](docs/tools.md)

---

## Automation

Record a task once, replay it forever, or put it on a schedule.

**Record**
```
"Start recording task expense-form"
```
Perform the workflow yourself. The agent captures every click, keystroke, and screenshot.

**Replay**
```
"Stop recording"
"Replay expense-form"
```
The agent re-executes using the current screen state — no hardcoded coordinates.

**Schedule**
```
"Every morning at 9am, open my email in Chrome, take a screenshot, and summarize what's there"
"Schedule expense-form every friday at 5pm"
```
```bash
opendesk scheduler start
```

Supported timing: `every 30m` · `every 2h` · `every day at 09:00` · `every friday at 17:00` · raw cron

Full guide → [docs/automation.md](docs/automation.md)

---

## Platform support

| Feature | macOS | Linux | Windows |
|---------|:-----:|:-----:|:-------:|
| Screenshot | ✓ | ✓ | ✓ |
| Mouse & keyboard | ✓ | ✓ | ✓ |
| UI element access | AppleScript | AT-SPI2 | UI Automation |
| Clipboard | pbcopy/pbpaste | xclip/xsel | pyperclip |
| OCR | Vision / tesseract | tesseract | WinRT / tesseract |
| App control | `open -a` | `xdg-open` | `start` |
| Task recording | ✓ | ✓ | ✓ |
| Scheduled tasks | ✓ | ✓ | ✓ |

---

## System permissions

### macOS
- **System Settings → Privacy & Security → Screen Recording** — enable for your terminal
- **System Settings → Privacy & Security → Accessibility** — enable for mouse/keyboard control

### Linux
```bash
sudo apt install xclip xdotool python3-atspi
```

### Windows
No extra permissions needed — opendesk uses Win32 APIs by default.

---

## Docs

- [Quickstart](docs/quickstart.md)
- [Tools reference](docs/tools.md)
- [Integrations](docs/integrations.md)
- [Architecture](docs/architecture.md)

---

## Citation

```bibtex
@software{opendesk,
  author  = {Abraham, Abhijith Neil},
  title   = {opendesk: Open Desktop Automation Framework},
  year    = {2025},
  url     = {https://github.com/vitalops/opendesk},
  version = {0.1.2},
  license = {MIT}
}
```

---

## License

MIT
