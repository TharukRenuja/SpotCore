#!/usr/bin/env python3
"""Check repository-relative links without fetching websites or building the SDK."""
import re
import subprocess
from pathlib import Path
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r'\]\(([^)]+)\)|href="([^"]+)"')


def missing_links(path, text):
    for match in LINK.finditer(text):
        target = (match[1] or match[2]).strip().strip('<>')
        url = urlsplit(target)
        if url.scheme or url.netloc or not url.path:
            continue
        if not (path.parent / unquote(url.path)).exists():
            yield f'{path.relative_to(ROOT)}:{text.count(chr(10), 0, match.start()) + 1}: {target}'


def main():
    tracked = subprocess.check_output(['git', 'ls-files', '-z', '--', '*.md'], cwd=ROOT).decode().split('\0')
    errors = []
    for name in filter(None, tracked):
        path = ROOT / name
        if path.exists():
            errors.extend(missing_links(path, path.read_text()))
    if errors:
        raise SystemExit('\n'.join(errors))
    print('Repository-relative documentation links passed (external URLs and anchors not checked)')


if __name__ == '__main__':
    main()
