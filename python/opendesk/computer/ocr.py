"""Screen-region OCR — extract visible text from any screen area.

Backends (tried in order)
--------------------------
1. ``pytesseract`` — best quality, cross-platform; needs Tesseract binary.
2. macOS Vision framework — zero extra deps on macOS 11+, via Swift subprocess.
3. Windows WinRT OCR — zero extra deps on Windows 10+, via PowerShell.
4. Graceful failure with clear install hints.

Usage::

    from opendesk.computer.ocr import extract_text_from_region

    # region = (x, y, width, height) in logical pixels; None = full screen
    text = extract_text_from_region(region=(100, 200, 800, 400))
"""

from __future__ import annotations

import io
import platform
import subprocess
import tempfile
from pathlib import Path

_PLATFORM = platform.system()


def ocr_image(png_bytes: bytes, *, width: int = 0, height: int = 0) -> str:
    """Run OCR on a PNG byte buffer using the best available backend.

    Tries pytesseract, then platform-native OCR (Vision on macOS, WinRT on
    Windows).  Returns the extracted text or an informative error/install-hint
    string starting with ``"OCR "`` so callers can distinguish.

    ``width`` / ``height`` are optional; when set and small, the image is
    upscaled for better OCR accuracy.
    """
    try:
        import pytesseract  # type: ignore[import-not-found]
        from PIL import Image  # type: ignore[import-not-found]
        img = Image.open(io.BytesIO(png_bytes))
        w = width or img.width
        h = height or img.height
        if 0 < w < 300:
            factor = max(2, 300 // w)
            img = img.resize((w * factor, h * factor), Image.LANCZOS)
        text = pytesseract.image_to_string(img, config="--psm 6")
        return text.strip() or "(no text detected)"
    except ImportError:
        pass
    except Exception as exc:
        return f"pytesseract error: {exc}"

    if _PLATFORM == "Darwin":
        try:
            return _macos_vision_ocr(png_bytes)
        except Exception:
            pass

    if _PLATFORM == "Windows":
        try:
            return _windows_winrt_ocr(png_bytes)
        except Exception:
            pass

    return (
        "OCR not available. Install pytesseract:\n"
        "  macOS:   brew install tesseract && pip install pytesseract\n"
        "  Ubuntu:  sudo apt install tesseract-ocr && pip install pytesseract\n"
        "  Windows: choco install tesseract && pip install pytesseract\n\n"
        "(On macOS 11+ and Windows 10+ a built-in OCR engine is also tried "
        "automatically without any extra installs.)"
    )


def extract_text_from_region(
    region: tuple[int, int, int, int] | None = None,
) -> str:
    """Capture a screen region and run OCR on it.

    Convenience wrapper that uses :func:`opendesk.computer.capture.capture_screen`
    plus :func:`ocr_image`.  Prefer calling them separately when integrating
    with a :class:`~opendesk.computer.Computer` so OCR can run on a captured
    :class:`~opendesk.computer.Pixmap` from any backend.
    """
    try:
        from opendesk.computer.capture import capture_screen
        png_bytes, w, h = capture_screen(region)
    except Exception as exc:
        return f"OCR error: could not capture screen: {exc}"
    return ocr_image(png_bytes, width=w, height=h)


def _macos_vision_ocr(png_bytes: bytes) -> str:
    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
        f.write(png_bytes)
        tmp_png = Path(f.name)

    swift_src = f"""
import Vision
import AppKit

let url = URL(fileURLWithPath: "{tmp_png}")
guard let img = NSImage(contentsOf: url),
      let cgImg = img.cgImage(forProposedRect: nil, context: nil, hints: nil)
else {{ exit(0) }}

let req = VNRecognizeTextRequest()
req.recognitionLevel = .accurate
req.usesLanguageCorrection = true
let handler = VNImageRequestHandler(cgImage: cgImg, options: [:])
try? handler.perform([req])
let lines = (req.results ?? []).compactMap {{ $0.topCandidates(1).first?.string }}
print(lines.joined(separator: "\\n"))
"""
    with tempfile.NamedTemporaryFile(suffix=".swift", delete=False, mode="w") as sf:
        sf.write(swift_src)
        swift_path = Path(sf.name)

    try:
        r = subprocess.run(
            ["swift", str(swift_path)],
            capture_output=True, text=True, timeout=20,
        )
        if r.returncode == 0 and r.stdout.strip():
            return r.stdout.strip()
        raise RuntimeError(f"Swift Vision OCR failed: {r.stderr.strip()}")
    finally:
        tmp_png.unlink(missing_ok=True)
        swift_path.unlink(missing_ok=True)


def _windows_winrt_ocr(png_bytes: bytes) -> str:
    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
        f.write(png_bytes)
        tmp_png = Path(f.name)

    tmp_ps = str(tmp_png).replace("\\", "/")

    ps_script = f"""
Add-Type -AssemblyName System.Runtime.WindowsRuntime
[Windows.Storage.StorageFile, Windows.Storage, ContentType=WindowsRuntime] | Out-Null
[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType=WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics, ContentType=WindowsRuntime] | Out-Null

$filePath = '{tmp_ps}'
$file = [Windows.Storage.StorageFile]::GetFileFromPathAsync($filePath).AsTask().Result
$stream = $file.OpenAsync([Windows.Storage.FileAccessMode]::Read).AsTask().Result
$decoder = [Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream).AsTask().Result
$bitmap = $decoder.GetSoftwareBitmapAsync().AsTask().Result
$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
$result = $engine.RecognizeAsync($bitmap).AsTask().Result
$result.Lines | ForEach-Object {{ $_.Text }}
"""
    try:
        r = subprocess.run(
            ["powershell", "-NonInteractive", "-Command", ps_script],
            capture_output=True, text=True, timeout=25,
        )
        if r.returncode == 0 and r.stdout.strip():
            return r.stdout.strip()
        raise RuntimeError(f"WinRT OCR failed: {r.stderr.strip()}")
    finally:
        tmp_png.unlink(missing_ok=True)
