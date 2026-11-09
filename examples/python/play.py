"""Play a URI and print events until interrupted. Requires a local playback engine."""
import asyncio
import os
from pathlib import Path
import sys

from spotcore import Session, SessionOptions


async def main():
    cookie = Path(os.environ["SPOTCORE_COOKIE_FILE"]).read_text().strip()
    spotify = Session()
    try:
        info = await spotify.login(cookie, SessionOptions(
            app_id="spotcore-python-example", device_name="SpotCore Python",
            playback=True, data_dir=None,
        ), os.environ.get("SOLOIST_API_KEY"))
        print(f"Engine: {info.engine}")
        if info.fallback_reason:
            print(f"Native fallback: {info.fallback_reason}")
        await spotify.play(sys.argv[1])
        while True:
            event = await spotify.next_event()
            print(event.kind, event.state, event.item.name if event.item else "")
    finally:
        await spotify.shutdown()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
