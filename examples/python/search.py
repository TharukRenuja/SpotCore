"""Read-only HTTP search. Install the local wheel before running."""
import asyncio
import os
from pathlib import Path

from spotcore import Session, SessionOptions


async def main():
    cookie = Path(os.environ["SPOTCORE_COOKIE_FILE"]).read_text().strip()
    spotify = Session()
    try:
        info = await spotify.login(cookie, SessionOptions(
            app_id="spotcore-python-example", device_name="SpotCore Python",
            playback=False, data_dir=None,
        ), None)
        print(f"{info.account.name}: {info.account.tier} ({info.engine})")
        results = await spotify.search_tracks("Miles Davis", 10, 0)
        for track in results.items:
            print(f"{track.name} — {track.uri}")
    finally:
        await spotify.shutdown()


if __name__ == "__main__":
    asyncio.run(main())
