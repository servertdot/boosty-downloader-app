"""Compatibility launcher for Boosty Downloader used by the desktop app."""

from __future__ import annotations

import contextvars
import json
import re
import sys
import time
import uuid
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from typing import Any
from urllib.parse import parse_qs, urlencode, urlparse, urlunparse

from yt_dlp.utils import smuggle_url

from boosty_downloader.application.use_cases.download_single_post import (
    DownloadSinglePostUseCase,
)
from boosty_downloader.infrastructure.external_videos_downloader.external_videos_downloader import (
    ExternalVideosDownloader,
)


PROGRESS_PREFIX = "__BOOSTY_PROGRESS__"
_MARKUP_RE = re.compile(r"\[/?[^\]]+\]")
_SIZE_RE = re.compile(r"\[([^/\]]+)\s*/\s*([^\]]+)\]")

_boosty_referer: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "boosty_referer", default=None
)
_original_download_external_video = DownloadSinglePostUseCase.download_external_videos
_original_download_video = ExternalVideosDownloader.download_video


def _vimeo_player_url(url: str) -> str | None:
    parsed = urlparse(url)
    if (parsed.hostname or "").lower() not in {
        "vimeo.com",
        "www.vimeo.com",
        "player.vimeo.com",
    }:
        return None

    parts = [part for part in parsed.path.split("/") if part]
    video_index = next(
        (index for index, part in enumerate(parts) if re.fullmatch(r"\d+", part)),
        None,
    )
    if video_index is None:
        return None

    video_id = parts[video_index]
    query = parse_qs(parsed.query, keep_blank_values=True)
    if "h" not in query and len(parts) > video_index + 1:
        possible_hash = parts[video_index + 1]
        if re.fullmatch(r"[a-zA-Z0-9]+", possible_hash):
            query["h"] = [possible_hash]

    return urlunparse(
        ("https", "player.vimeo.com", f"/video/{video_id}", "", urlencode(query, doseq=True), "")
    )


async def _download_external_video_with_post_context(
    self: DownloadSinglePostUseCase, external_video: object
):
    post_url = (
        f"https://boosty.to/{self.context.author_name}/posts/{self.post_dto.id}"
    )
    token = _boosty_referer.set(post_url)
    try:
        return await _original_download_external_video(self, external_video)
    finally:
        _boosty_referer.reset(token)


def _download_video_with_vimeo_embed(
    self: ExternalVideosDownloader,
    url: str,
    destination_directory,
    progress_hook=None,
):
    player_url = _vimeo_player_url(url)
    referer = _boosty_referer.get()
    if player_url and referer:
        url = smuggle_url(player_url, {"referer": referer})
    return _original_download_video(
        self,
        url=url,
        destination_directory=destination_directory,
        progress_hook=progress_hook,
    )


DownloadSinglePostUseCase.download_external_videos = (
    _download_external_video_with_post_context
)
ExternalVideosDownloader.download_video = _download_video_with_vimeo_embed


def _strip_markup(value: str) -> str:
    return _MARKUP_RE.sub("", value).strip()


class DesktopProgressReporter:
    """Plain-text progress reporter for the desktop app (no Rich in-place bars)."""

    def __init__(
        self,
        console: Any | None = None,
        logger: Any | None = None,
    ) -> None:
        self.console = console
        self._logger = logger
        self._tasks: dict[uuid.UUID, dict[str, Any]] = {}
        self._last_emit_at = 0.0
        self._last_signature: tuple[Any, ...] | None = None

    def start(self) -> None:
        return None

    def stop(self) -> None:
        self._tasks.clear()
        self._emit({"label": "", "percent": None, "detail": "", "active": False}, force=True)

    def create_task(
        self, name: str, total: int | None = None, indent_level: int = 0
    ) -> uuid.UUID:
        task_uuid = uuid.uuid4()
        self._tasks[task_uuid] = {
            "name": name,
            "description": name,
            "total": float(total) if total is not None else None,
            "completed": 0.0,
            "level": indent_level,
            "updated_at": time.monotonic(),
        }
        self._emit_current(force=True)
        return task_uuid

    def update_task(
        self,
        task_uuid: uuid.UUID,
        advance: int = 1,
        total: int | None = None,
        description: str | None = None,
    ) -> None:
        task = self._tasks.get(task_uuid)
        if task is None:
            return
        if total is not None:
            task["total"] = float(total)
        task["completed"] = float(task["completed"]) + float(advance)
        if description is not None:
            task["description"] = description
        task["updated_at"] = time.monotonic()
        self._emit_current()

    def complete_task(self, task_uuid: uuid.UUID) -> None:
        task = self._tasks.pop(task_uuid, None)
        if task is not None and task["total"] is not None:
            task["completed"] = task["total"]
        self._emit_current(force=True)

    def info(self, message: str) -> None:
        self._log("info", message)

    def success(self, message: str) -> None:
        self._log("info", f"✔ {message}")

    def warn(self, message: str) -> None:
        self._log("warning", f"⚠ {message}")

    def error(self, message: str) -> None:
        self._log("error", f"✖ {message}")

    def notice(self, message: str) -> None:
        print(f"NOTICE: {_strip_markup(message)}", flush=True)

    def _log(self, level: str, message: str) -> None:
        cleaned = _strip_markup(message)
        if self._logger is not None:
            getattr(self._logger, level)(cleaned)
        else:
            print(cleaned, flush=True)

    def _emit_current(self, *, force: bool = False) -> None:
        if not self._tasks:
            self._emit(
                {"label": "", "percent": None, "detail": "", "active": False},
                force=force,
            )
            return

        task = max(
            self._tasks.values(),
            key=lambda item: (item["level"], item["updated_at"]),
        )
        raw_description = str(task.get("description") or task["name"])
        cleaned = _strip_markup(raw_description)
        detail = ""
        label = cleaned
        size_match = _SIZE_RE.search(cleaned)
        if size_match:
            detail = f"{size_match.group(1).strip()} / {size_match.group(2).strip()}"
            label = _SIZE_RE.sub("", cleaned).strip(" :-")
        elif task["total"] is not None:
            detail = f"{int(task['completed'])} / {int(task['total'])}"

        percent = None
        if task["total"] is not None and task["total"] > 0:
            percent = max(0.0, min(100.0, (task["completed"] / task["total"]) * 100.0))

        self._emit(
            {
                "label": label or "Загрузка",
                "percent": None if percent is None else round(percent, 1),
                "detail": detail,
                "active": True,
            },
            force=force,
        )

    def _emit(self, payload: dict[str, Any], *, force: bool = False) -> None:
        signature = (
            payload.get("active"),
            payload.get("label"),
            payload.get("percent"),
            payload.get("detail"),
        )
        now = time.monotonic()
        if (
            not force
            and signature == self._last_signature
            and now - self._last_emit_at < 0.2
        ):
            return
        if (
            not force
            and self._last_signature is not None
            and signature[0] is True
            and self._last_signature[0] is True
            and signature[1] == self._last_signature[1]
            and signature[3] == self._last_signature[3]
            and signature[2] is not None
            and self._last_signature[2] is not None
            and abs(float(signature[2]) - float(self._last_signature[2])) < 0.5
            and now - self._last_emit_at < 0.2
        ):
            return

        print(f"{PROGRESS_PREFIX}{json.dumps(payload, ensure_ascii=False)}", flush=True)
        self._last_emit_at = now
        self._last_signature = signature


@asynccontextmanager
async def _use_desktop_reporter(
    reporter: DesktopProgressReporter,
) -> AsyncGenerator[DesktopProgressReporter, None]:
    try:
        reporter.start()
        yield reporter
    finally:
        reporter.stop()


def _install_desktop_progress_reporter() -> None:
    import boosty_downloader.cli.console_progress_reporter as progress_module

    progress_module.ProgressReporter = DesktopProgressReporter  # type: ignore[misc, assignment]
    progress_module.use_reporter = _use_desktop_reporter  # type: ignore[assignment]

    try:
        import boosty_downloader.application.di.app_environment as app_environment

        app_environment.ProgressReporter = DesktopProgressReporter  # type: ignore[attr-defined]
        app_environment.use_reporter = _use_desktop_reporter  # type: ignore[attr-defined]
    except Exception:
        # Module may not be imported yet; console_progress_reporter patch is enough.
        pass


if __name__ == "__main__":
    _install_desktop_progress_reporter()
    from boosty_downloader.main import entry_point

    entry_point()
