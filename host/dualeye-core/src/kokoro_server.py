"""Kokoro text-to-speech over HTTP, for DualEye (see tts.rs).

POST /synthesize {"text": "...", "voice": "if_sara", "speed": 1.0}
answers a 16-bit mono WAV at Kokoro's 24 kHz. Only listens where told,
which DualEye makes 127.0.0.1.
"""

import argparse
import io
import json
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from threading import Lock

import numpy as np
from kokoro_onnx import Kokoro

# A voice name's first letter is its language: espeak-ng's name for it.
LANGUAGES = {"a": "en-us", "b": "en-gb", "i": "it", "e": "es", "f": "fr-fr", "p": "pt-br", "h": "hi", "j": "ja", "z": "cmn"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--voices", required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()

    kokoro = Kokoro(args.model, args.voices)
    # onnxruntime runs one inference at a time well; requests take turns.
    lock = Lock()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            if self.path != "/synthesize":
                return self.send_error(404)
            try:
                request = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
                voice = request["voice"]
                lang = LANGUAGES.get(voice[:1], "en-us")
                with lock:
                    samples, rate = kokoro.create(request["text"], voice=voice, speed=float(request.get("speed", 1.0)), lang=lang)
            except Exception as e:  # noqa: BLE001 - the error goes back to DualEye
                return self.send_error(400, str(e)[:300])
            pcm = (np.clip(samples, -1.0, 1.0) * 32767.0).astype("<i2").tobytes()
            buf = io.BytesIO()
            with wave.open(buf, "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(rate)
                w.writeframes(pcm)
            body = buf.getvalue()
            self.send_response(200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    ThreadingHTTPServer((args.host, args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
