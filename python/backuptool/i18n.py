# SPDX-License-Identifier: GPL-3.0-or-later
"""Lightweight i18n: loads JSON catalogs from the ``lang/`` directory.

Adding a language only requires dropping a ``<code>.json`` file into ``lang/``
(e.g. ``fr.json``); it is picked up automatically – no code change needed.
"""
from __future__ import annotations

import json
import os

_LANG_DIR = os.path.join(os.path.dirname(__file__), "lang")
_cache: dict[str, dict] = {}


def available() -> list[str]:
    """Return the list of available language codes (e.g. ['de', 'en', 'fa'])."""
    langs = []
    try:
        for f in os.listdir(_LANG_DIR):
            if f.endswith(".json"):
                langs.append(f[:-5])
    except OSError:
        pass
    return sorted(langs) or ["en"]


def load(lang: str) -> dict:
    if lang not in _cache:
        path = os.path.join(_LANG_DIR, f"{lang}.json")
        try:
            with open(path, encoding="utf-8") as f:
                _cache[lang] = json.load(f)
        except (OSError, ValueError):
            _cache[lang] = {}
    return _cache[lang]


class Translator:
    """Resolves keys for the active language, falling back to English then key."""

    def __init__(self, lang: str = "en"):
        self.en = load("en")
        self.set_language(lang)

    def set_language(self, lang: str) -> None:
        self.lang = lang
        self.d = load(lang)

    def is_rtl(self) -> bool:
        return bool(self.d.get("rtl"))

    def name_of(self, lang: str) -> str:
        return load(lang).get("language_name", lang)

    def __call__(self, key: str) -> str:
        return self.d.get(key) or self.en.get(key) or key
