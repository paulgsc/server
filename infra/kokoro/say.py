"""Home's speaker for the weekly call (paulgsc/some-ui#2635, ST36's third slice).

The call's words are said on home, by Kokoro (hexgrad/Kokoro-82M, v1.0), so the
same text in the same voice gives the same audio, byte for byte, every time:

- The model, the voices and every library come from one image pinned by digest
  (infra/docker/Dockerfile.kokoro), and the model file is checked against its
  SHA-256 at start.
- Kokoro's vocoder draws random noise on every call (`torch.rand`/`torch.randn`
  in `kokoro/istftnet.py`), and the stock server never seeds it, so two runs of
  one line differ. This server seeds torch's generator before every line and
  says one line at a time, under one lock, so nothing else moves it.

Each request has its own thread, so `/health` answers while a line is being
said: a long line once timed Docker's health check out, and the check's late
answer was written to a closed socket. A line that fails for any reason is
answered 500 with the error's kind, never a dropped connection, so the
recorder's log says what failed (paulgsc/some-ui#2635).

    GET  /health        200 {"model": <sha256>, "voices": [...]}
    POST /say           {"text": "...", "voice": "am_michael"} -> audio/wav,
                        24 kHz mono 16-bit PCM

Nothing leaves the container: it needs no network, and it logs a line's length
and voice, never its words.
"""

import ctypes
import gc
import hashlib
import io
import json
import sys
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import numpy as np
import torch
from kokoro import KModel, KPipeline

MODELS = Path("/app/api/src/models/v1_0")
VOICES = Path("/app/api/src/voices/v1_0")
# hexgrad/Kokoro-82M's kokoro-v1_0.pth.
MODEL_SHA256 = "496dba118d1a58f5f3db2efc88dbdc216e0483fc89fe6e47ee1f2c53f18ad1e4"
SEED = 0
RATE = 24_000
MOST_CHARS = 4_000
MOST_BODY = 64 * 1024


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load():
    found = sha256(MODELS / "kokoro-v1_0.pth")
    if found != MODEL_SHA256:
        sys.exit(f"kokoro-v1_0.pth is {found}, not {MODEL_SHA256}: not the pinned model")
    torch.set_num_threads(1)
    model = KModel(repo_id="hexgrad/Kokoro-82M", config=str(MODELS / "config.json"), model=str(MODELS / "kokoro-v1_0.pth")).eval()
    pipelines = {}
    # Every voice is loaded here, from the image, so a request only picks one
    # by name and never names a file.
    packs = {path.stem: torch.load(path, weights_only=True) for path in sorted(VOICES.glob("*.pt"))}
    return model, pipelines, packs


MODEL, PIPELINES, PACKS = load()
VOICE_NAMES = sorted(PACKS)
# One line at a time: the seed holds only if nothing else draws from torch's
# generator between `manual_seed` and the line's last sample.
SAYING = threading.Lock()
# glibc keeps what a line frees in its heap, so the process grew line by line
# until the container's cap killed it, twelve lines into a week on home
# (paulgsc/some-ui#2635). After each line it is handed back to the system.
LIBC = ctypes.CDLL("libc.so.6")


def say(text: str, voice: str) -> bytes:
    """The line as a WAV file, the same bytes for the same text and voice."""
    lang = voice[0]
    if lang not in PIPELINES:
        PIPELINES[lang] = KPipeline(lang_code=lang, repo_id="hexgrad/Kokoro-82M", model=MODEL)
    pack = PACKS[voice]
    torch.manual_seed(SEED)
    with torch.no_grad():
        parts = [result.audio.numpy() for result in PIPELINES[lang](text, voice=pack, speed=1.0) if result.audio is not None]
    if not parts:
        raise ValueError("nothing to say")
    pcm = (np.clip(np.concatenate(parts), -1.0, 1.0) * 32767).astype("<i2").tobytes()
    del parts
    gc.collect()
    LIBC.malloc_trim(0)
    out = io.BytesIO()
    with wave.open(out, "wb") as file:
        file.setnchannels(1)
        file.setsampwidth(2)
        file.setframerate(RATE)
        file.writeframes(pcm)
    return out.getvalue()


class Handler(BaseHTTPRequestHandler):
    def send(self, status: int, body: bytes, kind: str) -> None:
        try:
            self.send_response(status)
            self.send_header("Content-Type", kind)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            # The asker stopped waiting; there is no one left to answer.
            pass

    def refuse(self, status: int, why: str) -> None:
        self.send(status, json.dumps({"error": why}).encode(), "application/json")

    def do_GET(self) -> None:
        if self.path != "/health":
            return self.refuse(404, "no such path")
        self.send(200, json.dumps({"model": MODEL_SHA256, "voices": VOICE_NAMES}).encode(), "application/json")

    def do_POST(self) -> None:
        if self.path != "/say":
            return self.refuse(404, "no such path")
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0 or length > MOST_BODY:
            return self.refuse(413, f"a body of 1 to {MOST_BODY} bytes")
        try:
            asked = json.loads(self.rfile.read(length))
        except ValueError:
            return self.refuse(400, "the body is not JSON")
        if not isinstance(asked, dict) or set(asked) != {"text", "voice"}:
            return self.refuse(422, "the body is text and voice, nothing else")
        text, voice = asked["text"], asked["voice"]
        if not isinstance(text, str) or not text.strip() or len(text) > MOST_CHARS:
            return self.refuse(422, f"text is 1 to {MOST_CHARS} characters")
        if not isinstance(voice, str) or voice not in PACKS:
            return self.refuse(422, "voice is one /health lists")
        try:
            with SAYING:
                audio = say(text, voice)
        except ValueError as err:
            return self.refuse(422, str(err))
        except Exception as err:  # noqa: BLE001 - any failure is answered, never a dropped connection
            # Its kind only: the message can quote the line's words.
            kind = type(err).__name__
            sys.stderr.write(f"a line of {len(text)} characters in {voice} failed: {kind}\n")
            return self.refuse(500, f"the line could not be said: {kind}")
        self.send(200, audio, "audio/wav")

    def log_message(self, format: str, *args) -> None:
        # The request line names only the path; never a line's words.
        sys.stderr.write(f"{self.address_string()} {format % args}\n")


if __name__ == "__main__":
    server = ThreadingHTTPServer(("0.0.0.0", 8880), Handler)
    server.daemon_threads = True
    server.serve_forever()
